use alloy::primitives::{Bytes, U256};
use alloy::rpc::types::TransactionRequest;
use alloy::sol;
use eyre::Context as _;
use serde_json::{json, Value};

use crate::{
    amount,
    chains::evm::EvmChain,
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::helpers::{evm_addr_for_account, resolve_wallet_and_account};
use super::super::key_loading::load_evm_signer;
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::{parse_usd_value, summarize_sim_error};
use super::common::wait_for_allowance;
use super::HandlerCtx;

sol! {
    #[sol(rpc)]
    contract IAavePoolV3 {
        function supply(address asset, uint256 amount, address onBehalfOf, uint16 referralCode) external;
        function withdraw(address asset, uint256 amount, address to) external returns (uint256);
        function borrow(address asset, uint256 amount, uint256 interestRateMode, uint16 referralCode, address onBehalfOf) external;
        function repay(address asset, uint256 amount, uint256 interestRateMode, address onBehalfOf) external returns (uint256);
        function getUserAccountData(address user) external view returns (uint256, uint256, uint256, uint256, uint256, uint256);
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn default_aave_pool_for_chain(chain: &str) -> Option<&'static str> {
    // Aave v3 Pool (mainnets). Sources: Aave docs / explorers.
    match chain.trim() {
        "ethereum" => Some("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"),
        "base" => Some("0xA238Dd80C259a72e81d7e4664a9801593F98d1c5"),
        // Aave v3 Pool address is shared across several EVM networks.
        "arbitrum" | "optimism" | "polygon" => Some("0x794a61358D6845594F94dc1DB02A252b5b4814aD"),
        _ => None,
    }
}

fn interest_rate_mode_to_u256(s: &str) -> Result<U256, ToolError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "variable" | "var" => Ok(U256::from(2_u64)),
        "stable" => Ok(U256::from(1_u64)),
        _ => Err(ToolError::new(
            "invalid_request",
            "interest_rate_mode must be one of: variable, stable",
        )),
    }
}

fn parse_amount_base(amount_s: &str, units: &str, decimals: u8) -> Result<U256, ToolError> {
    if amount_s.trim().eq_ignore_ascii_case("max") {
        return Ok(U256::MAX);
    }
    let base_u128 = if units.trim() == "base" {
        amount::parse_amount_base_u128(amount_s)
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))?
    } else {
        amount::parse_amount_ui_to_base_u128(amount_s, u32::from(decimals))
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))?
    };
    Ok(U256::from(base_u128))
}

fn usdc_base_to_usd_f64(base_amount: &U256) -> eyre::Result<f64> {
    // Convert a USDC base-unit integer (6 decimals) into a decimal string, then parse.
    // This avoids float arithmetic in lint-restricted codepaths.
    let s = base_amount.to_string();
    let s = s.trim();
    if s.is_empty() {
        eyre::bail!("empty usdc base amount");
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        eyre::bail!("invalid usdc base amount");
    }
    if s == "0" {
        return Ok(0.0_f64);
    }

    let dec = if s.len() <= 6 {
        let mut frac = String::with_capacity(6);
        for _ in 0..(6 - s.len()) {
            frac.push('0');
        }
        frac.push_str(s);
        format!("0.{frac}")
    } else {
        let split = s.len() - 6;
        let (whole, frac) = s.split_at(split);
        let frac_trimmed = frac.trim_end_matches('0');
        if frac_trimmed.is_empty() {
            whole.to_owned()
        } else {
            format!("{whole}.{frac_trimmed}")
        }
    };

    dec.parse::<f64>().context("parse usdc amount")
}

struct AaveCallInfo {
    op: WriteOp,
    history_type: &'static str,
    call_data: Vec<u8>,
    action_label: &'static str,
}

fn build_aave_calldata(
    pool: &IAavePoolV3::IAavePoolV3Instance<alloy::providers::RootProvider>,
    tool_name: &str,
    token_addr: alloy::primitives::Address,
    amount_base: U256,
    interest_rate_mode: U256,
    from: alloy::primitives::Address,
) -> Result<AaveCallInfo, ToolError> {
    match tool_name {
        "lend_tokens" => {
            let call = pool.supply(token_addr, amount_base, from, 0_u16);
            let data = call.calldata().to_vec();
            Ok(AaveCallInfo {
                op: WriteOp::Lend,
                history_type: "lend",
                call_data: data,
                action_label: "supply",
            })
        }
        "withdraw_lending" => {
            let call = pool.withdraw(token_addr, amount_base, from);
            let data = call.calldata().to_vec();
            Ok(AaveCallInfo {
                op: WriteOp::WithdrawLending,
                history_type: "withdraw_lending",
                call_data: data,
                action_label: "withdraw",
            })
        }
        "borrow_tokens" => {
            let call = pool.borrow(token_addr, amount_base, interest_rate_mode, 0_u16, from);
            let data = call.calldata().to_vec();
            Ok(AaveCallInfo {
                op: WriteOp::Borrow,
                history_type: "borrow",
                call_data: data,
                action_label: "borrow",
            })
        }
        "repay_borrow" => {
            let call = pool.repay(token_addr, amount_base, interest_rate_mode, from);
            let data = call.calldata().to_vec();
            Ok(AaveCallInfo {
                op: WriteOp::RepayBorrow,
                history_type: "repay_borrow",
                call_data: data,
                action_label: "repay",
            })
        }
        _ => Err(ToolError::new("invalid_request", "unknown tool")),
    }
}

struct ApprovalContext<'a> {
    tool_name: &'a str,
    wallet_name: &'a str,
    idx: u32,
    chain: &'a str,
    token_addr: alloy::primitives::Address,
    pool_addr: alloy::primitives::Address,
    amount_base: U256,
    from: alloy::primitives::Address,
}

struct ParsedAaveArgs<'a> {
    chain: &'a str,
    token_s: &'a str,
    amount_s: &'a str,
    units: &'a str,
    pool_s: &'a str,
    irm: &'a str,
}

fn validate_aave_args(args: &Value) -> Result<ParsedAaveArgs<'_>, ToolError> {
    let chain = arg_str(args, "chain").unwrap_or("");
    if chain.is_empty() || chain == "solana" || chain == "bitcoin" {
        return Err(ToolError::new(
            "invalid_request",
            "Aave handler requires an EVM chain",
        ));
    }
    let protocol = arg_str(args, "protocol").unwrap_or("auto");
    let protocol = if protocol == "auto" { "aave" } else { protocol };
    if protocol != "aave" {
        return Err(ToolError::new("invalid_request", "protocol must be aave"));
    }
    let token_s = arg_str(args, "token").unwrap_or("");
    let amount_s = arg_str(args, "amount").unwrap_or("");
    let units = arg_str(args, "amount_units").unwrap_or("ui");
    if token_s.is_empty() || amount_s.is_empty() {
        return Err(ToolError::new(
            "invalid_request",
            "missing token/amount (provide tx envelope fields to use adapter fallback)",
        ));
    }
    let pool_s = arg_str(args, "pool_address")
        .or_else(|| default_aave_pool_for_chain(chain))
        .unwrap_or("");
    if pool_s.trim().is_empty() {
        return Err(ToolError::new(
            "invalid_request",
            "missing Aave pool address for this chain (provide pool_address)",
        ));
    }
    let irm = arg_str(args, "interest_rate_mode").unwrap_or("variable");
    Ok(ParsedAaveArgs {
        chain,
        token_s,
        amount_s,
        units,
        pool_s,
        irm,
    })
}

struct BlocklistCheckCtx<'a> {
    tool_name: &'a str,
    wallet_name: &'a str,
    idx: u32,
    chain: &'a str,
    pool_addr: alloy::primitives::Address,
    enable_ofac: bool,
}

async fn check_aave_blocklists(
    shared: &mut super::super::super::SharedState,
    bc: &BlocklistCheckCtx<'_>,
) -> Result<(), ToolError> {
    if shared.scam_blocklist_contains_evm(bc.pool_addr).await {
        let _audit_log = shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": bc.tool_name, "wallet": bc.wallet_name, "account_index": bc.idx, "chain": bc.chain,
          "usd_value": 0.0_f64, "usd_value_known": false, "policy_decision": null, "confirm_required": false,
          "confirm_result": null, "txid": null, "error_code": "scam_address_blocked",
          "result": "blocked_scam_blocklist", "to": format!("{:#x}", bc.pool_addr),
        }));
        return Err(ToolError::new(
            "scam_address_blocked",
            "recipient is blocked by the scam address blocklist",
        ));
    }
    if bc.enable_ofac && shared.ofac_sdn_contains_evm(bc.pool_addr).await {
        let _audit_log = shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": bc.tool_name, "wallet": bc.wallet_name, "account_index": bc.idx, "chain": bc.chain,
          "usd_value": 0.0_f64, "usd_value_known": false, "policy_decision": null, "confirm_required": false,
          "confirm_result": null, "txid": null, "error_code": "ofac_sdn_blocked",
          "result": "blocked_ofac_sdn", "to": format!("{:#x}", bc.pool_addr),
        }));
        return Err(ToolError::new(
            "ofac_sdn_blocked",
            "recipient is blocked by the OFAC SDN list",
        ));
    }
    Ok(())
}

async fn resolve_aave_usd_value(
    shared: &mut super::super::super::SharedState,
    evm: &EvmChain,
    args: &Value,
    token_addr: alloy::primitives::Address,
    amount_base: U256,
) -> Result<(f64, bool), ToolError> {
    let (mut usd_value, mut usd_value_known) = parse_usd_value(args);
    if !usd_value_known {
        if amount_base == U256::MAX {
            return Err(ToolError::new(
                "invalid_request",
                "amount=max requires usd_value for policy enforcement",
            ));
        }
        let usd = if evm.uniswap.as_ref().is_some_and(|u| u.usdc == token_addr) {
            usdc_base_to_usd_f64(&amount_base)
                .map_err(|e| ToolError::new("price_error", e.to_string()))?
        } else {
            shared.ensure_db().await;
            let db = shared.db();
            price::evm_token_price_usd_cached(evm, &shared.cfg, token_addr, amount_base, 50, db)
                .await
                .map_err(|e| ToolError::new("price_error", e.to_string()))?
                .usd
        };
        usd_value = usd;
        usd_value_known = usd_value.is_finite();
    }
    Ok((usd_value, usd_value_known))
}

struct AaveAuditCtx<'a> {
    tool_name: &'a str,
    wallet_name: &'a str,
    idx: u32,
    chain: &'a str,
    usd_value: f64,
    usd_value_known: bool,
}

fn log_aave_sim_failure(
    ks: &Keystore,
    ac: &AaveAuditCtx<'_>,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) {
    let _audit_log = ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": ac.tool_name, "wallet": ac.wallet_name, "account_index": ac.idx, "chain": ac.chain,
      "usd_value": ac.usd_value, "usd_value_known": ac.usd_value_known,
      "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm, "txid": null,
      "error_code": "simulation_failed", "result": "blocked_simulation", "protocol": "aave",
    }));
}

fn log_aave_approve_audit(
    ks: &Keystore,
    ac: &ApprovalContext<'_>,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
    txid: Option<&str>,
    error_code: Option<&str>,
    result: &str,
) {
    let _audit_log = ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": ac.tool_name, "wallet": ac.wallet_name,
      "account_index": ac.idx, "chain": ac.chain, "usd_value": 0.0_f64,
      "usd_value_known": false, "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result,
      "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm,
      "txid": txid, "error_code": error_code, "result": result,
      "type": "approve", "protocol": "aave",
    }));
}

async fn handle_aave_approval<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    evm: &EvmChain,
    ac: &ApprovalContext<'_>,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) -> Result<(), JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let allowance = evm
        .erc20_allowance(ac.token_addr, ac.from, ac.pool_addr)
        .await
        .map_err(|e| {
            ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new("rpc_error", e.to_string())),
            )
        })?;
    if allowance >= ac.amount_base {
        return Ok(());
    }

    let approve_tx = evm
        .build_erc20_approve(ac.from, ac.token_addr, ac.pool_addr, ac.amount_base)
        .map_err(|e| {
            ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new("build_error", e.to_string())),
            )
        })?;
    if let Err(e) = evm.simulate_tx_strict(&approve_tx).await {
        log_aave_approve_audit(
            &ctx.shared.ks,
            ac,
            outcome,
            None,
            Some("simulation_failed"),
            "simulation_failed",
        );
        return Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "approve (aave)"),
            )),
        ));
    }

    let w_rec = ctx
        .shared
        .ks
        .get_wallet_by_name(ac.wallet_name)
        .map_err(|e| {
            ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new("wallet_error", e.to_string())),
            )
        })?
        .ok_or_else(|| {
            ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new("wallet_error", "wallet not found")),
            )
        })?;
    let signer = load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &w_rec, ac.idx)
        .await
        .map_err(|e| {
            ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new("signer_error", e.to_string())),
            )
        })?;
    let tx_hash = evm.send_tx(signer.clone(), approve_tx).await.map_err(|e| {
        ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("send_error", e.to_string())),
        )
    })?;
    let tx_hash_s = format!("{tx_hash:#x}");

    let _tx_log = ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": "approve",
      "chain": ac.chain, "wallet": ac.wallet_name, "account_index": ac.idx, "protocol": "aave",
      "token": format!("{:#x}", ac.token_addr), "spender": format!("{:#x}", ac.pool_addr),
      "amount_base": ac.amount_base.to_string(), "usd_value": 0.0_f64, "txid": tx_hash_s,
    }));
    log_aave_approve_audit(
        &ctx.shared.ks,
        ac,
        outcome,
        Some(&tx_hash_s),
        None,
        "broadcasted",
    );

    if !wait_for_allowance(evm, ac.token_addr, ac.from, ac.pool_addr, ac.amount_base).await {
        return Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "approval_pending",
                "approval submitted but not yet confirmed; retry shortly",
            )),
        ));
    }
    Ok(())
}

async fn cache_account_data(
    shared: &mut super::super::super::SharedState,
    pool: &IAavePoolV3::IAavePoolV3Instance<alloy::providers::RootProvider>,
    from: alloy::primitives::Address,
    chain: &str,
    wallet_name: &str,
    idx: u32,
) {
    shared.ensure_db().await;
    if let Some(db) = shared.db() {
        let key = format!("lending:aave:account_data:{chain}:{wallet_name}:{idx}");
        if let Ok(data) = pool.getUserAccountData(from).call().await {
            if let Ok(now) = crate::db::Db::now_ms() {
                let stale_at = now.saturating_add(10_000);
                let cache_val = serde_json::json!([
                    data._0.to_string(),
                    data._1.to_string(),
                    data._2.to_string(),
                    data._3.to_string(),
                    data._4.to_string(),
                    data._5.to_string()
                ]);
                let _cache_upsert_res = db
                    .upsert_json(&key, &cache_val.to_string(), now, stale_at)
                    .await;
            }
        }
    }
}

struct ResolvedAaveParams {
    token_addr: alloy::primitives::Address,
    amount_base: U256,
    usd_value: f64,
    usd_value_known: bool,
    call_info: AaveCallInfo,
    pool: IAavePoolV3::IAavePoolV3Instance<alloy::providers::RootProvider>,
    symbol: String,
}

async fn resolve_aave_token_params(
    shared: &mut super::super::super::SharedState,
    evm: &EvmChain,
    tool_name: &str,
    args: &Value,
    parsed: &ParsedAaveArgs<'_>,
    pool_addr: alloy::primitives::Address,
    from: alloy::primitives::Address,
) -> Result<ResolvedAaveParams, ToolError> {
    let token_addr = EvmChain::parse_address(parsed.token_s)
        .map_err(|e| ToolError::new("invalid_request", format!("parse token: {e:#}")))?;
    let (decimals, symbol) = evm
        .get_erc20_metadata(token_addr)
        .await
        .unwrap_or_else(|_| (18, "ERC20".into()));
    let amount_base = parse_amount_base(parsed.amount_s, parsed.units, decimals)?;
    if parsed.amount_s.trim().eq_ignore_ascii_case("max")
        && matches!(tool_name, "lend_tokens" | "borrow_tokens")
    {
        return Err(ToolError::new(
            "invalid_request",
            "amount=max is only supported for withdraw_lending and repay_borrow",
        ));
    }
    let (usd_value, usd_value_known) =
        resolve_aave_usd_value(shared, evm, args, token_addr, amount_base).await?;
    let interest_rate_mode = interest_rate_mode_to_u256(parsed.irm)?;
    let pool = IAavePoolV3::new(
        pool_addr,
        evm.provider()
            .map_err(|e| ToolError::new("rpc_error", e.to_string()))?,
    );
    let call_info = build_aave_calldata(
        &pool,
        tool_name,
        token_addr,
        amount_base,
        interest_rate_mode,
        from,
    )?;
    Ok(ResolvedAaveParams {
        token_addr,
        amount_base,
        usd_value,
        usd_value_known,
        call_info,
        pool,
        symbol,
    })
}

async fn confirm_aave_write<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    tool_name: &str,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    parsed: &ParsedAaveArgs<'_>,
    resolved: &ResolvedAaveParams,
) -> Result<super::super::policy_confirm::WriteConfirmOutcome, ToolError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let summary = format!(
        "Aave v3 {} on {}: {} amount={} (units={})",
        resolved.call_info.action_label,
        parsed.chain,
        resolved.symbol,
        parsed.amount_s.trim(),
        parsed.units,
    );
    maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: resolved.call_info.op,
            chain: parsed.chain,
            usd_value: resolved.usd_value,
            usd_value_known: resolved.usd_value_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: Some(parsed.pool_s),
            contract: Some(parsed.pool_s),
            leverage: None,
            summary: &summary,
        },
    )
    .await
}

pub async fn handle<R, W>(
    tool_name: &str,
    ctx: &mut HandlerCtx<'_, R, W>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let args = ctx.args.clone();
    let lock = ctx.shared.ks.acquire_write_lock()?;
    handle_inner(tool_name, ctx, &args, lock).await
}

async fn handle_inner<R, W>(
    tool_name: &str,
    ctx: &mut HandlerCtx<'_, R, W>,
    args: &Value,
    lock: std::fs::File,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    // Wrap in Option so we can .take() exactly once on each early-return path.
    let mut lock = Some(lock);
    let release = |l: &mut Option<std::fs::File>| -> eyre::Result<()> {
        if let Some(f) = l.take() {
            Keystore::release_lock(f)?;
        }
        Ok(())
    };

    let (w, idx) = resolve_wallet_and_account(ctx.shared, args)?;
    let (effective_policy, _) = ctx.shared.cfg.policy_for_wallet(Some(w.name.as_str()));

    let parsed = match validate_aave_args(args) {
        Ok(v) => v,
        Err(te) => {
            release(&mut lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    let evm = setup_evm_chain(ctx.shared, parsed.chain)?;
    let from = evm_addr_for_account(&w, idx)?;
    let pool_addr = EvmChain::parse_address(parsed.pool_s).context("parse pool_address")?;

    let bc = BlocklistCheckCtx {
        tool_name,
        wallet_name: &w.name,
        idx,
        chain: parsed.chain,
        pool_addr,
        enable_ofac: effective_policy.enable_ofac_sdn.get(),
    };
    if let Err(te) = check_aave_blocklists(ctx.shared, &bc).await {
        release(&mut lock)?;
        return Ok(ok(ctx.req_id.clone(), tool_err(te)));
    }

    let resolved = match resolve_aave_token_params(
        ctx.shared, &evm, tool_name, args, &parsed, pool_addr, from,
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            release(&mut lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    let outcome = match confirm_aave_write(ctx, tool_name, &w, idx, &parsed, &resolved).await {
        Ok(v) => v,
        Err(te) => {
            release(&mut lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    if matches!(tool_name, "lend_tokens" | "repay_borrow") && resolved.amount_base != U256::MAX {
        let ac = ApprovalContext {
            tool_name,
            wallet_name: &w.name,
            idx,
            chain: parsed.chain,
            token_addr: resolved.token_addr,
            pool_addr,
            amount_base: resolved.amount_base,
            from,
        };
        if let Err(resp) = handle_aave_approval(ctx, &evm, &ac, &outcome).await {
            release(&mut lock)?;
            return Ok(resp);
        }
    }

    let Some(lock_file) = lock.take() else {
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("internal_error", "keystore lock missing")),
        ));
    };
    let stx = SendAaveTxCtx {
        tool_name,
        parsed: &parsed,
        pool_addr,
        token_addr: resolved.token_addr,
        amount_base: resolved.amount_base,
        call_info: &resolved.call_info,
        audit: AaveAuditCtx {
            tool_name,
            wallet_name: &w.name,
            idx,
            chain: parsed.chain,
            usd_value: resolved.usd_value,
            usd_value_known: resolved.usd_value_known,
        },
        outcome: &outcome,
        pool: &resolved.pool,
        from,
        lock: lock_file,
    };
    send_aave_tx(ctx, &evm, &w, stx).await
}

fn setup_evm_chain(
    shared: &super::super::super::SharedState,
    chain: &str,
) -> eyre::Result<EvmChain> {
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

struct SendAaveTxCtx<'a> {
    tool_name: &'a str,
    parsed: &'a ParsedAaveArgs<'a>,
    pool_addr: alloy::primitives::Address,
    token_addr: alloy::primitives::Address,
    amount_base: U256,
    call_info: &'a AaveCallInfo,
    audit: AaveAuditCtx<'a>,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
    pool: &'a IAavePoolV3::IAavePoolV3Instance<alloy::providers::RootProvider>,
    from: alloy::primitives::Address,
    lock: std::fs::File,
}

async fn send_aave_tx<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    evm: &EvmChain,
    w: &crate::wallet::WalletRecord,
    stx: SendAaveTxCtx<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let tx = TransactionRequest {
        from: Some(stx.from),
        to: Some(stx.pool_addr.into()),
        input: Bytes::from(stx.call_info.call_data.clone()).into(),
        value: Some(U256::ZERO),
        ..Default::default()
    };

    if let Err(e) = evm.simulate_tx_strict(&tx).await {
        log_aave_sim_failure(&ctx.shared.ks, &stx.audit, stx.outcome);
        Keystore::release_lock(stx.lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, stx.tool_name),
            )),
        ));
    }

    let signer = load_evm_signer(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        w,
        stx.audit.idx,
    )
    .await?;
    let txid = evm.send_tx(signer, tx).await.context("send aave tx")?;

    ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": stx.call_info.history_type,
      "chain": stx.parsed.chain, "wallet": w.name, "account_index": stx.audit.idx, "protocol": "aave",
      "pool": format!("{:#x}", stx.pool_addr), "token": format!("{:#x}", stx.token_addr),
      "amount_base": if stx.amount_base == U256::MAX { "max".to_owned() } else { stx.amount_base.to_string() },
      "amount_units": stx.parsed.units, "interest_rate_mode": stx.parsed.irm, "usd_value": stx.audit.usd_value, "txid": format!("{txid:#x}"),
    }))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": stx.tool_name, "wallet": w.name, "account_index": stx.audit.idx, "chain": stx.parsed.chain,
      "usd_value": stx.audit.usd_value, "usd_value_known": stx.audit.usd_value_known,
      "policy_decision": stx.outcome.policy_decision, "confirm_required": stx.outcome.confirm_required,
      "confirm_result": stx.outcome.confirm_result, "daily_used_usd": stx.outcome.daily_used_usd,
      "forced_confirm": stx.outcome.forced_confirm, "txid": format!("{txid:#x}"),
      "error_code": null, "result": "broadcasted", "protocol": "aave",
    }));

    cache_account_data(
        ctx.shared,
        stx.pool,
        stx.from,
        stx.parsed.chain,
        &w.name,
        stx.audit.idx,
    )
    .await;
    Keystore::release_lock(stx.lock)?;

    let amount_base_out = if stx.amount_base == U256::MAX {
        "max".to_owned()
    } else {
        stx.amount_base.to_string()
    };
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
          "chain": stx.parsed.chain, "protocol": "aave", "pool": format!("{:#x}", stx.pool_addr),
          "token": format!("{:#x}", stx.token_addr), "amount_base": amount_base_out,
          "usd_value": stx.audit.usd_value, "txid": format!("{txid:#x}")
        })),
    ))
}
