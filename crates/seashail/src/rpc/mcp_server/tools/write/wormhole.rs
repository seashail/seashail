use alloy::primitives::{keccak256, Address, Bytes, B256, U256};
use alloy::rpc::types::TransactionRequest;
use alloy::sol;
use alloy::sol_types::SolType;
use base64::Engine as _;
use eyre::Context as _;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

use crate::{
    amount,
    chains::evm::EvmChain,
    chains::solana::SolanaChain,
    config::NetworkMode,
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::helpers::{
    evm_addr_for_account, resolve_wallet_and_account, sol_pubkey_for_account, solana_fallback_urls,
};
use super::super::key_loading::load_evm_signer;
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::{parse_usd_value, summarize_sim_error};
use super::common::wait_for_allowance;
use super::wormhole_solana;
use super::HandlerCtx;

use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address_with_program_id;

sol! {
    #[sol(rpc)]
    contract IWormholeTokenBridge {
        function transferTokens(address token, uint256 amount, uint16 recipientChain, bytes32 recipient, uint256 arbiterFee, uint32 nonce) external returns (uint64);
        function completeTransfer(bytes encodedVm) external;
    }
}

// Wormhole Solana token bridge program IDs. These are used to derive wrapped mint PDAs and ATAs
// for EVM->Solana bridging defaults. (Seashail `Testnet` maps to Solana devnet.)
const SOL_TOKEN_BRIDGE_MAINNET: &str = "wormDTUJ6AWPNvk59vGQbDvGJmqbDTdgWgAqcLBCgUb";
const SOL_TOKEN_BRIDGE_DEVNET: &str = "B6RHG3mfcckmrYN1UhmJzyS1XX3fZKbkeUcpJe9Sy3FE";

fn sol_token_bridge_program_id(mode: NetworkMode) -> eyre::Result<Pubkey> {
    let s = match mode {
        NetworkMode::Mainnet => SOL_TOKEN_BRIDGE_MAINNET,
        NetworkMode::Testnet => SOL_TOKEN_BRIDGE_DEVNET,
    };
    SolanaChain::parse_pubkey(s).context("parse solana wormhole token bridge program id")
}

fn sol_pda(program_id: &Pubkey, seeds: &[&[u8]]) -> Pubkey {
    let (pk, _) = Pubkey::find_program_address(seeds, program_id);
    pk
}

fn sol_wrapped_mint_pda(
    token_bridge: &Pubkey,
    token_chain: u16,
    token_address: [u8; 32],
) -> Pubkey {
    sol_pda(
        token_bridge,
        &[b"wrapped", &token_chain.to_be_bytes(), &token_address],
    )
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn is_loopback_http(url: &str) -> bool {
    fn host_prefix_ok(s: &str, prefix: &str) -> bool {
        if !s.starts_with(prefix) {
            return false;
        }
        matches!(s.as_bytes().get(prefix.len()), None | Some(b':' | b'/'))
    }
    let u = url.trim();
    host_prefix_ok(u, "http://127.0.0.1")
        || host_prefix_ok(u, "http://localhost")
        || host_prefix_ok(u, "http://[::1]")
}

pub(super) fn default_token_bridge_for_chain(chain: &str) -> Option<&'static str> {
    // Wormhole token bridge addresses (mainnet + common testnets). Source: wormhole docs.
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

fn wormhole_chain_id(chain: &str) -> Option<u16> {
    // Wormhole chain IDs for supported chains.
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
        "sepolia" => Some(10002), // Ethereum Sepolia
        "arbitrum-sepolia" => Some(10003),
        "base-sepolia" => Some(10004),
        "optimism-sepolia" => Some(10005),
        "avalanche-fuji" => Some(10006),
        "polygon-amoy" => Some(10007),
        "bnb-testnet" => Some(10008),
        _ => None,
    }
}

fn parse_amount_base(amount_s: &str, units: &str, decimals: u8) -> Result<U256, ToolError> {
    if amount_s.trim().eq_ignore_ascii_case("max") {
        return Err(ToolError::new(
            "invalid_request",
            "amount=max is not supported for wormhole bridge_tokens",
        ));
    }
    let base_u128 = if units.trim() == "base" {
        amount::parse_amount_base_u128(amount_s)
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))?
    } else {
        amount::parse_amount_ui_to_base_u128(amount_s, u32::from(decimals))
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))?
    };
    Ok(U256::from(base_u128))
}

fn usdc_base_to_usd_f64(base_amount: &U256) -> eyre::Result<f64> {
    let s = base_amount.to_string();
    let s = s.trim();
    if s.is_empty() {
        eyre::bail!("empty usdc base amount");
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        eyre::bail!("invalid usdc base amount");
    }
    if s == "0" {
        return Ok(0.0_f64);
    }
    let dec = if s.len() <= 6 {
        let mut frac = String::with_capacity(6);
        for _ in 0..(6 - s.len()) {
            frac.push('0');
        }
        frac.push_str(s);
        format!("0.{frac}")
    } else {
        let split = s.len() - 6;
        let (whole, frac) = s.split_at(split);
        let frac_trimmed = frac.trim_end_matches('0');
        if frac_trimmed.is_empty() {
            whole.to_owned()
        } else {
            format!("{whole}.{frac_trimmed}")
        }
    };
    dec.parse::<f64>().context("parse usdc amount")
}

fn evm_address_to_bytes32(a: Address) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out[12..].copy_from_slice(a.as_slice());
    out
}

fn bytes32_hex(b: [u8; 32]) -> String {
    hex::encode(b)
}

fn log_message_published_sig() -> B256 {
    keccak256("LogMessagePublished(address,uint64,uint32,bytes,uint8)".as_bytes())
}

fn extract_wormhole_message(
    receipt: &alloy::rpc::types::TransactionReceipt,
) -> Option<(Address, u64)> {
    type LogData = alloy::sol! { tuple(uint64, uint32, bytes, uint8) };
    let sig = log_message_published_sig();
    for l in receipt.inner.logs() {
        let Some(t0) = l.topics().first() else {
            continue;
        };
        if *t0 != sig {
            continue;
        }
        let Some(t1) = l.topics().get(1) else {
            continue;
        };
        let sender_bytes = t1.as_slice().get(12..32)?;
        let sender = Address::from_slice(sender_bytes);
        let decoded = <LogData as SolType>::abi_decode(l.data().data.as_ref()).ok()?;
        let seq: u64 = decoded.0;
        return Some((sender, seq));
    }
    None
}

async fn fetch_signed_vaa_bytes_b64(
    base_url: &str,
    src_chain_id: u16,
    emitter_hex: &str,
    sequence: u64,
) -> eyre::Result<Option<String>> {
    let base = base_url.trim().trim_end_matches('/');
    if !base.starts_with("https://") && !is_loopback_http(base) {
        eyre::bail!("wormholescan_api_base_url must use https (or loopback for local testing)");
    }

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

/// Polls Wormholescan up to 60 times (500ms apart) for the signed VAA. Returns `Some(b64)` on
/// success or `None` if the VAA was not available. Sets `*redeem_error` on fetch failure.
async fn poll_signed_vaa(
    base_url: &str,
    src_chain_id: u16,
    emitter_hex: &str,
    sequence: u64,
    redeem_error: &mut Option<String>,
) -> Option<String> {
    for _ in 0..60_u32 {
        match fetch_signed_vaa_bytes_b64(base_url, src_chain_id, emitter_hex, sequence).await {
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

struct RedeemParams<'a> {
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    vaa_bytes: &'a [u8],
    dest_is_solana: bool,
    recipient_sol_owner: Option<Pubkey>,
    to_chain: &'a str,
    dst_token_bridge_s: &'a str,
    from: Address,
    effective_policy: &'a crate::policy::Policy,
    bridge_id: &'a str,
    usd_value: f64,
}

/// Attempts to redeem the signed VAA on the destination chain (Solana or EVM).
/// Returns `(Option<redeem_txid>, Option<redeem_error>)`.
async fn attempt_redeem<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    rp: &RedeemParams<'_>,
) -> eyre::Result<(Option<String>, Option<String>)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if rp.dest_is_solana {
        return redeem_on_solana(ctx, rp).await;
    }
    redeem_on_evm(ctx, rp).await
}

async fn redeem_on_solana<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    rp: &RedeemParams<'_>,
) -> eyre::Result<(Option<String>, Option<String>)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let dst_lock = ctx.shared.ks.acquire_write_lock()?;
    let result = if let Some(owner) = rp.recipient_sol_owner {
        match wormhole_solana::redeem_transfer_vaa_to_solana(
            ctx.shared,
            ctx.conn,
            ctx.stdin,
            ctx.stdout,
            wormhole_solana::RedeemVaaParams {
                wallet: rp.w,
                account_index: rp.idx,
                recipient_owner: owner,
                vaa_bytes: rp.vaa_bytes,
            },
        )
        .await
        {
            Ok(sig) => {
                ctx.shared.ks.append_tx_history(&json!({
                  "ts": utc_now_iso(),
                  "day": Keystore::current_utc_day_key(),
                  "type": "bridge_redeem",
                  "chain": "solana",
                  "wallet": rp.w.name,
                  "account_index": rp.idx,
                  "provider": "wormhole",
                  "bridge_id": rp.bridge_id,
                  "usd_value": rp.usd_value,
                  "txid": sig
                }))?;
                (Some(sig), None)
            }
            Err(e) => (None, Some(format!("{e:#}"))),
        }
    } else {
        (
            None,
            Some("missing solana recipient owner for auto-redeem".to_owned()),
        )
    };
    Keystore::release_lock(dst_lock)?;
    Ok(result)
}

async fn redeem_on_evm<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    rp: &RedeemParams<'_>,
) -> eyre::Result<(Option<String>, Option<String>)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let dst_lock = ctx.shared.ks.acquire_write_lock()?;
    let dst_rpc_url = ctx
        .shared
        .cfg
        .rpc
        .evm_rpc_urls
        .get(rp.to_chain)
        .ok_or_else(|| eyre::eyre!("unknown evm chain: {}", rp.to_chain))?
        .clone();
    let dst_chain_id = *ctx
        .shared
        .cfg
        .rpc
        .evm_chain_ids
        .get(rp.to_chain)
        .ok_or_else(|| eyre::eyre!("missing evm chain id: {}", rp.to_chain))?;
    let mut dst = EvmChain::for_name(
        rp.to_chain,
        dst_chain_id,
        &dst_rpc_url,
        &ctx.shared.cfg.http,
    );
    if let Some(fb) = ctx.shared.cfg.rpc.evm_fallback_rpc_urls.get(rp.to_chain) {
        dst.fallback_rpc_urls.clone_from(fb);
    }

    let dst_token_bridge_addr =
        EvmChain::parse_address(rp.dst_token_bridge_s).context("parse dest token bridge")?;

    let redeem_error: Option<String> = if ctx
        .shared
        .scam_blocklist_contains_evm(dst_token_bridge_addr)
        .await
    {
        Some("destination token bridge is blocked by scam blocklist".to_owned())
    } else if rp.effective_policy.enable_ofac_sdn.get()
        && ctx
            .shared
            .ofac_sdn_contains_evm(dst_token_bridge_addr)
            .await
    {
        Some("destination token bridge is blocked by OFAC SDN".to_owned())
    } else {
        None
    };

    let result = if redeem_error.is_none() {
        evm_redeem_send(ctx, rp, &dst, dst_token_bridge_addr).await?
    } else {
        (None, redeem_error)
    };
    Keystore::release_lock(dst_lock)?;
    Ok(result)
}

async fn evm_redeem_send<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    rp: &RedeemParams<'_>,
    dst: &EvmChain,
    dst_token_bridge_addr: Address,
) -> eyre::Result<(Option<String>, Option<String>)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let dst_bridge = IWormholeTokenBridge::new(dst_token_bridge_addr, dst.provider()?);
    let redeem_call = dst_bridge.completeTransfer(rp.vaa_bytes.to_vec().into());
    let redeem_call_data = redeem_call.calldata().to_vec();
    let redeem_tx = TransactionRequest {
        from: Some(rp.from),
        to: Some(dst_token_bridge_addr.into()),
        input: Bytes::from(redeem_call_data).into(),
        value: Some(U256::ZERO),
        ..Default::default()
    };

    if let Err(e) = dst.simulate_tx_strict(&redeem_tx).await {
        return Ok((None, Some(summarize_sim_error(&e, "redeem (wormhole)"))));
    }

    let dst_signer =
        load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, rp.w, rp.idx).await?;
    match dst.send_tx(dst_signer, redeem_tx).await {
        Ok(h) => {
            ctx.shared.ks.append_tx_history(&json!({
              "ts": utc_now_iso(),
              "day": Keystore::current_utc_day_key(),
              "type": "bridge_redeem",
              "chain": rp.to_chain,
              "wallet": rp.w.name,
              "account_index": rp.idx,
              "provider": "wormhole",
              "bridge_id": rp.bridge_id,
              "usd_value": rp.usd_value,
              "txid": format!("{h:#x}")
            }))?;
            Ok((Some(format!("{h:#x}")), None))
        }
        Err(e) => Ok((None, Some(format!("{e:#}")))),
    }
}

struct ParsedBridge {
    chain: String,
    to_chain: String,
    token_s: String,
    amount_s: String,
    units: String,
    token_bridge_s: String,
    dst_token_bridge_s: String,
    redeem: bool,
    src_wh_chain_id: u16,
    dst_wh_chain_id: u16,
    w: crate::wallet::WalletRecord,
    idx: u32,
    effective_policy: crate::policy::Policy,
}

fn parse_bridge_args<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
) -> eyre::Result<Result<(ParsedBridge, std::fs::File), JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let chain = arg_str(&ctx.args, "chain").unwrap_or("").to_owned();
    if chain.is_empty() || chain == "solana" || chain == "bitcoin" {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "wormhole bridge_tokens requires an EVM chain",
            )),
        )));
    }
    let provider = arg_str(&ctx.args, "bridge_provider")
        .or_else(|| arg_str(&ctx.args, "provider"))
        .unwrap_or("wormhole");
    if provider != "wormhole" {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "bridge_provider must be wormhole",
            )),
        )));
    }
    let to_chain = arg_str(&ctx.args, "to_chain").unwrap_or("").to_owned();
    let token_s = arg_str(&ctx.args, "token").unwrap_or("").to_owned();
    let amount_s = arg_str(&ctx.args, "amount").unwrap_or("").to_owned();
    let units = arg_str(&ctx.args, "amount_units")
        .unwrap_or("ui")
        .to_owned();
    if to_chain.is_empty() || token_s.is_empty() || amount_s.is_empty() {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing to_chain/token/amount (provide tx envelope fields to use adapter fallback)",
            )),
        )));
    }
    let src_wh_chain_id = wormhole_chain_id(&chain)
        .ok_or_else(|| eyre::eyre!("unknown wormhole chain id for source chain: {chain}"))?;
    let dst_wh_chain_id = wormhole_chain_id(&to_chain).ok_or_else(|| {
        eyre::eyre!("unknown wormhole chain id for destination chain: {to_chain}")
    })?;
    let token_bridge_s = arg_str(&ctx.args, "token_bridge_address")
        .or_else(|| default_token_bridge_for_chain(&chain))
        .unwrap_or("")
        .to_owned();
    if token_bridge_s.trim().is_empty() {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing Wormhole token bridge address for this chain (provide token_bridge_address)",
            )),
        )));
    }
    let redeem = ctx
        .args
        .get("redeem")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let dst_token_bridge_s = arg_str(&ctx.args, "to_token_bridge_address")
        .or_else(|| default_token_bridge_for_chain(&to_chain))
        .unwrap_or("")
        .to_owned();

    let lock = ctx.shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(ctx.shared, &ctx.args)?;
    let (effective_policy, _) = ctx.shared.cfg.policy_for_wallet(Some(w.name.as_str()));

    Ok(Ok((
        ParsedBridge {
            chain,
            to_chain,
            token_s,
            amount_s,
            units,
            token_bridge_s,
            dst_token_bridge_s,
            redeem,
            src_wh_chain_id,
            dst_wh_chain_id,
            w,
            idx,
            effective_policy,
        },
        lock,
    )))
}

struct RecipientInfo {
    bytes32: [u8; 32],
    display: String,
    sol_owner: Option<Pubkey>,
}

async fn resolve_recipient<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pb: &ParsedBridge,
    from: Address,
    token_addr: Address,
) -> eyre::Result<Result<RecipientInfo, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let recipient_s = arg_str(&ctx.args, "recipient").unwrap_or("").to_owned();
    let recipient_ta = arg_str(&ctx.args, "recipient_token_account")
        .unwrap_or("")
        .to_owned();

    if pb.to_chain != "solana" {
        let recipient_addr = if recipient_s.is_empty() {
            from
        } else {
            EvmChain::parse_address(&recipient_s).context("parse recipient")?
        };
        return Ok(Ok(RecipientInfo {
            bytes32: evm_address_to_bytes32(recipient_addr),
            display: format!("{recipient_addr:#x}"),
            sol_owner: None,
        }));
    }

    let mode = ctx.shared.cfg.effective_network_mode();
    let token_bridge_prog = sol_token_bridge_program_id(mode)?;
    let token_addr32 = evm_address_to_bytes32(token_addr);
    let wrapped_mint = sol_wrapped_mint_pda(&token_bridge_prog, pb.src_wh_chain_id, token_addr32);

    let owner = if recipient_s.is_empty() {
        sol_pubkey_for_account(&pb.w, pb.idx).context("resolve default solana recipient owner")?
    } else {
        SolanaChain::parse_pubkey(&recipient_s).context("parse solana recipient owner")?
    };
    if pb.effective_policy.enable_ofac_sdn.get() && ctx.shared.ofac_sdn_contains_solana(owner).await
    {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        )));
    }
    if ctx.shared.scam_blocklist_contains_solana(owner).await {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient is blocked by the scam address blocklist",
            )),
        )));
    }

    let token_account = if recipient_ta.is_empty() {
        let sol = SolanaChain::new_with_fallbacks(
            &ctx.shared.cfg.rpc.solana_rpc_url,
            solana_fallback_urls(ctx.shared, mode),
            &ctx.shared.cfg.http.jupiter_base_url,
            ctx.shared.cfg.http.jupiter_api_key.as_deref(),
            ctx.shared.cfg.rpc.solana_default_compute_unit_limit,
            ctx.shared
                .cfg
                .rpc
                .solana_default_compute_unit_price_micro_lamports,
        );
        let token_program = sol
            .get_account_optional(&wrapped_mint)
            .await
            .ok()
            .flatten()
            .map_or_else(spl_token::id, |a| a.owner);
        get_associated_token_address_with_program_id(&owner, &wrapped_mint, &token_program)
    } else {
        SolanaChain::parse_pubkey(&recipient_ta).context("parse recipient_token_account")?
    };
    Ok(Ok(RecipientInfo {
        bytes32: token_account.to_bytes(),
        display: token_account.to_string(),
        sol_owner: Some(owner),
    }))
}

struct BridgeSendResult {
    txid: B256,
    usd_value: f64,
}

async fn validate_bridge_addr<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pb: &ParsedBridge,
) -> eyre::Result<Result<Address, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let token_bridge_addr =
        EvmChain::parse_address(&pb.token_bridge_s).context("parse token_bridge")?;
    if ctx
        .shared
        .scam_blocklist_contains_evm(token_bridge_addr)
        .await
    {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient is blocked by the scam address blocklist",
            )),
        )));
    }
    if pb.effective_policy.enable_ofac_sdn.get()
        && ctx.shared.ofac_sdn_contains_evm(token_bridge_addr).await
    {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        )));
    }
    Ok(Ok(token_bridge_addr))
}

async fn resolve_usd_and_confirm<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pb: &ParsedBridge,
    evm: &EvmChain,
    token_addr: Address,
    amount_base: U256,
    symbol: &str,
) -> eyre::Result<
    Result<(f64, bool, super::super::policy_confirm::WriteConfirmOutcome), JsonRpcResponse>,
>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (mut usd_value, mut usd_value_known) = parse_usd_value(&ctx.args);
    if !usd_value_known {
        let usd = if evm.uniswap.as_ref().is_some_and(|u| u.usdc == token_addr) {
            usdc_base_to_usd_f64(&amount_base).context("convert usdc amount")?
        } else {
            ctx.shared.ensure_db().await;
            let db = ctx.shared.db();
            price::evm_token_price_usd_cached(evm, &ctx.shared.cfg, token_addr, amount_base, 50, db)
                .await
                .context("price token via uniswap")?
                .usd
        };
        usd_value = usd;
        usd_value_known = usd_value.is_finite();
    }

    let summary = format!(
        "Wormhole bridge on {} -> {}: {symbol} amount={} (units={})",
        pb.chain,
        pb.to_chain,
        pb.amount_s.trim(),
        pb.units
    );
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "bridge_tokens",
            wallet: Some(pb.w.name.as_str()),
            account_index: Some(pb.idx),
            op: WriteOp::Bridge,
            chain: &pb.chain,
            usd_value,
            usd_value_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: Some(&pb.token_bridge_s),
            contract: Some(&pb.token_bridge_s),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => return Ok(Err(ok(ctx.req_id.clone(), tool_err(te)))),
    };

    Ok(Ok((usd_value, usd_value_known, outcome)))
}

async fn approve_and_send_bridge<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pb: &ParsedBridge,
    evm: &EvmChain,
    from: Address,
    token_addr: Address,
    recipient: &RecipientInfo,
) -> eyre::Result<Result<BridgeSendResult, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let token_bridge_addr = match validate_bridge_addr(ctx, pb).await? {
        Ok(v) => v,
        Err(resp) => return Ok(Err(resp)),
    };

    let (decimals, symbol) = evm
        .get_erc20_metadata(token_addr)
        .await
        .unwrap_or_else(|_| (18, "ERC20".into()));
    let amount_base = match parse_amount_base(&pb.amount_s, &pb.units, decimals) {
        Ok(v) => v,
        Err(te) => return Ok(Err(ok(ctx.req_id.clone(), tool_err(te)))),
    };

    let (usd_value, usd_value_known, outcome) =
        match resolve_usd_and_confirm(ctx, pb, evm, token_addr, amount_base, &symbol).await? {
            Ok(v) => v,
            Err(resp) => return Ok(Err(resp)),
        };

    // Approve the token bridge to spend tokens.
    let allowance = evm
        .erc20_allowance(token_addr, from, token_bridge_addr)
        .await
        .context("read erc20 allowance")?;
    if allowance < amount_base {
        let ac = ApprovalCtx {
            evm,
            from,
            token_addr,
            token_bridge_addr,
            amount_base,
            outcome: &outcome,
        };
        if let Err(resp) = send_approval(ctx, pb, &ac).await? {
            return Ok(Err(resp));
        }
    }

    // Build and send `transferTokens`.
    let bridge = IWormholeTokenBridge::new(token_bridge_addr, evm.provider()?);
    let nonce = crate::db::Db::now_ms()
        .ok()
        .and_then(|ms| u32::try_from((ms / 1000).rem_euclid(i64::from(u32::MAX))).ok())
        .unwrap_or(1_u32);
    let call = bridge.transferTokens(
        token_addr,
        amount_base,
        pb.dst_wh_chain_id,
        recipient.bytes32.into(),
        U256::ZERO,
        nonce,
    );
    let call_data = call.calldata().to_vec();
    let tx = TransactionRequest {
        from: Some(from),
        to: Some(token_bridge_addr.into()),
        input: Bytes::from(call_data).into(),
        value: Some(U256::ZERO),
        ..Default::default()
    };

    if let Err(e) = evm.simulate_tx_strict(&tx).await {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "bridge (wormhole)"),
            )),
        )));
    }

    let signer =
        load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &pb.w, pb.idx).await?;
    let txid = evm.send_tx(signer, tx).await?;

    ctx.shared.ks.append_tx_history(&json!({
        "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
        "type": "bridge", "chain": pb.chain, "wallet": pb.w.name, "account_index": pb.idx,
        "provider": "wormhole", "to_chain": pb.to_chain, "token": format!("{token_addr:#x}"),
        "amount_base": amount_base.to_string(), "amount_units": pb.units,
        "recipient": recipient.display, "token_bridge": format!("{token_bridge_addr:#x}"),
        "usd_value": usd_value, "txid": format!("{txid:#x}")
    }))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(), "tool": "bridge_tokens", "wallet": pb.w.name, "account_index": pb.idx,
        "chain": pb.chain, "usd_value": usd_value, "usd_value_known": usd_value_known,
        "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required,
        "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd,
        "forced_confirm": outcome.forced_confirm, "txid": format!("{txid:#x}"),
        "error_code": null, "result": "broadcasted", "provider": "wormhole"
    }));

    Ok(Ok(BridgeSendResult { txid, usd_value }))
}

struct ApprovalCtx<'a> {
    evm: &'a EvmChain,
    from: Address,
    token_addr: Address,
    token_bridge_addr: Address,
    amount_base: U256,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

async fn send_approval<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pb: &ParsedBridge,
    ac: &ApprovalCtx<'_>,
) -> eyre::Result<Result<(), JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let approve_tx = ac
        .evm
        .build_erc20_approve(ac.from, ac.token_addr, ac.token_bridge_addr, ac.amount_base)
        .context("build approve tx")?;
    if let Err(e) = ac.evm.simulate_tx_strict(&approve_tx).await {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "approve (wormhole)"),
            )),
        )));
    }
    let signer =
        load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &pb.w, pb.idx).await?;
    let tx_hash = ac.evm.send_tx(signer.clone(), approve_tx).await?;
    let tx_hash_s = format!("{tx_hash:#x}");
    ctx.shared.ks.append_tx_history(&json!({
        "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
        "type": "approve", "chain": pb.chain, "wallet": pb.w.name, "account_index": pb.idx,
        "provider": "wormhole", "token": format!("{:#x}", ac.token_addr),
        "spender": format!("{:#x}", ac.token_bridge_addr), "amount_base": ac.amount_base.to_string(),
        "usd_value": 0.0_f64, "txid": tx_hash_s
    }))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(), "tool": "bridge_tokens", "wallet": pb.w.name, "account_index": pb.idx,
        "chain": pb.chain, "usd_value": 0.0_f64, "usd_value_known": false,
        "policy_decision": ac.outcome.policy_decision, "confirm_required": ac.outcome.confirm_required,
        "confirm_result": ac.outcome.confirm_result, "daily_used_usd": ac.outcome.daily_used_usd,
        "forced_confirm": ac.outcome.forced_confirm, "txid": tx_hash_s,
        "error_code": null, "result": "broadcasted", "type": "approve", "provider": "wormhole"
    }));
    if !wait_for_allowance(
        ac.evm,
        ac.token_addr,
        ac.from,
        ac.token_bridge_addr,
        ac.amount_base,
    )
    .await
    {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "approval_pending",
                "approval submitted but not yet confirmed; retry shortly",
            )),
        )));
    }
    Ok(Ok(()))
}

async fn finalize_bridge<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pb: &ParsedBridge,
    evm: &EvmChain,
    bsr: &BridgeSendResult,
    from: Address,
    token_addr: Address,
    recipient_sol_owner: Option<Pubkey>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let txid = bsr.txid;
    let txid_s = format!("{txid:#x}");
    let token_s = format!("{token_addr:#x}");

    let Ok(receipt) = evm.wait_for_tx_receipt(txid, Duration::from_secs(60)).await else {
        return Ok(ok(
            ctx.req_id.clone(),
            tool_ok(json!({
                "chain": pb.chain, "bridge_provider": "wormhole", "to_chain": pb.to_chain,
                "txid": txid_s, "usd_value": bsr.usd_value, "bridge_id": null,
                "notes": "bridge tx broadcasted, but receipt was not observed in time; retry later and derive a wormhole bridge_id from the tx receipt logs"
            })),
        ));
    };

    let Some((emitter_addr, sequence)) = extract_wormhole_message(&receipt) else {
        return Ok(ok(
            ctx.req_id.clone(),
            tool_ok(json!({
                "chain": pb.chain, "bridge_provider": "wormhole", "to_chain": pb.to_chain,
                "txid": txid_s, "usd_value": bsr.usd_value, "bridge_id": null,
                "notes": "bridge tx confirmed, but wormhole LogMessagePublished was not found in logs"
            })),
        ));
    };

    let emitter_hex = bytes32_hex(evm_address_to_bytes32(emitter_addr));
    let bridge_id = format!("wormhole:{}:{emitter_hex}:{sequence}", pb.src_wh_chain_id);

    let (vaa_available, redeem_txid, redeem_error) = try_vaa_and_redeem(
        ctx,
        pb,
        &emitter_hex,
        sequence,
        from,
        recipient_sol_owner,
        &bridge_id,
    )
    .await?;

    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
            "chain": pb.chain, "bridge_provider": "wormhole", "to_chain": pb.to_chain,
            "token": token_s, "amount_base": "", "usd_value": bsr.usd_value,
            "txid": txid_s, "bridge_id": bridge_id,
            "wormhole": {
                "source_chain_id": pb.src_wh_chain_id, "destination_chain_id": pb.dst_wh_chain_id,
                "emitter": emitter_hex, "sequence": sequence,
                "vaa_available": vaa_available, "redeem_txid": redeem_txid, "redeem_error": redeem_error
            }
        })),
    ))
}

async fn try_vaa_and_redeem<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pb: &ParsedBridge,
    emitter_hex: &str,
    sequence: u64,
    from: Address,
    recipient_sol_owner: Option<Pubkey>,
    bridge_id: &str,
) -> eyre::Result<(bool, Option<String>, Option<String>)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mut vaa_available = false;
    let mut redeem_txid: Option<String> = None;
    let mut redeem_error: Option<String> = None;
    let dest_is_solana = pb.to_chain == "solana";

    if pb.redeem && !dest_is_solana && !pb.dst_token_bridge_s.trim().is_empty() {
        let default_dst = default_token_bridge_for_chain(&pb.to_chain).unwrap_or("");
        if !default_dst.trim().is_empty()
            && !pb
                .dst_token_bridge_s
                .trim()
                .eq_ignore_ascii_case(default_dst.trim())
        {
            redeem_error = Some(
                "auto-redeem skipped because to_token_bridge_address is custom (requires manual redemption)"
                    .to_owned(),
            );
        }
    }

    if pb.redeem
        && redeem_error.is_none()
        && (dest_is_solana || !pb.dst_token_bridge_s.trim().is_empty())
    {
        let vaa_b64 = poll_signed_vaa(
            &ctx.shared.cfg.http.wormholescan_api_base_url,
            pb.src_wh_chain_id,
            emitter_hex,
            sequence,
            &mut redeem_error,
        )
        .await;

        if let Some(vaa_b64) = vaa_b64 {
            vaa_available = true;
            let vaa_bytes = if let Ok(v) = base64::engine::general_purpose::STANDARD.decode(vaa_b64)
            {
                v
            } else {
                redeem_error = Some("invalid vaaBytes from wormholescan".to_owned());
                Vec::new()
            };
            if redeem_error.is_none() {
                let rp = RedeemParams {
                    w: &pb.w,
                    idx: pb.idx,
                    vaa_bytes: &vaa_bytes,
                    dest_is_solana,
                    recipient_sol_owner,
                    to_chain: &pb.to_chain,
                    dst_token_bridge_s: &pb.dst_token_bridge_s,
                    from,
                    effective_policy: &pb.effective_policy,
                    bridge_id,
                    usd_value: 0.0,
                };
                let (rtx, rerr) = attempt_redeem(ctx, &rp).await?;
                redeem_txid = rtx;
                if let Some(e) = rerr {
                    redeem_error = Some(e);
                }
            }
        }
    }

    Ok((vaa_available, redeem_txid, redeem_error))
}

pub async fn handle<R, W>(ctx: &mut HandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (pb, lock) = match parse_bridge_args(ctx)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let rpc_url = ctx
        .shared
        .cfg
        .rpc
        .evm_rpc_urls
        .get(pb.chain.as_str())
        .ok_or_else(|| eyre::eyre!("unknown evm chain: {}", pb.chain))?
        .clone();
    let chain_id = *ctx
        .shared
        .cfg
        .rpc
        .evm_chain_ids
        .get(pb.chain.as_str())
        .ok_or_else(|| eyre::eyre!("missing evm chain id: {}", pb.chain))?;
    let mut evm = EvmChain::for_name(&pb.chain, chain_id, &rpc_url, &ctx.shared.cfg.http);
    if let Some(fb) = ctx
        .shared
        .cfg
        .rpc
        .evm_fallback_rpc_urls
        .get(pb.chain.as_str())
    {
        evm.fallback_rpc_urls.clone_from(fb);
    }

    let from = evm_addr_for_account(&pb.w, pb.idx)?;
    let token_addr = EvmChain::parse_address(&pb.token_s).context("parse token")?;

    let recipient = match resolve_recipient(ctx, &pb, from, token_addr).await? {
        Ok(v) => v,
        Err(resp) => {
            Keystore::release_lock(lock)?;
            return Ok(resp);
        }
    };

    let bsr = match approve_and_send_bridge(ctx, &pb, &evm, from, token_addr, &recipient).await? {
        Ok(v) => v,
        Err(resp) => {
            Keystore::release_lock(lock)?;
            return Ok(resp);
        }
    };

    Keystore::release_lock(lock)?;
    finalize_bridge(ctx, &pb, &evm, &bsr, from, token_addr, recipient.sol_owner).await
}
