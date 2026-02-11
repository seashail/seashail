use crate::chains::{bitcoin::BitcoinChain, evm::EvmChain, solana::SolanaChain};
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, evm_native_symbol, is_native_token, resolve_wallet_and_account,
    sol_pubkey_for_account, solana_fallback_urls,
};

const SOL_DECIMALS: i32 = 9;
const EVM_DECIMALS: i32 = 18;
const BTC_DECIMALS: i32 = 8;

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let chain_filter = args.get("chain").and_then(Value::as_str).unwrap_or("");
    let tokens = args
        .get("tokens")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(std::borrow::ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let chains: Vec<String> = if chain_filter.is_empty() {
        shared
            .cfg
            .default_chains_for_mode(effective_network_mode(shared, conn))
    } else {
        vec![chain_filter.to_owned()]
    };

    let mut out = vec![];
    for chain in chains {
        let v = if chain == "solana" {
            balance_solana(shared, conn, &w, idx, &tokens).await?
        } else if chain == "bitcoin" {
            balance_bitcoin(shared, conn, &w, idx).await?
        } else {
            balance_evm(shared, &w, idx, &chain, &tokens).await?
        };
        out.push(v);
    }

    Ok(ok(
        req_id,
        tool_ok(json!({ "wallet": w.name, "account_index": idx, "balances": out })),
    ))
}

async fn balance_solana(
    shared: &SharedState,
    conn: &ConnState,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    tokens: &[String],
) -> eyre::Result<Value> {
    let mode = effective_network_mode(shared, conn);
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
    let owner = sol_pubkey_for_account(w, idx)?;

    let lamports = match sol.get_sol_balance(owner).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(json!({
              "chain": "solana",
              "error": format!("{e:#}"),
              "native": { "symbol": "SOL", "amount": "0", "decimals": SOL_DECIMALS },
              "tokens": []
            }));
        }
    };

    let toks = solana_spl_balances(&sol, owner, tokens).await?;
    Ok(json!({
      "chain": "solana",
      "native": { "symbol": "SOL", "amount": lamports.to_string(), "decimals": SOL_DECIMALS },
      "tokens": toks
    }))
}

async fn solana_spl_balances(
    sol: &SolanaChain,
    owner: solana_sdk::pubkey::Pubkey,
    tokens: &[String],
) -> eyre::Result<Vec<Value>> {
    let mut toks = vec![];
    for t in tokens {
        if is_native_token(t) {
            continue;
        }
        let mint = SolanaChain::parse_pubkey(t)?;
        match sol.get_spl_balance(owner, mint).await {
            Ok((amount, decimals)) => toks.push(json!({
              "mint": t,
              "amount": amount.to_string(),
              "decimals": decimals
            })),
            Err(_) => toks.push(json!({
              "mint": t,
              "amount": "0",
              "decimals": 0_i32
            })),
        }
    }
    Ok(toks)
}

async fn balance_evm(
    shared: &SharedState,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    chain: &str,
    tokens: &[String],
) -> eyre::Result<Value> {
    let rpc_url = shared
        .cfg
        .rpc
        .evm_rpc_urls
        .get(chain)
        .ok_or_else(|| eyre::eyre!("unknown evm chain: {chain}"))?
        .clone();
    let chain_id = *shared
        .cfg
        .rpc
        .evm_chain_ids
        .get(chain)
        .ok_or_else(|| eyre::eyre!("missing evm chain id: {chain}"))?;

    let mut evm = EvmChain::for_name(chain, chain_id, &rpc_url, &shared.cfg.http);
    if let Some(fb) = shared.cfg.rpc.evm_fallback_rpc_urls.get(chain) {
        evm.fallback_rpc_urls.clone_from(fb);
    }

    let owner = evm_addr_for_account(w, idx)?;

    let wei = match evm.get_native_balance(owner).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(json!({
              "chain": chain,
              "error": format!("{e:#}"),
              "native": { "symbol": evm_native_symbol(chain), "amount": "0", "decimals": EVM_DECIMALS },
              "tokens": []
            }));
        }
    };

    let toks = evm_erc20_balances(&evm, owner, tokens).await?;
    Ok(json!({
      "chain": chain,
      "native": { "symbol": evm_native_symbol(chain), "amount": wei.to_string(), "decimals": EVM_DECIMALS },
      "tokens": toks
    }))
}

async fn balance_bitcoin(
    shared: &SharedState,
    conn: &ConnState,
    w: &crate::wallet::WalletRecord,
    idx: u32,
) -> eyre::Result<Value> {
    let mode = effective_network_mode(shared, conn);
    let addr = if mode == crate::config::NetworkMode::Testnet {
        w.bitcoin_addresses_testnet.get(idx as usize)
    } else {
        w.bitcoin_addresses_mainnet.get(idx as usize)
    }
    .cloned()
    .unwrap_or_default();

    if addr.is_empty() {
        return Ok(json!({
          "chain": "bitcoin",
          "error": "wallet has no bitcoin address for this account",
          "native": { "symbol": "BTC", "amount": "0", "decimals": BTC_DECIMALS },
          "tokens": []
        }));
    }

    let base = if mode == crate::config::NetworkMode::Testnet {
        shared.cfg.http.bitcoin_api_base_url_testnet.clone()
    } else {
        shared.cfg.http.bitcoin_api_base_url_mainnet.clone()
    };
    let btc = BitcoinChain::new(&base)?;

    match btc.get_address_balance_sats(&addr).await {
        Ok((confirmed, unconfirmed)) => Ok(json!({
          "chain": "bitcoin",
          "address": addr,
          "native": { "symbol": "BTC", "amount": confirmed.to_string(), "decimals": BTC_DECIMALS },
          "unconfirmed": { "amount": unconfirmed.to_string(), "decimals": BTC_DECIMALS },
          "tokens": []
        })),
        Err(e) => Ok(json!({
          "chain": "bitcoin",
          "error": format!("{e:#}"),
          "native": { "symbol": "BTC", "amount": "0", "decimals": BTC_DECIMALS },
          "tokens": []
        })),
    }
}

async fn evm_erc20_balances(
    evm: &EvmChain,
    owner: alloy::primitives::Address,
    tokens: &[String],
) -> eyre::Result<Vec<Value>> {
    let mut toks = vec![];
    for t in tokens {
        if is_native_token(t) {
            continue;
        }
        let token = EvmChain::parse_address(t)?;
        let (amount, decimals, symbol) = evm.get_erc20_balance(token, owner).await?;
        toks.push(json!({
          "contract": t,
          "symbol": symbol,
          "amount": amount.to_string(),
          "decimals": decimals
        }));
    }
    Ok(toks)
}
