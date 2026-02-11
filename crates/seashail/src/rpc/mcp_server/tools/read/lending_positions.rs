use alloy::sol;
use carbon_core::deserialize::CarbonDeserialize as _;
use eyre::Context as _;
use serde_json::{json, Value};

use crate::{chains::evm::EvmChain, errors::ToolError};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, resolve_wallet_and_account, sol_pubkey_for_account, solana_fallback_urls,
};
use super::super::value_helpers::defi_adapter_fetch;
use crate::chains::solana::SolanaChain;

async fn persist_lending_snapshot(
    shared: &mut SharedState,
    chain: &str,
    protocol: &str,
    wallet: &str,
    idx: u32,
    payload: &Value,
) {
    shared.ensure_db().await;
    if let Some(db) = shared.db() {
        if let Ok(now_ms) = crate::db::Db::now_ms() {
            let json_s = payload.to_string();
            if !json_s.is_empty() {
                drop(
                    db.upsert_health_snapshot(&crate::db::HealthSnapshotInput {
                        surface: "lending",
                        chain,
                        provider: protocol,
                        wallet,
                        account_index: i64::from(idx),
                        fetched_at_ms: now_ms,
                        payload_json: &json_s,
                    })
                    .await,
                );
            }
        }
    }
}

sol! {
    #[sol(rpc)]
    contract IAavePoolV3Read {
        function getUserAccountData(address) external view returns (uint256, uint256, uint256, uint256, uint256, uint256);
    }
}

sol! {
    #[sol(rpc)]
    contract ICometV3Read {
        function baseToken() external view returns (address);
        function borrowBalanceOf(address) external view returns (uint256);
    }
}

/// Bundled parameters for per-protocol lending handlers, avoiding long argument lists.
struct LendingParams<'a> {
    req_id: Value,
    args: &'a Value,
    chain: &'a str,
    protocol: &'a str,
    address: &'a str,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    shared: &'a mut SharedState,
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn default_aave_pool_for_chain(chain: &str) -> Option<&'static str> {
    match chain.trim() {
        "ethereum" => Some("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"),
        "base" => Some("0xA238Dd80C259a72e81d7e4664a9801593F98d1c5"),
        "arbitrum" | "optimism" | "polygon" => Some("0x794a61358D6845594F94dc1DB02A252b5b4814aD"),
        _ => None,
    }
}

fn default_comet_for_chain(chain: &str) -> Option<&'static str> {
    // Compound v3 Comet addresses (USDC markets). Source: compound-finance/comet deployments.
    match chain.trim() {
        "ethereum" => Some("0xc3d688B66703497DAA19211EEdff47f25384cdc3"),
        "base" => Some("0xb125E6687d4313864e53df431d5425969c15Eb2F"),
        "arbitrum" => Some("0x9c4ec768c28520B50860ea7a15bd7213a9fF58bf"),
        "optimism" => Some("0x2e44e174f7D53F0212823acC11C01A11d58c5bCB"),
        "polygon" => Some("0xF25212E676D1F7F89Cd72fFEe66158f541246445"),
        // Testnets
        "sepolia" => Some("0xAec1F48e02Cfb822Be958B68C7957156EB3F0b6e"),
        _ => None,
    }
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

/// Try reading a cached JSON value from the DB if available and fresh.
async fn try_cached_value(shared: &SharedState, cache_key: &str) -> Option<Value> {
    let db = shared.db()?;
    let now = crate::db::Db::now_ms().ok()?;
    let row = db.get_json_if_fresh(cache_key, now).await.ok().flatten()?;
    serde_json::from_str::<Value>(&row.json).ok()
}

/// Resolve the protocol string, defaulting based on chain.
fn resolve_protocol(args: &Value, chain: &str) -> String {
    let raw = args
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .trim()
        .to_owned();
    if raw == "auto" {
        if chain == "solana" {
            "kamino".to_owned()
        } else {
            "aave".to_owned()
        }
    } else {
        raw
    }
}

/// Resolve the user address for a given chain.
fn resolve_lending_address(
    chain: &str,
    w: &crate::wallet::WalletRecord,
    idx: u32,
) -> Result<String, ToolError> {
    if chain == "solana" {
        let pk = sol_pubkey_for_account(w, idx)
            .map_err(|e| ToolError::new("invalid_request", e.to_string()))?;
        Ok(pk.to_string())
    } else if chain == "bitcoin" {
        Err(ToolError::new(
            "invalid_request",
            "bitcoin is not supported for lending positions",
        ))
    } else {
        let a = evm_addr_for_account(w, idx)
            .map_err(|e| ToolError::new("invalid_request", e.to_string()))?;
        Ok(format!("{a:?}"))
    }
}

/// Dispatch to the appropriate protocol-specific handler.
async fn dispatch_lending(p: LendingParams<'_>, conn: &ConnState) -> eyre::Result<JsonRpcResponse> {
    let chain = p.chain;
    let protocol = p.protocol;
    let address = p.address;

    if chain == "solana" && protocol == "kamino" {
        return handle_kamino(p).await;
    }
    if chain == "solana" && protocol == "marginfi" {
        return handle_marginfi(p, conn).await;
    }
    if chain != "solana" && chain != "bitcoin" && protocol == "aave" {
        return handle_aave(p).await;
    }
    if chain != "solana" && chain != "bitcoin" && protocol == "compound" {
        return handle_compound(p).await;
    }

    let v = match defi_adapter_fetch(
        p.shared.cfg.http.defi_adapter_base_url.as_ref(),
        "lending/positions",
        &[
            ("chain", chain),
            ("protocol", protocol),
            ("address", address),
        ],
    )
    .await
    {
        Ok(v) => v,
        Err(te) => return Ok(ok(p.req_id, tool_err(te))),
    };

    let payload = json!({
        "chain": chain, "protocol": protocol, "address": address,
        "source": "defi_adapter", "positions": v
    });
    persist_lending_snapshot(
        p.shared,
        chain,
        protocol,
        p.w.name.as_str(),
        p.idx,
        &payload,
    )
    .await;
    Ok(ok(p.req_id, tool_ok(payload)))
}

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let chain = args
        .get("chain")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if chain.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing chain")),
        ));
    }
    let protocol = resolve_protocol(&args, chain);
    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let address = match resolve_lending_address(chain, &w, idx) {
        Ok(a) => a,
        Err(te) => return Ok(ok(req_id, tool_err(te))),
    };
    let params = LendingParams {
        req_id,
        args: &args,
        chain,
        protocol: &protocol,
        address: &address,
        w: &w,
        idx,
        shared,
    };
    dispatch_lending(params, conn).await
}

/// Best-effort write to the JSON cache.
async fn cache_json(shared: &SharedState, key: &str, v: &Value, ttl_ms: u64) {
    if let Some(db) = shared.db() {
        if let Ok(now) = crate::db::Db::now_ms() {
            let ttl_i64 = i64::try_from(ttl_ms).unwrap_or(i64::MAX);
            let stale_at = now.saturating_add(ttl_i64);
            let _res = db
                .upsert_json(
                    key,
                    &serde_json::to_string(v).unwrap_or_default(),
                    now,
                    stale_at,
                )
                .await;
        }
    }
}

/// Build a cached-fallback Kamino response, or `None` if there is no cache entry.
async fn kamino_cached_response(
    shared: &SharedState,
    cache_key: &str,
    chain: &str,
    protocol: &str,
    address: &str,
    market: &str,
) -> Option<Value> {
    try_cached_value(shared, cache_key).await.map(|v| {
        tool_ok(json!({
          "chain": chain,
          "protocol": protocol,
          "address": address,
          "source": "cache",
          "market": market,
          "obligations": v
        }))
    })
}

/// Validate kamino config and resolve base URL + market. Returns (`base_url`, market) or error.
fn kamino_validate_config(
    args: &Value,
    shared: &SharedState,
) -> Result<(String, String), ToolError> {
    let base_url = shared.cfg.http.kamino_api_base_url.trim().to_owned();
    if base_url.is_empty() {
        return Err(ToolError::new(
            "kamino_not_configured",
            "kamino_api_base_url is empty",
        ));
    }
    ensure_https_or_loopback(&base_url, "kamino_api_base_url")?;
    let market = arg_str(args, "market")
        .unwrap_or(shared.cfg.http.kamino_default_lend_market.as_str())
        .trim()
        .to_owned();
    if market.is_empty() {
        return Err(ToolError::new(
            "invalid_request",
            "missing market (and default kamino_default_lend_market is empty)",
        ));
    }
    Ok((base_url, market))
}

/// Context for processing a Kamino API response.
struct KaminoResponseCtx<'a> {
    shared: &'a SharedState,
    cache_key: &'a str,
    chain: &'a str,
    protocol: &'a str,
    address: &'a str,
    market: &'a str,
    wallet: &'a str,
    idx: u32,
}

/// Process the Kamino API fetch result, caching on success and falling back to cache on failure.
async fn kamino_process_response(
    fetched: Result<reqwest::Response, reqwest::Error>,
    ctx: &KaminoResponseCtx<'_>,
    req_id: Value,
) -> eyre::Result<JsonRpcResponse> {
    match fetched {
        Ok(resp) if resp.status().is_success() => {
            let v: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ok(
                        req_id,
                        tool_err(ToolError::new(
                            "kamino_read_failed",
                            format!("decode kamino json: {e:#}"),
                        )),
                    ));
                }
            };
            cache_json(ctx.shared, ctx.cache_key, &v, 30_000).await;
            if let Some(db) = ctx.shared.db() {
                if let Ok(now_ms) = crate::db::Db::now_ms() {
                    let out = json!({
                      "chain": ctx.chain,
                      "protocol": ctx.protocol,
                      "address": ctx.address,
                      "source": "kamino_api",
                      "market": ctx.market,
                      "obligations": v
                    });
                    let json_s = out.to_string();
                    if !json_s.is_empty() {
                        drop(
                            db.upsert_health_snapshot(&crate::db::HealthSnapshotInput {
                                surface: "lending",
                                chain: ctx.chain,
                                provider: ctx.protocol,
                                wallet: ctx.wallet,
                                account_index: i64::from(ctx.idx),
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
                  "chain": ctx.chain,
                  "protocol": ctx.protocol,
                  "address": ctx.address,
                  "source": "kamino_api",
                  "market": ctx.market,
                  "obligations": v
                })),
            ))
        }
        Ok(resp) => {
            if let Some(cached) = kamino_cached_response(
                ctx.shared,
                ctx.cache_key,
                ctx.chain,
                ctx.protocol,
                ctx.address,
                ctx.market,
            )
            .await
            {
                return Ok(ok(req_id, cached));
            }
            Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "kamino_read_failed",
                    format!("kamino obligations http {}", resp.status()),
                )),
            ))
        }
        Err(e) => {
            if let Some(cached) = kamino_cached_response(
                ctx.shared,
                ctx.cache_key,
                ctx.chain,
                ctx.protocol,
                ctx.address,
                ctx.market,
            )
            .await
            {
                return Ok(ok(req_id, cached));
            }
            Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "kamino_read_failed",
                    format!("kamino obligations fetch failed: {e:#}"),
                )),
            ))
        }
    }
}

async fn handle_kamino(p: LendingParams<'_>) -> eyre::Result<JsonRpcResponse> {
    let (base_url, market) = match kamino_validate_config(p.args, p.shared) {
        Ok(v) => v,
        Err(te) => return Ok(ok(p.req_id.clone(), tool_err(te))),
    };

    p.shared.ensure_db().await;
    let cache_key = format!("lending:kamino:obligations:{market}:{}", p.address);
    let url = format!(
        "{}/kamino-market/{}/users/{}/obligations",
        base_url.trim_end_matches('/'),
        market,
        p.address
    );
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(2_500))
        .build()
    {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                p.req_id,
                tool_err(ToolError::new(
                    "upstream_error",
                    format!("build http client: {e:#}"),
                )),
            ));
        }
    };

    let fetched = client.get(url).send().await;
    let wallet = p.w.name.as_str();
    let ctx = KaminoResponseCtx {
        shared: p.shared,
        cache_key: &cache_key,
        chain: p.chain,
        protocol: p.protocol,
        address: p.address,
        market: &market,
        wallet,
        idx: p.idx,
    };
    kamino_process_response(fetched, &ctx, p.req_id).await
}

/// Try to look up the `marginfi_account` address from the DB cache.
async fn marginfi_cached_account(shared: &SharedState, cache_key: &str) -> String {
    let Some(db) = shared.db() else {
        return String::new();
    };
    let Ok(now) = crate::db::Db::now_ms() else {
        return String::new();
    };
    let Ok(Some(row)) = db.get_json_if_fresh(cache_key, now).await else {
        return String::new();
    };
    serde_json::from_str::<Value>(&row.json)
        .ok()
        .and_then(|v| {
            v.get("marginfi_account")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_default()
}

/// Resolve the marginfi account address from args or cache.
async fn marginfi_resolve_account(args: &Value, shared: &SharedState, cache_key: &str) -> String {
    let override_val = arg_str(args, "marginfi_account")
        .map(str::to_owned)
        .unwrap_or_default();
    if !override_val.is_empty() {
        return override_val;
    }
    marginfi_cached_account(shared, cache_key).await
}

async fn handle_marginfi(p: LendingParams<'_>, conn: &ConnState) -> eyre::Result<JsonRpcResponse> {
    let LendingParams {
        req_id,
        args,
        chain,
        protocol,
        address,
        w,
        idx,
        shared,
    } = p;
    let group = arg_str(args, "group")
        .unwrap_or(shared.cfg.http.marginfi_default_group.as_str())
        .trim()
        .to_owned();

    shared.ensure_db().await;
    let cache_key = format!("lending:marginfi:account:{group}:{}:{}", w.name, idx);
    let marginfi_account = marginfi_resolve_account(args, shared, &cache_key).await;

    if marginfi_account.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "marginfi_not_initialized",
                "no known marginfi_account for this wallet/account; run a marginfi lend/borrow once or pass marginfi_account explicitly",
            )),
        ));
    }

    let pk = match SolanaChain::parse_pubkey(marginfi_account.as_str()) {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "invalid_request",
                    format!("invalid marginfi_account pubkey: {e:#}"),
                )),
            ));
        }
    };

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

    let Some(acc) = sol.get_account_optional(&pk).await? else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "marginfi_read_failed",
                "marginfi_account does not exist on-chain",
            )),
        ));
    };

    let Some(decoded) =
        carbon_marginfi_v2_decoder::accounts::marginfi_account::MarginfiAccount::deserialize(
            acc.data.as_slice(),
        )
    else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "marginfi_read_failed",
                "failed to decode marginfi account (unexpected account layout)",
            )),
        ));
    };
    let decoded_json = serde_json::to_value(&decoded).unwrap_or_else(|_| json!({}));
    let payload = json!({
      "chain": chain, "protocol": protocol, "address": address,
      "source": "rpc", "group": group, "marginfi_account": marginfi_account,
      "account": decoded_json
    });
    persist_lending_snapshot(shared, chain, protocol, w.name.as_str(), idx, &payload).await;

    Ok(ok(req_id, tool_ok(payload)))
}

/// Build an `EvmChain` from shared config for a given chain name.
fn build_evm_chain(shared: &SharedState, chain: &str) -> eyre::Result<EvmChain> {
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
    Ok(evm)
}

/// Format Aave account data tuple into a JSON value.
fn aave_account_data_json(
    data: (
        alloy::primitives::U256,
        alloy::primitives::U256,
        alloy::primitives::U256,
        alloy::primitives::U256,
        alloy::primitives::U256,
        alloy::primitives::U256,
    ),
) -> Value {
    json!({
        "total_collateral_base": data.0.to_string(),
        "total_debt_base": data.1.to_string(),
        "available_borrows_base": data.2.to_string(),
        "current_liquidation_threshold": data.3.to_string(),
        "ltv": data.4.to_string(),
        "health_factor": data.5.to_string(),
    })
}

async fn handle_aave(p: LendingParams<'_>) -> eyre::Result<JsonRpcResponse> {
    let LendingParams {
        req_id,
        args,
        chain,
        protocol,
        address,
        w,
        idx,
        shared,
    } = p;
    let pool_s = arg_str(args, "pool_address")
        .or_else(|| default_aave_pool_for_chain(chain))
        .unwrap_or("")
        .to_owned();
    if pool_s.trim().is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "missing Aave pool address for this chain (provide pool_address)",
            )),
        ));
    }

    let evm = build_evm_chain(shared, chain)?;
    let pool_addr = EvmChain::parse_address(&pool_s).context("parse pool_address")?;
    let user_addr = EvmChain::parse_address(address).context("parse address")?;
    let pool = IAavePoolV3Read::new(pool_addr, evm.provider()?);

    shared.ensure_db().await;
    let cache_key = format!("lending:aave:account_data:{chain}:{}:{}", w.name, idx);

    match pool.getUserAccountData(user_addr).call().await {
        Ok(data) => {
            let account_data =
                aave_account_data_json((data._0, data._1, data._2, data._3, data._4, data._5));
            cache_json(shared, &cache_key, &account_data, 10_000).await;
            let payload = json!({
                "chain": chain, "protocol": protocol, "address": address,
                "source": "rpc", "pool": pool_s, "account_data": account_data
            });
            persist_lending_snapshot(shared, chain, protocol, w.name.as_str(), idx, &payload).await;
            Ok(ok(req_id, tool_ok(payload)))
        }
        Err(e) => {
            if let Some(v) = try_cached_value(shared, &cache_key).await {
                return Ok(ok(
                    req_id,
                    tool_ok(json!({
                        "chain": chain, "protocol": protocol, "address": address,
                        "source": "cache", "pool": pool_s, "account_data": v
                    })),
                ));
            }
            Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "aave_read_failed",
                    format!("aave getUserAccountData failed: {e:#}"),
                )),
            ))
        }
    }
}

/// Context for processing a Compound Comet fetch result.
struct CompoundResponseCtx<'a> {
    shared: &'a SharedState,
    cache_key: &'a str,
    chain: &'a str,
    protocol: &'a str,
    address: &'a str,
    comet_s: &'a str,
    wallet: &'a str,
    idx: u32,
}

/// Process a Compound Comet fetch result, caching on success and falling back to cache on failure.
async fn compound_process_result(
    fetched: eyre::Result<(alloy::primitives::Address, alloy::primitives::U256)>,
    ctx: &CompoundResponseCtx<'_>,
    req_id: Value,
) -> eyre::Result<JsonRpcResponse> {
    match fetched {
        Ok((base, borrow)) => {
            let payload = json!({
                "comet": ctx.comet_s, "base_token": format!("{base:#x}"), "borrow_base": borrow.to_string()
            });
            cache_json(ctx.shared, ctx.cache_key, &payload, 10_000).await;
            if let Some(db) = ctx.shared.db() {
                if let Ok(now_ms) = crate::db::Db::now_ms() {
                    let out = json!({
                        "chain": ctx.chain, "protocol": ctx.protocol, "address": ctx.address, "source": "rpc",
                        "comet": ctx.comet_s, "base_token": format!("{base:#x}"), "borrow_base": borrow.to_string(),
                        "notes": "Compound v3 (Comet) is single-base-asset per market. Returns baseToken + borrowBalanceOf. Collateral balances are reported separately."
                    });
                    let json_s = out.to_string();
                    if !json_s.is_empty() {
                        drop(
                            db.upsert_health_snapshot(&crate::db::HealthSnapshotInput {
                                surface: "lending",
                                chain: ctx.chain,
                                provider: ctx.protocol,
                                wallet: ctx.wallet,
                                account_index: i64::from(ctx.idx),
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
                    "chain": ctx.chain, "protocol": ctx.protocol, "address": ctx.address, "source": "rpc",
                    "comet": ctx.comet_s, "base_token": format!("{base:#x}"), "borrow_base": borrow.to_string(),
                    "notes": "Compound v3 (Comet) is single-base-asset per market. Returns baseToken + borrowBalanceOf. Collateral balances are reported separately."
                })),
            ))
        }
        Err(e) => {
            if let Some(v) = try_cached_value(ctx.shared, ctx.cache_key).await {
                return Ok(ok(
                    req_id,
                    tool_ok(json!({
                        "chain": ctx.chain, "protocol": ctx.protocol, "address": ctx.address,
                        "source": "cache", "positions": v
                    })),
                ));
            }
            Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "compound_read_failed",
                    format!("compound comet read failed: {e:#}"),
                )),
            ))
        }
    }
}

async fn handle_compound(p: LendingParams<'_>) -> eyre::Result<JsonRpcResponse> {
    let LendingParams {
        req_id,
        args,
        chain,
        protocol,
        address,
        w,
        idx,
        shared,
    } = p;
    let comet_s = arg_str(args, "comet_address")
        .or_else(|| default_comet_for_chain(chain))
        .unwrap_or("")
        .to_owned();
    if comet_s.trim().is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "missing Comet address for this chain (provide comet_address)",
            )),
        ));
    }

    let evm = build_evm_chain(shared, chain)?;
    let comet_addr = EvmChain::parse_address(&comet_s).context("parse comet_address")?;
    let user_addr = EvmChain::parse_address(address).context("parse address")?;
    let comet = ICometV3Read::new(comet_addr, evm.provider()?);

    shared.ensure_db().await;
    let cache_key = format!("lending:compound:basic:{chain}:{}:{}", w.name, idx);

    let fetched = async {
        let base: alloy::primitives::Address =
            comet.baseToken().call().await.context("comet baseToken")?;
        let borrow: alloy::primitives::U256 = comet
            .borrowBalanceOf(user_addr)
            .call()
            .await
            .context("comet borrowBalanceOf")?;
        Ok::<_, eyre::Report>((base, borrow))
    }
    .await;

    let ctx = CompoundResponseCtx {
        shared,
        cache_key: &cache_key,
        chain,
        protocol,
        address,
        comet_s: &comet_s,
        wallet: w.name.as_str(),
        idx,
    };
    compound_process_result(fetched, &ctx, req_id).await
}
