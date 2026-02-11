use alloy::primitives::{Bytes, U256};
use alloy::rpc::types::TransactionRequest;
use alloy::sol;
use eyre::Context as _;
use serde_json::{json, Value};
use tokio::io::BufReader;

use crate::{
    amount,
    chains::evm::EvmChain,
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{evm_addr_for_account, resolve_wallet_and_account};
use super::super::key_loading::load_evm_signer;
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::{parse_usd_value, summarize_sim_error};
use super::common::wait_for_allowance;

sol! {
    #[sol(rpc)]
    contract ICometV3 {
        function baseToken() external view returns (address);
        function supply(address asset, uint256 amount) external;
        function withdraw(address asset, uint256 amount) external;
        function borrowBalanceOf(address account) external view returns (uint256);
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
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

fn parse_amount_base(amount_s: &str, units: &str, decimals: u8) -> Result<U256, ToolError> {
    if amount_s.trim().eq_ignore_ascii_case("max") {
        return Err(ToolError::new(
            "invalid_request",
            "amount=max is not supported for native Compound v3 execution (provide an explicit amount)",
        ));
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

/// Context for a Compound v3 approval operation.
struct CompoundApprovalCtx<'a> {
    evm: &'a EvmChain,
    shared: &'a SharedState,
    signer: alloy::signers::local::PrivateKeySigner,
    from: alloy::primitives::Address,
    token_addr: alloy::primitives::Address,
    comet_addr: alloy::primitives::Address,
    amount_base: U256,
    tool_name: &'a str,
    w_name: &'a str,
    idx: u32,
    chain: &'a str,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

/// Audit log for a failed approval simulation.
fn compound_approval_sim_fail_audit(ctx: &CompoundApprovalCtx<'_>) {
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": ctx.tool_name, "wallet": ctx.w_name,
      "account_index": ctx.idx, "chain": ctx.chain,
      "usd_value": 0.0_f64, "usd_value_known": false,
      "policy_decision": ctx.outcome.policy_decision,
      "confirm_required": ctx.outcome.confirm_required,
      "confirm_result": ctx.outcome.confirm_result,
      "daily_used_usd": ctx.outcome.daily_used_usd,
      "forced_confirm": ctx.outcome.forced_confirm,
      "txid": null, "error_code": "simulation_failed",
      "result": "simulation_failed", "type": "approve", "protocol": "compound",
    }));
}

async fn handle_compound_approval(
    ctx: CompoundApprovalCtx<'_>,
) -> Result<(), (ToolError, Option<alloy::primitives::B256>)> {
    let allowance = ctx
        .evm
        .erc20_allowance(ctx.token_addr, ctx.from, ctx.comet_addr)
        .await
        .map_err(|e| (ToolError::new("internal_error", format!("{e:#}")), None))?;
    if allowance >= ctx.amount_base {
        return Ok(());
    }
    let approve_tx = ctx
        .evm
        .build_erc20_approve(ctx.from, ctx.token_addr, ctx.comet_addr, ctx.amount_base)
        .map_err(|e| (ToolError::new("internal_error", format!("{e:#}")), None))?;
    if let Err(e) = ctx.evm.simulate_tx_strict(&approve_tx).await {
        compound_approval_sim_fail_audit(&ctx);
        return Err((
            ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "approve (compound)"),
            ),
            None,
        ));
    }
    // Destructure before send_tx takes ownership of signer.
    let CompoundApprovalCtx {
        evm,
        shared,
        signer,
        from,
        token_addr,
        comet_addr,
        amount_base,
        tool_name,
        w_name,
        idx,
        chain,
        outcome,
    } = ctx;
    let tx_hash = evm
        .send_tx(signer, approve_tx)
        .await
        .map_err(|e| (ToolError::new("internal_error", format!("{e:#}")), None))?;
    let tx_hash_s = format!("{tx_hash:#x}");
    shared
        .ks
        .append_tx_history(&json!({
          "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
          "type": "approve", "chain": chain, "wallet": w_name,
          "account_index": idx, "protocol": "compound",
          "token": format!("{token_addr:#x}"), "spender": format!("{comet_addr:#x}"),
          "amount_base": amount_base.to_string(), "usd_value": 0.0_f64, "txid": tx_hash_s,
        }))
        .map_err(|e| {
            (
                ToolError::new("internal_error", format!("{e:#}")),
                Some(tx_hash),
            )
        })?;
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": tool_name, "wallet": w_name,
      "account_index": idx, "chain": chain,
      "usd_value": 0.0_f64, "usd_value_known": false,
      "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result,
      "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm,
      "txid": tx_hash_s, "error_code": null,
      "result": "broadcasted", "type": "approve", "protocol": "compound",
    }));

    let ok_allow = wait_for_allowance(evm, token_addr, from, comet_addr, amount_base).await;
    if !ok_allow {
        return Err((
            ToolError::new(
                "approval_pending",
                "approval submitted but not yet confirmed; retry shortly",
            ),
            None,
        ));
    }
    Ok(())
}

/// Validated Compound parameters after arg parsing and chain setup.
struct CompoundParams<'a> {
    lock: std::fs::File,
    w: crate::wallet::WalletRecord,
    idx: u32,
    chain: &'a str,
    comet_s: &'a str,
    comet_addr: alloy::primitives::Address,
    token_addr: alloy::primitives::Address,
    amount_base: U256,
    amount_s: &'a str,
    units: &'a str,
    symbol: String,
    usd_value: f64,
    usd_value_known: bool,
}

struct CompoundAction<'a> {
    tool_name: &'a str,
    evm: &'a EvmChain,
    p: CompoundParams<'a>,
    op: WriteOp,
    history_type: &'a str,
    call_data: Vec<u8>,
    action_label: &'a str,
}

/// Audit-log a simulation failure for a compound action.
fn compound_sim_fail_audit(
    shared: &SharedState,
    tool_name: &str,
    p: &CompoundParams<'_>,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) {
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": tool_name, "wallet": p.w.name,
      "account_index": p.idx, "chain": p.chain, "usd_value": p.usd_value,
      "usd_value_known": p.usd_value_known,
      "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result,
      "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm,
      "txid": null, "error_code": "simulation_failed",
      "result": "blocked_simulation", "protocol": "compound",
    }));
}

/// Record tx history and audit log for a successful compound action, then respond.
fn compound_record_and_respond(
    shared: &SharedState,
    p: CompoundParams<'_>,
    tool_name: &str,
    history_type: &str,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
    txid: alloy::primitives::B256,
    req_id: Value,
) -> eyre::Result<JsonRpcResponse> {
    shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
      "type": history_type, "chain": p.chain, "wallet": p.w.name,
      "account_index": p.idx, "protocol": "compound",
      "comet": format!("{:#x}", p.comet_addr),
      "token": format!("{:#x}", p.token_addr),
      "amount_base": p.amount_base.to_string(), "amount_units": p.units,
      "usd_value": p.usd_value, "txid": format!("{txid:#x}"),
    }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": tool_name, "wallet": p.w.name,
      "account_index": p.idx, "chain": p.chain, "usd_value": p.usd_value,
      "usd_value_known": p.usd_value_known,
      "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result,
      "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm,
      "txid": format!("{txid:#x}"), "error_code": null,
      "result": "broadcasted", "to": format!("{:#x}", p.comet_addr),
      "protocol": "compound",
    }));

    Keystore::release_lock(p.lock)?;
    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": p.chain, "protocol": "compound", "tool": tool_name,
          "comet": p.comet_s, "token": format!("{:#x}", p.token_addr),
          "amount": p.amount_s, "amount_units": p.units,
          "usd_value": p.usd_value, "usd_value_known": p.usd_value_known,
          "txid": format!("{txid:#x}"),
        })),
    ))
}

/// Simulate, send, log, and respond for a Compound v3 action.
async fn compound_execute_and_respond<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    req_id: Value,
    act: CompoundAction<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let CompoundAction {
        tool_name,
        evm,
        p,
        op,
        history_type,
        call_data,
        action_label,
    } = act;
    let summary = format!(
        "Compound v3 {action_label} on {}: {} amount={} (units={})",
        p.chain,
        p.symbol,
        p.amount_s.trim(),
        p.units,
    );
    let outcome = match maybe_confirm_write(
        shared,
        conn,
        stdin,
        stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(p.w.name.as_str()),
            account_index: Some(p.idx),
            op,
            chain: p.chain,
            usd_value: p.usd_value,
            usd_value_known: p.usd_value_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: Some(p.comet_s),
            contract: Some(p.comet_s),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(p.lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };

    let signer = load_evm_signer(shared, conn, stdin, stdout, &p.w, p.idx).await?;

    if matches!(tool_name, "lend_tokens" | "repay_borrow") {
        if let Err((te, _)) = handle_compound_approval(CompoundApprovalCtx {
            evm,
            shared: &*shared,
            signer: signer.clone(),
            from: signer.address(),
            token_addr: p.token_addr,
            comet_addr: p.comet_addr,
            amount_base: p.amount_base,
            tool_name,
            w_name: &p.w.name,
            idx: p.idx,
            chain: p.chain,
            outcome: &outcome,
        })
        .await
        {
            Keystore::release_lock(p.lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    }

    let tx = TransactionRequest {
        from: Some(signer.address()),
        to: Some(p.comet_addr.into()),
        input: Bytes::from(call_data).into(),
        value: Some(U256::ZERO),
        ..Default::default()
    };

    if let Err(e) = evm.simulate_tx_strict(&tx).await {
        compound_sim_fail_audit(shared, tool_name, &p, &outcome);
        Keystore::release_lock(p.lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, tool_name),
            )),
        ));
    }

    let txid = evm.send_tx(signer, tx).await.context("send compound tx")?;
    compound_record_and_respond(shared, p, tool_name, history_type, &outcome, txid, req_id)
}

/// Validated arguments for the compound handler, prior to on-chain resolution.
struct CompoundValidatedArgs<'a> {
    lock: std::fs::File,
    w: crate::wallet::WalletRecord,
    idx: u32,
    chain: &'a str,
    token_s: &'a str,
    amount_s: &'a str,
    units: &'a str,
    comet_s: &'a str,
}

/// Parse and validate all compound handler arguments (chain, protocol, token, amount, comet).
fn validate_compound_args<'a>(
    args: &'a Value,
    shared: &SharedState,
    req_id: &Value,
) -> eyre::Result<Result<CompoundValidatedArgs<'a>, JsonRpcResponse>> {
    let lock = shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(shared, args)?;

    let chain = arg_str(args, "chain").unwrap_or("");
    if chain.is_empty() || chain == "solana" || chain == "bitcoin" {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "Compound handler requires an EVM chain",
            )),
        )));
    }

    let protocol = arg_str(args, "protocol").unwrap_or("auto");
    let protocol = if protocol == "auto" {
        "compound"
    } else {
        protocol
    };
    if protocol != "compound" {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "protocol must be compound",
            )),
        )));
    }

    let token_s = arg_str(args, "token").unwrap_or("");
    let amount_s = arg_str(args, "amount").unwrap_or("");
    let units = arg_str(args, "amount_units").unwrap_or("ui");
    if token_s.is_empty() || amount_s.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing token/amount (provide tx envelope fields to use adapter fallback)",
            )),
        )));
    }

    let comet_s = arg_str(args, "comet_address")
        .or_else(|| default_comet_for_chain(chain))
        .unwrap_or("");
    if comet_s.trim().is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing Comet address for this chain (provide comet_address)",
            )),
        )));
    }

    Ok(Ok(CompoundValidatedArgs {
        lock,
        w,
        idx,
        chain,
        token_s,
        amount_s,
        units,
        comet_s,
    }))
}

/// Set up the EVM chain client for the compound handler.
fn setup_compound_evm(shared: &SharedState, chain: &str) -> eyre::Result<EvmChain> {
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

/// Audit-log a scam blocklist rejection for a compound operation.
fn compound_blocklist_audit(
    shared: &SharedState,
    tool_name: &str,
    w_name: &str,
    idx: u32,
    chain: &str,
    comet_addr: alloy::primitives::Address,
) {
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": tool_name, "wallet": w_name,
      "account_index": idx, "chain": chain, "usd_value": 0.0_f64,
      "usd_value_known": false, "policy_decision": null,
      "confirm_required": false, "confirm_result": null, "txid": null,
      "error_code": "scam_address_blocked",
      "result": "blocked_scam_blocklist",
      "to": format!("{comet_addr:#x}"),
    }));
}

/// Resolved on-chain token info for a compound operation.
struct CompoundTokenResolved {
    token_addr: alloy::primitives::Address,
    symbol: String,
    amount_base: U256,
    usd_value: f64,
    usd_value_known: bool,
}

/// Resolve on-chain token addresses, metadata, amount, and price for a compound operation.
async fn resolve_compound_token(
    evm: &EvmChain,
    shared: &mut SharedState,
    args: &Value,
    va: &CompoundValidatedArgs<'_>,
    comet_addr: alloy::primitives::Address,
) -> eyre::Result<Result<CompoundTokenResolved, ToolError>> {
    let comet = ICometV3::new(comet_addr, evm.provider()?);
    let base_token: alloy::primitives::Address =
        comet.baseToken().call().await.context("comet baseToken")?;

    let token_addr = if va.token_s.eq_ignore_ascii_case("base") {
        base_token
    } else {
        EvmChain::parse_address(va.token_s).context("parse token")?
    };

    let (decimals, symbol) = evm
        .get_erc20_metadata(token_addr)
        .await
        .context("erc20 metadata")?;
    let amount_base = match parse_amount_base(va.amount_s, va.units, decimals) {
        Ok(v) => v,
        Err(te) => return Ok(Err(te)),
    };

    let (mut usd_value, mut usd_value_known) = parse_usd_value(args);
    if !usd_value_known {
        shared.ensure_db().await;
        let db = shared.db();
        let p =
            price::evm_token_price_usd_cached(evm, &shared.cfg, token_addr, amount_base, 50, db)
                .await
                .context("price token via uniswap")?;
        usd_value = p.usd;
        usd_value_known = usd_value.is_finite();
    }

    Ok(Ok(CompoundTokenResolved {
        token_addr,
        symbol,
        amount_base,
        usd_value,
        usd_value_known,
    }))
}

/// Resolved calldata for a compound operation.
struct CompoundCalldata {
    op: WriteOp,
    history_type: &'static str,
    data: Vec<u8>,
    action_label: &'static str,
}

/// Build the comet calldata for the given tool operation.
#[allow(clippy::unnecessary_wraps)]
fn build_compound_calldata(
    comet: &ICometV3::ICometV3Instance<alloy::providers::RootProvider>,
    tool_name: &str,
    token_addr: alloy::primitives::Address,
    amount_base: U256,
) -> eyre::Result<Option<CompoundCalldata>> {
    match tool_name {
        "lend_tokens" => {
            let call = comet.supply(token_addr, amount_base);
            let data: Vec<u8> = call.calldata().to_vec();
            Ok(Some(CompoundCalldata {
                op: WriteOp::Lend,
                history_type: "lend",
                data,
                action_label: "supply",
            }))
        }
        "withdraw_lending" => {
            let call = comet.withdraw(token_addr, amount_base);
            let data: Vec<u8> = call.calldata().to_vec();
            Ok(Some(CompoundCalldata {
                op: WriteOp::WithdrawLending,
                history_type: "withdraw_lending",
                data,
                action_label: "withdraw",
            }))
        }
        "borrow_tokens" => {
            let call = comet.withdraw(token_addr, amount_base);
            let data: Vec<u8> = call.calldata().to_vec();
            Ok(Some(CompoundCalldata {
                op: WriteOp::Borrow,
                history_type: "borrow",
                data,
                action_label: "borrow",
            }))
        }
        "repay_borrow" => {
            let call = comet.supply(token_addr, amount_base);
            let data: Vec<u8> = call.calldata().to_vec();
            Ok(Some(CompoundCalldata {
                op: WriteOp::RepayBorrow,
                history_type: "repay_borrow",
                data,
                action_label: "repay",
            }))
        }
        _ => Ok(None),
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
    let va = match validate_compound_args(&args, shared, &req_id)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let evm = setup_compound_evm(shared, va.chain)?;
    let _from = evm_addr_for_account(&va.w, va.idx)?;
    let comet_addr = EvmChain::parse_address(va.comet_s).context("parse comet_address")?;

    if shared.scam_blocklist_contains_evm(comet_addr).await {
        compound_blocklist_audit(shared, tool_name, &va.w.name, va.idx, va.chain, comet_addr);
        Keystore::release_lock(va.lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "scam_address_blocked",
                "blocked by scam address blocklist",
            )),
        ));
    }

    let resolved = match resolve_compound_token(&evm, shared, &args, &va, comet_addr).await? {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(va.lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };
    let CompoundTokenResolved {
        token_addr,
        symbol,
        amount_base,
        usd_value,
        usd_value_known,
    } = resolved;

    let comet = ICometV3::new(comet_addr, evm.provider()?);
    let base_token: alloy::primitives::Address =
        comet.baseToken().call().await.context("comet baseToken")?;
    if matches!(tool_name, "borrow_tokens" | "repay_borrow") && token_addr != base_token {
        Keystore::release_lock(va.lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "Compound v3 only supports borrowing/repaying the market base token (use token=\"base\" or the baseToken address)",
            )),
        ));
    }

    let Some(CompoundCalldata {
        op,
        history_type,
        data: call_data,
        action_label,
    }) = build_compound_calldata(&comet, tool_name, token_addr, amount_base)?
    else {
        Keystore::release_lock(va.lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "unknown tool")),
        ));
    };

    let params = CompoundParams {
        lock: va.lock,
        w: va.w,
        idx: va.idx,
        chain: va.chain,
        comet_s: va.comet_s,
        comet_addr,
        token_addr,
        amount_base,
        amount_s: va.amount_s,
        units: va.units,
        symbol,
        usd_value,
        usd_value_known,
    };

    compound_execute_and_respond(
        shared,
        conn,
        stdin,
        stdout,
        req_id,
        CompoundAction {
            tool_name,
            evm: &evm,
            p: params,
            op,
            history_type,
            call_data,
            action_label,
        },
    )
    .await
}
