use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::super::super::SharedState;

use std::collections::BTreeMap;

type CountUsd = (u64, f64);
type ByType = BTreeMap<String, CountUsd>;
type DayAgg = (u64, f64, ByType);
type ByDay = BTreeMap<String, DayAgg>;

fn aggregate_items(items: &[Value]) -> (f64, ByType, ByType, ByDay) {
    use crate::financial_math::accum;

    let mut total_usd = 0.0_f64;
    let mut by_type: ByType = BTreeMap::new();
    let mut by_chain: ByType = BTreeMap::new();
    let mut by_day: ByDay = BTreeMap::new();

    for v in items {
        let usd = v
            .get("usd_value")
            .and_then(Value::as_f64)
            .unwrap_or(0.0_f64);
        if usd.is_finite() && usd >= 0.0_f64 {
            accum(&mut total_usd, usd);
        }

        let t = v
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let c = v
            .get("chain")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let d = v
            .get("day")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();

        let type_entry = by_type.entry(t.clone()).or_insert((0_u64, 0.0_f64));
        type_entry.0 += 1;
        accum(&mut type_entry.1, usd);

        let chain_entry = by_chain.entry(c.clone()).or_insert((0_u64, 0.0_f64));
        chain_entry.0 += 1;
        accum(&mut chain_entry.1, usd);

        let day_entry = by_day.entry(d).or_insert((0_u64, 0.0_f64, BTreeMap::new()));
        day_entry.0 += 1;
        accum(&mut day_entry.1, usd);
        let day_type_entry = day_entry.2.entry(t).or_insert((0_u64, 0.0_f64));
        day_type_entry.0 += 1;
        accum(&mut day_type_entry.1, usd);
    }

    (total_usd, by_type, by_chain, by_day)
}

fn parse_scope(args: &Value, shared: &SharedState) -> Option<(Value, String)> {
    // This scope JSON is shared with `get_portfolio(include_history=true)` so callers can
    // query snapshot-based P&L from the same persisted history.
    let scope_in = args.get("snapshot_scope").and_then(Value::as_object);
    let wallets = scope_in
        .and_then(|o| o.get("wallets"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<String>>()
        })
        .filter(|v| !v.is_empty())
        .or_else(|| {
            args.get("wallet")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|w| vec![w.to_owned()])
        });

    let chains = scope_in
        .and_then(|o| o.get("chains"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<String>>()
        })
        .filter(|v| !v.is_empty())
        .or_else(|| {
            args.get("chain")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|c| vec![c.to_owned()])
        })
        .unwrap_or_else(|| {
            shared
                .cfg
                .default_chains_for_mode(shared.cfg.effective_network_mode())
        });

    if chains.is_empty() {
        return None;
    }

    let scope = json!({
        "wallets": wallets.as_ref().filter(|v| !v.is_empty()),
        "chains": chains,
    });
    Some((scope.clone(), scope.to_string()))
}

pub async fn handle(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(500)
        .clamp(1, 10_000);
    let wallet = args.get("wallet").and_then(|v| v.as_str());
    let chain = args.get("chain").and_then(|v| v.as_str());
    let type_filter = args.get("type").and_then(|v| v.as_str());
    let since_ts = args.get("since_ts").and_then(|v| v.as_str());
    let until_ts = args.get("until_ts").and_then(|v| v.as_str());

    let items = shared.ks.read_tx_history_filtered(
        limit,
        wallet,
        chain,
        type_filter,
        since_ts,
        until_ts,
    )?;

    let (total_usd, by_type, by_chain, by_day) = aggregate_items(&items);

    let by_type_out: Vec<Value> = by_type
        .into_iter()
        .map(|(t, (count, usd))| json!({ "type": t, "count": count, "usd_value": usd }))
        .collect();
    let by_chain_out: Vec<Value> = by_chain
        .into_iter()
        .map(|(c, (count, usd))| json!({ "chain": c, "count": count, "usd_value": usd }))
        .collect();
    let by_day_out: Vec<Value> = by_day
        .into_iter()
        .map(|(day, (count, usd, types))| {
            let types_out: Vec<Value> = types
                .into_iter()
                .map(|(t, (tc, tu))| json!({ "type": t, "count": tc, "usd_value": tu }))
                .collect();
            json!({ "day": day, "count": count, "usd_value": usd, "by_type": types_out })
        })
        .collect();

    let snapshot_pnl = if let Some((scope, scope_json)) = parse_scope(args, shared) {
        shared.ensure_db().await;
        if let Some(db) = shared.db() {
            let rows = db
                .list_portfolio_snapshot_totals_for_scope(&scope_json, 2)
                .await
                .unwrap_or_default();
            let latest = rows.first().cloned();
            let prev = rows.get(1).cloned();

            let now_ms = crate::db::Db::now_ms().unwrap_or(0);
            let cutoff_day_ms = now_ms.saturating_sub(24 * 60 * 60 * 1000);
            let cutoff_week_ms = now_ms.saturating_sub(7 * 24 * 60 * 60 * 1000);
            let cutoff_month_ms = now_ms.saturating_sub(30 * 24 * 60 * 60 * 1000);

            let total_day = db
                .portfolio_snapshot_total_at_or_before(&scope_json, cutoff_day_ms)
                .await
                .ok()
                .flatten();
            let total_week = db
                .portfolio_snapshot_total_at_or_before(&scope_json, cutoff_week_ms)
                .await
                .ok()
                .flatten();
            let total_month = db
                .portfolio_snapshot_total_at_or_before(&scope_json, cutoff_month_ms)
                .await
                .ok()
                .flatten();

            match (latest, prev) {
                (Some(latest), Some(prev)) => {
                    let delta_prev =
                        crate::financial_math::sub_f64(latest.total_usd, prev.total_usd);
                    let delta_since_day = total_day
                        .as_ref()
                        .map(|r| crate::financial_math::sub_f64(latest.total_usd, r.total_usd));
                    let delta_since_week = total_week
                        .as_ref()
                        .map(|r| crate::financial_math::sub_f64(latest.total_usd, r.total_usd));
                    let delta_since_month = total_month
                        .as_ref()
                        .map(|r| crate::financial_math::sub_f64(latest.total_usd, r.total_usd));

                    Some(json!({
                      "scope": scope,
                      "latest_total_usd": latest.total_usd,
                      "latest_snapshot_id": latest.snapshot_id,
                      "latest_fetched_at_ms": latest.fetched_at_ms,
                      "delta_since_prev_snapshot_usd": delta_prev,
                      "prev_total_usd": prev.total_usd,
                      "prev_snapshot_id": prev.snapshot_id,
                      "prev_fetched_at_ms": prev.fetched_at_ms,
                      "delta_24h_usd": delta_since_day,
                      "delta_7d_usd": delta_since_week,
                      "delta_30d_usd": delta_since_month
                    }))
                }
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(ok(
        req_id,
        tool_ok(json!({
          "count": items.len(),
          "totals": {
            "usd_value": total_usd,
            "trades": items.len()
          },
          "by_type": by_type_out,
          "by_chain": by_chain_out,
          "by_day": by_day_out,
          "snapshot_pnl": snapshot_pnl
        })),
    ))
}
