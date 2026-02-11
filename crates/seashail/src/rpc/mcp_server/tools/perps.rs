use crate::{
    chains::solana::SolanaChain,
    errors::ToolError,
    financial_math,
    keystore::{utc_now_iso, Keystore},
    perps::{hyperliquid, jupiter_perps},
    policy_engine::WriteOp,
};
use eyre::Context as _;
use serde_json::{json, Value};
use solana_signer::Signer as _;
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use tokio::io::BufReader;

use super::super::jsonrpc::{err, ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::state::effective_network_mode;
use super::super::{ConnState, SharedState};
use super::helpers::{resolve_wallet_and_account, u128_to_u64};
use super::key_loading::{load_evm_signer, load_solana_keypair};
use super::policy_confirm::{maybe_confirm_write, WriteConfirmOutcome, WriteConfirmRequest};

fn hyperliquid_base_url(shared: &SharedState, conn: &ConnState) -> String {
    let mode = effective_network_mode(shared, conn);
    match mode {
        crate::config::NetworkMode::Mainnet => shared.cfg.http.hyperliquid_base_url_mainnet.clone(),
        crate::config::NetworkMode::Testnet => shared.cfg.http.hyperliquid_base_url_testnet.clone(),
    }
}

fn find_market<'a>(
    markets: &'a [hyperliquid::HyperliquidMarket],
    coin: &str,
) -> Option<&'a hyperliquid::HyperliquidMarket> {
    let t = coin.trim();
    if t.is_empty() {
        return None;
    }
    markets.iter().find(|m| m.coin.eq_ignore_ascii_case(t))
}

fn parse_f64(s: &str, label: &'static str) -> Result<f64, ToolError> {
    s.trim()
        .parse::<f64>()
        .map_err(|_e| ToolError::new("invalid_request", format!("invalid {label}")))
}

fn parse_u32(v: Option<u64>, label: &'static str) -> Result<u32, ToolError> {
    let Some(x) = v else {
        return Err(ToolError::new(
            "invalid_request",
            format!("missing {label}"),
        ));
    };
    u32::try_from(x).map_err(|_e| ToolError::new("invalid_request", format!("invalid {label}")))
}

fn parse_provider(args: &Value) -> &str {
    args.get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("hyperliquid")
}

fn hyperliquid_cached_markets_response(
    req_id: Value,
    args: &Value,
    arr: &[Value],
) -> JsonRpcResponse {
    let market = args
        .get("market")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if market.is_empty() {
        return ok(
            req_id,
            tool_ok(json!({
              "provider": "hyperliquid",
              "cached": true,
              "markets": arr
            })),
        );
    }

    let out: Vec<Value> = arr
        .iter()
        .filter(|it| {
            it.get("coin")
                .and_then(Value::as_str)
                .is_some_and(|c| c.eq_ignore_ascii_case(market))
        })
        .cloned()
        .collect();
    if out.is_empty() {
        return ok(
            req_id,
            tool_err(ToolError::new("unknown_market", "unknown market")),
        );
    }
    ok(
        req_id,
        tool_ok(json!({
          "provider": "hyperliquid",
          "cached": true,
          "markets": out
        })),
    )
}

const fn compute_budget_program_id() -> solana_sdk::pubkey::Pubkey {
    // Base58("ComputeBudget111111111111111111111111111111")
    solana_sdk::pubkey::Pubkey::new_from_array([
        3, 6, 70, 111, 229, 33, 23, 50, 255, 236, 173, 186, 114, 195, 155, 231, 188, 140, 229, 187,
        197, 247, 18, 107, 44, 67, 155, 58, 64, 0, 0, 0,
    ])
}

fn compute_budget_set_compute_unit_limit(units: u32) -> solana_sdk::instruction::Instruction {
    let mut data = Vec::with_capacity(1 + 4);
    data.push(2); // SetComputeUnitLimit
    data.extend_from_slice(&units.to_le_bytes());
    solana_sdk::instruction::Instruction {
        program_id: compute_budget_program_id(),
        accounts: vec![],
        data,
    }
}

fn compute_budget_set_compute_unit_price(
    micro_lamports: u64,
) -> solana_sdk::instruction::Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(3); // SetComputeUnitPrice
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    solana_sdk::instruction::Instruction {
        program_id: compute_budget_program_id(),
        accounts: vec![],
        data,
    }
}

// ---------------------------------------------------------------------------
// get_market_data helpers
// ---------------------------------------------------------------------------

async fn hyperliquid_market_data(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let base_url = hyperliquid_base_url(shared, conn);
    let client = hyperliquid::HyperliquidClient::new(&base_url)?;
    shared.ensure_db().await;
    let key = "perps:hyperliquid:markets";
    let now_ms = crate::db::Db::now_ms().ok();
    let markets = match client.meta_and_asset_ctxs().await {
        Ok(markets) => {
            if let (Some(db), Some(now_ms)) = (shared.db(), now_ms) {
                let json_s = serde_json::to_string(&markets).unwrap_or_default();
                if !json_s.is_empty() {
                    let _cache_write = db.upsert_json(key, &json_s, now_ms, now_ms + 30_000).await;
                }
            }
            markets
        }
        Err(e) => {
            return hyperliquid_market_data_fallback(req_id, args, shared, now_ms, e).await;
        }
    };
    let market = args.get("market").and_then(|v| v.as_str()).unwrap_or("");
    if !market.trim().is_empty() {
        let Some(m) = find_market(&markets, market) else {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("unknown_market", "unknown market")),
            ));
        };
        return Ok(ok(
            req_id,
            tool_ok(json!({ "provider": "hyperliquid", "markets": [m] })),
        ));
    }
    Ok(ok(
        req_id,
        tool_ok(json!({ "provider": "hyperliquid", "markets": markets })),
    ))
}

async fn hyperliquid_market_data_fallback(
    req_id: Value,
    args: &Value,
    shared: &SharedState,
    now_ms: Option<i64>,
    e: eyre::Report,
) -> eyre::Result<JsonRpcResponse> {
    let key = "perps:hyperliquid:markets";
    let cached = if let (Some(db), Some(now_ms)) = (shared.db(), now_ms) {
        match db.get_json_if_fresh(key, now_ms).await {
            Ok(Some(row)) => serde_json::from_str::<Value>(&row.json).ok(),
            _ => None,
        }
    } else {
        None
    };

    if let Some(Value::Array(arr)) = cached {
        return Ok(hyperliquid_cached_markets_response(req_id, args, &arr));
    }

    Ok(ok(
        req_id,
        tool_err(ToolError::new(
            "provider_unavailable",
            format!("hyperliquid market data fetch failed: {e:#}"),
        )),
    ))
}

async fn jupiter_market_data(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let mode = effective_network_mode(shared, conn);
    if mode != crate::config::NetworkMode::Mainnet {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "provider_unavailable",
                "jupiter_perps is only supported on Solana mainnet",
            )),
        ));
    }

    let filter = args.get("market").and_then(|v| v.as_str()).unwrap_or("");
    let mut out = vec![];
    for sym in ["SOL", "BTC", "ETH"] {
        if !filter.trim().is_empty() && !sym.eq_ignore_ascii_case(filter.trim()) {
            continue;
        }
        let price_usd = crate::price::binance_price_usd(&shared.cfg, sym).await.ok();
        out.push(json!({
          "coin": sym,
          "price_usd": price_usd,
          "pool": jupiter_perps::JLP_POOL_MAINNET,
          "custody": jupiter_perps::custody_for_market_mainnet(sym),
          "collateral_custody": jupiter_perps::CUSTODY_USDC_MAINNET,
          "program_id": jupiter_perps::PROGRAM_ID_MAINNET
        }));
    }

    shared.ensure_db().await;
    if let Some(db) = shared.db() {
        if let Ok(now_ms) = crate::db::Db::now_ms() {
            let key = "perps:jupiter:markets";
            let json_s = serde_json::to_string(&out).unwrap_or_default();
            if !json_s.is_empty() {
                let _cache_write = db.upsert_json(key, &json_s, now_ms, now_ms + 30_000).await;
            }
        }
    }

    Ok(ok(
        req_id,
        tool_ok(json!({ "provider": "jupiter_perps", "markets": out })),
    ))
}

async fn handle_get_market_data(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let provider = parse_provider(args);
    match provider {
        "hyperliquid" => hyperliquid_market_data(req_id, args, shared, conn).await,
        "jupiter_perps" => jupiter_market_data(req_id, args, shared, conn).await,
        other => Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                format!("unsupported provider: {other}"),
            )),
        )),
    }
}

// ---------------------------------------------------------------------------
// get_positions helpers
// ---------------------------------------------------------------------------

async fn hyperliquid_positions(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let base_url = hyperliquid_base_url(shared, conn);
    let client = hyperliquid::HyperliquidClient::new(&base_url)?;
    let (w, idx) = resolve_wallet_and_account(shared, args)?;
    let addr = w
        .evm_addresses
        .get(idx as usize)
        .cloned()
        .unwrap_or_default();
    if addr.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "missing_address",
                "wallet has no evm address",
            )),
        ));
    }
    shared.ensure_db().await;
    let key = format!("perps:hyperliquid:positions:{}:{}", w.name, idx);
    let now_ms = crate::db::Db::now_ms().ok();

    let st = match client
        .info(json!({ "type": "clearinghouseState", "user": addr }))
        .await
    {
        Ok(st) => {
            if let (Some(db), Some(now_ms)) = (shared.db(), now_ms) {
                let json_s = st.to_string();
                drop(db.upsert_json(&key, &json_s, now_ms, now_ms + 10_000).await);

                // Persist latest snapshot for position monitoring.
                drop(
                    db.upsert_health_snapshot(&crate::db::HealthSnapshotInput {
                        surface: "perps",
                        chain: "hyperliquid",
                        provider: "hyperliquid",
                        wallet: w.name.as_str(),
                        account_index: i64::from(idx),
                        fetched_at_ms: now_ms,
                        payload_json: &json_s,
                    })
                    .await,
                );
            }
            st
        }
        Err(e) => {
            return hyperliquid_positions_fallback(req_id, shared, now_ms, &key, e).await;
        }
    };

    Ok(ok(
        req_id,
        tool_ok(json!({ "provider": "hyperliquid", "state": st })),
    ))
}

async fn hyperliquid_positions_fallback(
    req_id: Value,
    shared: &SharedState,
    now_ms: Option<i64>,
    key: &str,
    e: eyre::Report,
) -> eyre::Result<JsonRpcResponse> {
    let cached_state = if let (Some(db), Some(now_ms)) = (shared.db(), now_ms) {
        match db.get_json_if_fresh(key, now_ms).await {
            Ok(Some(row)) => serde_json::from_str::<Value>(&row.json).ok(),
            _ => None,
        }
    } else {
        None
    };

    if let Some(v) = cached_state {
        return Ok(ok(
            req_id,
            tool_ok(json!({
              "provider": "hyperliquid",
              "cached": true,
              "state": v
            })),
        ));
    }
    Ok(ok(
        req_id,
        tool_err(ToolError::new(
            "provider_unavailable",
            format!("hyperliquid get_positions failed: {e:#}"),
        )),
    ))
}

/// Context needed to scan Jupiter perps positions on-chain.
struct JupiterPositionScanCtx {
    program_id: solana_sdk::pubkey::Pubkey,
    pool: solana_sdk::pubkey::Pubkey,
    collateral_custody: solana_sdk::pubkey::Pubkey,
    owner: solana_sdk::pubkey::Pubkey,
}

fn jupiter_position_to_json(
    sym: &str,
    side_label: &str,
    pos: &solana_sdk::pubkey::Pubkey,
    pos_dec: &jupiter_perps::PositionAccount,
) -> Value {
    let size_usd = financial_math::token_base_to_usd(u128::from(pos_dec.size_usd), 6, 1.0);
    let collateral_usd =
        financial_math::token_base_to_usd(u128::from(pos_dec.collateral_usd), 6, 1.0);
    json!({
      "market": sym,
      "side": side_label,
      "position": pos.to_string(),
      "size_usd": size_usd,
      "collateral_usd": collateral_usd,
      "open_time": pos_dec.open_time,
      "update_time": pos_dec.update_time,
      "raw": {
        "owner": pos_dec.owner.to_string(),
        "pool": pos_dec.pool.to_string(),
        "custody": pos_dec.custody.to_string(),
        "collateral_custody": pos_dec.collateral_custody.to_string(),
        "side_u8": pos_dec.side,
        "price_u64": pos_dec.price.to_string(),
        "realised_pnl_usd_i64": pos_dec.realised_pnl_usd.to_string(),
        "cumulative_interest_snapshot_u128": pos_dec.cumulative_interest_snapshot.to_string(),
        "locked_amount": pos_dec.locked_amount.to_string(),
        "bump": pos_dec.bump
      }
    })
}

async fn jupiter_scan_positions(
    sol: &SolanaChain,
    ctx: &JupiterPositionScanCtx,
) -> eyre::Result<Vec<Value>> {
    let mut out = vec![];
    for sym in ["SOL", "BTC", "ETH"] {
        let Some(custody_s) = jupiter_perps::custody_for_market_mainnet(sym) else {
            continue;
        };
        let custody = jupiter_perps::parse_pubkey(custody_s).context("parse custody")?;

        for (side_label, side) in [
            ("long", jupiter_perps::Side::Long),
            ("short", jupiter_perps::Side::Short),
        ] {
            let (pos, _b) = jupiter_perps::position_pda(
                &ctx.program_id,
                &ctx.owner,
                &ctx.pool,
                &custody,
                &ctx.collateral_custody,
                side,
            );

            let Some(acc) = sol.get_account_optional(&pos).await? else {
                continue;
            };
            let Ok(pos_dec) = jupiter_perps::decode_position_account(&acc.data) else {
                continue;
            };
            if pos_dec.size_usd == 0 {
                continue;
            }
            out.push(jupiter_position_to_json(sym, side_label, &pos, &pos_dec));
        }
    }
    Ok(out)
}

fn jupiter_parse_common_pubkeys() -> eyre::Result<JupiterPositionScanCtx> {
    let program_id = jupiter_perps::parse_pubkey(jupiter_perps::PROGRAM_ID_MAINNET)
        .context("parse jupiter perps program id")?;
    let pool =
        jupiter_perps::parse_pubkey(jupiter_perps::JLP_POOL_MAINNET).context("parse jlp pool")?;
    let collateral_custody = jupiter_perps::parse_pubkey(jupiter_perps::CUSTODY_USDC_MAINNET)
        .context("parse collateral custody")?;
    // owner is set to default here; the caller must fill it in.
    Ok(JupiterPositionScanCtx {
        program_id,
        pool,
        collateral_custody,
        owner: solana_sdk::pubkey::Pubkey::default(),
    })
}

async fn jupiter_positions(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let mode = effective_network_mode(shared, conn);
    if mode != crate::config::NetworkMode::Mainnet {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "provider_unavailable",
                "jupiter_perps is only supported on Solana mainnet",
            )),
        ));
    }

    let (w, idx) = resolve_wallet_and_account(shared, args)?;
    let addr = w
        .solana_addresses
        .get(idx as usize)
        .cloned()
        .unwrap_or_default();
    if addr.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "missing_address",
                "wallet has no solana address",
            )),
        ));
    }
    let owner = SolanaChain::parse_pubkey(&addr)?;

    let mut ctx = match jupiter_parse_common_pubkeys() {
        Ok(v) => v,
        Err(e) => {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("internal_error", format!("{e:#}"))),
            ));
        }
    };
    ctx.owner = owner;

    let sol = SolanaChain::new_with_fallbacks(
        &shared.cfg.rpc.solana_rpc_url,
        match effective_network_mode(shared, conn) {
            crate::config::NetworkMode::Mainnet => &shared.cfg.rpc.solana_fallback_rpc_urls_mainnet,
            crate::config::NetworkMode::Testnet => &shared.cfg.rpc.solana_fallback_rpc_urls_devnet,
        },
        &shared.cfg.http.jupiter_base_url,
        shared.cfg.http.jupiter_api_key.as_deref(),
        shared.cfg.rpc.solana_default_compute_unit_limit,
        shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );

    let out = jupiter_scan_positions(&sol, &ctx).await?;

    jupiter_positions_cache(shared, &w.name, idx, &out).await;

    // Persist latest snapshot for position monitoring.
    shared.ensure_db().await;
    if let Some(db) = shared.db() {
        if let Ok(now_ms) = crate::db::Db::now_ms() {
            let payload = json!({ "provider": "jupiter_perps", "positions": out });
            let json_s = payload.to_string();
            if !json_s.is_empty() {
                drop(
                    db.upsert_health_snapshot(&crate::db::HealthSnapshotInput {
                        surface: "perps",
                        chain: "solana",
                        provider: "jupiter_perps",
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
        tool_ok(json!({ "provider": "jupiter_perps", "positions": out })),
    ))
}

async fn jupiter_positions_cache(
    shared: &mut SharedState,
    wallet_name: &str,
    idx: u32,
    out: &[Value],
) {
    shared.ensure_db().await;
    if let Some(db) = shared.db() {
        if let Ok(now_ms) = crate::db::Db::now_ms() {
            let key = format!("perps:jupiter:positions:{wallet_name}:{idx}");
            let json_s = serde_json::to_string(&out).unwrap_or_default();
            if !json_s.is_empty() {
                let _cache_write = db.upsert_json(&key, &json_s, now_ms, now_ms + 10_000).await;
            }
        }
    }
}

async fn handle_get_positions(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let provider = parse_provider(args);
    match provider {
        "hyperliquid" => hyperliquid_positions(req_id, args, shared, conn).await,
        "jupiter_perps" => jupiter_positions(req_id, args, shared, conn).await,
        other => Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                format!("unsupported provider: {other}"),
            )),
        )),
    }
}

// ---------------------------------------------------------------------------
// write_perp: Jupiter helpers
// ---------------------------------------------------------------------------

/// All resolved pubkeys / addresses needed for a Jupiter perp write.
struct JupiterPerpSetup {
    program_id: solana_sdk::pubkey::Pubkey,
    pool: solana_sdk::pubkey::Pubkey,
    custody: solana_sdk::pubkey::Pubkey,
    collateral_custody: solana_sdk::pubkey::Pubkey,
    usdc_mint: solana_sdk::pubkey::Pubkey,
    position: solana_sdk::pubkey::Pubkey,
    side: jupiter_perps::Side,
    leverage_u32: u32,
    price_slippage_u64: u64,
    counter: u64,
}

/// Bundled context for Jupiter perp write operations.
struct PerpWriteCtx<'a> {
    req_id: Value,
    tool_name: &'a str,
    args: &'a Value,
    lock: std::fs::File,
}

/// Resolved execution context for a Jupiter perp instruction.
struct JupiterPerpExecCtx<'a> {
    setup: &'a JupiterPerpSetup,
    sol: &'a SolanaChain,
    keypair: &'a solana_sdk::signer::keypair::Keypair,
    wallet_name: &'a str,
    idx: u32,
}

/// Resolve the side for a Jupiter close operation. If `side_s` is empty,
/// probe on-chain for the open position.
async fn jupiter_infer_close_side(
    sol: &SolanaChain,
    setup: &JupiterPerpSetup,
    owner: &solana_sdk::pubkey::Pubkey,
) -> Result<jupiter_perps::Side, ToolError> {
    let (pos_long, _) = jupiter_perps::position_pda(
        &setup.program_id,
        owner,
        &setup.pool,
        &setup.custody,
        &setup.collateral_custody,
        jupiter_perps::Side::Long,
    );
    let (pos_short, _) = jupiter_perps::position_pda(
        &setup.program_id,
        owner,
        &setup.pool,
        &setup.custody,
        &setup.collateral_custody,
        jupiter_perps::Side::Short,
    );
    let long_open = sol
        .get_account_optional(&pos_long)
        .await
        .ok()
        .flatten()
        .and_then(|a| jupiter_perps::decode_position_account(&a.data).ok())
        .is_some_and(|p| p.size_usd > 0);
    let short_open = sol
        .get_account_optional(&pos_short)
        .await
        .ok()
        .flatten()
        .and_then(|a| jupiter_perps::decode_position_account(&a.data).ok())
        .is_some_and(|p| p.size_usd > 0);
    match (long_open, short_open) {
        (true, false) => Ok(jupiter_perps::Side::Long),
        (false, true) => Ok(jupiter_perps::Side::Short),
        (false, false) => Err(ToolError::new(
            "no_position",
            "no open position found for this market",
        )),
        (true, true) => Err(ToolError::new(
            "ambiguous_position",
            "both long and short positions exist; specify side",
        )),
    }
}

fn jupiter_resolve_side(
    tool_name: &str,
    side_s: &str,
) -> Result<Option<jupiter_perps::Side>, ToolError> {
    match tool_name {
        "open_perp_position" => match side_s.trim().to_ascii_lowercase().as_str() {
            "long" | "buy" => Ok(Some(jupiter_perps::Side::Long)),
            "short" | "sell" => Ok(Some(jupiter_perps::Side::Short)),
            _ => Err(ToolError::new(
                "invalid_request",
                "side must be long or short",
            )),
        },
        "close_perp_position" if side_s.trim().is_empty() => Ok(None),
        "close_perp_position" => match side_s.trim().to_ascii_lowercase().as_str() {
            "long" | "buy" => Ok(Some(jupiter_perps::Side::Long)),
            "short" | "sell" => Ok(Some(jupiter_perps::Side::Short)),
            _ => Err(ToolError::new(
                "invalid_request",
                "side must be long or short",
            )),
        },
        _ => Ok(Some(jupiter_perps::Side::Long)),
    }
}

/// Build the common prefix instructions (compute budget + ATA creates).
fn jupiter_perp_ixs_prefix(
    owner: &solana_sdk::pubkey::Pubkey,
    usdc_mint: &solana_sdk::pubkey::Pubkey,
    position_request: &solana_sdk::pubkey::Pubkey,
) -> Vec<solana_sdk::instruction::Instruction> {
    vec![
        compute_budget_set_compute_unit_price(100_000),
        compute_budget_set_compute_unit_limit(1_400_000),
        create_associated_token_account_idempotent(owner, owner, usdc_mint, &spl_token::id()),
        create_associated_token_account_idempotent(
            owner,
            position_request,
            usdc_mint,
            &spl_token::id(),
        ),
    ]
}

struct JupiterPerpLogEntry<'a> {
    tool_name: &'a str,
    wallet_name: &'a str,
    idx: u32,
    market: &'a str,
    side: jupiter_perps::Side,
    usd_value: f64,
    sig: &'a solana_sdk::signature::Signature,
    outcome: &'a WriteConfirmOutcome,
}

fn jupiter_perp_log(shared: &SharedState, entry: &JupiterPerpLogEntry<'_>) -> eyre::Result<()> {
    let ty = if entry.tool_name == "open_perp_position" {
        "perp_open"
    } else {
        "perp_close"
    };
    let side_label = match entry.side {
        jupiter_perps::Side::Long => "long",
        jupiter_perps::Side::Short => "short",
    };
    shared.ks.append_tx_history(&json!({
        "ts": utc_now_iso(),
        "day": Keystore::current_utc_day_key(),
        "type": ty,
        "provider": "jupiter_perps",
        "chain": "solana",
        "wallet": entry.wallet_name,
        "account_index": entry.idx,
        "market": entry.market,
        "side": side_label,
        "usd_value": entry.usd_value,
        "txid": entry.sig.to_string()
    }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(),
        "tool": entry.tool_name,
        "wallet": entry.wallet_name,
        "account_index": entry.idx,
        "chain": "solana",
        "usd_value": entry.usd_value,
        "usd_value_known": true,
        "policy_decision": entry.outcome.policy_decision,
        "confirm_required": entry.outcome.confirm_required,
        "confirm_result": entry.outcome.confirm_result,
        "forced_confirm": entry.outcome.forced_confirm,
        "daily_used_usd": entry.outcome.daily_used_usd,
        "txid": entry.sig.to_string(),
        "error_code": null,
        "result": "broadcasted",
        "provider": "jupiter_perps"
    }));
    Ok(())
}

/// Build and submit a Jupiter Perps OPEN position request.
/// Validate Jupiter open args (usd size units, market order type, non-empty size).
fn jupiter_validate_open_args(args: &Value) -> Result<&str, ToolError> {
    let size_units = args
        .get("size_units")
        .and_then(|v| v.as_str())
        .unwrap_or("usd");
    if size_units != "usd" {
        return Err(ToolError::new(
            "not_supported",
            "jupiter_perps only supports size_units=usd",
        ));
    }
    let order_type = args
        .get("order_type")
        .and_then(|v| v.as_str())
        .unwrap_or("market");
    if order_type != "market" {
        return Err(ToolError::new(
            "not_supported",
            "jupiter_perps only supports order_type=market",
        ));
    }
    let size_str = args.get("size").and_then(|v| v.as_str()).unwrap_or("");
    if size_str.trim().is_empty() {
        return Err(ToolError::new("invalid_request", "missing size"));
    }
    Ok(size_str)
}

/// Build the instruction set for a Jupiter OPEN position market request.
fn jupiter_build_open_ixs(
    setup: &JupiterPerpSetup,
    owner: &solana_sdk::pubkey::Pubkey,
    size_usd_u64: u64,
    collateral_u64: u64,
) -> eyre::Result<(
    Vec<solana_sdk::instruction::Instruction>,
    solana_sdk::pubkey::Pubkey,
)> {
    let (position_request, _rb) =
        jupiter_perps::position_request_pda(&setup.program_id, &setup.position, setup.counter, 1);
    let position_request_ata = get_associated_token_address(&position_request, &setup.usdc_mint);
    let funding_account = get_associated_token_address(owner, &setup.usdc_mint);
    let ix = jupiter_perps::build_create_increase_position_market_request_ix(
        &jupiter_perps::IncreasePositionMarketRequestAccounts {
            program_id: setup.program_id,
            owner: *owner,
            funding_account,
            pool: setup.pool,
            position: setup.position,
            position_request,
            position_request_ata,
            custody: setup.custody,
            collateral_custody: setup.collateral_custody,
            input_mint: setup.usdc_mint,
            referral: None,
        },
        &jupiter_perps::IncreaseParams {
            size_usd_delta: size_usd_u64,
            collateral_token_delta: collateral_u64,
            side: setup.side,
            price_slippage: setup.price_slippage_u64,
            jupiter_minimum_out: None,
            counter: setup.counter,
        },
    )?;
    let mut ixs = jupiter_perp_ixs_prefix(owner, &setup.usdc_mint, &position_request);
    ixs.push(ix);
    Ok((ixs, position_request))
}

/// Compute open-position size values from user input.
fn jupiter_open_compute_sizes(size_str: &str, leverage: u32) -> eyre::Result<(u64, u64, f64)> {
    let size_usd_base =
        crate::amount::parse_amount_ui_to_base_u128(size_str, 6).context("parse size")?;
    let size_usd_u64 = u128_to_u64(size_usd_base).context("size overflow")?;
    let collateral_base = size_usd_base.div_ceil(u128::from(leverage));
    let collateral_u64 = u128_to_u64(collateral_base).context("collateral overflow")?;
    let usd_value = financial_math::token_base_to_usd(size_usd_base, 6, 1.0);
    Ok((size_usd_u64, collateral_u64, usd_value))
}

/// Build a Jupiter perps write-confirmation request.
const fn jupiter_confirm_request<'a>(
    tool: &'a str,
    exec: &'a JupiterPerpExecCtx<'_>,
    usd_value: f64,
    summary: &'a str,
    op: WriteOp,
) -> WriteConfirmRequest<'a> {
    WriteConfirmRequest {
        tool,
        wallet: Some(exec.wallet_name),
        account_index: Some(exec.idx),
        op,
        chain: "solana",
        usd_value,
        usd_value_known: true,
        force_confirm: false,
        slippage_bps: None,
        to_address: None,
        contract: Some(jupiter_perps::PROGRAM_ID_MAINNET),
        leverage: Some(exec.setup.leverage_u32),
        summary,
    }
}

/// Completed Jupiter perps transaction details for logging and response.
struct JupiterPerpResult<'a> {
    tool_name: &'a str,
    sym_upper: &'a str,
    usd_value: f64,
    sig: solana_sdk::signature::Signature,
    outcome: WriteConfirmOutcome,
    position_request: solana_sdk::pubkey::Pubkey,
}

/// Log and return the result for a Jupiter perps operation.
fn jupiter_perp_ok_response(
    shared: &SharedState,
    exec: &JupiterPerpExecCtx<'_>,
    result: &JupiterPerpResult<'_>,
    lock: std::fs::File,
    req_id: Value,
) -> eyre::Result<JsonRpcResponse> {
    jupiter_perp_log(
        shared,
        &JupiterPerpLogEntry {
            tool_name: result.tool_name,
            wallet_name: exec.wallet_name,
            idx: exec.idx,
            market: result.sym_upper,
            side: exec.setup.side,
            usd_value: result.usd_value,
            sig: &result.sig,
            outcome: &result.outcome,
        },
    )?;
    Keystore::release_lock(lock)?;
    Ok(ok(
        req_id,
        tool_ok(json!({
          "provider": "jupiter_perps",
          "status": "request_submitted",
          "txid": result.sig.to_string(),
          "position": exec.setup.position.to_string(),
          "position_request": result.position_request.to_string()
        })),
    ))
}

/// Sign and send Jupiter perps instructions, returning sig or early error response.
async fn jupiter_sign_and_send(
    exec: &JupiterPerpExecCtx<'_>,
    ixs: Vec<solana_sdk::instruction::Instruction>,
) -> Result<solana_sdk::signature::Signature, ToolError> {
    exec.sol
        .sign_and_send_instructions(exec.keypair, ixs)
        .await
        .map_err(|e| ToolError::new("tx_failed", format!("jupiter_perps tx failed: {e:#}")))
}

/// Build and submit a Jupiter Perps OPEN position request.
async fn jupiter_open_perp<R, W>(
    shared: &SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    ctx: PerpWriteCtx<'_>,
    exec: &JupiterPerpExecCtx<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let size_str = match jupiter_validate_open_args(ctx.args) {
        Ok(s) => s,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };
    let (size_usd_u64, collateral_u64, usd_value) =
        jupiter_open_compute_sizes(size_str, exec.setup.leverage_u32)?;

    let market = ctx
        .args
        .get("market")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sym_upper = market.trim().to_ascii_uppercase();
    let side_label = match exec.setup.side {
        jupiter_perps::Side::Long => "LONG",
        jupiter_perps::Side::Short => "SHORT",
    };
    let summary = format!(
        "OPEN PERP on Jupiter Perps: {side_label} {sym_upper} ({usd_value} USD, {}x)\n\n\
         Note: Jupiter Perps uses a request-fulfillment model; this submits a request for keepers to execute.",
        exec.setup.leverage_u32
    );

    let confirm_req = jupiter_confirm_request(
        "open_perp_position",
        exec,
        usd_value,
        &summary,
        WriteOp::OpenPerpPosition,
    );
    let outcome = match maybe_confirm_write(shared, conn, stdin, stdout, &confirm_req).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };

    let owner = exec.keypair.pubkey();
    let (ixs, position_request) =
        match jupiter_build_open_ixs(exec.setup, &owner, size_usd_u64, collateral_u64) {
            Ok(v) => v,
            Err(e) => {
                Keystore::release_lock(ctx.lock)?;
                return Ok(ok(
                    ctx.req_id,
                    tool_err(ToolError::new("internal_error", format!("{e:#}"))),
                ));
            }
        };
    let sig = match jupiter_sign_and_send(exec, ixs).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };

    jupiter_perp_ok_response(
        shared,
        exec,
        &JupiterPerpResult {
            tool_name: "open_perp_position",
            sym_upper: &sym_upper,
            usd_value,
            sig,
            outcome,
            position_request,
        },
        ctx.lock,
        ctx.req_id,
    )
}

/// Build the instruction set for a Jupiter CLOSE position market request.
fn jupiter_build_close_ixs(
    setup: &JupiterPerpSetup,
    owner: &solana_sdk::pubkey::Pubkey,
) -> eyre::Result<(
    Vec<solana_sdk::instruction::Instruction>,
    solana_sdk::pubkey::Pubkey,
)> {
    let (position_request, _rb) =
        jupiter_perps::position_request_pda(&setup.program_id, &setup.position, setup.counter, 2);
    let position_request_ata = get_associated_token_address(&position_request, &setup.usdc_mint);
    let receiving_account = get_associated_token_address(owner, &setup.usdc_mint);
    let ix = jupiter_perps::build_create_decrease_position_market_request_ix(
        &jupiter_perps::DecreasePositionMarketRequestAccounts {
            program_id: setup.program_id,
            owner: *owner,
            receiving_account,
            pool: setup.pool,
            position: setup.position,
            position_request,
            position_request_ata,
            custody: setup.custody,
            collateral_custody: setup.collateral_custody,
            desired_mint: setup.usdc_mint,
            referral: None,
        },
        &jupiter_perps::DecreaseParams {
            collateral_usd_delta: 0,
            size_usd_delta: 0,
            price_slippage: setup.price_slippage_u64,
            jupiter_minimum_out: None,
            entire_position: Some(true),
            counter: setup.counter,
        },
    )?;
    let mut ixs = jupiter_perp_ixs_prefix(owner, &setup.usdc_mint, &position_request);
    ixs.push(ix);
    Ok((ixs, position_request))
}

/// Validate the on-chain position for a Jupiter close and return the USD value.
async fn jupiter_validate_close_position(
    sol: &SolanaChain,
    setup: &JupiterPerpSetup,
) -> Result<f64, ToolError> {
    let acc = sol
        .get_account_optional(&setup.position)
        .await
        .map_err(|e| ToolError::new("internal_error", format!("{e:#}")))?
        .ok_or_else(|| ToolError::new("no_position", "position account not found"))?;
    let pos_dec = jupiter_perps::decode_position_account(&acc.data).map_err(|e| {
        ToolError::new(
            "no_position",
            format!("unable to decode position account: {e:#}"),
        )
    })?;
    if pos_dec.size_usd == 0 {
        return Err(ToolError::new("no_position", "position size is zero"));
    }
    Ok(financial_math::token_base_to_usd(
        u128::from(pos_dec.size_usd),
        6,
        1.0,
    ))
}

/// Build and submit a Jupiter Perps CLOSE position request.
async fn jupiter_close_perp<R, W>(
    shared: &SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    ctx: PerpWriteCtx<'_>,
    exec: &JupiterPerpExecCtx<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let size_str = ctx.args.get("size").and_then(|v| v.as_str()).unwrap_or("");
    if !size_str.trim().is_empty() {
        Keystore::release_lock(ctx.lock)?;
        return Ok(ok(
            ctx.req_id,
            tool_err(ToolError::new(
                "not_supported",
                "jupiter_perps close_perp_position currently only supports closing the entire position (omit size)",
            )),
        ));
    }

    let usd_value = match jupiter_validate_close_position(exec.sol, exec.setup).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };
    let market = ctx
        .args
        .get("market")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sym_upper = market.trim().to_ascii_uppercase();
    let side_label = match exec.setup.side {
        jupiter_perps::Side::Long => "LONG",
        jupiter_perps::Side::Short => "SHORT",
    };
    let summary = format!(
        "CLOSE PERP on Jupiter Perps: {side_label} {sym_upper}\n\n\
         Note: Jupiter Perps uses a request-fulfillment model; this submits a request for keepers to execute."
    );

    let confirm_req = jupiter_confirm_request(
        "close_perp_position",
        exec,
        usd_value,
        &summary,
        WriteOp::ClosePerpPosition,
    );
    let outcome = match maybe_confirm_write(shared, conn, stdin, stdout, &confirm_req).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };

    let owner = exec.keypair.pubkey();
    let (ixs, position_request) = match jupiter_build_close_ixs(exec.setup, &owner) {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(
                ctx.req_id,
                tool_err(ToolError::new("internal_error", format!("{e:#}"))),
            ));
        }
    };
    let sig = match jupiter_sign_and_send(exec, ixs).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };

    jupiter_perp_ok_response(
        shared,
        exec,
        &JupiterPerpResult {
            tool_name: "close_perp_position",
            sym_upper: &sym_upper,
            usd_value,
            sig,
            outcome,
            position_request,
        },
        ctx.lock,
        ctx.req_id,
    )
}

/// Validate Jupiter perps prerequisites (mainnet, tool support, market).
/// Returns early-exit `JsonRpcResponse` on validation failure.
fn jupiter_validate_prerequisites(
    ctx: &PerpWriteCtx<'_>,
    mode: crate::config::NetworkMode,
) -> Option<JsonRpcResponse> {
    if mode != crate::config::NetworkMode::Mainnet {
        return Some(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "provider_unavailable",
                "jupiter_perps is only supported on Solana mainnet",
            )),
        ));
    }
    if ctx.tool_name != "open_perp_position" && ctx.tool_name != "close_perp_position" {
        return Some(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "not_supported",
                format!(
                    "{} is not supported for provider=jupiter_perps",
                    ctx.tool_name
                ),
            )),
        ));
    }
    None
}

/// Resolve slippage-adjusted price for Jupiter perps.
async fn jupiter_resolve_price_slippage(
    shared: &SharedState,
    market: &str,
    side: jupiter_perps::Side,
    slippage_bps: u32,
) -> Result<u64, ToolError> {
    let slip = financial_math::bps_to_fraction(slippage_bps);
    let sym = market.trim().to_ascii_uppercase();
    let px = crate::price::binance_price_usd(&shared.cfg, &sym)
        .await
        .unwrap_or(0.0_f64);
    if px <= 0.0_f64 {
        return Err(ToolError::new(
            "price_unavailable",
            "unable to fetch price for slippage bound",
        ));
    }
    let px_adj =
        financial_math::worst_case_price(px, matches!(side, jupiter_perps::Side::Long), slip);
    let base = crate::amount::parse_amount_ui_to_base_u128(&format!("{px_adj:.6}"), 6)
        .map_err(|e| ToolError::new("invalid_request", format!("{e:#}")))?;
    u128_to_u64(base).map_err(|e| ToolError::new("invalid_request", format!("{e:#}")))
}

/// Parse all pubkeys needed for a Jupiter perps write from constant addresses + market custody.
fn jupiter_parse_write_pubkeys(
    custody_s: &str,
) -> eyre::Result<(
    solana_sdk::pubkey::Pubkey,
    solana_sdk::pubkey::Pubkey,
    solana_sdk::pubkey::Pubkey,
    solana_sdk::pubkey::Pubkey,
    solana_sdk::pubkey::Pubkey,
)> {
    let program_id =
        SolanaChain::parse_pubkey(jupiter_perps::PROGRAM_ID_MAINNET).context("program id")?;
    let pool = SolanaChain::parse_pubkey(jupiter_perps::JLP_POOL_MAINNET).context("pool")?;
    let custody = SolanaChain::parse_pubkey(custody_s).context("custody")?;
    let collateral_custody = SolanaChain::parse_pubkey(jupiter_perps::CUSTODY_USDC_MAINNET)
        .context("collateral custody")?;
    let usdc_mint =
        SolanaChain::parse_pubkey(jupiter_perps::USDC_MINT_MAINNET).context("usdc mint")?;
    Ok((program_id, pool, custody, collateral_custody, usdc_mint))
}

/// Resolve the side for a Jupiter write, potentially inferring it on-chain for close.
async fn jupiter_resolve_write_side(
    tool_name: &str,
    args: &Value,
    sol: &SolanaChain,
    setup_for_infer: &JupiterPerpSetup,
    owner: &solana_sdk::pubkey::Pubkey,
) -> Result<jupiter_perps::Side, ToolError> {
    let side_s = args.get("side").and_then(Value::as_str).unwrap_or("");
    match jupiter_resolve_side(tool_name, side_s)? {
        Some(s) => Ok(s),
        None => jupiter_infer_close_side(sol, setup_for_infer, owner).await,
    }
}

/// Parse leverage and slippage from args, resolve slippage-adjusted price.
async fn jupiter_resolve_leverage_and_slippage(
    shared: &SharedState,
    args: &Value,
    market: &str,
    side: jupiter_perps::Side,
) -> Result<(u32, u64), ToolError> {
    let leverage_u32 = parse_u32(
        args.get("leverage")
            .and_then(serde_json::Value::as_u64)
            .or(Some(1)),
        "leverage",
    )?;
    let slippage_bps = args
        .get("slippage_bps")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(50);
    let price_slippage_u64 =
        jupiter_resolve_price_slippage(shared, market, side, slippage_bps).await?;
    Ok((leverage_u32, price_slippage_u64))
}

/// Parsed Jupiter perps pubkeys (`program_id`, pool, custody, `collateral_custody`, `usdc_mint`).
struct JupiterParsedPubkeys {
    program_id: solana_sdk::pubkey::Pubkey,
    pool: solana_sdk::pubkey::Pubkey,
    custody: solana_sdk::pubkey::Pubkey,
    collateral_custody: solana_sdk::pubkey::Pubkey,
    usdc_mint: solana_sdk::pubkey::Pubkey,
}

/// Build a minimal `JupiterPerpSetup` for side inference (no position/leverage/slippage).
fn jupiter_infer_setup(pks: &JupiterParsedPubkeys) -> JupiterPerpSetup {
    JupiterPerpSetup {
        program_id: pks.program_id,
        pool: pks.pool,
        custody: pks.custody,
        collateral_custody: pks.collateral_custody,
        usdc_mint: pks.usdc_mint,
        position: solana_sdk::pubkey::Pubkey::default(),
        side: jupiter_perps::Side::Long,
        leverage_u32: 1,
        price_slippage_u64: 0,
        counter: 0,
    }
}

/// Build the final `JupiterPerpSetup` with resolved side, leverage, slippage, and position PDA.
fn jupiter_build_setup(
    pks: &JupiterParsedPubkeys,
    owner: &solana_sdk::pubkey::Pubkey,
    side: jupiter_perps::Side,
    leverage_u32: u32,
    price_slippage_u64: u64,
) -> JupiterPerpSetup {
    let (position, _bump) = jupiter_perps::position_pda(
        &pks.program_id,
        owner,
        &pks.pool,
        &pks.custody,
        &pks.collateral_custody,
        side,
    );
    JupiterPerpSetup {
        program_id: pks.program_id,
        pool: pks.pool,
        custody: pks.custody,
        collateral_custody: pks.collateral_custody,
        usdc_mint: pks.usdc_mint,
        position,
        side,
        leverage_u32,
        price_slippage_u64,
        counter: rand::random::<u64>() % 1_000_000_000,
    }
}

/// Top-level Jupiter perps write handler.
async fn handle_write_perp_jupiter<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    ctx: PerpWriteCtx<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mode = effective_network_mode(shared, conn);
    if let Some(resp) = jupiter_validate_prerequisites(&ctx, mode) {
        Keystore::release_lock(ctx.lock)?;
        return Ok(resp);
    }

    let PerpWriteCtx {
        req_id,
        tool_name,
        args,
        lock,
    } = ctx;
    let (w, idx) = resolve_wallet_and_account(shared, args)?;
    let market = args.get("market").and_then(Value::as_str).unwrap_or("");
    let Some(custody_s) = jupiter_perps::custody_for_market_mainnet(market) else {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "unknown_market",
                "unsupported market for jupiter_perps (supported: SOL, BTC, ETH)",
            )),
        ));
    };

    let sol = SolanaChain::new_with_fallbacks(
        &shared.cfg.rpc.solana_rpc_url,
        &shared.cfg.rpc.solana_fallback_rpc_urls_mainnet,
        &shared.cfg.http.jupiter_base_url,
        shared.cfg.http.jupiter_api_key.as_deref(),
        shared.cfg.rpc.solana_default_compute_unit_limit,
        shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );
    let keypair = load_solana_keypair(shared, conn, stdin, stdout, &w, idx).await?;
    let owner = keypair.pubkey();
    let (program_id, pool, custody, collateral_custody, usdc_mint) =
        jupiter_parse_write_pubkeys(custody_s)?;
    let pks = JupiterParsedPubkeys {
        program_id,
        pool,
        custody,
        collateral_custody,
        usdc_mint,
    };

    let infer_setup = jupiter_infer_setup(&pks);
    let side = match jupiter_resolve_write_side(tool_name, args, &sol, &infer_setup, &owner).await {
        Ok(s) => s,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };
    let (leverage_u32, price_slippage_u64) =
        match jupiter_resolve_leverage_and_slippage(shared, args, market, side).await {
            Ok(v) => v,
            Err(te) => {
                Keystore::release_lock(lock)?;
                return Ok(ok(req_id, tool_err(te)));
            }
        };

    let setup = jupiter_build_setup(&pks, &owner, side, leverage_u32, price_slippage_u64);
    let exec = JupiterPerpExecCtx {
        setup: &setup,
        sol: &sol,
        keypair: &keypair,
        wallet_name: &w.name,
        idx,
    };
    let write_ctx = PerpWriteCtx {
        req_id,
        tool_name,
        args,
        lock,
    };

    match tool_name {
        "open_perp_position" => {
            jupiter_open_perp(shared, conn, stdin, stdout, write_ctx, &exec).await
        }
        "close_perp_position" => {
            jupiter_close_perp(shared, conn, stdin, stdout, write_ctx, &exec).await
        }
        _ => {
            Keystore::release_lock(write_ctx.lock)?;
            Ok(ok(
                write_ctx.req_id,
                tool_err(ToolError::new("internal_error", "unreachable")),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// write_perp: Hyperliquid helpers
// ---------------------------------------------------------------------------

/// Resolve size in asset terms + USD value for Hyperliquid.
fn hyperliquid_compute_size(
    tool_name: &str,
    size_s: &str,
    size_units: &str,
    mid_px: f64,
    found_szi: Option<f64>,
) -> Result<(f64, f64), ToolError> {
    if tool_name == "close_perp_position" && size_s.trim().is_empty() {
        let szi = found_szi.ok_or_else(|| ToolError::new("no_position", "position not found"))?;
        let sz = financial_math::abs_f64(szi);
        let usd = financial_math::mul_f64(sz, mid_px);
        Ok((sz, usd))
    } else if size_units == "asset" {
        let sz = parse_f64(size_s, "size")?;
        Ok((sz, financial_math::mul_f64(sz, mid_px)))
    } else {
        let usd = parse_f64(size_s, "size")?;
        Ok((financial_math::div_f64(usd, mid_px), usd))
    }
}

/// Determine limit price and time-in-force for a Hyperliquid order.
fn hyperliquid_order_price(
    args: &Value,
    tool_name: &str,
    mid_px: f64,
    is_buy: bool,
    slippage: f64,
    sz_decimals: u32,
) -> Result<(String, &'static str), ToolError> {
    let order_type = args.get("order_type").and_then(|v| v.as_str()).unwrap_or(
        if tool_name == "place_limit_order" {
            "limit"
        } else {
            "market"
        },
    );
    if order_type == "limit" || tool_name == "place_limit_order" {
        let px_s = args.get("limit_px").and_then(|v| v.as_str()).unwrap_or("");
        if px_s.trim().is_empty() {
            return Err(ToolError::new(
                "invalid_request",
                "limit_px required for limit orders",
            ));
        }
        let px = parse_f64(px_s, "limit_px")?;
        let px_wire = hyperliquid::float_to_wire(px)
            .map_err(|e| ToolError::new("invalid_request", format!("{e:#}")))?;
        Ok((px_wire, "Gtc"))
    } else {
        let px = hyperliquid::slippage_limit_px(mid_px, is_buy, slippage, sz_decimals);
        let px_wire = hyperliquid::float_to_wire(px)
            .map_err(|e| ToolError::new("invalid_request", format!("{e:#}")))?;
        Ok((px_wire, "Ioc"))
    }
}

/// Look up the user's current position szi for close-full-position flow.
async fn hyperliquid_find_position_szi(
    client: &hyperliquid::HyperliquidClient,
    user: &str,
    coin: &str,
) -> eyre::Result<Option<f64>> {
    let st = client
        .info(json!({ "type": "clearinghouseState", "user": user }))
        .await
        .context("hyperliquid clearinghouseState")?;
    let mut found_szi = None;
    if let Some(pos) = st.get("assetPositions").and_then(|v| v.as_array()) {
        for p in pos {
            let market_coin = p
                .get("position")
                .and_then(|x| x.get("coin"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if !market_coin.eq_ignore_ascii_case(coin) {
                continue;
            }
            found_szi = p
                .get("position")
                .and_then(|x| x.get("szi"))
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok());
            break;
        }
    }
    Ok(found_szi)
}

fn hyperliquid_build_summary(
    tool_name: &str,
    is_buy: bool,
    coin: &str,
    usd_value: f64,
    leverage_u32: u32,
    limit_px: &str,
) -> String {
    match tool_name {
        "open_perp_position" => format!(
            "OPEN PERP on Hyperliquid: {} {} ({} USD, {}x)",
            if is_buy { "LONG" } else { "SHORT" },
            coin,
            usd_value,
            leverage_u32
        ),
        "place_limit_order" => format!(
            "PLACE LIMIT ORDER on Hyperliquid: {} {} ({} USD, {}x) @ {}",
            if is_buy { "BUY" } else { "SELL" },
            coin,
            usd_value,
            leverage_u32,
            limit_px
        ),
        "close_perp_position" => format!("CLOSE PERP on Hyperliquid: {coin}"),
        "modify_perp_order" => format!("MODIFY PERP ORDER on Hyperliquid: {coin}"),
        _ => "PERP".to_owned(),
    }
}

struct HyperliquidLogEntry<'a> {
    tool_name: &'a str,
    wallet_name: &'a str,
    idx: u32,
    coin: &'a str,
    is_buy: bool,
    sz_wire: &'a str,
    limit_px: &'a str,
    leverage_u32: u32,
    usd_value: f64,
    resp: &'a Value,
    outcome: &'a WriteConfirmOutcome,
}

fn hyperliquid_log_order(
    shared: &SharedState,
    entry: &HyperliquidLogEntry<'_>,
) -> eyre::Result<()> {
    let ty = match entry.tool_name {
        "open_perp_position" => "perp_open",
        "close_perp_position" => "perp_close",
        "modify_perp_order" => "perp_modify",
        "place_limit_order" => "perp_limit",
        _ => "perp",
    };
    shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": ty,
      "chain": "hyperliquid",
      "wallet": entry.wallet_name,
      "account_index": entry.idx,
      "provider": "hyperliquid",
      "market": entry.coin,
      "side": if entry.is_buy { "buy" } else { "sell" },
      "size_asset": entry.sz_wire,
      "limit_px": entry.limit_px,
      "leverage": entry.leverage_u32,
      "usd_value": entry.usd_value,
      "response": entry.resp
    }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(),
      "tool": entry.tool_name,
      "wallet": entry.wallet_name,
      "account_index": entry.idx,
      "chain": "hyperliquid",
      "usd_value": entry.usd_value,
      "usd_value_known": true,
      "policy_decision": entry.outcome.policy_decision,
      "confirm_required": entry.outcome.confirm_required,
      "confirm_result": entry.outcome.confirm_result,
      "forced_confirm": entry.outcome.forced_confirm,
      "daily_used_usd": entry.outcome.daily_used_usd,
      "txid": null,
      "error_code": null,
      "provider": "hyperliquid",
      "result": "submitted"
    }));
    Ok(())
}

/// Validated and resolved order parameters for Hyperliquid.
struct HyperliquidPreparedOrder<'a> {
    market: &'a hyperliquid::HyperliquidMarket,
    is_buy: bool,
    leverage_u32: u32,
    sz_wire: String,
    limit_px: String,
    tif: &'static str,
    usd_value: f64,
    summary: String,
    op: WriteOp,
}

/// Parse side from args and return `is_buy` flag.
fn hyperliquid_parse_side(args: &Value) -> Result<bool, ToolError> {
    let side = args.get("side").and_then(|v| v.as_str()).unwrap_or("");
    match side.trim().to_lowercase().as_str() {
        "long" | "buy" => Ok(true),
        "short" | "sell" => Ok(false),
        _ => Err(ToolError::new(
            "invalid_request",
            "side must be long or short",
        )),
    }
}

/// Validate leverage against venue max and parse slippage.
fn hyperliquid_parse_leverage_slippage(
    args: &Value,
    max_leverage: u32,
) -> Result<(u32, f64), ToolError> {
    let leverage_u32 = parse_u32(
        args.get("leverage")
            .and_then(serde_json::Value::as_u64)
            .or(Some(1)),
        "leverage",
    )?;
    if max_leverage > 0 && leverage_u32 > max_leverage {
        return Err(ToolError::new(
            "invalid_request",
            format!("leverage exceeds venue maxLeverage {max_leverage}"),
        ));
    }
    let slippage_bps = args
        .get("slippage_bps")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(50);
    Ok((leverage_u32, financial_math::bps_to_fraction(slippage_bps)))
}

/// Resolve size and price wires for a Hyperliquid order.
async fn hyperliquid_resolve_size_and_price(
    ctx: &PerpWriteCtx<'_>,
    client: &hyperliquid::HyperliquidClient,
    m: &hyperliquid::HyperliquidMarket,
    evm_address: &str,
    is_buy: bool,
    slippage: f64,
) -> Result<(String, String, &'static str, f64), ToolError> {
    let size_s = ctx.args.get("size").and_then(|v| v.as_str()).unwrap_or("");
    let size_units = ctx
        .args
        .get("size_units")
        .and_then(|v| v.as_str())
        .unwrap_or("usd");
    if size_s.trim().is_empty() && ctx.tool_name != "close_perp_position" {
        return Err(ToolError::new("invalid_request", "missing size"));
    }

    let mid_px = m.mid_px.or(m.mark_px).ok_or_else(|| {
        ToolError::new(
            "internal_error",
            format!("missing midPx/markPx for market {}", m.coin),
        )
    })?;

    let found_szi = if ctx.tool_name == "close_perp_position" && size_s.trim().is_empty() {
        if evm_address.is_empty() {
            return Err(ToolError::new(
                "missing_address",
                "wallet has no evm address",
            ));
        }
        hyperliquid_find_position_szi(client, evm_address, &m.coin)
            .await
            .map_err(|e| ToolError::new("internal_error", format!("{e:#}")))?
    } else {
        None
    };

    let (sz_asset, usd_value) =
        hyperliquid_compute_size(ctx.tool_name, size_s, size_units, mid_px, found_szi)?;
    let sz_wire = hyperliquid::float_to_wire(sz_asset)
        .map_err(|e| ToolError::new("invalid_request", format!("{e:#}")))?;
    let (limit_px, tif) = hyperliquid_order_price(
        ctx.args,
        ctx.tool_name,
        mid_px,
        is_buy,
        slippage,
        m.sz_decimals,
    )?;
    Ok((sz_wire, limit_px, tif, usd_value))
}

/// Map tool name to the corresponding write operation.
fn hyperliquid_tool_to_op(tool_name: &str) -> WriteOp {
    match tool_name {
        "close_perp_position" => WriteOp::ClosePerpPosition,
        "modify_perp_order" => WriteOp::ModifyPerpOrder,
        "place_limit_order" => WriteOp::PlaceLimitOrder,
        _ => WriteOp::OpenPerpPosition,
    }
}

/// Validate args and compute order parameters for a Hyperliquid perp order.
async fn hyperliquid_prepare_order<'a>(
    ctx: &PerpWriteCtx<'_>,
    client: &hyperliquid::HyperliquidClient,
    markets: &'a [hyperliquid::HyperliquidMarket],
    evm_address: &str,
) -> Result<HyperliquidPreparedOrder<'a>, ToolError> {
    let market = ctx
        .args
        .get("market")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let m = find_market(markets, market)
        .ok_or_else(|| ToolError::new("unknown_market", "unknown market"))?;

    let is_buy = hyperliquid_parse_side(ctx.args)?;
    let (leverage_u32, slippage) = hyperliquid_parse_leverage_slippage(ctx.args, m.max_leverage)?;

    let (sz_wire, limit_px, tif, usd_value) =
        hyperliquid_resolve_size_and_price(ctx, client, m, evm_address, is_buy, slippage).await?;

    let summary = hyperliquid_build_summary(
        ctx.tool_name,
        is_buy,
        &m.coin,
        usd_value,
        leverage_u32,
        &limit_px,
    );

    Ok(HyperliquidPreparedOrder {
        market: m,
        is_buy,
        leverage_u32,
        sz_wire,
        limit_px,
        tif,
        usd_value,
        summary,
        op: hyperliquid_tool_to_op(ctx.tool_name),
    })
}

/// Build a Hyperliquid write-confirmation request.
fn hyperliquid_confirm_request<'a>(
    tool_name: &'a str,
    wallet_name: &'a str,
    idx: u32,
    prep: &'a HyperliquidPreparedOrder<'_>,
) -> WriteConfirmRequest<'a> {
    WriteConfirmRequest {
        tool: tool_name,
        wallet: Some(wallet_name),
        account_index: Some(idx),
        op: prep.op,
        chain: "hyperliquid",
        usd_value: prep.usd_value,
        usd_value_known: true,
        force_confirm: false,
        slippage_bps: None,
        to_address: None,
        contract: Some("hyperliquid"),
        leverage: Some(prep.leverage_u32),
        summary: &prep.summary,
    }
}

/// Execute the on-chain Hyperliquid order: set leverage, optionally cancel, then place order.
async fn hyperliquid_execute_order(
    session: &hyperliquid::SessionParams<'_>,
    prep: &HyperliquidPreparedOrder<'_>,
    tool_name: &str,
    args: &Value,
    is_mainnet: bool,
) -> eyre::Result<Value> {
    let _leverage_res = hyperliquid::post_update_leverage(
        session,
        &hyperliquid::LeverageParams {
            asset: prep.market.asset,
            leverage: prep.leverage_u32,
            is_cross: true,
        },
    )
    .await;

    if tool_name == "modify_perp_order" {
        if let Some(oid) = args.get("oid").and_then(serde_json::Value::as_u64) {
            let _cancel_res = hyperliquid::post_cancel(
                session.client,
                session.wallet,
                is_mainnet,
                None,
                None,
                prep.market.asset,
                oid,
            )
            .await;
        } else {
            eyre::bail!("missing oid for modify_perp_order");
        }
    }

    hyperliquid::post_order(
        session,
        &hyperliquid::OrderParams {
            asset: prep.market.asset,
            is_buy: prep.is_buy,
            sz: &prep.sz_wire,
            limit_px: &prep.limit_px,
            reduce_only: tool_name == "close_perp_position",
            tif: prep.tif,
        },
    )
    .await
    .context("hyperliquid order")
}

/// Place (or modify) a Hyperliquid order after policy confirmation.
async fn hyperliquid_submit_order<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    ctx: PerpWriteCtx<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (w, idx) = resolve_wallet_and_account(shared, ctx.args)?;
    let base_url = hyperliquid_base_url(shared, conn);
    let client = hyperliquid::HyperliquidClient::new(&base_url)?;
    let markets = client.meta_and_asset_ctxs().await?;

    let evm_address = w
        .evm_addresses
        .get(idx as usize)
        .cloned()
        .unwrap_or_default();
    let prep = match hyperliquid_prepare_order(&ctx, &client, &markets, &evm_address).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };

    let confirm_req = hyperliquid_confirm_request(ctx.tool_name, &w.name, idx, &prep);
    let outcome = match maybe_confirm_write(shared, conn, stdin, stdout, &confirm_req).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(ctx.req_id, tool_err(te)));
        }
    };

    let is_mainnet = effective_network_mode(shared, conn) == crate::config::NetworkMode::Mainnet;
    let signer = load_evm_signer(shared, conn, stdin, stdout, &w, idx).await?;
    let session = hyperliquid::SessionParams {
        client: &client,
        wallet: &signer,
        is_mainnet,
        vault_address: None,
        expires_after: None,
    };

    let resp = match hyperliquid_execute_order(&session, &prep, ctx.tool_name, ctx.args, is_mainnet)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(ctx.lock)?;
            return Ok(ok(
                ctx.req_id,
                tool_err(ToolError::new("tx_failed", format!("{e:#}"))),
            ));
        }
    };

    hyperliquid_log_order(
        shared,
        &HyperliquidLogEntry {
            tool_name: ctx.tool_name,
            wallet_name: &w.name,
            idx,
            coin: &prep.market.coin,
            is_buy: prep.is_buy,
            sz_wire: &prep.sz_wire,
            limit_px: &prep.limit_px,
            leverage_u32: prep.leverage_u32,
            usd_value: prep.usd_value,
            resp: &resp,
            outcome: &outcome,
        },
    )?;

    Keystore::release_lock(ctx.lock)?;
    Ok(ok(
        ctx.req_id,
        tool_ok(json!({ "provider": "hyperliquid", "response": resp })),
    ))
}

// ---------------------------------------------------------------------------
// write_perp: dispatch
// ---------------------------------------------------------------------------

async fn handle_write_perp<R, W>(
    req_id: Value,
    tool_name: &str,
    args: &Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = shared.ks.acquire_write_lock()?;

    let provider = parse_provider(args);
    match provider {
        "jupiter_perps" => {
            let ctx = PerpWriteCtx {
                req_id,
                tool_name,
                args,
                lock,
            };
            handle_write_perp_jupiter(shared, conn, stdin, stdout, ctx).await
        }
        "hyperliquid" => {
            let ctx = PerpWriteCtx {
                req_id,
                tool_name,
                args,
                lock,
            };
            hyperliquid_submit_order(shared, conn, stdin, stdout, ctx).await
        }
        other => {
            Keystore::release_lock(lock)?;
            Ok(ok(
                req_id,
                tool_err(ToolError::new(
                    "invalid_request",
                    format!("unsupported provider: {other}"),
                )),
            ))
        }
    }
}

pub async fn handle<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match tool_name {
        "get_market_data" => handle_get_market_data(req_id, &args, shared, conn).await,
        "get_positions" => handle_get_positions(req_id, &args, shared, conn).await,
        "open_perp_position"
        | "place_limit_order"
        | "close_perp_position"
        | "modify_perp_order" => {
            handle_write_perp(req_id, tool_name, &args, shared, conn, stdin, stdout).await
        }
        _ => Ok(err(req_id, -32601, "unknown tool")),
    }
}
