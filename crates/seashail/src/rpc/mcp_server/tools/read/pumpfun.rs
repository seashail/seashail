use crate::errors::ToolError;
use base64::Engine as _;
use borsh::BorshDeserialize;
use eyre::Context as _;
use serde_json::{json, Value};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr as _;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::SharedState;

// pump.fun program id (mainnet). Used only for RPC-only discovery mode.
const PUMPFUN_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

// pump.fun PDA seeds (see pumpfun crate constants).
const PUMPFUN_BONDING_CURVE_SEED: &[u8] = b"bonding-curve";

#[derive(Debug, Clone)]
struct PumpfunFixtureItem {
    mint: String,
    bonding_curve: String,
    curve: Value,
}

fn load_fixture_items() -> Option<Vec<PumpfunFixtureItem>> {
    // CI-safe determinism: allow tests to inject a fixed pump.fun "recent coins" list.
    //
    // This is honored only in debug builds to avoid accidentally depending on fixtures in
    // release artifacts.
    if !cfg!(debug_assertions) {
        return None;
    }
    let s = std::env::var("SEASHAIL_PUMPFUN_DISCOVERY_FIXTURE_JSON")
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())?;
    let v: Value = serde_json::from_str(&s).ok()?;
    let arr = v.get("items").and_then(Value::as_array)?;
    let mut out: Vec<PumpfunFixtureItem> = vec![];
    for it in arr {
        let mint = it.get("mint").and_then(Value::as_str)?.trim().to_owned();
        let bonding_curve = it
            .get("bonding_curve")
            .and_then(Value::as_str)?
            .trim()
            .to_owned();
        let curve = it.get("curve").cloned().unwrap_or(Value::Null);

        // Keep tool invariants: fixture pubkeys should still parse.
        if Pubkey::from_str(mint.as_str()).is_err() {
            return None;
        }
        if Pubkey::from_str(bonding_curve.as_str()).is_err() {
            return None;
        }

        out.push(PumpfunFixtureItem {
            mint,
            bonding_curve,
            curve,
        });
    }
    Some(out)
}

#[derive(Debug, Clone, BorshDeserialize)]
struct PumpfunBondingCurveAccount {
    pub discriminator: u64,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    pub creator: Pubkey,
}

fn decode_pumpfun_curve(bytes: &[u8]) -> Option<PumpfunBondingCurveAccount> {
    // Some pump.fun account layouts may append additional fields over time. Using
    // `deserialize` (not `try_from_slice`) lets us decode the prefix we care about
    // without failing on trailing bytes.
    let mut s = bytes;
    PumpfunBondingCurveAccount::deserialize(&mut s).ok()
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

fn ensure_https_or_loopback(url: &str, name: &str) -> eyre::Result<()> {
    let u = url.trim();
    if u.starts_with("https://") || is_loopback_http(u) {
        return Ok(());
    }
    eyre::bail!("{name} must use https (or http://localhost for local testing)");
}

fn solana_rpc_urls(shared: &SharedState) -> Vec<String> {
    let mut out = vec![];
    let primary = shared.cfg.rpc.solana_rpc_url.trim();
    if !primary.is_empty() {
        out.push(primary.to_owned());
    }

    match shared.cfg.effective_network_mode() {
        crate::config::NetworkMode::Mainnet => {
            for u in &shared.cfg.rpc.solana_fallback_rpc_urls_mainnet {
                let u = u.trim();
                if !u.is_empty() && !out.iter().any(|x| x == u) {
                    out.push(u.to_owned());
                }
            }
        }
        crate::config::NetworkMode::Testnet => {
            // Solana uses a "devnet" public cluster for non-mainnet testing.
            for u in &shared.cfg.rpc.solana_fallback_rpc_urls_devnet {
                let u = u.trim();
                if !u.is_empty() && !out.iter().any(|x| x == u) {
                    out.push(u.to_owned());
                }
            }
        }
    }

    out
}

async fn solana_rpc_call(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
) -> eyre::Result<Value> {
    ensure_https_or_loopback(rpc_url, "solana rpc url")?;
    let resp = client
        .post(rpc_url.trim())
        .json(&json!({
          "jsonrpc": "2.0",
          "id": 1_i64,
          "method": method,
          "params": params,
        }))
        .send()
        .await
        .context("solana rpc call")?;
    let v: Value = resp.json().await.context("decode solana rpc json")?;
    if let Some(err) = v.get("error") {
        eyre::bail!("solana rpc error: {err}");
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

async fn solana_rpc_call_any(
    client: &reqwest::Client,
    rpc_urls: &[String],
    method: &str,
    params: Value,
) -> eyre::Result<Value> {
    let mut last: Option<eyre::Report> = None;
    for u in rpc_urls {
        match solana_rpc_call(client, u, method, params.clone()).await {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| eyre::eyre!("no solana rpc urls configured")))
}

fn pumpfun_bonding_curve_pda(program_id: &str, mint: &str) -> eyre::Result<Pubkey> {
    let program = Pubkey::from_str(program_id.trim()).context("parse pumpfun program id")?;
    let mint = Pubkey::from_str(mint.trim()).context("parse mint pubkey")?;
    let (pda, _bump) =
        Pubkey::find_program_address(&[PUMPFUN_BONDING_CURVE_SEED, mint.as_ref()], &program);
    Ok(pda)
}

fn decode_account_info_bytes(result: &Value) -> Option<(String, Vec<u8>)> {
    // result is the JSON "result" body from getAccountInfo.
    let value = result.get("value")?;
    let owner = value.get("owner")?.as_str()?.trim().to_owned();
    let data = value.get("data")?;
    let arr = data.as_array()?;
    let b64 = arr.first()?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some((owner, bytes))
}

async fn handle_list_new_coins(
    req_id: Value,
    args: &Value,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 200);

    if let Some(items) = load_fixture_items() {
        let out: Vec<Value> = items
            .into_iter()
            .take(limit as usize)
            .map(|it| {
                json!({
                  "mint": it.mint,
                  "bonding_curve": it.bonding_curve,
                  "curve": it.curve
                })
            })
            .collect();
        return Ok(ok(
            req_id,
            tool_ok(json!({
              "source": "rpc",
              "fixture": true,
              "limit": limit,
              "program_id": PUMPFUN_PROGRAM_ID,
              "items": out
            })),
        ));
    }

    // Adapter path (preferred when configured).
    if let Some(base) = shared.cfg.http.pumpfun_adapter_base_url.as_deref() {
        ensure_https_or_loopback(base, "pumpfun_adapter_base_url")?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1600))
            .build()
            .context("build http client")?;
        let url = format!(
            "{}/pumpfun/new-coins?limit={limit}",
            base.trim_end_matches('/')
        );
        let resp = client
            .get(url)
            .header("accept", "application/json")
            .send()
            .await
            .context("fetch pumpfun new coins")?;
        if !resp.status().is_success() {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "pumpfun_http_error",
                    format!("pumpfun adapter http {}", resp.status()),
                )),
            ));
        }
        let v: Value = resp.json().await.context("decode pumpfun json")?;
        return Ok(ok(
            req_id,
            tool_ok(json!({ "source": "adapter", "items": v })),
        ));
    }

    // RPC-only fallback: best-effort discovery from recent pump.fun program transactions.
    let rpc_urls = solana_rpc_urls(shared);
    if rpc_urls.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "pumpfun_not_configured",
                "pump.fun adapter is not configured and solana rpc url is empty",
            )),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(2_500))
        .build()
        .context("build http client")?;

    let program_id = args
        .get("program_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(PUMPFUN_PROGRAM_ID);

    // Pull more signatures than requested since not every tx yields a mint.
    let sig_limit: u64 = limit.saturating_mul(25).clamp(25, 2000);
    let sigs = solana_rpc_call_any(
        &client,
        &rpc_urls,
        "getSignaturesForAddress",
        json!([program_id, { "limit": sig_limit }]),
    )
    .await
    .unwrap_or(Value::Array(vec![]));

    let mut items: Vec<Value> = vec![];
    let mut seen = std::collections::BTreeSet::new();
    if let Some(arr) = sigs.as_array() {
        for s in arr {
            if items.len() >= limit as usize {
                break;
            }
            let sig = s
                .get("signature")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("");
            if sig.is_empty() {
                continue;
            }
            let slot = s.get("slot").and_then(Value::as_i64);
            let block_time = s.get("blockTime").and_then(Value::as_i64);

            let tx = solana_rpc_call_any(
                &client,
                &rpc_urls,
                "getTransaction",
                json!([sig, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0_i64 }]),
            )
            .await
            .unwrap_or(Value::Null);
            let mints: Vec<String> = tx
                .get("meta")
                .and_then(|m| m.get("postTokenBalances"))
                .and_then(Value::as_array)
                .map(|bals| {
                    bals.iter()
                        .filter_map(|b| b.get("mint").and_then(Value::as_str))
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(ToOwned::to_owned)
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();

            for mint in mints {
                if items.len() >= limit as usize {
                    break;
                }
                if !seen.insert(mint.clone()) {
                    continue;
                }
                // High precision: verify the mint is a pump.fun coin by checking that the
                // bonding-curve PDA exists and is owned by the pump.fun program, then decode it.
                let Ok(curve_pda) = pumpfun_bonding_curve_pda(program_id, &mint) else {
                    continue;
                };
                let curve_ai = solana_rpc_call_any(
                    &client,
                    &rpc_urls,
                    "getAccountInfo",
                    json!([curve_pda.to_string(), { "encoding": "base64" }]),
                )
                .await
                .unwrap_or(Value::Null);
                let Some((owner, curve_bytes)) = decode_account_info_bytes(&curve_ai) else {
                    continue;
                };
                if owner.trim() != program_id {
                    continue;
                }
                let Some(curve) = decode_pumpfun_curve(&curve_bytes) else {
                    continue;
                };

                items.push(json!({
                  "mint": mint,
                  "bonding_curve": curve_pda.to_string(),
                  "signature": sig,
                  "slot": slot,
                  "block_time": block_time,
                  "curve": {
                    "discriminator": curve.discriminator.to_string(),
                    "complete": curve.complete,
                    "creator": curve.creator.to_string(),
                    "virtual_token_reserves": curve.virtual_token_reserves.to_string(),
                    "virtual_sol_reserves": curve.virtual_sol_reserves.to_string(),
                    "real_token_reserves": curve.real_token_reserves.to_string(),
                    "real_sol_reserves": curve.real_sol_reserves.to_string(),
                    "token_total_supply": curve.token_total_supply.to_string()
                  }
                }));
            }
        }
    }

    Ok(ok(
        req_id,
        tool_ok(
            json!({ "source": "rpc", "limit": limit, "program_id": program_id, "items": items }),
        ),
    ))
}

async fn handle_get_coin_info(
    req_id: Value,
    args: &Value,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    if let Some(items) = load_fixture_items() {
        let mint = args
            .get("mint")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if mint.is_empty() {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("invalid_request", "missing mint")),
            ));
        }
        if let Some(it) = items.into_iter().find(|it| it.mint == mint) {
            return Ok(ok(
                req_id,
                tool_ok(json!({
                  "source": "rpc",
                  "fixture": true,
                  "program_id": PUMPFUN_PROGRAM_ID,
                  "mint": it.mint,
                  "bonding_curve": it.bonding_curve,
                  "curve": it.curve
                })),
            ));
        }
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "pumpfun_coin_not_found",
                "coin not found (fixture)",
            )),
        ));
    }

    let mint = args.get("mint").and_then(|v| v.as_str()).unwrap_or("");
    if mint.trim().is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing mint")),
        ));
    }

    // Adapter path (preferred when configured).
    if let Some(base) = shared.cfg.http.pumpfun_adapter_base_url.as_deref() {
        ensure_https_or_loopback(base, "pumpfun_adapter_base_url")?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1600))
            .build()
            .context("build http client")?;
        let url = format!(
            "{}/pumpfun/coin-info?mint={}",
            base.trim_end_matches('/'),
            mint.trim()
        );
        let resp = client
            .get(url)
            .header("accept", "application/json")
            .send()
            .await
            .context("fetch pumpfun coin info")?;
        if !resp.status().is_success() {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "pumpfun_http_error",
                    format!("pumpfun adapter http {}", resp.status()),
                )),
            ));
        }
        let v: Value = resp.json().await.context("decode pumpfun json")?;
        // Preserve the adapter payload while tagging the source.
        if let Some(obj) = v.as_object() {
            let mut o = obj.clone();
            o.insert("source".to_owned(), Value::String("adapter".to_owned()));
            return Ok(ok(req_id, tool_ok(Value::Object(o))));
        }
        return Ok(ok(
            req_id,
            tool_ok(json!({ "source": "adapter", "info": v })),
        ));
    }

    // RPC fallback: high precision pump.fun coin info from the bonding-curve PDA.
    let rpc_urls = solana_rpc_urls(shared);
    if rpc_urls.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "pumpfun_not_configured",
                "pump.fun adapter is not configured and solana rpc url is empty",
            )),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(2_500))
        .build()
        .context("build http client")?;

    let program_id = args
        .get("program_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(PUMPFUN_PROGRAM_ID);

    let Ok(curve_pda) = pumpfun_bonding_curve_pda(program_id, mint.trim()) else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "invalid mint/program_id")),
        ));
    };
    let curve_ai = solana_rpc_call_any(
        &client,
        &rpc_urls,
        "getAccountInfo",
        json!([curve_pda.to_string(), { "encoding": "base64" }]),
    )
    .await
    .unwrap_or(Value::Null);
    let Some((owner, curve_bytes)) = decode_account_info_bytes(&curve_ai) else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "pumpfun_coin_not_found",
                "no pump.fun bonding curve account found for this mint",
            )),
        ));
    };
    if owner.trim() != program_id {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "pumpfun_coin_not_found",
                "bonding curve PDA exists but is not owned by the pump.fun program",
            )),
        ));
    }
    let Some(curve) = decode_pumpfun_curve(&curve_bytes) else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "pumpfun_coin_not_found",
                "bonding curve PDA exists but could not be decoded",
            )),
        ));
    };

    Ok(ok(
        req_id,
        tool_ok(json!({
          "source": "rpc",
          "program_id": program_id,
          "mint": mint.trim(),
          "bonding_curve": curve_pda.to_string(),
          "curve": {
            "discriminator": curve.discriminator.to_string(),
            "complete": curve.complete,
            "creator": curve.creator.to_string(),
            "virtual_token_reserves": curve.virtual_token_reserves.to_string(),
            "virtual_sol_reserves": curve.virtual_sol_reserves.to_string(),
            "real_token_reserves": curve.real_token_reserves.to_string(),
            "real_sol_reserves": curve.real_sol_reserves.to_string(),
            "token_total_supply": curve.token_total_supply.to_string()
          }
        })),
    ))
}

pub async fn handle(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    match tool_name {
        "pumpfun_list_new_coins" => handle_list_new_coins(req_id, &args, shared).await,
        "pumpfun_get_coin_info" => handle_get_coin_info(req_id, &args, shared).await,
        _ => Ok(ok(
            req_id,
            tool_err(ToolError::new("unknown_tool", "unknown tool")),
        )),
    }
}
