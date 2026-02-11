use crate::{
    chains::{evm::EvmChain, solana::SolanaChain},
    config::NetworkMode,
    errors::SeashailError,
    policy::Policy,
    wallet::ImportedKind,
};
use alloy::primitives::{Address as EvmAddress, U256};
use eyre::Context as _;
use serde_json::Value;
use std::collections::BTreeMap;

use super::super::SharedState;

pub const MAX_REMOTE_TX_BYTES: u64 = 2 * 1024 * 1024;

pub fn evm_native_symbol(chain: &str) -> &'static str {
    match chain {
        "ethereum" | "base" | "arbitrum" | "optimism" | "sepolia" | "base-sepolia"
        | "arbitrum-sepolia" | "optimism-sepolia" => "ETH",
        "polygon" | "polygon-amoy" => "POL",
        "bnb" | "bnb-testnet" => "BNB",
        "avalanche" | "avalanche-fuji" => "AVAX",
        "monad" | "monad-testnet" => "MON",
        _ => "NATIVE",
    }
}

pub fn oneinch_supported_chain(chain: &str) -> bool {
    // 1inch coverage changes over time; keep this conservative and aligned with our defaults.
    matches!(
        chain,
        "ethereum" | "base" | "arbitrum" | "optimism" | "polygon" | "bnb" | "avalanche"
    )
}

pub const fn solana_fallback_urls(shared: &SharedState, mode: NetworkMode) -> &Vec<String> {
    match mode {
        NetworkMode::Mainnet => &shared.cfg.rpc.solana_fallback_rpc_urls_mainnet,
        NetworkMode::Testnet => &shared.cfg.rpc.solana_fallback_rpc_urls_devnet,
    }
}

pub async fn solana_airdrop_is_allowed(sol: &SolanaChain) -> eyre::Result<bool> {
    // Use genesis hash so custom RPC endpoints are classified correctly.
    // Mainnet-beta cluster id: 5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d
    let mainnet_genesis = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
    let gh = sol
        .get_genesis_hash()
        .await
        .context("get solana genesis hash")?;
    Ok(gh.to_string() != mainnet_genesis)
}

pub fn parse_policy(v: Value) -> eyre::Result<Policy> {
    let p: Policy = serde_json::from_value(v).context("parse policy")?;
    // Keep policy semantics unambiguous:
    //
    // - `auto_approve_usd` is the tier boundary between auto and "requires confirmation".
    // - `hard_block_over_usd` is the hard cap above which transactions are blocked.
    //
    // `confirm_up_to_usd` is kept for backwards compatibility with earlier configs/tool
    // schemas, but Seashail currently enforces a single hard cap. Require the two to match
    // so users don't end up with surprising behavior.
    let cap_mismatch =
        crate::financial_math::usd_cap_mismatch(p.confirm_up_to_usd, p.hard_block_over_usd);
    if cap_mismatch {
        eyre::bail!(
            "policy invalid: confirm_up_to_usd ({:.2}) must equal hard_block_over_usd ({:.2})",
            p.confirm_up_to_usd,
            p.hard_block_over_usd
        );
    }
    if !p.auto_approve_usd.is_finite()
        || !p.hard_block_over_usd.is_finite()
        || !p.max_usd_per_tx.is_finite()
        || !p.max_usd_per_day.is_finite()
        || !p.max_usd_per_position.is_finite()
        || !p.max_usd_per_nft_tx.is_finite()
        || !p.pumpfun_max_sol_per_buy.is_finite()
        || !p.max_usd_per_bridge_tx.is_finite()
        || !p.max_usd_per_lending_tx.is_finite()
        || !p.max_usd_per_stake_tx.is_finite()
        || !p.max_usd_per_liquidity_tx.is_finite()
        || !p.max_usd_per_prediction_tx.is_finite()
    {
        eyre::bail!("policy invalid: numeric limits must be finite");
    }
    if p.auto_approve_usd < 0.0_f64
        || p.hard_block_over_usd < 0.0_f64
        || p.max_usd_per_tx < 0.0_f64
        || p.max_usd_per_day < 0.0_f64
        || p.max_usd_per_position < 0.0_f64
        || p.max_usd_per_nft_tx < 0.0_f64
        || p.pumpfun_max_sol_per_buy < 0.0_f64
        || p.max_usd_per_bridge_tx < 0.0_f64
        || p.max_usd_per_lending_tx < 0.0_f64
        || p.max_usd_per_stake_tx < 0.0_f64
        || p.max_usd_per_liquidity_tx < 0.0_f64
        || p.max_usd_per_prediction_tx < 0.0_f64
    {
        eyre::bail!("policy invalid: numeric limits must be non-negative");
    }
    if p.auto_approve_usd > p.hard_block_over_usd {
        eyre::bail!(
            "policy invalid: auto_approve_usd ({:.2}) must be <= hard_block_over_usd ({:.2})",
            p.auto_approve_usd,
            p.hard_block_over_usd
        );
    }
    if p.max_leverage == 0 {
        eyre::bail!("policy invalid: max_leverage must be >= 1");
    }
    if p.pumpfun_max_buys_per_hour == 0 {
        eyre::bail!("policy invalid: pumpfun_max_buys_per_hour must be >= 1");
    }
    Ok(p)
}

pub fn decode_secret(
    kind: ImportedKind,
    private_key_chain: Option<&str>,
    secret: &str,
) -> eyre::Result<Vec<u8>> {
    match kind {
        ImportedKind::Mnemonic => Ok(secret.as_bytes().to_vec()),
        ImportedKind::PrivateKey => {
            let chain = private_key_chain.unwrap_or("evm");
            if chain == "evm" {
                let s = secret.strip_prefix("0x").unwrap_or(secret);
                let bytes = hex::decode(s).context("decode hex private key")?;
                if bytes.len() != 32 {
                    eyre::bail!("EVM private key must be 32 bytes");
                }
                Ok(bytes)
            } else if chain == "solana" {
                // Accept base58-encoded 64-byte keypair.
                let bytes = bs58::decode(secret)
                    .into_vec()
                    .context("decode base58 solana keypair")?;
                if bytes.len() != 64 {
                    eyre::bail!("Solana keypair must be 64 bytes (base58)");
                }
                Ok(bytes)
            } else {
                eyre::bail!("unknown private_key_chain: {chain}");
            }
        }
    }
}

pub fn resolve_wallet_and_account(
    shared: &SharedState,
    args: &Value,
) -> eyre::Result<(crate::wallet::WalletRecord, u32)> {
    let wallet_name = args.get("wallet").and_then(|v| v.as_str());
    let account_index = args
        .get("account_index")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());

    let (w, active_idx) = match wallet_name {
        Some(name) if !name.is_empty() => {
            let w = shared
                .ks
                .get_wallet_by_name(name)?
                .ok_or_else(|| SeashailError::WalletNotFound(name.to_owned()))?;
            let idx = account_index.unwrap_or(w.last_active_account);
            (w, idx)
        }
        _ => shared
            .ks
            .get_active_wallet()?
            .ok_or_else(|| SeashailError::WalletNotFound("active".into()))?,
    };

    let idx = account_index.unwrap_or(active_idx);
    if idx >= w.accounts {
        return Err(SeashailError::AccountIndexOutOfRange.into());
    }
    Ok((w, idx))
}

pub fn parse_portfolio_tokens_map(args: &Value) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(obj) = args.get("tokens").and_then(|v| v.as_object()) else {
        return out;
    };
    for (chain, v) in obj {
        let Some(arr) = v.as_array() else {
            continue;
        };
        let toks: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_owned()))
            .filter(|s| !s.is_empty())
            .collect();
        if !toks.is_empty() {
            out.insert(chain.trim().to_owned(), toks);
        }
    }
    out
}

pub fn evm_addr_for_account(w: &crate::wallet::WalletRecord, idx: u32) -> eyre::Result<EvmAddress> {
    let s = w
        .evm_addresses
        .get(idx as usize)
        .ok_or_else(|| eyre::eyre!("missing evm address for account"))?;
    EvmChain::parse_address(s)
}

pub fn sol_pubkey_for_account(
    w: &crate::wallet::WalletRecord,
    idx: u32,
) -> eyre::Result<solana_sdk::pubkey::Pubkey> {
    let s = w
        .solana_addresses
        .get(idx as usize)
        .ok_or_else(|| eyre::eyre!("missing solana address for account"))?;
    SolanaChain::parse_pubkey(s)
}

pub fn is_native_token(s: &str) -> bool {
    s.is_empty() || s.eq_ignore_ascii_case("native")
}

pub fn u128_to_u64(v: u128) -> eyre::Result<u64> {
    u64::try_from(v).context("amount too large")
}

pub fn u128_to_u256(v: u128) -> U256 {
    U256::from(v)
}

pub fn u256_pow10(exp: u32) -> U256 {
    let mut out = U256::from(1_u64);
    for _ in 0..exp {
        out = out.saturating_mul(U256::from(10_u64));
    }
    out
}
