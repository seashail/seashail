use crate::{
    amount,
    chains::solana::SolanaChain,
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
    rpc::mcp_server::{ConnState, SharedState},
};
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::helpers::{
    resolve_wallet_and_account, solana_airdrop_is_allowed, solana_fallback_urls, u128_to_u64,
};

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let lock = shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let chain = args
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("solana");
    if chain != "solana" {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_chain",
                "airdrop only supported for solana",
            )),
        ));
    }
    let addr_s = args.get("address").and_then(|v| v.as_str());
    let amount = args.get("amount").and_then(|v| v.as_str()).unwrap_or("");
    let units = args
        .get("amount_units")
        .and_then(|v| v.as_str())
        .unwrap_or("ui");
    if amount.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing amount")),
        ));
    }

    let address = addr_s
        .map(std::borrow::ToOwned::to_owned)
        .or_else(|| w.solana_addresses.get(idx as usize).cloned())
        .unwrap_or_default();
    if address.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "missing_address",
                "wallet has no solana address",
            )),
        ));
    }

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
    if !solana_airdrop_is_allowed(&sol).await? {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "airdrop_not_allowed",
                "airdrop is only supported on Solana devnet/testnet/local validators (not mainnet-beta)",
            )),
        ));
    }
    let to_pk = SolanaChain::parse_pubkey(&address)?;
    let lamports = if units == "base" {
        u128_to_u64(amount::parse_amount_base_u128(amount)?)?
    } else {
        u128_to_u64(amount::parse_amount_ui_to_base_u128(amount, 9)?)?
    };

    let sig = sol.request_airdrop(to_pk, lamports).await?;

    shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "airdrop",
      "chain": "solana",
      "wallet": w.name,
      "account_index": idx,
      "to": address,
      "amount_base": lamports.to_string(),
      "signature": sig.to_string()
    }))?;

    Keystore::release_lock(lock)?;
    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": "solana",
          "signature": sig.to_string(),
          "amount_base": lamports.to_string(),
          "to": address
        })),
    ))
}
