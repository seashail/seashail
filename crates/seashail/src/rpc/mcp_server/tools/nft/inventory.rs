use eyre::Context as _;
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    resolve_wallet_and_account, sol_pubkey_for_account, solana_fallback_urls,
};
use crate::chains::solana::SolanaChain;
use crate::errors::ToolError;

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 2000) as usize;

    if chain.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing chain")),
        ));
    }

    if chain != "solana" {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "not_supported",
                "NFT inventory is only supported on Solana",
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
    let owner = sol_pubkey_for_account(&w, idx)?;
    let mints = sol
        .list_nft_like_mints(owner, limit)
        .await
        .context("list nft-like mints")?;
    let items: Vec<Value> = mints
        .into_iter()
        .map(|m| {
            json!({
              "chain": "solana",
              "mint": m.to_string(),
              "owner": owner.to_string(),
            })
        })
        .collect();
    Ok(ok(req_id, tool_ok(json!({ "items": items }))))
}
