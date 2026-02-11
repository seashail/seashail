use crate::{
    amount,
    chains::{evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
    financial_math,
    keystore::{utc_now_iso, Keystore},
    price,
};
use alloy::primitives::U256;
use alloy::rpc::types::TransactionRequest;
use eyre::Context as _;
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::helpers::{
    evm_addr_for_account, is_native_token, sol_pubkey_for_account, solana_fallback_urls,
    u128_to_u256, u128_to_u64,
};
use super::super::key_loading::{load_evm_signer, load_solana_keypair};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmOutcome, WriteConfirmRequest};
use super::common::summarize_sim_error;
use super::HandlerCtx;
use crate::policy_engine::WriteOp;

struct InternalTransferParams<'a> {
    lock: std::fs::File,
    token: &'a str,
    amount: &'a str,
    units: &'a str,
    from_w: &'a crate::wallet::WalletRecord,
    from_idx: u32,
    to_w: &'a crate::wallet::WalletRecord,
    to_idx: u32,
    effective_policy: &'a crate::policy::Policy,
}

async fn handle_solana_internal_transfer<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: InternalTransferParams<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
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

    let amount_base = sol_internal_amount(&sol, p.token, p.amount, p.units).await?;
    let usd_value = sol_internal_usd(ctx, &sol, p.token, amount_base).await?;
    let usd_value_known = true;

    let to_pk = sol_pubkey_for_account(p.to_w, p.to_idx)?;

    if let Some(resp) =
        sol_internal_blocklist_check(ctx, &p, to_pk, usd_value, usd_value_known).await
    {
        return resp;
    }

    let outcome: Option<WriteConfirmOutcome> = if p.effective_policy.internal_transfers_exempt.get()
    {
        None
    } else {
        let summary = format!(
            "Internal transfer (Solana): {}:{} -> {}:{} ({:.2} USD)",
            p.from_w.name, p.from_idx, p.to_w.name, p.to_idx, usd_value
        );
        match maybe_confirm_write(
            ctx.shared,
            ctx.conn,
            ctx.stdin,
            ctx.stdout,
            &WriteConfirmRequest {
                tool: "transfer_between_wallets",
                wallet: Some(p.from_w.name.as_str()),
                account_index: Some(p.from_idx),
                op: WriteOp::InternalTransfer,
                chain: "solana",
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
            Ok(o) => Some(o),
            Err(te) => {
                Keystore::release_lock(p.lock)?;
                return Ok(ok(ctx.req_id.clone(), tool_err(te)));
            }
        }
    };

    let kp = load_solana_keypair(
        ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, p.from_w, p.from_idx,
    )
    .await?;
    let sig = if is_native_token(p.token) {
        sol.send_sol(&kp, to_pk, amount_base).await?
    } else {
        let mint = SolanaChain::parse_pubkey(p.token)?;
        sol.send_spl(&kp, to_pk, mint, amount_base).await?
    };

    sol_internal_record(
        ctx,
        &p,
        outcome.as_ref(),
        &sig,
        amount_base,
        usd_value,
        usd_value_known,
    )?;

    Keystore::release_lock(p.lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
          "chain": "solana",
          "signature": sig.to_string(),
          "usd_value": usd_value
        })),
    ))
}

async fn sol_internal_amount(
    sol: &SolanaChain,
    token: &str,
    amount: &str,
    units: &str,
) -> eyre::Result<u64> {
    let mint_decimals = if is_native_token(token) {
        9_u8
    } else {
        let mint = SolanaChain::parse_pubkey(token)?;
        sol.get_mint_decimals(mint)
            .await
            .context("get spl mint decimals")?
    };
    if units == "base" {
        u128_to_u64(amount::parse_amount_base_u128(amount)?)
    } else {
        u128_to_u64(amount::parse_amount_ui_to_base_u128(
            amount,
            u32::from(mint_decimals),
        )?)
    }
}

async fn sol_internal_usd<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    sol: &SolanaChain,
    token: &str,
    amount_base: u64,
) -> eyre::Result<f64>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if is_native_token(token) {
        let usd = {
            ctx.shared.ensure_db().await;
            let db = ctx.shared.db();
            price::native_token_price_usd_cached("solana", &ctx.shared.cfg, db)
                .await?
                .usd
        };
        Ok(financial_math::lamports_to_usd(amount_base, usd))
    } else {
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        ctx.shared.ensure_db().await;
        let db = ctx.shared.db();
        Ok(price::solana_token_price_usd_cached(
            sol,
            &ctx.shared.cfg,
            token,
            usdc_mint,
            amount_base,
            50,
            db,
        )
        .await?
        .usd)
    }
}

async fn sol_internal_blocklist_check<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: &InternalTransferParams<'_>,
    to_pk: solana_sdk::pubkey::Pubkey,
    usd_value: f64,
    usd_value_known: bool,
) -> Option<eyre::Result<JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if p.effective_policy.enable_ofac_sdn.get() && ctx.shared.ofac_sdn_contains_solana(to_pk).await
    {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(),
          "tool": "transfer_between_wallets",
          "wallet": p.from_w.name,
          "account_index": p.from_idx,
          "chain": "solana",
          "usd_value": usd_value,
          "usd_value_known": usd_value_known,
          "policy_decision": "auto_approve_internal",
          "confirm_required": false,
          "confirm_result": null,
          "txid": null,
          "error_code": "ofac_sdn_blocked",
          "result": "blocked_ofac_sdn",
          "to_wallet": p.to_w.name,
          "to_account_index": p.to_idx
        }));
        return Some(Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        )));
    }
    if ctx.shared.scam_blocklist_contains_solana(to_pk).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(),
          "tool": "transfer_between_wallets",
          "wallet": p.from_w.name,
          "account_index": p.from_idx,
          "chain": "solana",
          "usd_value": usd_value,
          "usd_value_known": usd_value_known,
          "policy_decision": "auto_approve_internal",
          "confirm_required": false,
          "confirm_result": null,
          "txid": null,
          "error_code": "scam_address_blocked",
          "result": "blocked_scam_blocklist",
          "to_wallet": p.to_w.name,
          "to_account_index": p.to_idx
        }));
        return Some(Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient is blocked by the scam address blocklist",
            )),
        )));
    }
    None
}

fn sol_internal_record<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    p: &InternalTransferParams<'_>,
    outcome: Option<&WriteConfirmOutcome>,
    sig: &solana_sdk::signature::Signature,
    amount_base: u64,
    usd_value: f64,
    usd_value_known: bool,
) -> eyre::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let ty = if p.effective_policy.internal_transfers_exempt.get() {
        "internal_transfer"
    } else {
        "internal_transfer_strict"
    };
    ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": ty,
      "chain": "solana",
      "wallet": p.from_w.name,
      "account_index": p.from_idx,
      "to_wallet": p.to_w.name,
      "to_account_index": p.to_idx,
      "token": if is_native_token(p.token) { "native" } else { p.token },
      "amount_base": amount_base.to_string(),
      "usd_value": usd_value,
      "signature": sig.to_string()
    }))?;

    let (policy_decision, confirm_required, confirm_result, daily_used_usd, forced_confirm) =
        if let Some(o) = outcome {
            (
                o.policy_decision,
                o.confirm_required,
                o.confirm_result,
                o.daily_used_usd,
                o.forced_confirm,
            )
        } else {
            ("auto_approve_internal", false, None, 0.0_f64, false)
        };
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(),
      "tool": "transfer_between_wallets",
      "wallet": p.from_w.name,
      "account_index": p.from_idx,
      "chain": "solana",
      "usd_value": usd_value,
      "usd_value_known": usd_value_known,
      "policy_decision": policy_decision,
      "confirm_required": confirm_required,
      "confirm_result": confirm_result,
      "daily_used_usd": daily_used_usd,
      "forced_confirm": forced_confirm,
      "txid": sig.to_string(),
      "error_code": null,
      "result": "broadcasted",
      "signature": sig.to_string(),
      "to_wallet": p.to_w.name,
      "to_account_index": p.to_idx
    }));
    Ok(())
}

pub async fn handle_ctx<R, W>(ctx: &mut HandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let parsed = match parse_internal_transfer_args(ctx, lock)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    if parsed.chain == "solana" {
        let p = InternalTransferParams {
            lock: parsed.lock,
            token: &parsed.token,
            amount: &parsed.amount,
            units: &parsed.units,
            from_w: &parsed.from_w,
            from_idx: parsed.from_idx,
            to_w: &parsed.to_w,
            to_idx: parsed.to_idx,
            effective_policy: &parsed.effective_policy,
        };
        return handle_solana_internal_transfer(ctx, p).await;
    }

    let p = InternalTransferParams {
        lock: parsed.lock,
        token: &parsed.token,
        amount: &parsed.amount,
        units: &parsed.units,
        from_w: &parsed.from_w,
        from_idx: parsed.from_idx,
        to_w: &parsed.to_w,
        to_idx: parsed.to_idx,
        effective_policy: &parsed.effective_policy,
    };
    handle_evm_internal_transfer(ctx, &parsed.chain, p).await
}

struct ParsedInternalTransfer {
    lock: std::fs::File,
    chain: String,
    token: String,
    amount: String,
    units: String,
    from_w: crate::wallet::WalletRecord,
    from_idx: u32,
    to_w: crate::wallet::WalletRecord,
    to_idx: u32,
    effective_policy: crate::policy::Policy,
}

fn arg_s(args: &Value, key: &str, default: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_owned()
}

fn arg_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
}

fn parse_internal_transfer_args<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    lock: std::fs::File,
) -> eyre::Result<Result<ParsedInternalTransfer, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let chain = arg_s(&ctx.args, "chain", "");
    let token = arg_s(&ctx.args, "token", "native");
    let amount = arg_s(&ctx.args, "amount", "");
    let units = arg_s(&ctx.args, "amount_units", "ui");
    let from_wallet = arg_s(&ctx.args, "from_wallet", "");
    let from_account_index = arg_u32(&ctx.args, "from_account_index");
    let to_wallet = arg_s(&ctx.args, "to_wallet", "");
    let to_account_index = arg_u32(&ctx.args, "to_account_index");

    if chain.trim().is_empty()
        || amount.trim().is_empty()
        || from_wallet.trim().is_empty()
        || to_wallet.trim().is_empty()
        || from_account_index.is_none()
        || to_account_index.is_none()
    {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing chain/amount/from_wallet/from_account_index/to_wallet/to_account_index",
            )),
        )));
    }
    let from_idx = from_account_index.unwrap_or(0);
    let to_idx = to_account_index.unwrap_or(0);

    let from_w = ctx
        .shared
        .ks
        .get_wallet_by_name(&from_wallet)?
        .ok_or_else(|| crate::errors::SeashailError::WalletNotFound(from_wallet.clone()))?;
    if from_idx >= from_w.accounts {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "account_index_out_of_range",
                "from_account_index out of range",
            )),
        )));
    }
    let to_w = ctx
        .shared
        .ks
        .get_wallet_by_name(&to_wallet)?
        .ok_or_else(|| crate::errors::SeashailError::WalletNotFound(to_wallet.clone()))?;
    if to_idx >= to_w.accounts {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "account_index_out_of_range",
                "to_account_index out of range",
            )),
        )));
    }

    let (effective_policy, _) = ctx.shared.cfg.policy_for_wallet(Some(from_w.name.as_str()));

    Ok(Ok(ParsedInternalTransfer {
        lock,
        chain,
        token,
        amount,
        units,
        from_w,
        from_idx,
        to_w,
        to_idx,
        effective_policy,
    }))
}

/// Initialize EVM chain from config.
fn init_evm_chain(
    shared: &crate::rpc::mcp_server::SharedState,
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

async fn handle_evm_internal_transfer<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    chain: &str,
    p: InternalTransferParams<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let evm = init_evm_chain(ctx.shared, chain)?;
    let from = evm_addr_for_account(p.from_w, p.from_idx)?;
    let to_addr = evm_addr_for_account(p.to_w, p.to_idx)?;

    if let Some(resp) = evm_internal_blocklist_check(ctx, chain, &p, to_addr).await {
        Keystore::release_lock(p.lock)?;
        return resp;
    }

    let btx = EvmBuildTxCtx {
        evm: &evm,
        chain,
        token: p.token,
        amount: p.amount,
        units: p.units,
        from,
        to_addr,
    };
    let (usd_value, amount_base, tx) = evm_internal_build_tx(ctx, &btx).await?;
    let usd_value_known = true;

    let outcome: Option<WriteConfirmOutcome> = if p.effective_policy.internal_transfers_exempt.get()
    {
        None
    } else {
        let summary = format!(
            "Internal transfer (EVM): {}:{} -> {}:{} ({:.2} USD)",
            p.from_w.name, p.from_idx, p.to_w.name, p.to_idx, usd_value
        );
        match maybe_confirm_write(
            ctx.shared,
            ctx.conn,
            ctx.stdin,
            ctx.stdout,
            &WriteConfirmRequest {
                tool: "transfer_between_wallets",
                wallet: Some(p.from_w.name.as_str()),
                account_index: Some(p.from_idx),
                op: WriteOp::InternalTransfer,
                chain,
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
            Ok(o) => Some(o),
            Err(te) => {
                Keystore::release_lock(p.lock)?;
                return Ok(ok(ctx.req_id.clone(), tool_err(te)));
            }
        }
    };

    if let Err(e) = evm.simulate_tx_strict(&tx).await {
        evm_internal_sim_fail_audit(ctx, &p, outcome.as_ref(), chain, usd_value, usd_value_known);
        Keystore::release_lock(p.lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "transfer_between_wallets"),
            )),
        ));
    }

    let signer = load_evm_signer(
        ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, p.from_w, p.from_idx,
    )
    .await?;
    let txid = evm.send_tx(signer, tx).await?;

    evm_internal_record(
        ctx,
        &p,
        outcome.as_ref(),
        &EvmInternalResult {
            chain,
            txid: &txid,
            amount_base,
            usd_value,
            usd_value_known,
        },
    )?;
    Keystore::release_lock(p.lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
          "chain": chain, "txid": format!("{txid:#x}"), "usd_value": usd_value
        })),
    ))
}

async fn evm_internal_blocklist_check<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    chain: &str,
    p: &InternalTransferParams<'_>,
    to_addr: alloy::primitives::Address,
) -> Option<eyre::Result<JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if p.effective_policy.enable_ofac_sdn.get() && ctx.shared.ofac_sdn_contains_evm(to_addr).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(),
          "tool": "transfer_between_wallets",
          "wallet": p.from_w.name, "account_index": p.from_idx,
          "chain": chain, "usd_value": 0.0_f64, "usd_value_known": false,
          "policy_decision": "auto_approve_internal",
          "confirm_required": false, "confirm_result": null,
          "txid": null, "error_code": "ofac_sdn_blocked",
          "result": "blocked_ofac_sdn",
          "to_wallet": p.to_w.name, "to_account_index": p.to_idx
        }));
        return Some(Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        )));
    }
    if ctx.shared.scam_blocklist_contains_evm(to_addr).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(),
          "tool": "transfer_between_wallets",
          "wallet": p.from_w.name, "account_index": p.from_idx,
          "chain": chain, "usd_value": 0.0_f64, "usd_value_known": false,
          "policy_decision": "auto_approve_internal",
          "confirm_required": false, "confirm_result": null,
          "txid": null, "error_code": "scam_address_blocked",
          "result": "blocked_scam_blocklist",
          "to_wallet": p.to_w.name, "to_account_index": p.to_idx
        }));
        return Some(Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient is blocked by the scam address blocklist",
            )),
        )));
    }
    None
}

struct EvmBuildTxCtx<'a> {
    evm: &'a EvmChain,
    chain: &'a str,
    token: &'a str,
    amount: &'a str,
    units: &'a str,
    from: alloy::primitives::Address,
    to_addr: alloy::primitives::Address,
}

async fn evm_internal_build_tx<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    b: &EvmBuildTxCtx<'_>,
) -> eyre::Result<(f64, U256, TransactionRequest)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if is_native_token(b.token) {
        let amount_base = if b.units == "base" {
            crate::chains::evm::parse_u256_dec(b.amount)?
        } else {
            u128_to_u256(amount::parse_amount_ui_to_base_u128(b.amount, 18)?)
        };
        let usd = {
            ctx.shared.ensure_db().await;
            let db = ctx.shared.db();
            price::native_token_price_usd_cached(b.chain, &ctx.shared.cfg, db)
                .await?
                .usd
        };
        let usd_value = financial_math::token_base_to_usd(
            crate::chains::evm::u256_low_u128(amount_base),
            18,
            usd,
        );
        let tx = EvmChain::build_native_transfer(b.from, b.to_addr, amount_base);
        Ok((usd_value, amount_base, tx))
    } else {
        let token_addr = EvmChain::parse_address(b.token)?;
        let (decimals, _sym) = b.evm.get_erc20_metadata(token_addr).await?;
        let amount_base = if b.units == "base" {
            crate::chains::evm::parse_u256_dec(b.amount)?
        } else {
            u128_to_u256(amount::parse_amount_ui_to_base_u128(
                b.amount,
                u32::from(decimals),
            )?)
        };
        let usd_value = {
            ctx.shared.ensure_db().await;
            let db = ctx.shared.db();
            price::evm_token_price_usd_cached(
                b.evm,
                &ctx.shared.cfg,
                token_addr,
                amount_base,
                50,
                db,
            )
            .await?
            .usd
        };
        let tx = b
            .evm
            .build_erc20_transfer(b.from, token_addr, b.to_addr, amount_base)?;
        Ok((usd_value, amount_base, tx))
    }
}

fn evm_internal_sim_fail_audit<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    p: &InternalTransferParams<'_>,
    outcome: Option<&WriteConfirmOutcome>,
    chain: &str,
    usd_value: f64,
    usd_value_known: bool,
) where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (policy_decision, confirm_required, confirm_result, daily_used_usd, forced_confirm) =
        if let Some(o) = outcome {
            (
                o.policy_decision,
                o.confirm_required,
                o.confirm_result,
                o.daily_used_usd,
                o.forced_confirm,
            )
        } else {
            ("auto_approve_internal", false, None, 0.0_f64, false)
        };
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(),
      "tool": "transfer_between_wallets",
      "wallet": p.from_w.name, "account_index": p.from_idx,
      "chain": chain, "usd_value": usd_value, "usd_value_known": usd_value_known,
      "policy_decision": policy_decision,
      "confirm_required": confirm_required, "confirm_result": confirm_result,
      "daily_used_usd": daily_used_usd,
      "forced_confirm": forced_confirm,
      "txid": null, "error_code": "simulation_failed",
      "result": "blocked_simulation",
    }));
}

/// Result of a completed EVM internal transfer.
struct EvmInternalResult<'a> {
    chain: &'a str,
    txid: &'a alloy::primitives::B256,
    amount_base: U256,
    usd_value: f64,
    usd_value_known: bool,
}

fn evm_internal_record<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    p: &InternalTransferParams<'_>,
    outcome: Option<&WriteConfirmOutcome>,
    r: &EvmInternalResult<'_>,
) -> eyre::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let ty = if p.effective_policy.internal_transfers_exempt.get() {
        "internal_transfer"
    } else {
        "internal_transfer_strict"
    };
    ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": ty,
      "chain": r.chain,
      "wallet": p.from_w.name, "account_index": p.from_idx,
      "to_wallet": p.to_w.name, "to_account_index": p.to_idx,
      "token": if is_native_token(p.token) { "native" } else { p.token },
      "amount_base": r.amount_base.to_string(),
      "usd_value": r.usd_value,
      "txid": format!("{:#x}", r.txid)
    }))?;

    let (policy_decision, confirm_required, confirm_result, daily_used_usd, forced_confirm) =
        if let Some(o) = outcome {
            (
                o.policy_decision,
                o.confirm_required,
                o.confirm_result,
                o.daily_used_usd,
                o.forced_confirm,
            )
        } else {
            ("auto_approve_internal", false, None, 0.0_f64, false)
        };
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(),
      "tool": "transfer_between_wallets",
      "wallet": p.from_w.name, "account_index": p.from_idx,
      "chain": r.chain, "usd_value": r.usd_value, "usd_value_known": r.usd_value_known,
      "policy_decision": policy_decision,
      "confirm_required": confirm_required, "confirm_result": confirm_result,
      "daily_used_usd": daily_used_usd,
      "forced_confirm": forced_confirm,
      "txid": format!("{:#x}", r.txid),
      "error_code": null, "result": "broadcasted",
    }));
    Ok(())
}
