use crate::{
    chains::{evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
    price,
};
use alloy::primitives::U256;
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{is_native_token, solana_fallback_urls, u128_to_u64};

async fn handle_solana_token_price(
    req_id: Value,
    chain: &str,
    token: &str,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
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
    let mint = SolanaChain::parse_pubkey(token)?;
    let decimals = sol.get_mint_decimals(mint).await?;
    let one = 10_u128
        .checked_pow(u32::from(decimals))
        .ok_or_else(|| eyre::eyre!("decimals too large"))?;
    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    shared.ensure_db().await;
    let db = shared.db();
    let p = price::solana_token_price_usd_cached(
        &sol,
        &shared.cfg,
        token,
        usdc_mint,
        u128_to_u64(one)?,
        50,
        db,
    )
    .await?;
    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": chain,
          "token": token,
          "usd": p.usd,
          "source": format!("{:?}", p.source)
        })),
    ))
}

async fn handle_evm_token_price(
    req_id: Value,
    chain: &str,
    token: &str,
    shared: &mut SharedState,
) -> eyre::Result<JsonRpcResponse> {
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
    let token_addr = EvmChain::parse_address(token)?;
    let (decimals, _symbol) = evm.get_erc20_metadata(token_addr).await?;
    let mut one = U256::from(1_u64);
    for _ in 0..decimals {
        one = one
            .checked_mul(U256::from(10_u64))
            .ok_or_else(|| eyre::eyre!("amount overflow"))?;
    }
    if let Some(u) = &evm.uniswap {
        if token_addr == u.usdc {
            return Ok(ok(
                req_id,
                tool_ok(
                    json!({ "chain": chain, "token": token, "usd": 1.0_f64, "source": "USDC" }),
                ),
            ));
        }
    }
    shared.ensure_db().await;
    let db = shared.db();
    let p = price::evm_token_price_usd_cached(&evm, &shared.cfg, token_addr, one, 50, db).await?;
    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": chain,
          "token": token,
          "usd": p.usd,
          "source": format!("{:?}", p.source)
        })),
    ))
}

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    let token = args.get("token").and_then(|v| v.as_str()).unwrap_or("");
    if chain.is_empty() || token.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing chain or token")),
        ));
    }

    if is_native_token(token) {
        shared.ensure_db().await;
        let db = shared.db();
        let Ok(p) = price::native_token_price_usd_cached(chain, &shared.cfg, db).await else {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "price_unavailable",
                    "native token USD price is unavailable for this chain",
                )),
            ));
        };
        return Ok(ok(
            req_id,
            tool_ok(json!({
              "chain": chain,
              "token": "native",
              "usd": p.usd,
              "source": format!("{:?}", p.source)
            })),
        ));
    }

    if chain == "solana" {
        return handle_solana_token_price(req_id, chain, token, shared, conn).await;
    }

    handle_evm_token_price(req_id, chain, token, shared).await
}
