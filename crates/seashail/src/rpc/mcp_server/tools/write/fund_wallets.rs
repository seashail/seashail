use crate::{
    amount,
    chains::{evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
    financial_math,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
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

#[derive(Debug, Clone)]
struct Destination {
    wallet: String,
    account_index: u32,
}

fn parse_destinations(args: &Value) -> eyre::Result<Vec<Destination>> {
    let Some(arr) = args.get("destinations").and_then(Value::as_array) else {
        eyre::bail!("missing destinations");
    };
    let mut out = vec![];
    for v in arr {
        let Some(obj) = v.as_object() else {
            continue;
        };
        let wallet = obj
            .get("wallet")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let idx = obj
            .get("account_index")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(u32::MAX);
        if wallet.is_empty() || idx == u32::MAX {
            continue;
        }
        out.push(Destination {
            wallet,
            account_index: idx,
        });
    }
    if out.is_empty() {
        eyre::bail!("destinations must be a non-empty array of {{wallet, account_index}}");
    }
    if out.len() > 100 {
        eyre::bail!("too many destinations (max 100)");
    }
    Ok(out)
}

pub async fn handle<R, W>(ctx: &mut HandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let parsed = match parse_fund_args(ctx, lock)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    if parsed.chain == "solana" {
        return fund_solana(ctx, parsed).await;
    }
    fund_evm(ctx, parsed).await
}

struct ParsedFund {
    lock: std::fs::File,
    chain: String,
    token: String,
    amount_each: String,
    units: String,
    from_w: crate::wallet::WalletRecord,
    from_idx: u32,
    effective_policy: crate::policy::Policy,
    destinations: Vec<Destination>,
}

fn parse_fund_args<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    lock: std::fs::File,
) -> eyre::Result<Result<ParsedFund, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let args = &ctx.args;
    let req_id = ctx.req_id.clone();

    let chain = args
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("native")
        .to_owned();
    let amount_each = args
        .get("amount_each")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let units = args
        .get("amount_units")
        .and_then(|v| v.as_str())
        .unwrap_or("ui")
        .to_owned();

    let from_wallet = args.get("from_wallet").and_then(|v| v.as_str());
    let from_account_index = args
        .get("from_account_index")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());

    if chain.is_empty() || amount_each.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "missing chain/amount_each",
            )),
        )));
    }

    let from_w = match from_wallet.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => ctx
            .shared
            .ks
            .get_wallet_by_name(name)?
            .ok_or_else(|| crate::errors::SeashailError::WalletNotFound(name.to_owned()))?,
        None => {
            ctx.shared
                .ks
                .get_active_wallet()?
                .ok_or_else(|| crate::errors::SeashailError::WalletNotFound("active".into()))?
                .0
        }
    };
    let from_idx = from_account_index.unwrap_or(from_w.last_active_account);
    if from_idx >= from_w.accounts {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            req_id,
            tool_err(ToolError::new(
                "account_index_out_of_range",
                "from_account_index out of range",
            )),
        )));
    }

    let (effective_policy, _) = ctx.shared.cfg.policy_for_wallet(Some(from_w.name.as_str()));

    let destinations = match parse_destinations(args) {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Ok(Err(ok(
                req_id,
                tool_err(ToolError::new("invalid_request", e.to_string())),
            )));
        }
    };

    Ok(Ok(ParsedFund {
        lock,
        chain,
        token,
        amount_each,
        units,
        from_w,
        from_idx,
        effective_policy,
        destinations,
    }))
}

async fn fund_solana<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: ParsedFund,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let req_id = ctx.req_id.clone();
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

    let mint_decimals = if is_native_token(&p.token) {
        9_u8
    } else {
        let mint = SolanaChain::parse_pubkey(&p.token)?;
        sol.get_mint_decimals(mint)
            .await
            .context("get spl mint decimals")?
    };
    let amount_base = if p.units == "base" {
        u128_to_u64(amount::parse_amount_base_u128(&p.amount_each)?)?
    } else {
        u128_to_u64(amount::parse_amount_ui_to_base_u128(
            &p.amount_each,
            u32::from(mint_decimals),
        )?)?
    };

    let usd_value_each = sol_usd_value_each(ctx, &sol, &p.token, amount_base).await?;

    let outcome: Option<WriteConfirmOutcome> = if p.effective_policy.internal_transfers_exempt.get()
    {
        None
    } else {
        let usd_total = financial_math::mul_f64(
            usd_value_each,
            f64::from(u32::try_from(p.destinations.len()).unwrap_or(0)),
        );
        let summary = format!(
            "Batch internal transfer (Solana): {} destinations from {}:{} ({:.2} USD total)",
            p.destinations.len(),
            p.from_w.name,
            p.from_idx,
            usd_total
        );
        match maybe_confirm_write(
            ctx.shared,
            ctx.conn,
            ctx.stdin,
            ctx.stdout,
            &WriteConfirmRequest {
                tool: "fund_wallets",
                wallet: Some(p.from_w.name.as_str()),
                account_index: Some(p.from_idx),
                op: WriteOp::InternalTransfer,
                chain: "solana",
                usd_value: usd_total,
                usd_value_known: true,
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
                return Ok(ok(req_id, tool_err(te)));
            }
        }
    };

    let kp = load_solana_keypair(
        ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &p.from_w, p.from_idx,
    )
    .await?;

    let mut results: Vec<Value> = Vec::new();
    for d in &p.destinations {
        let r = sol_fund_one(ctx, &p, &sol, &kp, amount_base, usd_value_each, d).await?;
        results.push(r);
    }

    sol_audit_and_respond(ctx, &p, outcome.as_ref(), usd_value_each, &results)?;
    Keystore::release_lock(p.lock)?;
    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": "solana", "results": results })),
    ))
}

async fn sol_usd_value_each<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    sol: &SolanaChain,
    token: &str,
    amount_base: u64,
) -> eyre::Result<f64>
where
    R: tokio::io::AsyncRead + Unpin + Send,
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

async fn sol_fund_one<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: &ParsedFund,
    sol: &SolanaChain,
    kp: &solana_sdk::signer::keypair::Keypair,
    amount_base: u64,
    usd_value_each: f64,
    d: &Destination,
) -> eyre::Result<Value>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let Some(to_w) = ctx.shared.ks.get_wallet_by_name(&d.wallet)? else {
        return Ok(json!({
            "wallet": d.wallet, "account_index": d.account_index,
            "ok": false, "error_code": "wallet_not_found"
        }));
    };
    if d.account_index >= to_w.accounts {
        return Ok(json!({
            "wallet": to_w.name, "account_index": d.account_index,
            "ok": false, "error_code": "account_index_out_of_range"
        }));
    }
    let to_pk = match sol_pubkey_for_account(&to_w, d.account_index) {
        Ok(pk) => pk,
        Err(e) => {
            return Ok(json!({
                "wallet": to_w.name, "account_index": d.account_index,
                "ok": false, "error_code": "invalid_destination", "error": e.to_string()
            }));
        }
    };
    if p.effective_policy.enable_ofac_sdn.get() && ctx.shared.ofac_sdn_contains_solana(to_pk).await
    {
        return Ok(json!({
            "wallet": to_w.name, "account_index": d.account_index,
            "ok": false, "error_code": "ofac_sdn_blocked"
        }));
    }
    if ctx.shared.scam_blocklist_contains_solana(to_pk).await {
        return Ok(json!({
            "wallet": to_w.name, "account_index": d.account_index,
            "ok": false, "error_code": "scam_address_blocked"
        }));
    }

    let sig = match if is_native_token(&p.token) {
        sol.send_sol(kp, to_pk, amount_base).await
    } else {
        let mint = SolanaChain::parse_pubkey(&p.token)?;
        sol.send_spl(kp, to_pk, mint, amount_base).await
    } {
        Ok(sig) => sig,
        Err(e) => {
            return Ok(json!({
                "wallet": to_w.name, "account_index": d.account_index,
                "ok": false, "error_code": "send_failed", "error": e.to_string()
            }));
        }
    };

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
        "to_wallet": to_w.name,
        "to_account_index": d.account_index,
        "token": if is_native_token(&p.token) { "native" } else { &p.token },
        "amount_base": amount_base.to_string(),
        "usd_value": usd_value_each,
        "signature": sig.to_string()
    }))?;

    Ok(json!({
        "wallet": to_w.name, "account_index": d.account_index,
        "ok": true, "signature": sig.to_string(),
        "usd_value": usd_value_each, "usd_value_known": true
    }))
}

fn sol_audit_and_respond<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    p: &ParsedFund,
    outcome: Option<&WriteConfirmOutcome>,
    usd_value_each: f64,
    results: &[Value],
) -> eyre::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send,
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

    ctx.shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(),
        "tool": "fund_wallets",
        "wallet": p.from_w.name,
        "account_index": p.from_idx,
        "chain": "solana",
        "usd_value": financial_math::mul_f64(
            usd_value_each,
            f64::from(u32::try_from(results.len()).unwrap_or(0)),
        ),
        "usd_value_known": true,
        "policy_decision": policy_decision,
        "confirm_required": confirm_required,
        "confirm_result": confirm_result,
        "daily_used_usd": daily_used_usd,
        "forced_confirm": forced_confirm,
        "txid": null,
        "error_code": null,
        "result": "completed_batch"
    }))?;
    Ok(())
}

async fn fund_evm<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: ParsedFund,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let req_id = ctx.req_id.clone();
    let rpc_url = ctx
        .shared
        .cfg
        .rpc
        .evm_rpc_urls
        .get(p.chain.as_str())
        .ok_or_else(|| eyre::eyre!("unknown evm chain: {}", p.chain))?
        .clone();
    let chain_id = *ctx
        .shared
        .cfg
        .rpc
        .evm_chain_ids
        .get(p.chain.as_str())
        .ok_or_else(|| eyre::eyre!("missing evm chain id: {}", p.chain))?;
    let mut evm = EvmChain::for_name(&p.chain, chain_id, &rpc_url, &ctx.shared.cfg.http);
    if let Some(fb) = ctx
        .shared
        .cfg
        .rpc
        .evm_fallback_rpc_urls
        .get(p.chain.as_str())
    {
        evm.fallback_rpc_urls.clone_from(fb);
    }
    let from = evm_addr_for_account(&p.from_w, p.from_idx)?;

    let (usd_value_each, amount_base, token_addr_opt) = evm_resolve_amount(ctx, &evm, &p).await?;

    let outcome: Option<WriteConfirmOutcome> = if p.effective_policy.internal_transfers_exempt.get()
    {
        None
    } else {
        let usd_total = financial_math::mul_f64(
            usd_value_each,
            f64::from(u32::try_from(p.destinations.len()).unwrap_or(0)),
        );
        let summary = format!(
            "Batch internal transfer (EVM): {} destinations from {}:{} ({:.2} USD total)",
            p.destinations.len(),
            p.from_w.name,
            p.from_idx,
            usd_total
        );
        match maybe_confirm_write(
            ctx.shared,
            ctx.conn,
            ctx.stdin,
            ctx.stdout,
            &WriteConfirmRequest {
                tool: "fund_wallets",
                wallet: Some(p.from_w.name.as_str()),
                account_index: Some(p.from_idx),
                op: WriteOp::InternalTransfer,
                chain: p.chain.as_str(),
                usd_value: usd_total,
                usd_value_known: true,
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
                return Ok(ok(req_id, tool_err(te)));
            }
        }
    };

    let signer = load_evm_signer(
        ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &p.from_w, p.from_idx,
    )
    .await?;

    let fc = EvmFundCtx {
        evm: &evm,
        from,
        signer: &signer,
        amount_base,
        usd_value_each,
        token_addr_opt,
    };
    let mut results: Vec<Value> = Vec::new();
    for d in &p.destinations {
        let r = evm_fund_one(ctx, &p, &fc, d).await?;
        results.push(r);
    }

    evm_audit_and_respond(ctx, &p, outcome.as_ref(), usd_value_each, &results)?;
    Keystore::release_lock(p.lock)?;
    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": p.chain, "results": results })),
    ))
}

async fn evm_resolve_amount<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    evm: &EvmChain,
    p: &ParsedFund,
) -> eyre::Result<(f64, U256, Option<alloy::primitives::Address>)>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if is_native_token(&p.token) {
        let amount_base = if p.units == "base" {
            crate::chains::evm::parse_u256_dec(&p.amount_each)?
        } else {
            u128_to_u256(amount::parse_amount_ui_to_base_u128(&p.amount_each, 18)?)
        };
        let base_u128 = amount_base.to_string().parse::<u128>().unwrap_or(0);
        let usd = {
            ctx.shared.ensure_db().await;
            let db = ctx.shared.db();
            price::native_token_price_usd_cached(&p.chain, &ctx.shared.cfg, db)
                .await?
                .usd
        };
        let usd_value_each = financial_math::token_base_to_usd(base_u128, 18, usd);
        Ok((usd_value_each, amount_base, None))
    } else {
        let token_addr = EvmChain::parse_address(&p.token)?;
        let (decimals, _sym) = evm.get_erc20_metadata(token_addr).await?;
        let amount_base = if p.units == "base" {
            crate::chains::evm::parse_u256_dec(&p.amount_each)?
        } else {
            u128_to_u256(amount::parse_amount_ui_to_base_u128(
                &p.amount_each,
                u32::from(decimals),
            )?)
        };
        let usd_value_each = {
            ctx.shared.ensure_db().await;
            let db = ctx.shared.db();
            price::evm_token_price_usd_cached(evm, &ctx.shared.cfg, token_addr, amount_base, 50, db)
                .await?
                .usd
        };
        Ok((usd_value_each, amount_base, Some(token_addr)))
    }
}

struct EvmFundCtx<'a> {
    evm: &'a EvmChain,
    from: alloy::primitives::Address,
    signer: &'a alloy::signers::local::PrivateKeySigner,
    amount_base: U256,
    usd_value_each: f64,
    token_addr_opt: Option<alloy::primitives::Address>,
}

async fn evm_fund_one<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: &ParsedFund,
    fc: &EvmFundCtx<'_>,
    d: &Destination,
) -> eyre::Result<Value>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let Some(to_w) = ctx.shared.ks.get_wallet_by_name(&d.wallet)? else {
        return Ok(json!({
            "wallet": d.wallet, "account_index": d.account_index,
            "ok": false, "error_code": "wallet_not_found"
        }));
    };
    if d.account_index >= to_w.accounts {
        return Ok(json!({
            "wallet": to_w.name, "account_index": d.account_index,
            "ok": false, "error_code": "account_index_out_of_range"
        }));
    }
    let to_addr = match evm_addr_for_account(&to_w, d.account_index) {
        Ok(a) => a,
        Err(e) => {
            return Ok(json!({
                "wallet": to_w.name, "account_index": d.account_index,
                "ok": false, "error_code": "invalid_destination", "error": e.to_string()
            }));
        }
    };
    if p.effective_policy.enable_ofac_sdn.get() && ctx.shared.ofac_sdn_contains_evm(to_addr).await {
        return Ok(json!({
            "wallet": to_w.name, "account_index": d.account_index,
            "ok": false, "error_code": "ofac_sdn_blocked"
        }));
    }
    if ctx.shared.scam_blocklist_contains_evm(to_addr).await {
        return Ok(json!({
            "wallet": to_w.name, "account_index": d.account_index,
            "ok": false, "error_code": "scam_address_blocked"
        }));
    }

    let tx: TransactionRequest = match fc.token_addr_opt {
        None => EvmChain::build_native_transfer(fc.from, to_addr, fc.amount_base),
        Some(token_addr) => {
            match fc
                .evm
                .build_erc20_transfer(fc.from, token_addr, to_addr, fc.amount_base)
            {
                Ok(t) => t,
                Err(e) => {
                    return Ok(json!({
                        "wallet": to_w.name, "account_index": d.account_index,
                        "ok": false, "error_code": "build_tx_failed", "error": e.to_string()
                    }));
                }
            }
        }
    };

    if let Err(e) = fc.evm.simulate_tx_strict(&tx).await {
        return Ok(json!({
            "wallet": to_w.name, "account_index": d.account_index,
            "ok": false, "error_code": "simulation_failed",
            "error": summarize_sim_error(&e, "fund_wallets")
        }));
    }

    let txid = match fc.evm.send_tx(fc.signer.clone(), tx).await {
        Ok(h) => h,
        Err(e) => {
            return Ok(json!({
                "wallet": to_w.name, "account_index": d.account_index,
                "ok": false, "error_code": "send_failed", "error": e.to_string()
            }));
        }
    };

    let ty = if p.effective_policy.internal_transfers_exempt.get() {
        "internal_transfer"
    } else {
        "internal_transfer_strict"
    };

    ctx.shared.ks.append_tx_history(&json!({
        "ts": utc_now_iso(),
        "day": Keystore::current_utc_day_key(),
        "type": ty,
        "chain": p.chain,
        "wallet": p.from_w.name,
        "account_index": p.from_idx,
        "to_wallet": to_w.name,
        "to_account_index": d.account_index,
        "token": if is_native_token(&p.token) { "native" } else { &p.token },
        "amount_base": fc.amount_base.to_string(),
        "usd_value": fc.usd_value_each,
        "txid": format!("{txid:#x}")
    }))?;

    Ok(json!({
        "wallet": to_w.name, "account_index": d.account_index,
        "ok": true, "txid": format!("{txid:#x}"),
        "usd_value": fc.usd_value_each, "usd_value_known": true
    }))
}

fn evm_audit_and_respond<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    p: &ParsedFund,
    outcome: Option<&WriteConfirmOutcome>,
    usd_value_each: f64,
    results: &[Value],
) -> eyre::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send,
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

    ctx.shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(),
        "tool": "fund_wallets",
        "wallet": p.from_w.name,
        "account_index": p.from_idx,
        "chain": p.chain,
        "usd_value": financial_math::mul_f64(
            usd_value_each,
            f64::from(u32::try_from(results.len()).unwrap_or(0)),
        ),
        "usd_value_known": true,
        "policy_decision": policy_decision,
        "confirm_required": confirm_required,
        "confirm_result": confirm_result,
        "daily_used_usd": daily_used_usd,
        "forced_confirm": forced_confirm,
        "txid": null,
        "error_code": null,
        "result": "completed_batch"
    }))?;
    Ok(())
}
