use crate::{
    chains::{evm::EvmChain, solana::SolanaChain},
    financial_math::{lamports_to_usd, token_base_to_usd},
    keystore::Keystore,
    price,
    wallet::WalletRecord,
};
use alloy::primitives::U256;
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, parse_portfolio_tokens_map, sol_pubkey_for_account, solana_fallback_urls,
    u256_pow10,
};

async fn solana_chain_item(
    shared: &SharedState,
    conn: &ConnState,
    db: Option<&crate::db::Db>,
    tokens_map: &std::collections::BTreeMap<String, Vec<String>>,
    w: &WalletRecord,
    account_index: u32,
) -> eyre::Result<Option<(Value, f64)>> {
    if w.solana_addresses.get(account_index as usize).is_none() {
        return Ok(None);
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
    let owner = sol_pubkey_for_account(w, account_index)?;
    let lamports = sol.get_sol_balance(owner).await.unwrap_or(0);
    let p = price::native_token_price_usd_cached("solana", &shared.cfg, db).await;
    let usd = p
        .map(|pp| lamports_to_usd(lamports, pp.usd))
        .unwrap_or(0.0_f64);

    let mut chain_tokens = vec![json!({
      "token": "native",
      "symbol": "SOL",
      "amount_base": lamports.to_string(),
      "decimals": 9_i32,
      "usd_value": usd
    })];

    if let Some(extra) = tokens_map.get("solana") {
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        for mint_s in extra {
            if mint_s == "native" {
                continue;
            }
            let Ok(mint) = SolanaChain::parse_pubkey(mint_s) else {
                continue;
            };
            let Ok((bal, decimals)) = sol.get_spl_balance(owner, mint).await else {
                continue;
            };
            let usd_value = if bal == 0 {
                0.0_f64
            } else {
                // Best-effort: price 1 unit to USDC via Jupiter.
                let one = 10_u64.checked_pow(u32::from(decimals)).unwrap_or(1);
                price::solana_token_price_usd_cached(
                    &sol,
                    &shared.cfg,
                    mint_s,
                    usdc_mint,
                    one.max(1),
                    50,
                    db,
                )
                .await
                .map(|tp| token_base_to_usd(u128::from(bal), decimals, tp.usd))
                .unwrap_or(0.0_f64)
            };
            chain_tokens.push(json!({
              "token": mint_s,
              "amount_base": bal.to_string(),
              "decimals": decimals,
              "usd_value": usd_value
            }));
        }
    }

    let chain_total_usd = sum_usd_values(&chain_tokens);
    let item = json!({
      "wallet": w.name,
      "account_index": account_index,
      "chain": "solana",
      "native": { "symbol": "SOL", "amount_lamports": lamports.to_string() },
      "tokens": chain_tokens,
      "usd_value": chain_total_usd
    });
    Ok(Some((item, chain_total_usd)))
}

async fn evm_chain_item(
    shared: &SharedState,
    db: Option<&crate::db::Db>,
    tokens_map: &std::collections::BTreeMap<String, Vec<String>>,
    w: &WalletRecord,
    account_index: u32,
    chain: &str,
) -> eyre::Result<Option<(Value, f64)>> {
    if w.evm_addresses.get(account_index as usize).is_none() {
        return Ok(None);
    }
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
    let owner = evm_addr_for_account(w, account_index)?;
    let wei = evm.get_native_balance(owner).await.unwrap_or_default();
    let p = price::native_token_price_usd_cached(chain, &shared.cfg, db).await;
    let usd = p
        .map(|pp| token_base_to_usd(crate::chains::evm::u256_low_u128(wei), 18, pp.usd))
        .unwrap_or(0.0_f64);

    let mut chain_tokens = vec![json!({
      "token": "native",
      "amount_base": wei.to_string(),
      "decimals": 18_i32,
      "usd_value": usd
    })];

    if let Some(extra) = tokens_map.get(chain) {
        let usdc = evm.uniswap.as_ref().map(|u| u.usdc);
        for tok_s in extra {
            if tok_s == "native" {
                continue;
            }
            let Ok(tok_addr) = EvmChain::parse_address(tok_s) else {
                continue;
            };
            let Ok((bal, decimals, symbol)) = evm.get_erc20_balance(tok_addr, owner).await else {
                continue;
            };
            let usd_value = if bal.is_zero() {
                0.0_f64
            } else if usdc.is_some_and(|u| tok_addr == u) {
                token_base_to_usd(crate::chains::evm::u256_low_u128(bal), decimals, 1.0_f64)
            } else if evm.uniswap.is_none() {
                0.0_f64
            } else {
                // Best-effort: price 1 unit to USDC via Uniswap V3.
                let one = u256_pow10(u32::from(decimals)).max(U256::from(1_u64));
                price::evm_token_price_usd_cached(&evm, &shared.cfg, tok_addr, one, 50, db)
                    .await
                    .map(|tp| {
                        token_base_to_usd(crate::chains::evm::u256_low_u128(bal), decimals, tp.usd)
                    })
                    .unwrap_or(0.0_f64)
            };
            chain_tokens.push(json!({
              "token": tok_s,
              "symbol": symbol,
              "amount_base": bal.to_string(),
              "decimals": decimals,
              "usd_value": usd_value
            }));
        }
    }

    let chain_total_usd = sum_usd_values(&chain_tokens);
    let item = json!({
      "wallet": w.name,
      "account_index": account_index,
      "chain": chain,
      "native": { "amount_wei": wei.to_string() },
      "tokens": chain_tokens,
      "usd_value": chain_total_usd
    });
    Ok(Some((item, chain_total_usd)))
}

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let include_history = args
        .get("include_history")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let include_health = args
        .get("include_health")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let history_limit = args
        .get("history_limit")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 365) as usize;

    let wallet_filter = args.get("wallets").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(std::borrow::ToOwned::to_owned))
            .collect::<Vec<_>>()
    });
    let chain_filter = args.get("chains").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(std::borrow::ToOwned::to_owned))
            .collect::<Vec<_>>()
    });
    let tokens_map = parse_portfolio_tokens_map(&args);
    shared.ensure_db().await;
    let db = shared.db();

    let wallets = shared.ks.list_wallets()?;
    // Use `as_ref()` so we don't move `wallet_filter`; we may include it in the history scope below.
    let selected_wallets: Vec<_> = match wallet_filter.as_ref() {
        Some(names) if !names.is_empty() => wallets
            .into_iter()
            .filter(|w| names.contains(&w.name))
            .collect(),
        _ => wallets,
    };

    let mut chains: Vec<String> = shared
        .cfg
        .default_chains_for_mode(effective_network_mode(shared, conn));
    if let Some(cf) = chain_filter {
        if !cf.is_empty() {
            chains = cf;
        }
    }

    let mut items = vec![];
    let mut total_usd = 0.0_f64;
    for w in selected_wallets {
        for account_index in 0..w.accounts {
            for chain in &chains {
                if chain == "solana" {
                    if let Some((item, usd)) =
                        solana_chain_item(shared, conn, db, &tokens_map, &w, account_index).await?
                    {
                        accum_usd(&mut total_usd, usd);
                        items.push(item);
                    }
                } else if let Some((item, usd)) =
                    evm_chain_item(shared, db, &tokens_map, &w, account_index, chain).await?
                {
                    accum_usd(&mut total_usd, usd);
                    items.push(item);
                }
            }
        }
    }

    let (history_out, pnl_out) = if include_history {
        portfolio_history(
            shared,
            wallet_filter.as_ref(),
            &chains,
            &items,
            history_limit,
        )
        .await
    } else {
        (None, None)
    };

    let health_out = if include_health {
        portfolio_health(shared, wallet_filter.as_ref()).await
    } else {
        None
    };

    Ok(ok(
        req_id,
        tool_ok(json!({
          "items": items,
          "total_usd": total_usd,
          "pnl": pnl_out,
          "history": history_out,
          "health": health_out
        })),
    ))
}

async fn portfolio_history(
    shared: &mut SharedState,
    wallet_filter: Option<&Vec<String>>,
    chains: &[String],
    items: &[Value],
    history_limit: usize,
) -> (Option<Value>, Option<Value>) {
    shared.ensure_db().await;
    let Some(db) = shared.db() else {
        return (None, None);
    };
    let Ok(now_ms) = crate::db::Db::now_ms() else {
        return (None, None);
    };
    let day = Keystore::current_utc_day_key();
    let scope = json!({
        "wallets": wallet_filter.filter(|v| !v.is_empty()),
        "chains": chains,
    });
    let scope_json = scope.to_string();

    // Best-effort DB writes; do not fail the tool on persistence issues.
    if let Ok(snapshot_id) = db
        .insert_portfolio_snapshot(now_ms, &day, &scope_json)
        .await
    {
        for item in items {
            let wallet = item.get("wallet").and_then(Value::as_str).unwrap_or("");
            let chain = item.get("chain").and_then(Value::as_str).unwrap_or("");
            let idx = item
                .get("account_index")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let usd = item
                .get("usd_value")
                .and_then(Value::as_f64)
                .unwrap_or(0.0_f64);
            drop(
                db.insert_portfolio_snapshot_item(
                    snapshot_id,
                    wallet,
                    i64::try_from(idx).unwrap_or(0),
                    chain,
                    usd,
                    &item.to_string(),
                )
                .await,
            );
        }
    }

    let Ok(rows) = db
        .list_portfolio_snapshot_totals_for_scope(&scope_json, history_limit)
        .await
    else {
        return (None, None);
    };
    let pnl_out = match (rows.first(), rows.get(1)) {
        (Some(latest), Some(prev)) => {
            let delta = crate::financial_math::sub_f64(latest.total_usd, prev.total_usd);
            Some(json!({
                "delta_since_prev_snapshot_usd": delta,
                "latest_total_usd": latest.total_usd,
                "prev_total_usd": prev.total_usd,
                "latest_snapshot_id": latest.snapshot_id,
                "prev_snapshot_id": prev.snapshot_id
            }))
        }
        _ => None,
    };
    let snaps: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "snapshot_id": r.snapshot_id,
                "fetched_at_ms": r.fetched_at_ms,
                "day": r.day,
                "total_usd": r.total_usd
            })
        })
        .collect();
    let history_out = Some(json!({ "scope": scope, "snapshots": snaps }));
    (history_out, pnl_out)
}

async fn portfolio_health(
    shared: &mut SharedState,
    wallet_filter: Option<&Vec<String>>,
) -> Option<Value> {
    shared.ensure_db().await;
    let db = shared.db()?;
    let rows = db.list_health_snapshots().await.ok()?;
    let wanted_wallets: Option<std::collections::BTreeSet<&String>> = wallet_filter
        .filter(|v| !v.is_empty())
        .map(|v| v.iter().collect());
    let mut out: Vec<Value> = vec![];
    for r in rows {
        if wanted_wallets
            .as_ref()
            .is_some_and(|set| !set.contains(&r.wallet))
        {
            continue;
        }
        let payload_v: Value =
            serde_json::from_str(&r.payload_json).unwrap_or(Value::String(r.payload_json));
        out.push(json!({
            "surface": r.surface,
            "chain": r.chain,
            "provider": r.provider,
            "wallet": r.wallet,
            "account_index": r.account_index,
            "fetched_at_ms": r.fetched_at_ms,
            "payload": payload_v
        }));
    }
    Some(json!({ "snapshots": out }))
}

/// Sum USD values from token entries in a chain result.
fn sum_usd_values(tokens: &[Value]) -> f64 {
    let values: Vec<f64> = tokens
        .iter()
        .filter_map(|t| t.get("usd_value").and_then(Value::as_f64))
        .collect();
    crate::financial_math::sum_f64(&values)
}

/// Accumulate a chain total into the running portfolio total.
fn accum_usd(total: &mut f64, usd: f64) {
    crate::financial_math::accum(total, usd);
}
