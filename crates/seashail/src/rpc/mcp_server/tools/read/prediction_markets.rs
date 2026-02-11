use serde_json::{json, Value};

use crate::errors::ToolError;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::{ConnState, SharedState};

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn arg_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn arg_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
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

fn parse_u256_decimal_str(s: &str) -> Result<polymarket_client_sdk::types::U256, ToolError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(ToolError::new("invalid_request", "missing token_id"));
    }
    let (digits, radix) = if let Some(rest) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X"))
    {
        (rest, 16)
    } else {
        (t, 10)
    };
    polymarket_client_sdk::types::U256::from_str_radix(digits, radix)
        .map_err(|e| ToolError::new("invalid_request", format!("invalid token_id {s:?}: {e}")))
}

fn coerce_vec_str(v: &Value) -> Vec<String> {
    // Gamma sometimes returns fields as either JSON arrays or JSON-encoded strings.
    // We accept:
    // - ["a","b"]
    // - "[\"a\",\"b\"]"
    // - "a,b"
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(ToOwned::to_owned)
            .collect();
    }
    let Some(s) = v.as_str() else {
        return vec![];
    };
    let t = s.trim();
    if t.is_empty() {
        return vec![];
    }
    if t.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Value>(t) {
            if let Some(arr) = parsed.as_array() {
                return arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            }
        }
    }
    t.trim_matches(|c| c == '[' || c == ']' || c == '"')
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| x.trim_matches('"'))
        .map(ToOwned::to_owned)
        .collect()
}

fn normalize_search_results(v: &Value) -> Value {
    let mut markets_flat: Vec<Value> = vec![];
    let Some(events) = v.get("events").and_then(Value::as_array) else {
        return json!({ "raw": v, "markets": markets_flat });
    };
    for ev in events {
        let ev_title = ev.get("title").and_then(Value::as_str).unwrap_or("").trim();
        let ev_slug = ev.get("slug").and_then(Value::as_str).unwrap_or("").trim();
        let ev_id = ev.get("id").and_then(Value::as_str).unwrap_or("").trim();

        let Some(markets) = ev.get("markets").and_then(Value::as_array) else {
            continue;
        };
        for m in markets {
            let question = m
                .get("question")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let condition_id = m
                .get("conditionId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let slug = m.get("slug").and_then(Value::as_str).unwrap_or("").trim();

            let outcomes = m.get("outcomes").map(coerce_vec_str).unwrap_or_default();
            let token_ids = m
                .get("clobTokenIds")
                .map(coerce_vec_str)
                .unwrap_or_default();
            let prices = m
                .get("outcomePrices")
                .map(coerce_vec_str)
                .unwrap_or_default();

            let mut outcome_tokens: Vec<Value> = vec![];
            let n = outcomes.len().max(token_ids.len()).max(prices.len());
            for i in 0..n {
                outcome_tokens.push(json!({
                  "outcome": outcomes.get(i).cloned(),
                  "token_id": token_ids.get(i).cloned(),
                  "price": prices.get(i).cloned()
                }));
            }

            markets_flat.push(json!({
              "event": { "id": ev_id, "title": ev_title, "slug": ev_slug },
              "market": {
                "question": question,
                "slug": slug,
                "condition_id": condition_id,
                "outcomes": outcomes,
                "clob_token_ids": token_ids,
                "outcome_prices": prices,
                "outcome_tokens": outcome_tokens
              }
            }));
        }
    }
    json!({ "raw": v, "markets": markets_flat })
}

pub async fn handle(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &SharedState,
    _conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let chain = arg_str(&args, "chain").unwrap_or("polygon");
    let protocol = arg_str(&args, "protocol").unwrap_or("polymarket");
    if protocol != "polymarket" {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "protocol must be polymarket",
            )),
        ));
    }
    if chain != "polygon" && chain != "polygon-amoy" && chain != "amoy" {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "chain must be polygon (or polygon-amoy for testing)",
            )),
        ));
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(3_000))
        .build()
    {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "upstream_error",
                    format!("build http client: {e:#}"),
                )),
            ));
        }
    };

    match tool_name {
        "search_prediction_markets" => handle_search(req_id, &args, chain, &client, shared).await,
        "get_prediction_orderbook" => handle_orderbook(req_id, &args, chain, &client, shared).await,
        _ => Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "unknown tool")),
        )),
    }
}

async fn handle_search(
    req_id: Value,
    args: &Value,
    chain: &str,
    client: &reqwest::Client,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let Some(q) = arg_str(args, "query") else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing query")),
        ));
    };
    let limit = arg_u64(args, "limit").unwrap_or(10).clamp(1, 100);
    let page = arg_u64(args, "page").unwrap_or(1).max(1);
    let include_closed = arg_bool(args, "include_closed").unwrap_or(false);

    let base = shared.cfg.http.polymarket_gamma_base_url.trim().to_owned();
    if base.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "polymarket_not_configured",
                "polymarket_gamma_base_url is empty (configure http.polymarket_gamma_base_url)",
            )),
        ));
    }
    if let Err(te) = ensure_https_or_loopback(&base, "polymarket_gamma_base_url") {
        return Ok(ok(req_id, tool_err(te)));
    }
    let url = format!("{}/public-search", base.trim_end_matches('/'));

    let params: Vec<(String, String)> = vec![
        ("q".to_owned(), q.to_owned()),
        ("limit_per_type".to_owned(), limit.to_string()),
        ("page".to_owned(), page.to_string()),
        ("cache".to_owned(), "true".to_owned()),
        ("optimized".to_owned(), "true".to_owned()),
        ("search_tags".to_owned(), "false".to_owned()),
        ("search_profiles".to_owned(), "false".to_owned()),
        (
            "keep_closed_markets".to_owned(),
            if include_closed {
                "true".to_owned()
            } else {
                "false".to_owned()
            },
        ),
    ];
    let resp = match client.get(url).query(&params).send().await {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "upstream_error",
                    format!("polymarket gamma search: {e:#}"),
                )),
            ));
        }
    };
    if !resp.status().is_success() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "upstream_error",
                format!("polymarket gamma http {}", resp.status()),
            )),
        ));
    }
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "upstream_error",
                    format!("decode polymarket gamma json: {e:#}"),
                )),
            ));
        }
    };

    let normalized = normalize_search_results(&v);
    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": chain,
          "protocol": "polymarket",
          "source": "polymarket_gamma_api",
          "query": q,
          "page": page,
          "limit": limit,
          "include_closed": include_closed,
          "results": normalized,
        })),
    ))
}

async fn handle_orderbook(
    req_id: Value,
    args: &Value,
    chain: &str,
    client: &reqwest::Client,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let Some(token_id) = arg_str(args, "token_id") else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing token_id")),
        ));
    };
    let base = shared.cfg.http.polymarket_clob_base_url.trim().to_owned();
    if base.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "polymarket_not_configured",
                "polymarket_clob_base_url is empty (configure http.polymarket_clob_base_url)",
            )),
        ));
    }
    if let Err(te) = ensure_https_or_loopback(&base, "polymarket_clob_base_url") {
        return Ok(ok(req_id, tool_err(te)));
    }
    let url = format!("{}/book", base.trim_end_matches('/'));

    let token_id_u256 = match parse_u256_decimal_str(token_id) {
        Ok(v) => v,
        Err(te) => return Ok(ok(req_id, tool_err(te))),
    };
    let token_id_dec = token_id_u256.to_string();

    let resp = match client
        .get(url)
        .query(&[("token_id", token_id_dec.as_str())])
        .send()
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "upstream_error",
                    format!("polymarket clob orderbook: {e:#}"),
                )),
            ));
        }
    };
    if !resp.status().is_success() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "upstream_error",
                format!("polymarket clob http {}", resp.status()),
            )),
        ));
    }
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "upstream_error",
                    format!("decode polymarket clob json: {e:#}"),
                )),
            ));
        }
    };

    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": chain,
          "protocol": "polymarket",
          "source": "polymarket_clob_api",
          "token_id": token_id_dec,
          "orderbook": v
        })),
    ))
}
