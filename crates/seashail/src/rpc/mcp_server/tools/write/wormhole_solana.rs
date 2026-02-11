// VAA redemption flow (redeem_transfer_vaa_to_solana).

use alloy::network::TransactionBuilder as _;
use alloy::primitives::{keccak256, U256};
use alloy::rpc::types::TransactionRequest;
use alloy::sol;
use alloy::sol_types::SolCall as _;
use base64::Engine as _;
use borsh::{BorshDeserialize, BorshSerialize};
use eyre::{Context as _, ContextCompat as _};
use serde_json::{json, Value};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer as _,
};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use tokio::io::BufReader;
use tokio::time::{sleep, Duration};

use crate::{
    amount,
    chains::{evm::EvmChain, solana::SolanaChain},
    config::NetworkMode,
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, resolve_wallet_and_account, sol_pubkey_for_account, solana_fallback_urls,
};
use super::super::key_loading::{load_evm_signer, load_solana_keypair};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::{parse_usd_value, summarize_sim_error};

sol! {
    contract IWormholeTokenBridge {
        function completeTransfer(bytes memory encodedVm) external;
    }
}

fn default_token_bridge_for_chain(chain: &str) -> Option<&'static str> {
    // Keep consistent with the EVM Wormhole handler defaults.
    match chain.trim() {
        // Mainnets
        "ethereum" => Some("0x3ee18B2214AFF97000D974cf647E7C347E8fa585"),
        "arbitrum" => Some("0x0b2402144Bb366A632D14B83F244D2e0e21bD39c"),
        "optimism" => Some("0x1D68124e65faFC907325e3EDbF8c4d84499DAa8b"),
        "polygon" => Some("0x5a58505a96D1dbf8dF91cB21B54419FC36e93fdE"),
        "base" => Some("0x8d2de8d2f73F1dfe8B72d0d8E9FfFBCf7AaC8AEf"),
        "bnb" => Some("0xB6F6D86a8f9879A9c87f643768d9efc38c1Da6E7"),
        "avalanche" => Some("0x0e082F06FF657D94310cB8cE8B0D9a04541d8052"),

        // Testnets
        "sepolia" => Some("0xDB5492265f6038831E89f495670FF909aDe94bd9"),
        "arbitrum-sepolia" | "polygon-amoy" => Some("0xC7A204bDBFe983FCD8d8E61D02b475D4073fF97e"),
        "optimism-sepolia" => Some("0x99737Ec4B815d816c49A385943baf0380e75c0Ac"),
        "base-sepolia" => Some("0x86F55A04690fdE37C5C5F6D0cA379B2eD2f334f9"),
        "bnb-testnet" => Some("0x9dcF9D205C9De35334D646BeE44b2D2859712A09"),
        "avalanche-fuji" => Some("0x61E44E506Ca5659E6c0bba9b678586fA2d729756"),
        _ => None,
    }
}

// Mainnet program IDs (Wormhole core + token bridge on Solana).
const SOL_CORE_BRIDGE_MAINNET: &str = "worm2ZoG2kUd4vFXhvjh93UUH596ayRfgQ2MgjNMTth";
const SOL_TOKEN_BRIDGE_MAINNET: &str = "wormDTUJ6AWPNvk59vGQbDvGJmqbDTdgWgAqcLBCgUb";

// Devnet program IDs (Wormhole core + token bridge on Solana).
// Note: Seashail's `NetworkMode::Testnet` maps to Solana devnet.
const SOL_CORE_BRIDGE_DEVNET: &str = "Bridge1p5gheXUvJ6jGWGeCsgPKgnE3YgdGKRVCMY9o";
const SOL_TOKEN_BRIDGE_DEVNET: &str = "B6RHG3mfcckmrYN1UhmJzyS1XX3fZKbkeUcpJe9Sy3FE";

const USDC_MINT_MAINNET: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn wormhole_chain_id(chain: &str) -> Option<u16> {
    // Match the mapping used by the EVM Wormhole handler.
    match chain.trim() {
        "solana" => Some(1),
        "ethereum" => Some(2),
        "bnb" => Some(4),
        "polygon" => Some(5),
        "avalanche" => Some(6),
        "arbitrum" => Some(23),
        "optimism" => Some(24),
        "base" => Some(30),

        // Testnets
        "sepolia" => Some(10002),
        "arbitrum-sepolia" => Some(10003),
        "base-sepolia" => Some(10004),
        "optimism-sepolia" => Some(10005),
        "avalanche-fuji" => Some(10006),
        "polygon-amoy" => Some(10007),
        "bnb-testnet" => Some(10008),
        _ => None,
    }
}

fn sol_wormhole_program_ids(mode: NetworkMode) -> eyre::Result<(Pubkey, Pubkey)> {
    let (core, token) = match mode {
        NetworkMode::Mainnet => (SOL_CORE_BRIDGE_MAINNET, SOL_TOKEN_BRIDGE_MAINNET),
        NetworkMode::Testnet => (SOL_CORE_BRIDGE_DEVNET, SOL_TOKEN_BRIDGE_DEVNET),
    };
    Ok((
        SolanaChain::parse_pubkey(core).context("parse solana core bridge program id")?,
        SolanaChain::parse_pubkey(token).context("parse solana token bridge program id")?,
    ))
}

fn pda(program_id: &Pubkey, seeds: &[&[u8]]) -> Pubkey {
    let (pk, _) = Pubkey::find_program_address(seeds, program_id);
    pk
}

fn core_bridge_guardian_set(core_bridge: &Pubkey, index: u32) -> Pubkey {
    pda(core_bridge, &[b"GuardianSet", &index.to_be_bytes()])
}

fn core_bridge_bridge_state(core_bridge: &Pubkey) -> Pubkey {
    pda(core_bridge, &[b"Bridge"])
}

fn core_bridge_fee_collector(core_bridge: &Pubkey) -> Pubkey {
    pda(core_bridge, &[b"fee_collector"])
}

fn core_bridge_sequence(core_bridge: &Pubkey, emitter: &Pubkey) -> Pubkey {
    pda(core_bridge, &[b"Sequence", emitter.as_ref()])
}

fn core_bridge_claim(
    core_bridge: &Pubkey,
    emitter_chain: u16,
    emitter_address: [u8; 32],
    sequence: u64,
) -> Pubkey {
    pda(
        core_bridge,
        &[
            &emitter_address,
            &emitter_chain.to_be_bytes(),
            &sequence.to_be_bytes(),
        ],
    )
}

fn core_bridge_posted_vaa(core_bridge: &Pubkey, body_hash: [u8; 32]) -> Pubkey {
    pda(core_bridge, &[b"PostedVAA", &body_hash])
}

fn token_bridge_config(token_bridge: &Pubkey) -> Pubkey {
    pda(token_bridge, &[b"config"])
}

fn token_bridge_emitter(token_bridge: &Pubkey) -> Pubkey {
    pda(token_bridge, &[b"emitter"])
}

fn token_bridge_authority_signer(token_bridge: &Pubkey) -> Pubkey {
    pda(token_bridge, &[b"authority_signer"])
}

fn token_bridge_custody_signer(token_bridge: &Pubkey) -> Pubkey {
    pda(token_bridge, &[b"custody_signer"])
}

fn token_bridge_mint_signer(token_bridge: &Pubkey) -> Pubkey {
    pda(token_bridge, &[b"mint_signer"])
}

fn token_bridge_endpoint(
    token_bridge: &Pubkey,
    emitter_chain: u16,
    emitter_address: [u8; 32],
) -> Pubkey {
    pda(
        token_bridge,
        &[&emitter_chain.to_be_bytes(), &emitter_address],
    )
}

fn token_bridge_wrapped_mint(
    token_bridge: &Pubkey,
    token_chain: u16,
    token_address: [u8; 32],
) -> Pubkey {
    pda(
        token_bridge,
        &[b"wrapped", &token_chain.to_be_bytes(), &token_address],
    )
}

fn token_bridge_wrapped_meta(token_bridge: &Pubkey, mint: &Pubkey) -> Pubkey {
    pda(token_bridge, &[b"meta", mint.as_ref()])
}

#[derive(Debug, Clone)]
struct VaaSig {
    guardian_index: u8,
    r: [u8; 32],
    s: [u8; 32],
    v: u8,
}

#[derive(Debug, Clone)]
struct ParsedVaa {
    version: u8,
    guardian_set_index: u32,
    signatures: Vec<VaaSig>,
    timestamp: u32,
    nonce: u32,
    emitter_chain: u16,
    emitter_address: [u8; 32],
    sequence: u64,
    consistency_level: u8,
    payload: Vec<u8>,
}

fn read_u16_be(b: &[u8], i: &mut usize) -> eyre::Result<u16> {
    let v: [u8; 2] = b
        .get(*i..(*i + 2))
        .ok_or_else(|| eyre::eyre!("unexpected eof"))?
        .try_into()
        .context("u16 slice conversion")?;
    *i += 2;
    Ok(u16::from_be_bytes(v))
}

fn read_u32_be(b: &[u8], i: &mut usize) -> eyre::Result<u32> {
    let v: [u8; 4] = b
        .get(*i..(*i + 4))
        .ok_or_else(|| eyre::eyre!("unexpected eof"))?
        .try_into()
        .context("u32 slice conversion")?;
    *i += 4;
    Ok(u32::from_be_bytes(v))
}

fn read_u64_be(b: &[u8], i: &mut usize) -> eyre::Result<u64> {
    let v: [u8; 8] = b
        .get(*i..(*i + 8))
        .ok_or_else(|| eyre::eyre!("unexpected eof"))?
        .try_into()
        .context("u64 slice conversion")?;
    *i += 8;
    Ok(u64::from_be_bytes(v))
}

fn parse_vaa(vaa: &[u8]) -> eyre::Result<ParsedVaa> {
    let mut i = 0_usize;
    let version = *vaa.get(i).ok_or_else(|| eyre::eyre!("empty vaa"))?;
    i += 1;
    let guardian_set_index = read_u32_be(vaa, &mut i).context("guardian_set_index")?;
    let sig_count = *vaa.get(i).ok_or_else(|| eyre::eyre!("missing sig_count"))? as usize;
    i += 1;

    let mut signatures = Vec::with_capacity(sig_count);
    for _ in 0..sig_count {
        let guardian_index = *vaa.get(i).ok_or_else(|| eyre::eyre!("sig eof (index)"))?;
        i += 1;
        let r: [u8; 32] = vaa
            .get(i..i + 32)
            .ok_or_else(|| eyre::eyre!("sig eof (r)"))?
            .try_into()
            .context("sig r conversion")?;
        i += 32;
        let s: [u8; 32] = vaa
            .get(i..i + 32)
            .ok_or_else(|| eyre::eyre!("sig eof (s)"))?
            .try_into()
            .context("sig s conversion")?;
        i += 32;
        let v = *vaa.get(i).ok_or_else(|| eyre::eyre!("sig eof (v)"))?;
        i += 1;
        signatures.push(VaaSig {
            guardian_index,
            r,
            s,
            v,
        });
    }

    let timestamp = read_u32_be(vaa, &mut i).context("timestamp")?;
    let nonce = read_u32_be(vaa, &mut i).context("nonce")?;
    let emitter_chain = read_u16_be(vaa, &mut i).context("emitter_chain")?;
    let emitter_address: [u8; 32] = vaa
        .get(i..i + 32)
        .ok_or_else(|| eyre::eyre!("emitter_address eof"))?
        .try_into()
        .context("emitter_address conversion")?;
    i += 32;
    let sequence = read_u64_be(vaa, &mut i).context("sequence")?;
    let consistency_level = *vaa.get(i).ok_or_else(|| eyre::eyre!("consistency eof"))?;
    i += 1;
    let payload = vaa.get(i..).unwrap_or_default().to_vec();

    Ok(ParsedVaa {
        version,
        guardian_set_index,
        signatures,
        timestamp,
        nonce,
        emitter_chain,
        emitter_address,
        sequence,
        consistency_level,
        payload,
    })
}

#[derive(Default, BorshSerialize, BorshDeserialize, Clone)]
struct CorePostVaaData {
    pub version: u8,
    pub guardian_set_index: u32,
    pub timestamp: u32,
    pub nonce: u32,
    pub emitter_chain: u16,
    pub emitter_address: [u8; 32],
    pub sequence: u64,
    pub consistency_level: u8,
    pub payload: Vec<u8>,
}

#[derive(Default, BorshSerialize, BorshDeserialize)]
struct CoreVerifySignaturesData {
    // wormhole core constant MAX_LEN_GUARDIAN_KEYS == 19
    pub signers: [i8; 19],
}

fn core_body_hash(vaa: &CorePostVaaData) -> [u8; 32] {
    // Matches Wormhole core bridge `hash_vaa` / `check_integrity`:
    // keccak256(big-endian serialized body fields).
    let mut buf = Vec::with_capacity(4 + 4 + 2 + 32 + 8 + 1 + vaa.payload.len());
    buf.extend_from_slice(&vaa.timestamp.to_be_bytes());
    buf.extend_from_slice(&vaa.nonce.to_be_bytes());
    buf.extend_from_slice(&vaa.emitter_chain.to_be_bytes());
    buf.extend_from_slice(&vaa.emitter_address);
    buf.extend_from_slice(&vaa.sequence.to_be_bytes());
    buf.push(vaa.consistency_level);
    buf.extend_from_slice(&vaa.payload);
    keccak256(buf).0
}

#[derive(Default, BorshSerialize, BorshDeserialize)]
struct TokenBridgeTransferNativeData {
    pub nonce: u32,
    pub amount: u64,
    pub fee: u64,
    pub target_address: [u8; 32],
    pub target_chain: u16,
}

#[derive(Default, BorshSerialize, BorshDeserialize)]
struct TokenBridgeTransferWrappedData {
    pub nonce: u32,
    pub amount: u64,
    pub fee: u64,
    pub target_address: [u8; 32],
    pub target_chain: u16,
}

#[derive(Default, BorshSerialize, BorshDeserialize)]
struct TokenBridgeEmpty;

#[derive(Default, BorshSerialize, BorshDeserialize, Clone)]
struct TokenBridgeWrappedMeta {
    pub chain: u16,
    pub token_address: [u8; 32],
    pub original_decimals: u8,
}

#[repr(u8)]
enum CoreIx {
    PostVaa = 2,
    VerifySignatures = 7,
}

#[repr(u8)]
enum TokenBridgeIx {
    CompleteWrapped = 3,
    TransferWrapped = 4,
    TransferNative = 5,
}

fn borsh_ix<T: BorshSerialize>(tag: u8, data: &T) -> eyre::Result<Vec<u8>> {
    // Borsh tuple encoding: enum discriminant (u8) + data.
    let mut out = Vec::with_capacity(1);
    out.push(tag);
    out.extend_from_slice(&borsh::to_vec(data)?);
    Ok(out)
}

#[derive(Default, BorshSerialize, BorshDeserialize)]
struct GuardianSetData {
    pub index: u32,
    pub keys: Vec<[u8; 20]>,
    pub creation_time: u32,
    pub expiration_time: u32,
}

#[derive(Default, BorshSerialize, BorshDeserialize)]
struct CoreMessageData {
    pub vaa_version: u8,
    pub consistency_level: u8,
    pub vaa_time: u32,
    pub vaa_signature_account: Pubkey,
    pub submission_time: u32,
    pub nonce: u32,
    pub sequence: u64,
    pub emitter_chain: u16,
    pub emitter_address: [u8; 32],
    pub payload: Vec<u8>,
}

fn parse_posted_message_sequence(acc_data: &[u8]) -> eyre::Result<CoreMessageData> {
    let magic = acc_data
        .get(0..3)
        .ok_or_else(|| eyre::eyre!("posted message too short"))?;
    if magic != b"msg" && magic != b"msu" {
        eyre::bail!("posted message magic mismatch");
    }
    let mut rest = acc_data
        .get(3..)
        .ok_or_else(|| eyre::eyre!("posted message too short for body"))?;
    let msg = CoreMessageData::deserialize(&mut rest).context("borsh decode message data")?;
    Ok(msg)
}

// Secp256k1 instruction packing. The runtime secp program keccak-hashes the `message` bytes.
// Wormhole expects `message` to be the 32-byte VAA body hash (keccak(body)).
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct SecpSignatureOffsets {
    pub signature_offset: u16,
    pub signature_instruction_index: u8,
    pub eth_address_offset: u16,
    pub eth_address_instruction_index: u8,
    pub message_data_offset: u16,
    pub message_data_size: u16,
    pub message_instruction_index: u8,
}

const SECP_OFFSETS_SIZE: usize = 11;
const SECP_SIG_SIZE: usize = 64;
const SECP_ADDR_SIZE: usize = 20;

fn build_secp256k1_ix(
    message_32: [u8; 32],
    sigs: &[(u8, [u8; 20], [u8; 64], u8)],
) -> eyre::Result<Instruction> {
    if sigs.len() > u8::MAX as usize {
        eyre::bail!("too many signatures");
    }

    let data_start = 1_usize
        .checked_add(
            sigs.len()
                .checked_mul(SECP_OFFSETS_SIZE)
                .context("overflow")?,
        )
        .context("overflow")?;

    let mut offsets = Vec::with_capacity(sigs.len());
    let mut buf: Vec<u8> = Vec::new();
    for (_guardian_idx, eth_addr, sig64, recid) in sigs {
        let start = data_start.checked_add(buf.len()).context("overflow")?;
        let sig_off = start;
        let addr_off = sig_off.checked_add(SECP_SIG_SIZE + 1).context("overflow")?;
        let msg_off = addr_off.checked_add(SECP_ADDR_SIZE).context("overflow")?;

        offsets.push(SecpSignatureOffsets {
            signature_offset: u16::try_from(sig_off).context("sig offset")?,
            signature_instruction_index: 0,
            eth_address_offset: u16::try_from(addr_off).context("addr offset")?,
            eth_address_instruction_index: 0,
            message_data_offset: u16::try_from(msg_off).context("msg offset")?,
            message_data_size: 32,
            message_instruction_index: 0,
        });

        buf.extend_from_slice(sig64);
        buf.push(*recid);
        buf.extend_from_slice(eth_addr);
        buf.extend_from_slice(&message_32);
    }

    let mut out = Vec::with_capacity(data_start + buf.len());
    out.push(u8::try_from(sigs.len()).context("sig count exceeds u8")?);
    for o in offsets {
        let b = bincode::serialize(&o).context("bincode serialize offsets")?;
        if b.len() != SECP_OFFSETS_SIZE {
            eyre::bail!("unexpected secp offsets size");
        }
        out.extend_from_slice(&b);
    }
    out.extend_from_slice(&buf);

    Ok(Instruction {
        program_id: solana_sdk::secp256k1_program::id(),
        accounts: vec![],
        data: out,
    })
}

async fn fetch_signed_vaa_bytes_b64(
    base_url: &str,
    src_chain_id: u16,
    emitter_hex: &str,
    sequence: u64,
) -> eyre::Result<Option<String>> {
    // Keep consistent with the EVM Wormhole handler.
    let base = base_url.trim().trim_end_matches('/');
    let url = format!("{base}/signed_vaa/{src_chain_id}/{emitter_hex}/{sequence}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(2_000))
        .build()
        .context("build http client")?;
    let resp = client
        .get(url)
        .send()
        .await
        .context("wormholescan request")?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        eyre::bail!("wormholescan http {}", resp.status());
    }
    let v: Value = resp.json().await.context("wormholescan json")?;
    let vaa = v
        .get("vaaBytes")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    Ok(vaa)
}

fn core_post_vaa_from_parsed(parsed: &ParsedVaa) -> CorePostVaaData {
    CorePostVaaData {
        version: parsed.version,
        guardian_set_index: parsed.guardian_set_index,
        timestamp: parsed.timestamp,
        nonce: parsed.nonce,
        emitter_chain: parsed.emitter_chain,
        emitter_address: parsed.emitter_address,
        sequence: parsed.sequence,
        consistency_level: parsed.consistency_level,
        payload: parsed.payload.clone(),
    }
}

async fn post_vaa_and_run_token_bridge_ix(
    sol: &SolanaChain,
    payer_kp: &Keypair,
    core_bridge: Pubkey,
    parsed: &ParsedVaa,
    after_post_ixs: Vec<Instruction>,
    token_bridge_ix: Instruction,
) -> eyre::Result<solana_sdk::signature::Signature> {
    // Fetch guardian set keys so we can build a valid secp256k1 instruction.
    let guardian_set_pk = core_bridge_guardian_set(&core_bridge, parsed.guardian_set_index);
    let guardian_acc = sol
        .get_account(&guardian_set_pk)
        .await
        .context("get guardian set account")?;
    let mut gs_bytes: &[u8] = guardian_acc.data.as_slice();
    let gs: GuardianSetData =
        GuardianSetData::deserialize(&mut gs_bytes).context("borsh decode guardian set")?;

    let core_vaa = core_post_vaa_from_parsed(parsed);
    let body_hash = core_body_hash(&core_vaa);

    // Build secp signatures: VAA provides (guardian_index, r, s, v). Map guardian_index -> eth address.
    let mut secp_sigs: Vec<(u8, [u8; 20], [u8; 64], u8)> = Vec::new();
    let mut signers = [-1_i8; 19];
    for (pos, s) in parsed.signatures.iter().enumerate() {
        let gi = s.guardian_index as usize;
        if gi >= gs.keys.len() {
            continue;
        }
        if gi >= signers.len() {
            continue;
        }
        let eth_addr = match gs.keys.get(gi) {
            Some(a) => *a,
            None => continue,
        };
        let mut sig64 = [0_u8; 64];
        sig64[0..32].copy_from_slice(&s.r);
        sig64[32..64].copy_from_slice(&s.s);
        let sig_idx = u8::try_from(pos).unwrap_or(u8::MAX);
        if let Some(slot) = signers.get_mut(gi) {
            *slot = i8::try_from(sig_idx).unwrap_or(-1);
        }
        secp_sigs.push((s.guardian_index, eth_addr, sig64, s.v));
    }

    if secp_sigs.is_empty() {
        eyre::bail!("vaa contains no usable signatures");
    }

    let secp_ix = build_secp256k1_ix(body_hash, &secp_sigs).context("build secp256k1 ix")?;

    // Core bridge: verify_signatures (creates signature_set) + post_vaa.
    let signature_set = Keypair::new();
    let verify_data = CoreVerifySignaturesData { signers };
    let verify_ix = Instruction {
        program_id: core_bridge,
        accounts: vec![
            AccountMeta::new(payer_kp.pubkey(), true),
            AccountMeta::new_readonly(guardian_set_pk, false),
            AccountMeta::new(signature_set.pubkey(), true),
            AccountMeta::new_readonly(solana_sdk::sysvar::instructions::id(), false),
            AccountMeta::new_readonly(solana_sdk::sysvar::rent::id(), false),
            AccountMeta::new_readonly(solana_system_interface::program::id(), false),
        ],
        data: borsh_ix(CoreIx::VerifySignatures as u8, &verify_data)
            .context("encode verify_signatures")?,
    };

    let posted_vaa_pk = core_bridge_posted_vaa(&core_bridge, body_hash);
    let bridge_state = core_bridge_bridge_state(&core_bridge);
    let post_ix = Instruction {
        program_id: core_bridge,
        accounts: vec![
            AccountMeta::new_readonly(guardian_set_pk, false),
            AccountMeta::new_readonly(bridge_state, false),
            AccountMeta::new_readonly(signature_set.pubkey(), false),
            AccountMeta::new(posted_vaa_pk, false),
            AccountMeta::new(payer_kp.pubkey(), true),
            AccountMeta::new_readonly(solana_sdk::sysvar::clock::id(), false),
            AccountMeta::new_readonly(solana_sdk::sysvar::rent::id(), false),
            AccountMeta::new_readonly(solana_system_interface::program::id(), false),
        ],
        data: borsh_ix(CoreIx::PostVaa as u8, &core_vaa).context("encode post_vaa")?,
    };

    // Full instruction list: secp must be immediately before verify_signatures (per core program).
    let mut ixs = vec![secp_ix, verify_ix, post_ix];
    ixs.extend(after_post_ixs);
    ixs.push(token_bridge_ix);

    sol.sign_and_send_instructions_multi(payer_kp, &[&signature_set], ixs)
        .await
        .context("sign+send wormhole redeem tx")
}

fn parse_transfer_payload(payload: &[u8]) -> eyre::Result<(u16, [u8; 32], [u8; 32])> {
    // Wormhole token bridge transfer payload id 1:
    // 1 | amount[32] | token_address[32] | token_chain[2] | to[32] | to_chain[2] | fee[32]
    if payload.len() < 1 + 32 + 32 + 2 + 32 + 2 + 32 {
        eyre::bail!("transfer payload too short");
    }
    let pid = payload
        .first()
        .ok_or_else(|| eyre::eyre!("empty transfer payload"))?;
    if *pid != 1 {
        eyre::bail!("unexpected transfer payload id");
    }
    let token_address: [u8; 32] = payload
        .get(33..65)
        .ok_or_else(|| eyre::eyre!("token_address out of bounds"))?
        .try_into()
        .context("token_address conversion")?;
    let token_chain_bytes: [u8; 2] = payload
        .get(65..67)
        .ok_or_else(|| eyre::eyre!("token_chain out of bounds"))?
        .try_into()
        .context("token_chain conversion")?;
    let token_chain = u16::from_be_bytes(token_chain_bytes);
    let to: [u8; 32] = payload
        .get(67..99)
        .ok_or_else(|| eyre::eyre!("to address out of bounds"))?
        .try_into()
        .context("to address conversion")?;
    Ok((token_chain, token_address, to))
}

pub struct RedeemVaaParams<'a> {
    pub wallet: &'a crate::wallet::WalletRecord,
    pub account_index: u32,
    pub recipient_owner: Pubkey,
    pub vaa_bytes: &'a [u8],
}

struct CompleteWrappedIxParams<'a> {
    wallet: &'a crate::wallet::WalletRecord,
    account_index: u32,
    token_bridge: Pubkey,
    core_bridge: Pubkey,
    parsed: &'a ParsedVaa,
    to_acct: Pubkey,
    wrapped_mint: Pubkey,
    token_program: Pubkey,
}

fn build_complete_wrapped_ix(p: &CompleteWrappedIxParams<'_>) -> eyre::Result<Instruction> {
    let config = token_bridge_config(&p.token_bridge);
    let endpoint = token_bridge_endpoint(
        &p.token_bridge,
        p.parsed.emitter_chain,
        p.parsed.emitter_address,
    );
    let claim = core_bridge_claim(
        &p.core_bridge,
        p.parsed.emitter_chain,
        p.parsed.emitter_address,
        p.parsed.sequence,
    );
    let posted = core_bridge_posted_vaa(
        &p.core_bridge,
        core_body_hash(&core_post_vaa_from_parsed(p.parsed)),
    );
    let wrapped_meta = token_bridge_wrapped_meta(&p.token_bridge, &p.wrapped_mint);
    let mint_signer = token_bridge_mint_signer(&p.token_bridge);

    Ok(Instruction {
        program_id: p.token_bridge,
        accounts: vec![
            AccountMeta::new(sol_pubkey_for_account(p.wallet, p.account_index)?, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(posted, false),
            AccountMeta::new(claim, false),
            AccountMeta::new_readonly(endpoint, false),
            AccountMeta::new(p.to_acct, false),
            AccountMeta::new(p.to_acct, false), // to_fees defaults to `to`
            AccountMeta::new(p.wrapped_mint, false),
            AccountMeta::new_readonly(wrapped_meta, false),
            AccountMeta::new_readonly(mint_signer, false),
            AccountMeta::new_readonly(solana_sdk::sysvar::rent::id(), false),
            AccountMeta::new_readonly(solana_system_interface::program::id(), false),
            AccountMeta::new_readonly(p.core_bridge, false),
            AccountMeta::new_readonly(p.token_program, false),
        ],
        data: borsh_ix(TokenBridgeIx::CompleteWrapped as u8, &TokenBridgeEmpty)
            .context("encode complete_wrapped")?,
    })
}

pub async fn redeem_transfer_vaa_to_solana<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    params: RedeemVaaParams<'_>,
) -> eyre::Result<String>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let RedeemVaaParams {
        wallet,
        account_index,
        recipient_owner,
        vaa_bytes,
    } = params;
    let mode = shared.cfg.effective_network_mode();
    let (core_bridge, token_bridge) = sol_wormhole_program_ids(mode)?;
    let sol = SolanaChain::new_with_fallbacks(
        &shared.cfg.rpc.solana_rpc_url,
        solana_fallback_urls(shared, mode),
        &shared.cfg.http.jupiter_base_url,
        shared.cfg.http.jupiter_api_key.as_deref(),
        shared.cfg.rpc.solana_default_compute_unit_limit,
        shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );

    let parsed = parse_vaa(vaa_bytes).context("parse vaa")?;
    let (token_chain, token_address, to_acct_bytes32) =
        parse_transfer_payload(&parsed.payload).context("parse transfer payload")?;
    let to_acct = Pubkey::new_from_array(to_acct_bytes32);
    let wrapped_mint = token_bridge_wrapped_mint(&token_bridge, token_chain, token_address);

    let mint_acc = sol
        .get_account_optional(&wrapped_mint)
        .await
        .context("get wrapped mint (optional)")?;
    let Some(mint_acc) = mint_acc else {
        eyre::bail!(
            "wrapped mint not found on solana; token may not be attested yet (create wrapped mint first)"
        );
    };
    let token_program = mint_acc.owner;

    let expected_ata = get_associated_token_address_with_program_id(
        &recipient_owner,
        &wrapped_mint,
        &token_program,
    );
    let after_post_ixs: Vec<Instruction> = if expected_ata == to_acct {
        vec![
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &sol_pubkey_for_account(wallet, account_index)?,
                &recipient_owner,
                &wrapped_mint,
                &token_program,
            ),
        ]
    } else {
        vec![]
    };

    let complete_ix = build_complete_wrapped_ix(&CompleteWrappedIxParams {
        wallet,
        account_index,
        token_bridge,
        core_bridge,
        parsed: &parsed,
        to_acct,
        wrapped_mint,
        token_program,
    })?;

    let kp = load_solana_keypair(shared, conn, stdin, stdout, wallet, account_index).await?;
    let sig = post_vaa_and_run_token_bridge_ix(
        &sol,
        &kp,
        core_bridge,
        &parsed,
        after_post_ixs,
        complete_ix,
    )
    .await?;

    Ok(sig.to_string())
}

// Validated and parsed arguments for the handle function.
struct HandleArgs<'a> {
    to_chain: &'a str,
    token_mint_s: &'a str,
    amount_s: &'a str,
    units: &'a str,
    dst_wh_chain_id: u16,
    redeem: bool,
}

fn validate_handle_args<'a>(
    req_id: &Value,
    args: &'a Value,
) -> Result<HandleArgs<'a>, Box<JsonRpcResponse>> {
    let chain = arg_str(args, "chain").unwrap_or("");
    if chain != "solana" {
        return Err(Box::new(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "wormhole_solana handler requires chain=solana",
            )),
        )));
    }
    let provider = arg_str(args, "bridge_provider")
        .or_else(|| arg_str(args, "provider"))
        .unwrap_or("wormhole");
    if provider != "wormhole" {
        return Err(Box::new(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "bridge_provider must be wormhole",
            )),
        )));
    }

    let to_chain = arg_str(args, "to_chain").unwrap_or("");
    let token_mint_s = arg_str(args, "token").unwrap_or("");
    let amount_s = arg_str(args, "amount").unwrap_or("");
    let units = arg_str(args, "amount_units").unwrap_or("ui");
    if to_chain.is_empty() || token_mint_s.is_empty() || amount_s.is_empty() {
        return Err(Box::new(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing to_chain/token/amount (provide tx envelope fields to use adapter fallback)",
            )),
        )));
    }

    let Some(dst_wh_chain_id) = wormhole_chain_id(to_chain) else {
        return Err(Box::new(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                format!("unknown wormhole chain id for destination chain: {to_chain}"),
            )),
        )));
    };
    if dst_wh_chain_id == 1 {
        return Err(Box::new(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "solana->solana wormhole bridging is not supported",
            )),
        )));
    }

    let redeem = args.get("redeem").and_then(Value::as_bool).unwrap_or(true);

    Ok(HandleArgs {
        to_chain,
        token_mint_s,
        amount_s,
        units,
        dst_wh_chain_id,
        redeem,
    })
}

fn parse_amount_base(amount_s: &str, units: &str, mint_decimals: u8) -> eyre::Result<u64> {
    if units == "base" {
        amount::parse_amount_base_u128(amount_s)
            .ok()
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| eyre::eyre!("invalid amount (base)"))
    } else {
        let base = amount::parse_amount_ui_to_base_u128(amount_s, u32::from(mint_decimals))
            .map_err(|e| eyre::eyre!("invalid amount: {e:#}"))?;
        u64::try_from(base).map_err(|_e| eyre::eyre!("amount too large"))
    }
}

async fn compute_bridge_usd_value(
    shared: &mut SharedState,
    sol: &SolanaChain,
    token_mint_s: &str,
    amount_base_u64: u64,
    args: &Value,
) -> (f64, bool) {
    let (mut usd_value, mut usd_value_known) = parse_usd_value(args);
    if !usd_value_known {
        shared.ensure_db().await;
        let db = shared.db();
        usd_value = price::solana_token_price_usd_cached(
            sol,
            &shared.cfg,
            token_mint_s,
            USDC_MINT_MAINNET,
            amount_base_u64,
            50,
            db,
        )
        .await
        .unwrap_or(price::TokenPriceUsd {
            usd: f64::NAN,
            source: price::PriceSource::Jupiter,
        })
        .usd;
        usd_value_known = usd_value.is_finite();
    }
    (usd_value, usd_value_known)
}

struct TransferIxParams {
    token_bridge: Pubkey,
    core_bridge: Pubkey,
    payer: Pubkey,
    from_ata: Pubkey,
    msg_pk: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    amount_base_u64: u64,
    recipient_bytes32: [u8; 32],
    dst_wh_chain_id: u16,
    nonce: u32,
}

fn build_wormhole_transfer_ix(
    p: &TransferIxParams,
    wrapped_meta: Option<TokenBridgeWrappedMeta>,
) -> eyre::Result<Instruction> {
    let config = token_bridge_config(&p.token_bridge);
    let authority_signer = token_bridge_authority_signer(&p.token_bridge);
    let emitter = token_bridge_emitter(&p.token_bridge);
    let bridge_state = core_bridge_bridge_state(&p.core_bridge);
    let seq = core_bridge_sequence(&p.core_bridge, &emitter);
    let fee_collector = core_bridge_fee_collector(&p.core_bridge);

    if let Some(meta) = wrapped_meta {
        let wrapped_mint =
            token_bridge_wrapped_mint(&p.token_bridge, meta.chain, meta.token_address);
        let meta_pk = token_bridge_wrapped_meta(&p.token_bridge, &wrapped_mint);
        Ok(Instruction {
            program_id: p.token_bridge,
            accounts: vec![
                AccountMeta::new(p.payer, true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(p.from_ata, false),
                AccountMeta::new_readonly(p.payer, true),
                AccountMeta::new(wrapped_mint, false),
                AccountMeta::new_readonly(meta_pk, false),
                AccountMeta::new_readonly(authority_signer, false),
                AccountMeta::new(bridge_state, false),
                AccountMeta::new(p.msg_pk, true),
                AccountMeta::new_readonly(emitter, false),
                AccountMeta::new(seq, false),
                AccountMeta::new(fee_collector, false),
                AccountMeta::new_readonly(solana_sdk::sysvar::clock::id(), false),
                AccountMeta::new_readonly(solana_sdk::sysvar::rent::id(), false),
                AccountMeta::new_readonly(solana_system_interface::program::id(), false),
                AccountMeta::new_readonly(p.core_bridge, false),
                AccountMeta::new_readonly(p.token_program, false),
            ],
            data: borsh_ix(
                TokenBridgeIx::TransferWrapped as u8,
                &TokenBridgeTransferWrappedData {
                    nonce: p.nonce,
                    amount: p.amount_base_u64,
                    fee: 0,
                    target_address: p.recipient_bytes32,
                    target_chain: p.dst_wh_chain_id,
                },
            )
            .context("encode transfer_wrapped")?,
        })
    } else {
        let custody = pda(&p.token_bridge, &[p.mint.as_ref()]);
        let custody_signer = token_bridge_custody_signer(&p.token_bridge);
        Ok(Instruction {
            program_id: p.token_bridge,
            accounts: vec![
                AccountMeta::new(p.payer, true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(p.from_ata, false),
                AccountMeta::new(p.mint, false),
                AccountMeta::new(custody, false),
                AccountMeta::new_readonly(authority_signer, false),
                AccountMeta::new_readonly(custody_signer, false),
                AccountMeta::new(bridge_state, false),
                AccountMeta::new(p.msg_pk, true),
                AccountMeta::new_readonly(emitter, false),
                AccountMeta::new(seq, false),
                AccountMeta::new(fee_collector, false),
                AccountMeta::new_readonly(solana_sdk::sysvar::clock::id(), false),
                AccountMeta::new_readonly(solana_sdk::sysvar::rent::id(), false),
                AccountMeta::new_readonly(solana_system_interface::program::id(), false),
                AccountMeta::new_readonly(p.core_bridge, false),
                AccountMeta::new_readonly(p.token_program, false),
            ],
            data: borsh_ix(
                TokenBridgeIx::TransferNative as u8,
                &TokenBridgeTransferNativeData {
                    nonce: p.nonce,
                    amount: p.amount_base_u64,
                    fee: 0,
                    target_address: p.recipient_bytes32,
                    target_chain: p.dst_wh_chain_id,
                },
            )
            .context("encode transfer_native")?,
        })
    }
}

struct BridgeHistoryParams<'a> {
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    to_chain: &'a str,
    token_mint_s: &'a str,
    amount_base_u64: u64,
    units: &'a str,
    recipient_evm: &'a alloy::primitives::Address,
    sig: &'a solana_sdk::signature::Signature,
    bridge_id: &'a str,
    usd_value: f64,
    usd_value_known: bool,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

fn record_bridge_history(shared: &SharedState, p: &BridgeHistoryParams<'_>) -> eyre::Result<()> {
    shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "bridge",
      "chain": "solana",
      "wallet": p.w.name,
      "account_index": p.idx,
      "provider": "wormhole",
      "to_chain": p.to_chain,
      "token": p.token_mint_s,
      "amount_base": p.amount_base_u64.to_string(),
      "amount_units": p.units,
      "recipient": format!("{:#x}", p.recipient_evm),
      "txid": p.sig.to_string(),
      "bridge_id": p.bridge_id
    }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(),
      "tool": "bridge_tokens",
      "wallet": p.w.name,
      "account_index": p.idx,
      "chain": "solana",
      "usd_value": p.usd_value,
      "usd_value_known": p.usd_value_known,
      "policy_decision": p.outcome.policy_decision,
      "confirm_required": p.outcome.confirm_required,
      "confirm_result": p.outcome.confirm_result,
      "daily_used_usd": p.outcome.daily_used_usd,
      "forced_confirm": p.outcome.forced_confirm,
      "txid": p.sig.to_string(),
      "error_code": null,
      "result": "broadcasted",
      "provider": "wormhole"
    }));
    Ok(())
}

async fn poll_vaa_b64(
    shared: &SharedState,
    emitter_hex: &str,
    sequence: u64,
    redeem_error: &mut Option<String>,
) -> Option<String> {
    if redeem_error.is_some() {
        return None;
    }
    for _ in 0..60_u32 {
        match fetch_signed_vaa_bytes_b64(
            &shared.cfg.http.wormholescan_api_base_url,
            1,
            emitter_hex,
            sequence,
        )
        .await
        {
            Ok(Some(v)) => return Some(v),
            Ok(None) => {}
            Err(e) => {
                *redeem_error = Some(format!("wormholescan fetch failed: {e:#}"));
                return None;
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    None
}

struct EvmRedeemParams<'a> {
    to_chain: &'a str,
    dst_token_bridge: &'a str,
    vaa_bytes: &'a [u8],
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
}

async fn submit_evm_redeem<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    p: &EvmRedeemParams<'_>,
    redeem_txid: &mut Option<String>,
    redeem_error: &mut Option<String>,
) -> eyre::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let dst_lock = shared.ks.acquire_write_lock()?;
    let dst_rpc_url = shared
        .cfg
        .rpc
        .evm_rpc_urls
        .get(p.to_chain)
        .ok_or_else(|| eyre::eyre!("unknown evm chain: {}", p.to_chain))?
        .clone();
    let dst_chain_id = *shared
        .cfg
        .rpc
        .evm_chain_ids
        .get(p.to_chain)
        .ok_or_else(|| eyre::eyre!("missing evm chain id: {}", p.to_chain))?;
    let mut dst = EvmChain::for_name(p.to_chain, dst_chain_id, &dst_rpc_url, &shared.cfg.http);
    if let Some(fb) = shared.cfg.rpc.evm_fallback_rpc_urls.get(p.to_chain) {
        dst.fallback_rpc_urls.clone_from(fb);
    }

    let from_evm = evm_addr_for_account(p.w, p.idx)?;
    let dst_token_bridge_addr =
        EvmChain::parse_address(p.dst_token_bridge).context("parse dest token bridge")?;
    let call_data = IWormholeTokenBridge::completeTransferCall {
        encodedVm: p.vaa_bytes.to_vec().into(),
    }
    .abi_encode();
    let redeem_tx = TransactionRequest::default()
        .with_from(from_evm)
        .with_to(dst_token_bridge_addr)
        .with_input(call_data)
        .with_value(U256::ZERO);
    if let Err(e) = dst.simulate_tx_strict(&redeem_tx).await {
        *redeem_error = Some(summarize_sim_error(&e, "redeem (wormhole)"));
    } else {
        let dst_signer = load_evm_signer(shared, conn, stdin, stdout, p.w, p.idx).await?;
        match dst.send_tx(dst_signer, redeem_tx).await {
            Ok(h) => *redeem_txid = Some(format!("{h:#x}")),
            Err(e) => *redeem_error = Some(format!("{e:#}")),
        }
    }
    Keystore::release_lock(dst_lock)?;
    Ok(())
}

struct RedeemResult {
    vaa_available: bool,
    redeem_txid: Option<String>,
    redeem_error: Option<String>,
}

struct PollRedeemParams<'a> {
    redeem: bool,
    to_chain: &'a str,
    emitter_hex: &'a str,
    sequence: u64,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
}

async fn poll_and_redeem_evm<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    p: &PollRedeemParams<'_>,
) -> eyre::Result<RedeemResult>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mut vaa_available = false;
    let mut redeem_txid: Option<String> = None;
    let mut redeem_error: Option<String> = None;

    let dst_token_bridge = default_token_bridge_for_chain(p.to_chain)
        .unwrap_or("")
        .to_owned();
    if p.redeem && dst_token_bridge.trim().is_empty() {
        redeem_error = Some("missing destination token bridge address".to_owned());
    }

    let vaa_b64 = poll_vaa_b64(shared, p.emitter_hex, p.sequence, &mut redeem_error).await;

    if let Some(vaa_b64) = vaa_b64 {
        vaa_available = true;
        let vaa_bytes = if let Ok(v) = base64::engine::general_purpose::STANDARD.decode(vaa_b64) {
            v
        } else {
            redeem_error = Some("invalid vaaBytes from wormholescan".to_owned());
            Vec::new()
        };
        if p.redeem && redeem_error.is_none() {
            submit_evm_redeem(
                shared,
                conn,
                stdin,
                stdout,
                &EvmRedeemParams {
                    to_chain: p.to_chain,
                    dst_token_bridge: &dst_token_bridge,
                    vaa_bytes: &vaa_bytes,
                    w: p.w,
                    idx: p.idx,
                },
                &mut redeem_txid,
                &mut redeem_error,
            )
            .await?;
        }
    }

    Ok(RedeemResult {
        vaa_available,
        redeem_txid,
        redeem_error,
    })
}

struct HandlePrepared {
    w: crate::wallet::WalletRecord,
    idx: u32,
    sol: SolanaChain,
    mint: Pubkey,
    token_program: Pubkey,
    amount_base_u64: u64,
    usd_value: f64,
    usd_value_known: bool,
    outcome: super::super::policy_confirm::WriteConfirmOutcome,
    recipient_evm: alloy::primitives::Address,
    core_bridge: Pubkey,
    token_bridge: Pubkey,
    payer_kp: Keypair,
}

fn rpc_err(req_id: &Value, code: &'static str, e: &eyre::Report) -> JsonRpcResponse {
    ok(
        req_id.clone(),
        tool_err(ToolError::new(code, format!("{e:#}"))),
    )
}

async fn prepare_handle<R, W>(
    req_id: &Value,
    args: &Value,
    validated: &HandleArgs<'_>,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> Result<HandlePrepared, JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (w, idx) = resolve_wallet_and_account(shared, args)
        .map_err(|e| rpc_err(req_id, "invalid_request", &e))?;
    let mode = shared.cfg.effective_network_mode();
    let sol = SolanaChain::new_with_fallbacks(
        &shared.cfg.rpc.solana_rpc_url,
        solana_fallback_urls(shared, mode),
        &shared.cfg.http.jupiter_base_url,
        shared.cfg.http.jupiter_api_key.as_deref(),
        shared.cfg.rpc.solana_default_compute_unit_limit,
        shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );
    let mint = SolanaChain::parse_pubkey(validated.token_mint_s)
        .map_err(|e| rpc_err(req_id, "invalid_request", &e))?;
    let mint_acc = sol
        .get_account(&mint)
        .await
        .map_err(|e| rpc_err(req_id, "rpc_error", &e))?;
    let token_program = mint_acc.owner;
    let mint_decimals = sol
        .get_mint_decimals(mint)
        .await
        .map_err(|e| rpc_err(req_id, "rpc_error", &e))?;
    let amount_base_u64 = parse_amount_base(validated.amount_s, validated.units, mint_decimals)
        .map_err(|e| rpc_err(req_id, "invalid_request", &e))?;
    let (usd_value, usd_value_known) =
        compute_bridge_usd_value(shared, &sol, validated.token_mint_s, amount_base_u64, args).await;
    let summary = format!(
        "Wormhole bridge on solana -> {}: mint={} amount={} (units={})",
        validated.to_chain,
        validated.token_mint_s,
        validated.amount_s.trim(),
        validated.units,
    );
    let outcome = maybe_confirm_write(
        shared,
        conn,
        stdin,
        stdout,
        &WriteConfirmRequest {
            tool: "bridge_tokens",
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: WriteOp::Bridge,
            chain: "solana",
            usd_value,
            usd_value_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: None,
            contract: None,
            leverage: None,
            summary: &summary,
        },
    )
    .await
    .map_err(|te| ok(req_id.clone(), tool_err(te)))?;
    let payer_kp = load_solana_keypair(shared, conn, stdin, stdout, &w, idx)
        .await
        .map_err(|e| rpc_err(req_id, "key_error", &e))?;
    let recipient_evm = arg_str(args, "recipient")
        .map(|s| EvmChain::parse_address(s).context("parse recipient evm address"))
        .transpose()
        .map_err(|e| rpc_err(req_id, "invalid_request", &e))?
        .unwrap_or_else(|| evm_addr_for_account(&w, idx).unwrap_or_default());
    let (core_bridge, token_bridge) =
        sol_wormhole_program_ids(mode).map_err(|e| rpc_err(req_id, "invalid_request", &e))?;
    Ok(HandlePrepared {
        w,
        idx,
        sol,
        mint,
        token_program,
        amount_base_u64,
        usd_value,
        usd_value_known,
        outcome,
        recipient_evm,
        core_bridge,
        token_bridge,
        payer_kp,
    })
}

async fn build_and_send_bridge_tx(
    req_id: &Value,
    validated: &HandleArgs<'_>,
    p: &HandlePrepared,
) -> Result<(solana_sdk::signature::Signature, Pubkey), JsonRpcResponse> {
    let wrapped_meta_pk = token_bridge_wrapped_meta(&p.token_bridge, &p.mint);
    let wrapped_meta_acc = p
        .sol
        .get_account_optional(&wrapped_meta_pk)
        .await
        .map_err(|e| rpc_err(req_id, "rpc_error", &e))?;
    let is_wrapped = wrapped_meta_acc
        .as_ref()
        .is_some_and(|a| a.owner == p.token_bridge);
    let wrapped_meta = if let (true, Some(a)) = (is_wrapped, wrapped_meta_acc) {
        let mut d: &[u8] = a.data.as_slice();
        Some(
            TokenBridgeWrappedMeta::deserialize(&mut d)
                .map_err(|e| rpc_err(req_id, "decode_error", &eyre::eyre!(e)))?,
        )
    } else {
        None
    };
    let payer = p.payer_kp.pubkey();
    let from_ata = get_associated_token_address_with_program_id(&payer, &p.mint, &p.token_program);
    if p.sol
        .get_account_optional(&from_ata)
        .await
        .map_err(|e| rpc_err(req_id, "rpc_error", &e))?
        .is_none()
    {
        return Err(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "source token account (ATA) does not exist; receive tokens or create ATA first",
            )),
        ));
    }
    let authority_signer = token_bridge_authority_signer(&p.token_bridge);
    let approve_ix = spl_token::instruction::approve(
        &p.token_program,
        &from_ata,
        &authority_signer,
        &payer,
        &[],
        p.amount_base_u64,
    )
    .map_err(|e| rpc_err(req_id, "ix_error", &eyre::eyre!(e)))?;
    let revoke_ix = spl_token::instruction::revoke(&p.token_program, &from_ata, &payer, &[])
        .map_err(|e| rpc_err(req_id, "ix_error", &eyre::eyre!(e)))?;
    let nonce = crate::db::Db::now_ms()
        .ok()
        .and_then(|ms| u32::try_from((ms / 1000).rem_euclid(i64::from(u32::MAX))).ok())
        .unwrap_or(1_u32);
    let msg_kp = Keypair::new();
    let msg_pk = msg_kp.pubkey();
    let mut recipient_bytes32 = [0_u8; 32];
    recipient_bytes32[12..].copy_from_slice(p.recipient_evm.as_slice());
    let transfer_ix = build_wormhole_transfer_ix(
        &TransferIxParams {
            token_bridge: p.token_bridge,
            core_bridge: p.core_bridge,
            payer,
            from_ata,
            msg_pk,
            mint: p.mint,
            token_program: p.token_program,
            amount_base_u64: p.amount_base_u64,
            recipient_bytes32,
            dst_wh_chain_id: validated.dst_wh_chain_id,
            nonce,
        },
        wrapped_meta,
    )
    .map_err(|e| rpc_err(req_id, "ix_error", &e))?;
    let sig = p
        .sol
        .sign_and_send_instructions_multi(
            &p.payer_kp,
            &[&msg_kp],
            vec![approve_ix, transfer_ix, revoke_ix],
        )
        .await
        .map_err(|e| {
            let msg = summarize_sim_error(&e, "bridge (wormhole solana)");
            ok(
                req_id.clone(),
                tool_err(ToolError::new("simulation_failed", msg)),
            )
        })?;
    Ok((sig, msg_pk))
}

pub async fn handle<R, W>(
    req_id: Value,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let validated = match validate_handle_args(&req_id, &args) {
        Ok(v) => v,
        Err(resp) => return Ok(*resp),
    };

    let lock = shared.ks.acquire_write_lock()?;

    let prepared =
        match prepare_handle(&req_id, &args, &validated, shared, conn, stdin, stdout).await {
            Ok(v) => v,
            Err(resp) => {
                Keystore::release_lock(lock)?;
                return Ok(resp);
            }
        };

    let (sig, msg_pk) = match build_and_send_bridge_tx(&req_id, &validated, &prepared).await {
        Ok(v) => v,
        Err(resp) => {
            Keystore::release_lock(lock)?;
            return Ok(resp);
        }
    };

    let msg_acc = prepared
        .sol
        .get_account(&msg_pk)
        .await
        .context("get posted message")?;
    let msg = parse_posted_message_sequence(&msg_acc.data).context("parse posted message")?;
    let emitter_hex = hex::encode(msg.emitter_address);
    let bridge_id = format!("wormhole:1:{emitter_hex}:{}", msg.sequence);

    record_bridge_history(
        shared,
        &BridgeHistoryParams {
            w: &prepared.w,
            idx: prepared.idx,
            to_chain: validated.to_chain,
            token_mint_s: validated.token_mint_s,
            amount_base_u64: prepared.amount_base_u64,
            units: validated.units,
            recipient_evm: &prepared.recipient_evm,
            sig: &sig,
            bridge_id: &bridge_id,
            usd_value: prepared.usd_value,
            usd_value_known: prepared.usd_value_known,
            outcome: &prepared.outcome,
        },
    )?;

    Keystore::release_lock(lock)?;

    let redeem_result = poll_and_redeem_evm(
        shared,
        conn,
        stdin,
        stdout,
        &PollRedeemParams {
            redeem: validated.redeem,
            to_chain: validated.to_chain,
            emitter_hex: &emitter_hex,
            sequence: msg.sequence,
            w: &prepared.w,
            idx: prepared.idx,
        },
    )
    .await?;

    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": "solana",
          "bridge_provider": "wormhole",
          "to_chain": validated.to_chain,
          "token": validated.token_mint_s,
          "amount_base": prepared.amount_base_u64.to_string(),
          "usd_value": prepared.usd_value,
          "txid": sig.to_string(),
          "bridge_id": bridge_id,
          "wormhole": {
            "source_chain_id": 1_u16,
            "destination_chain_id": validated.dst_wh_chain_id,
            "emitter": emitter_hex,
            "sequence": msg.sequence,
            "vaa_available": redeem_result.vaa_available,
            "redeem_txid": redeem_result.redeem_txid,
            "redeem_error": redeem_result.redeem_error
          }
        })),
    ))
}
