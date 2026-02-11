use serde_json::{json, Value};

use crate::errors::ToolError;
use crate::keystore::{utc_now_iso, Keystore};
use crate::policy_engine::WriteOp;
use polymarket_client_sdk::auth::Signer as _;
use polymarket_client_sdk::clob::types::response::GeoblockResponse;
use rust_decimal::prelude::ToPrimitive as _;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::helpers::resolve_wallet_and_account;
use super::super::key_loading::load_evm_signer;
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::parse_usd_value;
use super::HandlerCtx;

type AuthdClobClient = polymarket_client_sdk::clob::Client<
    polymarket_client_sdk::auth::state::Authenticated<polymarket_client_sdk::auth::Normal>,
>;

type PolySigner = polymarket_client_sdk::auth::LocalSigner<k256::ecdsa::SigningKey>;

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
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

fn polymarket_chain_id(chain: &str) -> Result<u64, ToolError> {
    // Polymarket runs on Polygon PoS.
    match chain.trim().to_lowercase().as_str() {
        "polygon" => Ok(polymarket_client_sdk::POLYGON),
        // Useful for local testing against the Polygon testnet ecosystem.
        "polygon-amoy" | "amoy" => Ok(polymarket_client_sdk::AMOY),
        _ => Err(ToolError::new(
            "unsupported_chain",
            "polymarket tools support: polygon (or polygon-amoy for testing)",
        )),
    }
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

fn parse_decimal(s: &str) -> Result<polymarket_client_sdk::types::Decimal, ToolError> {
    use std::str::FromStr as _;
    polymarket_client_sdk::types::Decimal::from_str(s.trim()).map_err(|e| {
        ToolError::new(
            "invalid_request",
            format!("invalid decimal string {s:?}: {e}"),
        )
    })
}

fn tool_requires_polymarket_protocol(args: &Value) -> Result<(), ToolError> {
    let protocol = arg_str(args, "protocol").unwrap_or("polymarket");
    if protocol != "polymarket" {
        return Err(ToolError::new(
            "invalid_request",
            "protocol must be polymarket",
        ));
    }
    Ok(())
}

fn polymarket_not_configured(which: &'static str) -> ToolError {
    ToolError::new(
        "polymarket_not_configured",
        format!("Polymarket is not configured; set http.polymarket_{which}_base_url"),
    )
}

fn parse_order_type(
    args: &Value,
) -> Result<polymarket_client_sdk::clob::types::OrderType, ToolError> {
    let tif = arg_str(args, "time_in_force").unwrap_or("gtc");
    match tif.to_lowercase().as_str() {
        "gtc" => Ok(polymarket_client_sdk::clob::types::OrderType::GTC),
        "gtd" => Ok(polymarket_client_sdk::clob::types::OrderType::GTD),
        "fok" => Ok(polymarket_client_sdk::clob::types::OrderType::FOK),
        "fak" => Ok(polymarket_client_sdk::clob::types::OrderType::FAK),
        _ => Err(ToolError::new(
            "invalid_request",
            "time_in_force must be one of: gtc, gtd, fok, fak",
        )),
    }
}

fn parse_side(args: &Value) -> Result<polymarket_client_sdk::clob::types::Side, ToolError> {
    let side = arg_str(args, "side").unwrap_or("");
    match side.to_lowercase().as_str() {
        "buy" => Ok(polymarket_client_sdk::clob::types::Side::Buy),
        "sell" => Ok(polymarket_client_sdk::clob::types::Side::Sell),
        _ => Err(ToolError::new(
            "invalid_request",
            "side must be buy or sell",
        )),
    }
}

fn polymarket_err_to_tool(e: &polymarket_client_sdk::error::Error, action: &str) -> ToolError {
    use polymarket_client_sdk::error::Kind;
    match e.kind() {
        Kind::Geoblock => ToolError::new(
            "polymarket_geoblocked",
            format!(
                "Polymarket access is geo-restricted from this IP/jurisdiction ({action}). \
Polymarket is generally permissionless, but some locations (including the US) may be blocked."
            ),
        ),
        Kind::Status
        | Kind::Validation
        | Kind::Synchronization
        | Kind::Internal
        | Kind::WebSocket
        | _ => ToolError::new("upstream_error", format!("polymarket {action}: {e}")),
    }
}

fn geoblock_blocked(action: &str, geo: &GeoblockResponse) -> ToolError {
    ToolError::new(
        "polymarket_geoblocked",
        format!(
            "Polymarket access is geo-restricted from this IP/jurisdiction ({action}). \
Polymarket is generally permissionless, but some locations (including the US) may be blocked. \
Detected country={} region={}.",
            geo.country, geo.region
        ),
    )
}

/// Validated common Polymarket context shared between place/close operations.
struct PolymarketCtx {
    lock: std::fs::File,
    chain: String,
    chain_id: u64,
    clob_base: String,
    geoblock_base: String,
    w: crate::wallet::WalletRecord,
    idx: u32,
}

/// Parse and validate the common Polymarket parameters.
fn parse_polymarket_common<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    lock: std::fs::File,
) -> eyre::Result<Result<PolymarketCtx, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let req_id = ctx.req_id.clone();
    let chain = ctx
        .args
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if chain.trim().is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing chain")),
        )));
    }
    let chain_id = match polymarket_chain_id(&chain) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(Err(ok(req_id, tool_err(te))));
        }
    };
    let clob_base = ctx
        .shared
        .cfg
        .http
        .polymarket_clob_base_url
        .trim()
        .to_owned();
    if clob_base.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(req_id, tool_err(polymarket_not_configured("clob")))));
    }
    if let Err(te) = ensure_https_or_loopback(&clob_base, "polymarket_clob_base_url") {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(req_id, tool_err(te))));
    }
    let geoblock_base = ctx
        .shared
        .cfg
        .http
        .polymarket_geoblock_base_url
        .trim()
        .to_owned();
    if geoblock_base.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_config",
                "polymarket_geoblock_base_url is empty",
            )),
        )));
    }
    if let Err(te) = ensure_https_or_loopback(&geoblock_base, "polymarket_geoblock_base_url") {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(req_id, tool_err(te))));
    }
    let (w, idx) = resolve_wallet_and_account(ctx.shared, &ctx.args)?;
    Ok(Ok(PolymarketCtx {
        lock,
        chain,
        chain_id,
        clob_base,
        geoblock_base,
        w,
        idx,
    }))
}

/// Initialize an unauthenticated CLOB client and perform the geoblock check.
async fn polymarket_init_unauth(
    clob_base: &str,
    geoblock_base: &str,
    action: &str,
    lock: &std::fs::File,
    req_id: &Value,
) -> eyre::Result<Result<polymarket_client_sdk::clob::Client, JsonRpcResponse>> {
    let clob_cfg = polymarket_client_sdk::clob::Config::builder()
        .geoblock_host(geoblock_base.to_owned())
        .build();
    let unauth = match polymarket_client_sdk::clob::Client::new(clob_base, clob_cfg) {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(
                req_id.clone(),
                tool_err(ToolError::new(
                    "upstream_error",
                    format!("polymarket clob client init: {e}"),
                )),
            )));
        }
    };
    if let Ok(geo) = unauth.check_geoblock().await {
        if geo.blocked {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(
                req_id.clone(),
                tool_err(geoblock_blocked(action, &geo)),
            )));
        }
    }
    Ok(Ok(unauth))
}

/// Load EVM signer and convert to a Polymarket local signer.
async fn polymarket_load_signer<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pc: &PolymarketCtx,
) -> eyre::Result<Result<PolySigner, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let evm_wallet =
        match load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &pc.w, pc.idx).await {
            Ok(v) => v,
            Err(e) => {
                Keystore::release_lock(pc.lock.try_clone()?)?;
                return Ok(Err(ok(
                    ctx.req_id.clone(),
                    tool_err(ToolError::new("signer_error", e.to_string())),
                )));
            }
        };
    let sk = evm_wallet.credential().to_bytes();
    let mut signer = match PolySigner::from_slice(sk.as_slice()) {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(pc.lock.try_clone()?)?;
            return Ok(Err(ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new(
                    "signer_error",
                    format!("convert evm signer: {e}"),
                )),
            )));
        }
    };
    signer.set_chain_id(Some(pc.chain_id));
    Ok(Ok(signer))
}

/// Handle the `place_prediction` tool.
async fn handle_place_prediction<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pc: PolymarketCtx,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if let Err(te) = tool_requires_polymarket_protocol(&ctx.args) {
        Keystore::release_lock(pc.lock)?;
        return Ok(ok(ctx.req_id.clone(), tool_err(te)));
    }
    let unauth = match polymarket_init_unauth(
        &pc.clob_base,
        &pc.geoblock_base,
        "place_prediction",
        &pc.lock,
        &ctx.req_id,
    )
    .await?
    {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let parsed = match parse_place_prediction_args(ctx, &pc)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "place_prediction",
            wallet: Some(pc.w.name.as_str()),
            account_index: Some(pc.idx),
            op: WriteOp::PlacePrediction,
            chain: &pc.chain,
            usd_value: parsed.usd_value,
            usd_value_known: parsed.usd_value_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: None,
            contract: None,
            leverage: None,
            summary: &parsed.summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(pc.lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    let signer = match polymarket_load_signer(ctx, &pc).await? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let clob = match unauth.authentication_builder(&signer).authenticate().await {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(pc.lock)?;
            return Ok(ok(
                ctx.req_id.clone(),
                tool_err(polymarket_err_to_tool(&e, "authenticate")),
            ));
        }
    };

    let args = ctx.args.clone();
    let resp =
        match polymarket_build_and_post(&args, &pc, &clob, &signer, &parsed, &ctx.req_id).await? {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

    let _history = ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "type": "prediction_place", "wallet": pc.w.name,
      "account_index": pc.idx, "chain": pc.chain, "protocol": "polymarket",
      "token_id": parsed.token_id.to_string(), "order_id": resp.order_id,
      "status": format!("{:?}", resp.status), "success": resp.success,
      "usd_value": parsed.usd_value, "usd_value_known": parsed.usd_value_known,
    }));
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": "place_prediction", "wallet": pc.w.name,
      "account_index": pc.idx, "chain": pc.chain, "protocol": "polymarket",
      "token_id": parsed.token_id.to_string(), "order_id": resp.order_id,
      "usd_value": parsed.usd_value, "usd_value_known": parsed.usd_value_known,
      "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm, "result": "order_posted"
    }));

    Keystore::release_lock(pc.lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
          "chain": pc.chain, "protocol": "polymarket", "token_id": parsed.token_id.to_string(),
          "order_id": resp.order_id, "success": resp.success, "status": format!("{:?}", resp.status),
          "transaction_hashes": resp.transaction_hashes.iter().map(|h| format!("{h:?}")).collect::<Vec<_>>(),
          "trade_ids": resp.trade_ids, "error_msg": resp.error_msg,
          "usd_value": parsed.usd_value, "usd_value_known": parsed.usd_value_known,
        })),
    ))
}

/// Parsed arguments for `place_prediction`.
struct ParsedPlacePrediction {
    token_id: polymarket_client_sdk::types::U256,
    side: polymarket_client_sdk::clob::types::Side,
    order_type: polymarket_client_sdk::clob::types::OrderType,
    post_only: bool,
    order_kind: String,
    usd_value: f64,
    usd_value_known: bool,
    summary: String,
}

/// Parse and validate the market-order summary and USD value.
fn parse_market_summary(
    args: &Value,
    side: polymarket_client_sdk::clob::types::Side,
    token_id: polymarket_client_sdk::types::U256,
    usd_value: f64,
) -> Result<(String, f64, bool), ToolError> {
    let amt_s = arg_str(args, "amount_usdc")
        .ok_or_else(|| ToolError::new("invalid_request", "market orders require amount_usdc"))?;
    let amt = parse_decimal(amt_s)?;
    let usd = amt.to_f64().unwrap_or(usd_value).max(usd_value);
    Ok((
        format!("POLYMARKET market {side:?} token_id={token_id} amount_usdc={amt}"),
        usd,
        true,
    ))
}

/// Parse and validate the limit-order summary and USD value.
fn parse_limit_summary(
    args: &Value,
    side: polymarket_client_sdk::clob::types::Side,
    token_id: polymarket_client_sdk::types::U256,
    usd_value: f64,
    usd_value_known: bool,
) -> Result<(String, f64, bool), ToolError> {
    let price_s = arg_str(args, "price")
        .ok_or_else(|| ToolError::new("invalid_request", "limit orders require price"))?;
    let quantity_s = arg_str(args, "size")
        .ok_or_else(|| ToolError::new("invalid_request", "limit orders require size"))?;
    let price = parse_decimal(price_s)?;
    let quantity = parse_decimal(quantity_s)?;
    let (usd, known) = if usd_value_known {
        (usd_value, usd_value_known)
    } else if let Some(n) = (price * quantity).to_f64() {
        (n, true)
    } else {
        (usd_value, usd_value_known)
    };
    Ok((
        format!("POLYMARKET limit {side:?} token_id={token_id} price={price} size={quantity}"),
        usd,
        known,
    ))
}

fn parse_place_prediction_args<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    pc: &PolymarketCtx,
) -> eyre::Result<Result<ParsedPlacePrediction, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let req_id = ctx.req_id.clone();
    let Some(token_id_s) = arg_str(&ctx.args, "token_id") else {
        Keystore::release_lock(pc.lock.try_clone()?)?;
        return Ok(Err(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing token_id")),
        )));
    };
    let token_id = match parse_u256_decimal_str(token_id_s) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(pc.lock.try_clone()?)?;
            return Ok(Err(ok(req_id, tool_err(te))));
        }
    };
    let side = match parse_side(&ctx.args) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(pc.lock.try_clone()?)?;
            return Ok(Err(ok(req_id, tool_err(te))));
        }
    };
    let order_type = match parse_order_type(&ctx.args) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(pc.lock.try_clone()?)?;
            return Ok(Err(ok(req_id, tool_err(te))));
        }
    };
    let post_only = arg_bool(&ctx.args, "post_only").unwrap_or(false);
    let order_kind = arg_str(&ctx.args, "order_kind")
        .unwrap_or("limit")
        .to_owned();
    let (usd_value, usd_value_known) = parse_usd_value(&ctx.args);

    let (summary, usd_value, usd_value_known) = if order_kind == "market" {
        match parse_market_summary(&ctx.args, side, token_id, usd_value) {
            Ok(v) => v,
            Err(te) => {
                Keystore::release_lock(pc.lock.try_clone()?)?;
                return Ok(Err(ok(req_id, tool_err(te))));
            }
        }
    } else {
        match parse_limit_summary(&ctx.args, side, token_id, usd_value, usd_value_known) {
            Ok(v) => v,
            Err(te) => {
                Keystore::release_lock(pc.lock.try_clone()?)?;
                return Ok(Err(ok(req_id, tool_err(te))));
            }
        }
    };

    Ok(Ok(ParsedPlacePrediction {
        token_id,
        side,
        order_type,
        post_only,
        order_kind,
        usd_value,
        usd_value_known,
        summary,
    }))
}

/// Build a Polymarket market order signable.
async fn polymarket_build_market_signable(
    args: &Value,
    clob: &AuthdClobClient,
    parsed: &ParsedPlacePrediction,
    lock: &std::fs::File,
    req_id: &Value,
) -> eyre::Result<Result<polymarket_client_sdk::clob::types::SignableOrder, JsonRpcResponse>> {
    let amt_s = arg_str(args, "amount_usdc").unwrap_or("");
    let amount_usdc = match parse_decimal(amt_s) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(req_id.clone(), tool_err(te))));
        }
    };
    let amt = match polymarket_client_sdk::clob::types::Amount::usdc(amount_usdc) {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(
                req_id.clone(),
                tool_err(ToolError::new(
                    "invalid_request",
                    format!("invalid amount_usdc: {e}"),
                )),
            )));
        }
    };
    match clob
        .market_order()
        .token_id(parsed.token_id)
        .side(parsed.side)
        .order_type(parsed.order_type.clone())
        .post_only(parsed.post_only)
        .amount(amt)
        .build()
        .await
    {
        Ok(v) => Ok(Ok(v)),
        Err(e) => {
            Keystore::release_lock(lock.try_clone()?)?;
            Ok(Err(ok(
                req_id.clone(),
                tool_err(ToolError::new(
                    "invalid_request",
                    format!("build market order: {e}"),
                )),
            )))
        }
    }
}

/// Build a Polymarket limit order signable.
async fn polymarket_build_limit_signable(
    args: &Value,
    clob: &AuthdClobClient,
    parsed: &ParsedPlacePrediction,
    lock: &std::fs::File,
    req_id: &Value,
) -> eyre::Result<Result<polymarket_client_sdk::clob::types::SignableOrder, JsonRpcResponse>> {
    let price_s = arg_str(args, "price").unwrap_or("");
    let quantity_s = arg_str(args, "size").unwrap_or("");
    let price = match parse_decimal(price_s) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(req_id.clone(), tool_err(te))));
        }
    };
    let quantity = match parse_decimal(quantity_s) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(req_id.clone(), tool_err(te))));
        }
    };
    match clob
        .limit_order()
        .token_id(parsed.token_id)
        .side(parsed.side)
        .price(price)
        .size(quantity)
        .order_type(parsed.order_type.clone())
        .post_only(parsed.post_only)
        .build()
        .await
    {
        Ok(v) => Ok(Ok(v)),
        Err(e) => {
            Keystore::release_lock(lock.try_clone()?)?;
            Ok(Err(ok(
                req_id.clone(),
                tool_err(ToolError::new(
                    "invalid_request",
                    format!("build limit order: {e}"),
                )),
            )))
        }
    }
}

/// Sign and post a Polymarket order.
async fn polymarket_sign_and_post(
    clob: &AuthdClobClient,
    signer: &PolySigner,
    signable: polymarket_client_sdk::clob::types::SignableOrder,
    lock: &std::fs::File,
    req_id: &Value,
) -> eyre::Result<
    Result<polymarket_client_sdk::clob::types::response::PostOrderResponse, JsonRpcResponse>,
> {
    let order_signed = match clob.sign(signer, signable).await {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(
                req_id.clone(),
                tool_err(polymarket_err_to_tool(&e, "sign")),
            )));
        }
    };
    match clob.post_order(order_signed).await {
        Ok(v) => Ok(Ok(v)),
        Err(e) => {
            Keystore::release_lock(lock.try_clone()?)?;
            Ok(Err(ok(
                req_id.clone(),
                tool_err(polymarket_err_to_tool(&e, "post_order")),
            )))
        }
    }
}

/// Build, sign, and submit a Polymarket order (market or limit).
async fn polymarket_build_and_post(
    args: &Value,
    pc: &PolymarketCtx,
    clob: &AuthdClobClient,
    signer: &PolySigner,
    parsed: &ParsedPlacePrediction,
    req_id: &Value,
) -> eyre::Result<
    Result<polymarket_client_sdk::clob::types::response::PostOrderResponse, JsonRpcResponse>,
> {
    let signable = if parsed.order_kind == "market" {
        match polymarket_build_market_signable(args, clob, parsed, &pc.lock, req_id).await? {
            Ok(v) => v,
            Err(resp) => return Ok(Err(resp)),
        }
    } else {
        match polymarket_build_limit_signable(args, clob, parsed, &pc.lock, req_id).await? {
            Ok(v) => v,
            Err(resp) => return Ok(Err(resp)),
        }
    };
    polymarket_sign_and_post(clob, signer, signable, &pc.lock, req_id).await
}

/// Authenticate, cancel order, record history, and build response.
async fn close_prediction_execute<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pc: &PolymarketCtx,
    unauth: polymarket_client_sdk::clob::Client,
    order_id: &str,
    usd_value: f64,
    usd_value_known: bool,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let signer = match polymarket_load_signer(ctx, pc).await? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let auth_result = unauth.authentication_builder(&signer).authenticate().await;
    let clob = match auth_result {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(pc.lock.try_clone()?)?;
            return Ok(ok(
                ctx.req_id.clone(),
                tool_err(polymarket_err_to_tool(&e, "authenticate")),
            ));
        }
    };
    let cancel = match clob.cancel_order(order_id).await {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(pc.lock.try_clone()?)?;
            return Ok(ok(
                ctx.req_id.clone(),
                tool_err(polymarket_err_to_tool(&e, "cancel_order")),
            ));
        }
    };
    let _history = ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "type": "prediction_close", "wallet": pc.w.name,
      "account_index": pc.idx, "chain": pc.chain, "protocol": "polymarket",
      "order_id": order_id, "usd_value": usd_value, "usd_value_known": usd_value_known,
      "canceled": cancel.canceled,
    }));
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": "close_prediction", "wallet": pc.w.name,
      "account_index": pc.idx, "chain": pc.chain, "protocol": "polymarket",
      "order_id": order_id, "usd_value": usd_value, "usd_value_known": usd_value_known,
      "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm, "result": "order_canceled"
    }));
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
          "chain": pc.chain, "protocol": "polymarket", "order_id": order_id,
          "canceled": cancel.canceled, "not_canceled": cancel.not_canceled,
          "usd_value": usd_value, "usd_value_known": usd_value_known,
        })),
    ))
}

/// Handle the `close_prediction` tool.
async fn handle_close_prediction<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pc: PolymarketCtx,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if let Err(te) = tool_requires_polymarket_protocol(&ctx.args) {
        Keystore::release_lock(pc.lock)?;
        return Ok(ok(ctx.req_id.clone(), tool_err(te)));
    }
    let unauth = match polymarket_init_unauth(
        &pc.clob_base,
        &pc.geoblock_base,
        "close_prediction",
        &pc.lock,
        &ctx.req_id,
    )
    .await?
    {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let Some(order_id_str) = arg_str(&ctx.args, "order_id") else {
        Keystore::release_lock(pc.lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "missing order_id")),
        ));
    };
    let order_id = order_id_str.to_owned();
    let (usd_value, usd_value_known_raw) = parse_usd_value(&ctx.args);
    let usd_value_known = usd_value_known_raw || usd_value == 0.0_f64;

    let summary = format!("POLYMARKET cancel order_id={order_id}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "close_prediction",
            wallet: Some(pc.w.name.as_str()),
            account_index: Some(pc.idx),
            op: WriteOp::ClosePrediction,
            chain: &pc.chain,
            usd_value,
            usd_value_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: None,
            contract: None,
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(pc.lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    let resp = close_prediction_execute(
        ctx,
        &pc,
        unauth,
        &order_id,
        usd_value,
        usd_value_known,
        &outcome,
    )
    .await?;
    Keystore::release_lock(pc.lock)?;
    Ok(resp)
}

pub async fn handle<R, W>(
    tool_name: &str,
    ctx: &mut HandlerCtx<'_, R, W>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let pc = match parse_polymarket_common(ctx, lock)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    match tool_name {
        "place_prediction" => handle_place_prediction(ctx, pc).await,
        "close_prediction" => handle_close_prediction(ctx, pc).await,
        _ => {
            Keystore::release_lock(pc.lock)?;
            Ok(ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new("invalid_request", "unknown tool")),
            ))
        }
    }
}
