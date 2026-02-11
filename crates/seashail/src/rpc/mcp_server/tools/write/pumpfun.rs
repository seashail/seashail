use crate::{
    chains::solana::SolanaChain,
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
    marketplace_adapter,
    policy_engine::WriteOp,
};
use base64::Engine as _;
use eyre::Context as _;
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::helpers::{resolve_wallet_and_account, solana_fallback_urls};
use super::super::key_loading::load_solana_keypair;
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::HandlerCtx;

struct ParsedPumpfunOp {
    op: WriteOp,
    summary: String,
    asset: Value,
    amount_sol: f64,
    percent: f64,
}

fn parse_pumpfun_op(
    tool_name: &str,
    args: &Value,
    mint: &str,
    effective_policy: &crate::policy::Policy,
    ks: &Keystore,
    wallet_name: &str,
    idx: u32,
) -> Result<ParsedPumpfunOp, ToolError> {
    match tool_name {
        "pumpfun_buy" => {
            let amount_sol = args
                .get("amount_sol")
                .and_then(Value::as_f64)
                .unwrap_or(0.0_f64);
            if amount_sol <= 0.0_f64 || !amount_sol.is_finite() {
                return Err(ToolError::new("invalid_request", "amount_sol must be > 0"));
            }
            if amount_sol > effective_policy.pumpfun_max_sol_per_buy {
                return Err(ToolError::new(
                    "policy_pumpfun_max_sol_per_buy",
                    format!(
                        "amount_sol {:.4} exceeds pumpfun_max_sol_per_buy {:.4}",
                        amount_sol, effective_policy.pumpfun_max_sol_per_buy
                    ),
                ));
            }
            let entries = ks
                .read_tx_history_filtered(
                    2000,
                    Some(wallet_name),
                    Some("solana"),
                    Some("pumpfun_buy"),
                    Some(&(chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339()),
                    None,
                )
                .unwrap_or_default();
            let recent = entries
                .iter()
                .filter(|v| v.get("account_index").and_then(Value::as_u64) == Some(u64::from(idx)))
                .count();
            if recent >= effective_policy.pumpfun_max_buys_per_hour as usize {
                return Err(ToolError::new(
                    "policy_pumpfun_rate_limited",
                    "pump.fun buy rate limit exceeded (pumpfun_max_buys_per_hour)",
                ));
            }
            Ok(ParsedPumpfunOp {
                op: WriteOp::PumpfunBuy,
                summary: format!("PUMPFUN BUY {mint} spend {amount_sol:.4} SOL"),
                asset: json!({ "mint": mint, "amount_sol": amount_sol }),
                amount_sol,
                percent: 0.0_f64,
            })
        }
        "pumpfun_sell" => {
            let percent = args
                .get("percent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0_f64);
            if percent <= 0.0_f64 || percent > 100.0_f64 || !percent.is_finite() {
                return Err(ToolError::new(
                    "invalid_request",
                    "percent must be in (0, 100]",
                ));
            }
            Ok(ParsedPumpfunOp {
                op: WriteOp::PumpfunSell,
                summary: format!("PUMPFUN SELL {mint} {percent:.2}%"),
                asset: json!({ "mint": mint, "percent": percent }),
                amount_sol: 0.0_f64,
                percent,
            })
        }
        _ => Err(ToolError::new("unknown_tool", "unknown tool")),
    }
}

fn pumpfun_op_str(op: WriteOp) -> Result<&'static str, ToolError> {
    match op {
        WriteOp::PumpfunBuy => Ok("buy"),
        WriteOp::PumpfunSell => Ok("sell"),
        WriteOp::Send
        | WriteOp::Swap
        | WriteOp::OpenPerpPosition
        | WriteOp::ClosePerpPosition
        | WriteOp::ModifyPerpOrder
        | WriteOp::PlaceLimitOrder
        | WriteOp::BuyNft
        | WriteOp::SellNft
        | WriteOp::TransferNft
        | WriteOp::BidNft
        | WriteOp::Bridge
        | WriteOp::Lend
        | WriteOp::WithdrawLending
        | WriteOp::Borrow
        | WriteOp::RepayBorrow
        | WriteOp::Stake
        | WriteOp::Unstake
        | WriteOp::ProvideLiquidity
        | WriteOp::RemoveLiquidity
        | WriteOp::PlacePrediction
        | WriteOp::ClosePrediction
        | WriteOp::InternalTransfer => Err(ToolError::new(
            "internal_error",
            "pumpfun tool invoked with a non-pumpfun WriteOp",
        )),
    }
}

const fn pumpfun_history_type(op: WriteOp) -> &'static str {
    match op {
        WriteOp::PumpfunBuy => "pumpfun_buy",
        WriteOp::PumpfunSell => "pumpfun_sell",
        WriteOp::Send
        | WriteOp::Swap
        | WriteOp::OpenPerpPosition
        | WriteOp::ClosePerpPosition
        | WriteOp::ModifyPerpOrder
        | WriteOp::PlaceLimitOrder
        | WriteOp::BuyNft
        | WriteOp::SellNft
        | WriteOp::TransferNft
        | WriteOp::BidNft
        | WriteOp::Bridge
        | WriteOp::Lend
        | WriteOp::WithdrawLending
        | WriteOp::Borrow
        | WriteOp::RepayBorrow
        | WriteOp::Stake
        | WriteOp::Unstake
        | WriteOp::ProvideLiquidity
        | WriteOp::RemoveLiquidity
        | WriteOp::PlacePrediction
        | WriteOp::ClosePrediction
        | WriteOp::InternalTransfer => "pumpfun",
    }
}

/// Fetch and validate the Solana transaction envelope from the marketplace adapter.
async fn pumpfun_fetch_envelope(
    cfg_http: &crate::config::HttpConfig,
    op_str: &str,
    from_pubkey: &str,
    asset: Value,
    lock: &std::fs::File,
    req_id: &Value,
) -> eyre::Result<Result<marketplace_adapter::SolanaTxEnvelopeResponse, JsonRpcResponse>> {
    let env = match marketplace_adapter::fetch_solana_tx_envelope(
        cfg_http,
        "pumpfun",
        op_str,
        from_pubkey,
        asset,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(
                req_id.clone(),
                tool_err(ToolError::new("pumpfun_adapter_error", e.to_string())),
            )));
        }
    };
    if env
        .allowed_program_ids
        .iter()
        .find_map(|s| SolanaChain::parse_pubkey(s).ok())
        .is_none()
    {
        Keystore::release_lock(lock.try_clone()?)?;
        return Ok(Err(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_envelope",
                "empty allowed_program_ids",
            )),
        )));
    }
    Ok(Ok(env))
}

/// Sign and broadcast the pumpfun transaction on Solana.
async fn pumpfun_sign_and_send<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    env: &marketplace_adapter::SolanaTxEnvelopeResponse,
) -> eyre::Result<solana_sdk::signature::Signature>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let tx_bytes = base64::engine::general_purpose::STANDARD
        .decode(&env.tx_b64)
        .context("decode tx_b64")?;
    let allowed: Vec<solana_sdk::pubkey::Pubkey> = env
        .allowed_program_ids
        .iter()
        .filter_map(|s| SolanaChain::parse_pubkey(s).ok())
        .collect();
    let mode = effective_network_mode(ctx.shared, ctx.conn);
    let sol = SolanaChain::new_with_fallbacks(
        &ctx.shared.cfg.rpc.solana_rpc_url,
        solana_fallback_urls(ctx.shared, mode),
        &ctx.shared.cfg.http.jupiter_base_url,
        ctx.shared.cfg.http.jupiter_api_key.as_deref(),
        ctx.shared.cfg.rpc.solana_default_compute_unit_limit,
        ctx.shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );
    let kp = load_solana_keypair(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    sol.sign_and_send_versioned_allowlist(&kp, &tx_bytes, &allowed)
        .await
}

/// Completed pumpfun transaction details for logging and response.
struct PumpfunResult<'a> {
    tool_name: &'a str,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    mint: &'a str,
    op: WriteOp,
    amount_sol: f64,
    percent: f64,
    usd_value: f64,
    usd_value_known: bool,
    sig: &'a solana_sdk::signature::Signature,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

/// Record transaction history, audit log, release lock, and build response.
fn pumpfun_record_and_respond(
    shared: &crate::rpc::mcp_server::SharedState,
    lock: std::fs::File,
    r: &PumpfunResult<'_>,
    req_id: &Value,
) -> eyre::Result<JsonRpcResponse> {
    let ty = pumpfun_history_type(r.op);
    shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": ty, "chain": "solana", "wallet": r.w.name,
      "account_index": r.idx, "mint": r.mint,
      "amount_sol": (r.op == WriteOp::PumpfunBuy).then_some(r.amount_sol),
      "percent": (r.op == WriteOp::PumpfunSell).then_some(r.percent),
      "usd_value": r.usd_value, "signature": r.sig.to_string()
    }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": r.tool_name, "wallet": r.w.name,
      "account_index": r.idx, "chain": "solana",
      "usd_value": r.usd_value, "usd_value_known": r.usd_value_known,
      "policy_decision": r.outcome.policy_decision,
      "confirm_required": r.outcome.confirm_required,
      "confirm_result": r.outcome.confirm_result,
      "daily_used_usd": r.outcome.daily_used_usd,
      "forced_confirm": r.outcome.forced_confirm,
      "txid": r.sig.to_string(), "error_code": null,
      "result": "broadcasted", "signature": r.sig.to_string(), "mint": r.mint
    }));
    Keystore::release_lock(lock)?;
    Ok(ok(
        req_id.clone(),
        tool_ok(
            json!({ "chain": "solana", "signature": r.sig.to_string(), "usd_value": r.usd_value }),
        ),
    ))
}

/// Prepared pumpfun context after validation and envelope fetching.
struct PumpfunPrepared {
    w: crate::wallet::WalletRecord,
    idx: u32,
    mint: String,
    op: WriteOp,
    summary: String,
    amount_sol: f64,
    percent: f64,
    env: marketplace_adapter::SolanaTxEnvelopeResponse,
    force_confirm: bool,
}

/// Validated pumpfun data ready for the async fetch phase.
struct PumpfunValidated {
    w: crate::wallet::WalletRecord,
    idx: u32,
    mint: String,
    parsed: ParsedPumpfunOp,
    from_pubkey: String,
    op_str: String,
    force_confirm: bool,
}

/// Synchronous validation phase: parse args, validate mint, resolve wallet.
fn pumpfun_validate<R, W>(
    tool_name: &str,
    ctx: &HandlerCtx<'_, R, W>,
    lock: &std::fs::File,
) -> eyre::Result<Result<PumpfunValidated, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (w, idx) = resolve_wallet_and_account(ctx.shared, &ctx.args)?;
    let (policy, _) = ctx.shared.cfg.policy_for_wallet(Some(w.name.as_str()));
    let mint = ctx
        .args
        .get("mint")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_owned();
    if mint.is_empty() {
        Keystore::release_lock(lock.try_clone()?)?;
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "missing mint")),
        )));
    }
    let parsed = match parse_pumpfun_op(
        tool_name,
        &ctx.args,
        &mint,
        &policy,
        &ctx.shared.ks,
        &w.name,
        idx,
    ) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(ctx.req_id.clone(), tool_err(te))));
        }
    };
    let from_pubkey = w
        .solana_addresses
        .get(idx as usize)
        .ok_or_else(|| eyre::eyre!("missing solana address for account"))?
        .clone();
    let op_str = match pumpfun_op_str(parsed.op) {
        Ok(v) => v.to_owned(),
        Err(te) => {
            Keystore::release_lock(lock.try_clone()?)?;
            return Ok(Err(ok(ctx.req_id.clone(), tool_err(te))));
        }
    };
    let force_confirm = policy.require_user_confirm_for_remote_tx.get();
    Ok(Ok(PumpfunValidated {
        w,
        idx,
        mint,
        parsed,
        from_pubkey,
        op_str,
        force_confirm,
    }))
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
    let v = match pumpfun_validate(tool_name, ctx, &lock)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let env = match pumpfun_fetch_envelope(
        &ctx.shared.cfg.http,
        &v.op_str,
        &v.from_pubkey,
        v.parsed.asset,
        &lock,
        &ctx.req_id,
    )
    .await?
    {
        Ok(e) => e,
        Err(resp) => return Ok(resp),
    };
    let prep = PumpfunPrepared {
        w: v.w,
        idx: v.idx,
        mint: v.mint,
        op: v.parsed.op,
        summary: v.parsed.summary,
        amount_sol: v.parsed.amount_sol,
        percent: v.parsed.percent,
        env,
        force_confirm: v.force_confirm,
    };
    let usd_value = prep.env.usd_value.unwrap_or(0.0_f64);
    let usd_value_known = prep.env.usd_value.is_some();
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(prep.w.name.as_str()),
            account_index: Some(prep.idx),
            op: prep.op,
            chain: "solana",
            usd_value,
            usd_value_known,
            force_confirm: prep.force_confirm,
            slippage_bps: None,
            to_address: None,
            contract: Some("pumpfun"),
            leverage: None,
            summary: &prep.summary,
        },
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };
    let sig = pumpfun_sign_and_send(ctx, &prep.w, prep.idx, &prep.env).await?;
    pumpfun_record_and_respond(
        ctx.shared,
        lock,
        &PumpfunResult {
            tool_name,
            w: &prep.w,
            idx: prep.idx,
            mint: &prep.mint,
            op: prep.op,
            amount_sol: prep.amount_sol,
            percent: prep.percent,
            usd_value,
            usd_value_known,
            sig: &sig,
            outcome: &outcome,
        },
        &ctx.req_id,
    )
}
