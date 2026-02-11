use crate::errors::ToolError;
use eyre::Context as _;
use reqwest::Client;
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};

struct YieldFilters {
    chains: Vec<String>,
    query: String,
    min_tvl_usd: f64,
    min_apy: f64,
    stablecoin_only: bool,
    exclude_il_risk: bool,
    max_results: usize,
}

fn parse_yield_filters(args: &Value) -> YieldFilters {
    YieldFilters {
        chains: args
            .get("chains")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(std::borrow::ToOwned::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        query: args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_owned(),
        min_tvl_usd: args
            .get("min_tvl_usd")
            .and_then(Value::as_f64)
            .unwrap_or(10_000_000.0_f64),
        min_apy: args
            .get("min_apy")
            .and_then(Value::as_f64)
            .unwrap_or(0.0_f64),
        stablecoin_only: args
            .get("stablecoin_only")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        exclude_il_risk: args
            .get("exclude_il_risk")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        max_results: args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100)
            .try_into()
            .unwrap_or(100),
    }
}

fn filter_and_sort_pools(data: Vec<Value>, f: &YieldFilters) -> Vec<Value> {
    let q = f.query.to_ascii_lowercase();
    let chain_set = if f.chains.is_empty() {
        None
    } else {
        Some(
            f.chains
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .collect::<std::collections::BTreeSet<_>>(),
        )
    };

    let mut out: Vec<Value> = vec![];
    for p in data {
        let chain = p.get("chain").and_then(|x| x.as_str()).unwrap_or("");
        let project = p.get("project").and_then(|x| x.as_str()).unwrap_or("");
        let symbol = p.get("symbol").and_then(|x| x.as_str()).unwrap_or("");
        let tvl = p.get("tvlUsd").and_then(Value::as_f64).unwrap_or(0.0_f64);
        let apy = p.get("apy").and_then(Value::as_f64).unwrap_or(0.0_f64);

        if tvl < f.min_tvl_usd || apy < f.min_apy {
            continue;
        }
        if let Some(set) = &chain_set {
            if !set.contains(&chain.to_ascii_lowercase()) {
                continue;
            }
        }
        if f.stablecoin_only
            && !p
                .get("stablecoin")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            continue;
        }
        if f.exclude_il_risk && p.get("ilRisk").and_then(|x| x.as_str()).is_some() {
            continue;
        }
        if !q.is_empty() {
            let hay = format!("{project} {symbol} {chain}").to_ascii_lowercase();
            if !hay.contains(&q) {
                continue;
            }
        }

        out.push(json!({
          "project": project,
          "chain": chain,
          "symbol": symbol,
          "tvl_usd": tvl,
          "apy": apy,
          "apy_base": p.get("apyBase").and_then(Value::as_f64),
          "apy_reward": p.get("apyReward").and_then(Value::as_f64),
          "stablecoin": p.get("stablecoin").and_then(Value::as_bool).unwrap_or(false),
          "url": p.get("url").and_then(|x| x.as_str()),
          "pool_id": p.get("pool").and_then(|x| x.as_str()),
        }));
    }

    out.sort_by(|a, b| {
        let aa = a.get("apy").and_then(Value::as_f64).unwrap_or(0.0_f64);
        let bb = b.get("apy").and_then(Value::as_f64).unwrap_or(0.0_f64);
        bb.partial_cmp(&aa).unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(f.max_results);
    out
}

pub async fn handle(req_id: Value, args: Value) -> eyre::Result<JsonRpcResponse> {
    let filters = parse_yield_filters(&args);

    // Public, keyless endpoint. Keep as a constant default to avoid env var reliance.
    let url = "https://yields.llama.fi/pools";

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("build http client")?;
    let resp = client
        .get(url)
        .send()
        .await
        .context("fetch yields dataset")?;
    if !resp.status().is_success() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "upstream_error",
                format!("yields upstream returned HTTP {}", resp.status()),
            )),
        ));
    }
    let v: Value = resp.json().await.context("parse yields json")?;
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let out = filter_and_sort_pools(data, &filters);

    Ok(ok(
        req_id,
        tool_ok(json!({
          "source": url,
          "count": out.len(),
          "pools": out
        })),
    ))
}
