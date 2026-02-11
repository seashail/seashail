use eyre::Context as _;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

use crate::errors::ToolError;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::SharedState;
use super::super::value_helpers::defi_adapter_fetch;

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

async fn wormholescan_signed_vaa_b64(
    base_url: &str,
    src_chain_id: u16,
    emitter: &str,
    sequence: u64,
) -> eyre::Result<Option<String>> {
    let base = base_url.trim().trim_end_matches('/');
    if !base.starts_with("https://") && !is_loopback_http(base) {
        eyre::bail!("wormholescan_api_base_url must use https (or loopback for local testing)");
    }
    let url = format!("{base}/signed_vaa/{src_chain_id}/{emitter}/{sequence}");
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
    Ok(v.get("vaaBytes")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned))
}

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &mut SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let bridge_id = args
        .get("bridge_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if bridge_id.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing bridge_id")),
        ));
    }

    let provider = args
        .get("bridge_provider")
        .and_then(Value::as_str)
        .unwrap_or("wormhole")
        .trim()
        .to_owned();

    if provider == "wormhole" {
        return handle_wormhole(req_id, &args, &bridge_id, &provider, shared).await;
    }

    let v = match defi_adapter_fetch(
        shared.cfg.http.defi_adapter_base_url.as_ref(),
        "bridge/status",
        &[
            ("provider", provider.as_str()),
            ("bridge_id", bridge_id.as_str()),
        ],
    )
    .await
    {
        Ok(v) => v,
        Err(te) => return Ok(ok(req_id, tool_err(te))),
    };

    Ok(ok(
        req_id,
        tool_ok(json!({
          "bridge_id": bridge_id,
          "bridge_provider": provider,
          "source": "defi_adapter",
          "status": v
        })),
    ))
}

async fn handle_wormhole(
    req_id: Value,
    args: &Value,
    bridge_id: &str,
    provider: &str,
    shared: &mut SharedState,
) -> eyre::Result<JsonRpcResponse> {
    // Expected: wormhole:<src_chain_id>:<emitter_hex_64>:<sequence>
    let mut it = bridge_id.split(':');
    let head = it.next().unwrap_or("");
    let src_s = it.next().unwrap_or("");
    let emitter = it.next().unwrap_or("").trim();
    let seq_s = it.next().unwrap_or("");
    if head != "wormhole" || emitter.is_empty() || it.next().is_some() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "wormhole bridge_id must be formatted as wormhole:<src_chain_id>:<emitter_hex>:<sequence>",
            )),
        ));
    }
    let src_chain_id: u16 = match src_s.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("invalid_request", "invalid src_chain_id")),
            ));
        }
    };
    let sequence: u64 = match seq_s.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("invalid_request", "invalid sequence")),
            ));
        }
    };

    let include_vaa_bytes = args
        .get("include_vaa_bytes")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    shared.ensure_db().await;
    let cache_key = format!("bridge:wormhole:status:{bridge_id}");

    let mut last_err: Option<eyre::Report> = None;
    for _ in 0..3_u32 {
        match wormholescan_signed_vaa_b64(
            &shared.cfg.http.wormholescan_api_base_url,
            src_chain_id,
            emitter,
            sequence,
        )
        .await
        {
            Ok(vaa_opt) => {
                let status = json!({
                  "bridge_id": bridge_id,
                  "bridge_provider": provider,
                  "source": "wormholescan",
                  "wormhole": {
                    "source_chain_id": src_chain_id,
                    "emitter": emitter,
                    "sequence": sequence,
                    "vaa_available": vaa_opt.is_some(),
                    "vaa_bytes_b64": if include_vaa_bytes { vaa_opt } else { None }
                  }
                });
                if let Some(db) = shared.db() {
                    if let Ok(now) = crate::db::Db::now_ms() {
                        let stale_at = now.saturating_add(5_000);
                        if let Err(_e) = db
                            .upsert_json(&cache_key, &status.to_string(), now, stale_at)
                            .await
                        {
                            // Best-effort cache; ignore failures.
                        }
                    }
                }
                return Ok(ok(req_id, tool_ok(status)));
            }
            Err(e) => {
                last_err = Some(e);
                sleep(Duration::from_millis(200)).await;
            }
        }
    }

    // Best-effort cache fallback.
    if let Some(db) = shared.db() {
        if let Ok(now) = crate::db::Db::now_ms() {
            if let Ok(Some(row)) = db.get_json_if_fresh(&cache_key, now).await {
                if let Ok(v) = serde_json::from_str::<Value>(&row.json) {
                    return Ok(ok(req_id, tool_ok(v)));
                }
            }
        }
    }

    Ok(ok(
        req_id,
        tool_err(ToolError::new(
            "wormholescan_error",
            format!(
                "wormholescan status fetch failed: {}",
                last_err.map_or_else(|| "unknown error".to_owned(), |e| format!("{e:#}"))
            ),
        )),
    ))
}
