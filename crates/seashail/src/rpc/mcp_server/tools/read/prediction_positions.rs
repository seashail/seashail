use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{evm_addr_for_account, resolve_wallet_and_account};
use crate::errors::ToolError;

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

fn ensure_https_or_loopback(url: &str, name: &str) -> Result<(), ToolError> {
    let u = url.trim();
    if u.starts_with("https://") || is_loopback_http(u) {
        return Ok(());
    }
    Err(ToolError::new(
        "invalid_config",
        format!("{name} must use https (or http://localhost for local testing)"),
    ))
}

async fn fetch_polymarket_positions(
    base_url: &str,
    address: &str,
) -> Result<serde_json::Value, ToolError> {
    let url = format!("{}/positions", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(2_000))
        .build()
        .map_err(|e| ToolError::new("upstream_error", format!("build http client: {e:#}")))?;
    let resp = client
        .get(url)
        .query(&[("user", address)])
        .send()
        .await
        .map_err(|e| {
            ToolError::new(
                "upstream_error",
                format!("polymarket data fetch positions: {e:#}"),
            )
        })?;
    if !resp.status().is_success() {
        return Err(ToolError::new(
            "upstream_error",
            format!("polymarket data http {}", resp.status()),
        ));
    }
    resp.json().await.map_err(|e| {
        ToolError::new(
            "upstream_error",
            format!("decode polymarket positions json: {e:#}"),
        )
    })
}

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &mut SharedState,
    _conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let chain = args
        .get("chain")
        .and_then(Value::as_str)
        .unwrap_or("polygon")
        .trim()
        .to_owned();
    let protocol = args
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("polymarket")
        .trim()
        .to_owned();

    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let address = {
        let a = evm_addr_for_account(&w, idx)?;
        format!("{a:?}")
    };

    let base_url = shared.cfg.http.polymarket_data_base_url.trim().to_owned();
    if base_url.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "polymarket_not_configured",
                "polymarket_data_base_url is empty (configure http.polymarket_data_base_url)",
            )),
        ));
    }
    if let Err(te) = ensure_https_or_loopback(&base_url, "polymarket_data_base_url") {
        return Ok(ok(req_id, tool_err(te)));
    }

    let v = match fetch_polymarket_positions(&base_url, &address).await {
        Ok(v) => v,
        Err(te) => return Ok(ok(req_id, tool_err(te))),
    };

    // Best-effort: persist latest snapshot for position monitoring.
    shared.ensure_db().await;
    if let Some(db) = shared.db() {
        if let Ok(now_ms) = crate::db::Db::now_ms() {
            let json_s = json!({
                "chain": chain, "protocol": protocol, "address": address,
                "source": "polymarket_data_api", "positions": v
            })
            .to_string();
            if !json_s.is_empty() {
                drop(
                    db.upsert_health_snapshot(&crate::db::HealthSnapshotInput {
                        surface: "prediction",
                        chain: chain.as_str(),
                        provider: protocol.as_str(),
                        wallet: w.name.as_str(),
                        account_index: i64::from(idx),
                        fetched_at_ms: now_ms,
                        payload_json: &json_s,
                    })
                    .await,
                );
            }
        }
    }

    Ok(ok(
        req_id,
        tool_ok(json!({
            "chain": chain, "protocol": protocol, "address": address,
            "source": "polymarket_data_api", "positions": v
        })),
    ))
}
