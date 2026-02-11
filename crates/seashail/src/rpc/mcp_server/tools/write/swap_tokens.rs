use crate::{
    amount,
    chains::{evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
    financial_math,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
    rpc::mcp_server::SharedState,
};
use alloy::primitives::U256;
use eyre::{Context as _, ContextCompat as _};
use serde_json::json;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::helpers::{
    evm_addr_for_account, is_native_token, resolve_wallet_and_account, sol_pubkey_for_account,
    solana_fallback_urls, u128_to_u256, u128_to_u64,
};
use super::super::key_loading::{load_evm_signer, load_solana_keypair};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmOutcome, WriteConfirmRequest};
use super::common::{summarize_sim_error, wait_for_allowance};
use super::HandlerCtx;

/// Parameters for a Solana swap via Jupiter.
struct SolanaSwapParams<'a> {
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    lock: std::fs::File,
    effective_policy: &'a crate::policy::Policy,
    token_in: &'a str,
    token_out: &'a str,
    amount_in_s: &'a str,
    units: &'a str,
    slippage_bps: u32,
}

/// Bundled data for recording and responding to a completed Solana swap.
struct SolanaSwapResult<'a> {
    shared: &'a SharedState,
    lock: std::fs::File,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    token_in: &'a str,
    token_out: &'a str,
    amt_in: u64,
    expected_out: &'a str,
    slippage_bps: u32,
    usd_value: f64,
    sig: &'a solana_sdk::signature::Signature,
    outcome: &'a WriteConfirmOutcome,
    req_id: &'a serde_json::Value,
}

/// Record swap history/audit and build the success response for a Solana swap.
fn solana_swap_record_and_respond(r: SolanaSwapResult<'_>) -> eyre::Result<JsonRpcResponse> {
    r.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
      "type": "swap", "chain": "solana", "wallet": r.w.name,
      "account_index": r.idx, "token_in": r.token_in, "token_out": r.token_out,
      "amount_in_base": r.amt_in.to_string(), "expected_out_base": r.expected_out,
      "slippage_bps": r.slippage_bps, "usd_value": r.usd_value,
      "signature": r.sig.to_string()
    }))?;
    let _audit_log = r.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": "swap_tokens", "wallet": r.w.name,
      "account_index": r.idx, "chain": "solana", "usd_value": r.usd_value,
      "usd_value_known": true, "policy_decision": r.outcome.policy_decision,
      "confirm_required": r.outcome.confirm_required,
      "confirm_result": r.outcome.confirm_result,
      "daily_used_usd": r.outcome.daily_used_usd,
      "forced_confirm": r.outcome.forced_confirm,
      "txid": r.sig.to_string(), "error_code": null,
      "result": "broadcasted", "signature": r.sig.to_string(),
      "provider": "jupiter"
    }));
    Keystore::release_lock(r.lock)?;
    Ok(ok(
        r.req_id.clone(),
        tool_ok(json!({
          "chain": "solana", "signature": r.sig.to_string(),
          "usd_value": r.usd_value, "expected_out_base": r.expected_out
        })),
    ))
}

/// Resolve the Solana input amount to base units.
fn resolve_solana_amount(amount_in_s: &str, units: &str, decimals_in: u8) -> eyre::Result<u64> {
    if units == "base" {
        u128_to_u64(amount::parse_amount_base_u128(amount_in_s)?)
    } else {
        u128_to_u64(amount::parse_amount_ui_to_base_u128(
            amount_in_s,
            u32::from(decimals_in),
        )?)
    }
}

/// Compute USD value for a Solana token amount.
async fn solana_usd_value(
    shared: &mut SharedState,
    sol: &SolanaChain,
    token_in: &str,
    mint_in: &str,
    amt_in: u64,
) -> eyre::Result<f64> {
    let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    if is_native_token(token_in) {
        let usd = {
            shared.ensure_db().await;
            let db = shared.db();
            price::native_token_price_usd_cached("solana", &shared.cfg, db)
                .await?
                .usd
        };
        Ok(financial_math::lamports_to_usd(amt_in, usd))
    } else {
        shared.ensure_db().await;
        let db = shared.db();
        Ok(price::solana_token_price_usd_cached(
            sol,
            &shared.cfg,
            mint_in,
            usdc_mint,
            amt_in,
            50,
            db,
        )
        .await?
        .usd)
    }
}

async fn handle_solana_swap<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: SolanaSwapParams<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let SolanaSwapParams {
        w,
        idx,
        lock,
        effective_policy,
        token_in,
        token_out,
        amount_in_s,
        units,
        slippage_bps,
    } = p;
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
    let owner = sol_pubkey_for_account(w, idx)?;
    let mint_in = if is_native_token(token_in) {
        "So11111111111111111111111111111111111111112"
    } else {
        token_in
    };
    let mint_out = if is_native_token(token_out) {
        "So11111111111111111111111111111111111111112"
    } else {
        token_out
    };
    let decimals_in = sol
        .get_mint_decimals(SolanaChain::parse_pubkey(mint_in)?)
        .await
        .context("get mint_in decimals")?;
    let amt_in = resolve_solana_amount(amount_in_s, units, decimals_in)?;
    let usd_value = solana_usd_value(ctx.shared, &sol, token_in, mint_in, amt_in).await?;

    let summary = format!("SWAP on Solana via Jupiter: {token_in} -> {token_out}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "swap_tokens",
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: WriteOp::Swap,
            chain: "solana",
            usd_value,
            usd_value_known: true,
            force_confirm: effective_policy.require_user_confirm_for_remote_tx.get(),
            slippage_bps: Some(slippage_bps),
            to_address: None,
            contract: Some("jupiter"),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    let quote = sol
        .jupiter_quote(mint_in, mint_out, amt_in, slippage_bps)
        .await?;
    let expected_out = quote
        .get("outAmount")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_owned();
    let tx_bytes = sol.jupiter_swap_tx(quote, owner).await?;
    let kp = load_solana_keypair(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    let sig = sol.sign_and_send_versioned(&kp, &tx_bytes).await?;

    solana_swap_record_and_respond(SolanaSwapResult {
        shared: ctx.shared,
        lock,
        w,
        idx,
        token_in,
        token_out,
        amt_in,
        expected_out: &expected_out,
        slippage_bps,
        usd_value,
        sig: &sig,
        outcome: &outcome,
        req_id: &ctx.req_id,
    })
}

/// Parameters for an EVM swap (uniswap or 1inch).
struct EvmSwapParams<'a> {
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    lock: std::fs::File,
    effective_policy: &'a crate::policy::Policy,
    chain: &'a str,
    token_in: &'a str,
    token_out: &'a str,
    amount_in_s: &'a str,
    units: &'a str,
    slippage_bps: u32,
    evm: EvmChain,
    from_addr: alloy::primitives::Address,
}

/// Input parameters for resolving EVM amount and price.
struct EvmAmountPriceInput<'a> {
    shared: &'a mut SharedState,
    evm: &'a EvmChain,
    chain: &'a str,
    amount_in_s: &'a str,
    units: &'a str,
    native_in: bool,
    token_in_addr: alloy::primitives::Address,
    usdc_addr: alloy::primitives::Address,
}

/// Resolve EVM input amount and USD price.
async fn resolve_evm_amount_and_price(p: EvmAmountPriceInput<'_>) -> eyre::Result<(U256, f64)> {
    let decimals_in = if p.native_in {
        18_u32
    } else {
        u32::from(p.evm.get_erc20_metadata(p.token_in_addr).await?.0)
    };
    let amt_in: U256 = if p.units == "base" {
        crate::chains::evm::parse_u256_dec(p.amount_in_s)?
    } else {
        u128_to_u256(amount::parse_amount_ui_to_base_u128(
            p.amount_in_s,
            decimals_in,
        )?)
    };
    let usd_value = if p.native_in {
        let usd = {
            p.shared.ensure_db().await;
            let db = p.shared.db();
            price::native_token_price_usd_cached(p.chain, &p.shared.cfg, db)
                .await?
                .usd
        };
        financial_math::token_base_to_usd(crate::chains::evm::u256_low_u128(amt_in), 18, usd)
    } else if p.token_in_addr == p.usdc_addr {
        financial_math::token_base_to_usd(crate::chains::evm::u256_low_u128(amt_in), 6, 1.0)
    } else {
        p.shared.ensure_db().await;
        let db = p.shared.db();
        price::evm_token_price_usd_cached(p.evm, &p.shared.cfg, p.token_in_addr, amt_in, 50, db)
            .await?
            .usd
    };
    Ok((amt_in, usd_value))
}

/// Parameters for an EVM ERC-20 approval check.
struct EvmApprovalParams<'a> {
    evm: &'a EvmChain,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    chain: &'a str,
    token_in_addr: alloy::primitives::Address,
    from_addr: alloy::primitives::Address,
    spender: alloy::primitives::Address,
    amt_in: U256,
    outcome: &'a WriteConfirmOutcome,
    provider: &'a str,
}

/// Handle ERC-20 approval for an EVM swap if needed. Returns the approval tx hash if one was sent.
async fn handle_evm_approval<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    ap: EvmApprovalParams<'_>,
) -> eyre::Result<Result<Option<String>, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let allowance = ap
        .evm
        .erc20_allowance(ap.token_in_addr, ap.from_addr, ap.spender)
        .await?;
    if allowance >= ap.amt_in {
        return Ok(Ok(None));
    }
    let approve_tx =
        ap.evm
            .build_erc20_approve(ap.from_addr, ap.token_in_addr, ap.spender, ap.amt_in)?;
    if let Err(e) = ap.evm.simulate_tx_strict(&approve_tx).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": "swap_tokens", "wallet": ap.w.name, "account_index": ap.idx, "chain": ap.chain, "usd_value": 0.0_f64, "usd_value_known": false, "policy_decision": ap.outcome.policy_decision, "confirm_required": ap.outcome.confirm_required, "confirm_result": ap.outcome.confirm_result, "daily_used_usd": ap.outcome.daily_used_usd, "forced_confirm": ap.outcome.forced_confirm, "txid": null, "error_code": "simulation_failed", "result": "simulation_failed", "type": "approve", "provider": ap.provider }));
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, &format!("approve ({})", ap.provider)),
            )),
        )));
    }
    let wallet = load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, ap.w, ap.idx).await?;
    let tx_hash = ap.evm.send_tx(wallet.clone(), approve_tx).await?;
    let tx_hash_s = format!("{tx_hash:#x}");
    ctx.shared.ks.append_tx_history(&json!({ "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": "approve", "chain": ap.chain, "wallet": ap.w.name, "account_index": ap.idx, "provider": ap.provider, "token": format!("{:#x}", ap.token_in_addr), "spender": format!("{:#x}", ap.spender), "amount_base": ap.amt_in.to_string(), "usd_value": 0.0_f64, "tx_hash": tx_hash_s }))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": "swap_tokens", "wallet": ap.w.name, "account_index": ap.idx, "chain": ap.chain, "usd_value": 0.0_f64, "usd_value_known": false, "policy_decision": ap.outcome.policy_decision, "confirm_required": ap.outcome.confirm_required, "confirm_result": ap.outcome.confirm_result, "daily_used_usd": ap.outcome.daily_used_usd, "forced_confirm": ap.outcome.forced_confirm, "txid": tx_hash_s, "error_code": null, "result": "broadcasted", "tx_hash": tx_hash_s, "type": "approve", "provider": ap.provider }));
    if !wait_for_allowance(
        ap.evm,
        ap.token_in_addr,
        ap.from_addr,
        ap.spender,
        ap.amt_in,
    )
    .await
    {
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "approval_pending",
                "approval submitted but not yet confirmed; retry the swap shortly",
            )),
        )));
    }
    Ok(Ok(Some(tx_hash_s)))
}

/// Bundled data for recording a completed EVM swap.
struct EvmSwapResult<'a> {
    shared: &'a SharedState,
    lock: std::fs::File,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    chain: &'a str,
    provider: &'a str,
    token_in: &'a str,
    token_out: &'a str,
    amt_in: U256,
    out: U256,
    min_out: Option<U256>,
    slippage_bps: u32,
    usd_value: f64,
    tx_hash: alloy::primitives::B256,
    outcome: &'a WriteConfirmOutcome,
    req_id: &'a serde_json::Value,
}

/// Record an EVM swap to history + audit log and build the success response.
fn record_evm_swap_and_respond(r: EvmSwapResult<'_>) -> eyre::Result<JsonRpcResponse> {
    let tx_hash_s = format!("{:#x}", r.tx_hash);
    let mut hist = json!({ "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": "swap", "chain": r.chain, "wallet": r.w.name, "account_index": r.idx, "provider": r.provider, "token_in": r.token_in, "token_out": r.token_out, "amount_in_base": r.amt_in.to_string(), "expected_out_base": r.out.to_string(), "slippage_bps": r.slippage_bps, "usd_value": r.usd_value, "tx_hash": tx_hash_s });
    if let Some(mo) = r.min_out {
        if let Some(obj) = hist.as_object_mut() {
            obj.insert("min_out_base".to_owned(), json!(mo.to_string()));
        }
    }
    r.shared.ks.append_tx_history(&hist)?;
    let _audit_log = r.shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": "swap_tokens", "wallet": r.w.name, "account_index": r.idx, "chain": r.chain, "usd_value": r.usd_value, "usd_value_known": true, "policy_decision": r.outcome.policy_decision, "confirm_required": r.outcome.confirm_required, "confirm_result": r.outcome.confirm_result, "daily_used_usd": r.outcome.daily_used_usd, "forced_confirm": r.outcome.forced_confirm, "txid": tx_hash_s, "error_code": null, "result": "broadcasted", "tx_hash": tx_hash_s, "provider": r.provider }));
    Keystore::release_lock(r.lock)?;
    let mut resp = json!({ "chain": r.chain, "provider": r.provider, "tx_hash": tx_hash_s, "usd_value": r.usd_value, "expected_out_base": r.out.to_string() });
    if let Some(mo) = r.min_out {
        if let Some(obj) = resp.as_object_mut() {
            obj.insert("min_out_base".to_owned(), json!(mo.to_string()));
        }
    }
    Ok(ok(r.req_id.clone(), tool_ok(resp)))
}

async fn handle_evm_uniswap_swap<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: EvmSwapParams<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let EvmSwapParams {
        w,
        idx,
        lock,
        chain,
        token_in,
        token_out,
        amount_in_s,
        units,
        slippage_bps,
        evm,
        from_addr,
        ..
    } = p;
    let u = evm
        .uniswap
        .as_ref()
        .context("uniswap addresses not configured")?;
    let (native_in, token_in_addr) = if is_native_token(token_in) {
        (true, u.wrapped_native)
    } else {
        (false, EvmChain::parse_address(token_in)?)
    };
    let (native_out, token_out_addr) = if is_native_token(token_out) {
        (true, u.wrapped_native)
    } else {
        (false, EvmChain::parse_address(token_out)?)
    };
    let router02 = u.router02;
    let usdc_addr = u.usdc;
    let (amt_in, usd_value) = resolve_evm_amount_and_price(EvmAmountPriceInput {
        shared: ctx.shared,
        evm: &evm,
        chain,
        amount_in_s,
        units,
        native_in,
        token_in_addr,
        usdc_addr,
    })
    .await?;
    let router_s = format!("{router02:#x}");
    let summary = format!("SWAP on {chain} via Uniswap: {token_in} -> {token_out}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "swap_tokens",
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: WriteOp::Swap,
            chain,
            usd_value,
            usd_value_known: usd_value.is_finite(),
            force_confirm: false,
            slippage_bps: Some(slippage_bps),
            to_address: None,
            contract: Some(&router_s),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    let (out, fee) = find_best_uniswap_quote(&evm, token_in_addr, token_out_addr, amt_in).await?;
    let min_out = compute_min_out(out, slippage_bps)?;
    if !native_in {
        match handle_evm_approval(
            ctx,
            EvmApprovalParams {
                evm: &evm,
                w,
                idx,
                chain,
                token_in_addr,
                from_addr,
                spender: router02,
                amt_in,
                outcome: &outcome,
                provider: "uniswap",
            },
        )
        .await?
        {
            Ok(_) => {}
            Err(resp) => {
                Keystore::release_lock(lock)?;
                return Ok(resp);
            }
        }
    }

    let swap_req = crate::chains::evm::UniswapSwapRequest {
        from: from_addr,
        token_in: token_in_addr,
        token_out: token_out_addr,
        amount_in: amt_in,
        amount_out_min: min_out,
        fee,
        native_in,
        native_out,
    };
    let swap_tx = evm.build_uniswap_swap_tx(&swap_req)?;
    if let Err(e) = evm.simulate_tx_strict(&swap_tx).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": "swap_tokens", "wallet": w.name, "account_index": idx, "chain": chain, "usd_value": usd_value, "usd_value_known": true, "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm, "txid": null, "error_code": "simulation_failed", "result": "simulation_failed", "type": "swap", "provider": "uniswap" }));
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "swap (uniswap)"),
            )),
        ));
    }
    let wallet = load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    let tx_hash = evm.send_tx(wallet, swap_tx).await?;
    record_evm_swap_and_respond(EvmSwapResult {
        shared: ctx.shared,
        lock,
        w,
        idx,
        chain,
        provider: "uniswap",
        token_in,
        token_out,
        amt_in,
        out,
        min_out: Some(min_out),
        slippage_bps,
        usd_value,
        tx_hash,
        outcome: &outcome,
        req_id: &ctx.req_id,
    })
}

/// Find the best Uniswap fee tier quote.
async fn find_best_uniswap_quote(
    evm: &EvmChain,
    token_in_addr: alloy::primitives::Address,
    token_out_addr: alloy::primitives::Address,
    amt_in: U256,
) -> eyre::Result<(U256, u32)> {
    let fees = [500_u32, 3000_u32, 10_000_u32];
    let mut best = None;
    for fee in fees {
        if let Ok(out) = evm
            .quote_uniswap_exact_in(token_in_addr, token_out_addr, amt_in, fee)
            .await
        {
            if best.map_or(true, |(b, _)| out > b) {
                best = Some((out, fee));
            }
        }
    }
    best.ok_or_else(|| eyre::eyre!("no uniswap quote"))
}

/// Compute minimum output after slippage.
fn compute_min_out(out: U256, slippage_bps: u32) -> eyre::Result<U256> {
    let min = out
        .checked_mul(U256::from(10_000_u64 - u64::from(slippage_bps)))
        .ok_or_else(|| eyre::eyre!("overflow"))?
        / U256::from(10_000_u64);
    Ok(min)
}

async fn handle_evm_oneinch_swap<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: EvmSwapParams<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let EvmSwapParams {
        w,
        idx,
        lock,
        effective_policy,
        chain,
        token_in,
        token_out,
        amount_in_s,
        units,
        slippage_bps,
        evm,
        from_addr,
    } = p;
    if ctx
        .shared
        .cfg
        .http
        .oneinch_api_key
        .as_ref()
        .map_or(true, |k| k.trim().is_empty())
    {
        Keystore::release_lock(lock)?;
        return Ok(ok(ctx.req_id.clone(), tool_err(ToolError::new("missing_api_key", "1inch is optional and requires an API key (set http.oneinch_api_key in config.toml)"))));
    }
    let u = evm
        .uniswap
        .as_ref()
        .context("uniswap addresses not configured")?;
    let usdc_addr = u.usdc;
    let native_sentinel = EvmChain::parse_address("0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE")?;
    let (native_in, token_in_addr) = if is_native_token(token_in) {
        (true, native_sentinel)
    } else {
        (false, EvmChain::parse_address(token_in)?)
    };
    let (_native_out, token_out_addr) = if is_native_token(token_out) {
        (true, native_sentinel)
    } else {
        (false, EvmChain::parse_address(token_out)?)
    };
    let (amt_in, usd_value) = resolve_evm_amount_and_price(EvmAmountPriceInput {
        shared: ctx.shared,
        evm: &evm,
        chain,
        amount_in_s,
        units,
        native_in,
        token_in_addr,
        usdc_addr,
    })
    .await?;

    let (swap_tx, expected_out) = match oneinch_get_swap_tx(
        &evm,
        from_addr,
        token_in_addr,
        token_out_addr,
        amt_in,
        slippage_bps,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(resp)));
        }
    };
    let contract_addr = extract_tx_to_address(&swap_tx)?;
    let router_s = format!("{contract_addr:#x}");
    let summary = format!("SWAP on {chain} via 1inch: {token_in} -> {token_out}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "swap_tokens",
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: WriteOp::Swap,
            chain,
            usd_value,
            usd_value_known: usd_value.is_finite(),
            force_confirm: effective_policy.require_user_confirm_for_remote_tx.get(),
            slippage_bps: Some(slippage_bps),
            to_address: None,
            contract: Some(&router_s),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };
    if !native_in {
        let spender = evm.oneinch_spender().await?;
        match handle_evm_approval(
            ctx,
            EvmApprovalParams {
                evm: &evm,
                w,
                idx,
                chain,
                token_in_addr,
                from_addr,
                spender,
                amt_in,
                outcome: &outcome,
                provider: "1inch",
            },
        )
        .await?
        {
            Ok(_) => {}
            Err(resp) => {
                Keystore::release_lock(lock)?;
                return Ok(resp);
            }
        }
    }
    if let Err(e) = evm.simulate_tx_strict(&swap_tx).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": "swap_tokens", "wallet": w.name, "account_index": idx, "chain": chain, "usd_value": usd_value, "usd_value_known": true, "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm, "txid": null, "error_code": "simulation_failed", "result": "simulation_failed", "type": "swap", "provider": "1inch" }));
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "swap (1inch)"),
            )),
        ));
    }
    let wallet = load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    let tx_hash = evm.send_tx(wallet, swap_tx).await?;
    record_evm_swap_and_respond(EvmSwapResult {
        shared: ctx.shared,
        lock,
        w,
        idx,
        chain,
        provider: "1inch",
        token_in,
        token_out,
        amt_in,
        out: expected_out,
        min_out: None,
        slippage_bps,
        usd_value,
        tx_hash,
        outcome: &outcome,
        req_id: &ctx.req_id,
    })
}

/// Fetch the 1inch swap transaction, returning an error response on failure.
async fn oneinch_get_swap_tx(
    evm: &EvmChain,
    from_addr: alloy::primitives::Address,
    token_in_addr: alloy::primitives::Address,
    token_out_addr: alloy::primitives::Address,
    amt_in: U256,
    slippage_bps: u32,
) -> Result<(alloy::rpc::types::TransactionRequest, U256), ToolError> {
    evm.oneinch_swap_tx(
        from_addr,
        token_in_addr,
        token_out_addr,
        amt_in,
        slippage_bps,
    )
    .await
    .map_err(|e| ToolError::new("oneinch_error", format!("{e:#}")))
}

/// Extract the `to` address from a transaction request.
fn extract_tx_to_address(
    tx: &alloy::rpc::types::TransactionRequest,
) -> eyre::Result<alloy::primitives::Address> {
    crate::chains::evm::extract_tx_to_address(tx)
}

/// Parsed swap arguments from the incoming request.
struct SwapArgs {
    chain: String,
    token_in: String,
    token_out: String,
    amount_in_s: String,
    units: String,
    slippage_bps: u32,
    provider_raw: String,
}

/// Parse and validate swap arguments from the request args.
fn parse_swap_args(args: &serde_json::Value) -> SwapArgs {
    let chain = args
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let token_in = args
        .get("token_in")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let token_out = args
        .get("token_out")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let amount_in_s = args
        .get("amount_in")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let units = args
        .get("amount_units")
        .and_then(|v| v.as_str())
        .unwrap_or("ui")
        .to_owned();
    let slippage_bps = args
        .get("slippage_bps")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(100);
    let provider_raw = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_owned();
    SwapArgs {
        chain,
        token_in,
        token_out,
        amount_in_s,
        units,
        slippage_bps,
        provider_raw,
    }
}

/// Set up an EVM chain instance from config.
fn setup_evm_chain(shared: &SharedState, chain: &str) -> eyre::Result<EvmChain> {
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

pub async fn handle_ctx<R, W>(ctx: &mut HandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(ctx.shared, &ctx.args)?;
    let (effective_policy, _) = ctx.shared.cfg.policy_for_wallet(Some(w.name.as_str()));
    let a = parse_swap_args(&ctx.args);

    if a.chain.is_empty()
        || a.token_in.is_empty()
        || a.token_out.is_empty()
        || a.amount_in_s.is_empty()
    {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing chain/token_in/token_out/amount_in",
            )),
        ));
    }

    let provider = if a.provider_raw == "auto" {
        if a.chain == "solana" {
            "jupiter"
        } else {
            "uniswap"
        }
    } else {
        a.provider_raw.as_str()
    };

    if a.chain == "solana" {
        return handle_solana_branch(ctx, lock, &w, idx, &effective_policy, &a, provider).await;
    }

    handle_evm_branch(ctx, lock, &w, idx, &effective_policy, &a, provider).await
}

/// Handle the Solana swap branch of `handle_ctx`.
async fn handle_solana_branch<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    lock: std::fs::File,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    effective_policy: &crate::policy::Policy,
    a: &SwapArgs,
    provider: &str,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if provider != "jupiter" {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_provider",
                "solana swaps must use provider=jupiter",
            )),
        ));
    }
    handle_solana_swap(
        ctx,
        SolanaSwapParams {
            w,
            idx,
            lock,
            effective_policy,
            token_in: a.token_in.as_str(),
            token_out: a.token_out.as_str(),
            amount_in_s: a.amount_in_s.as_str(),
            units: a.units.as_str(),
            slippage_bps: a.slippage_bps,
        },
    )
    .await
}

/// Handle the EVM swap branch of `handle_ctx`.
async fn handle_evm_branch<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    lock: std::fs::File,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    effective_policy: &crate::policy::Policy,
    a: &SwapArgs,
    provider: &str,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let evm = setup_evm_chain(ctx.shared, &a.chain)?;
    if evm.uniswap.is_none() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "uniswap_unavailable",
                "uniswap addresses not configured for this chain",
            )),
        ));
    }
    let from_addr = evm_addr_for_account(w, idx)?;

    if provider == "uniswap" {
        return handle_evm_uniswap_swap(
            ctx,
            EvmSwapParams {
                w,
                idx,
                lock,
                effective_policy,
                chain: &a.chain,
                token_in: &a.token_in,
                token_out: &a.token_out,
                amount_in_s: &a.amount_in_s,
                units: &a.units,
                slippage_bps: a.slippage_bps,
                evm,
                from_addr,
            },
        )
        .await;
    }

    if provider == "1inch" {
        return handle_evm_oneinch_swap(
            ctx,
            EvmSwapParams {
                w,
                idx,
                lock,
                effective_policy,
                chain: &a.chain,
                token_in: &a.token_in,
                token_out: &a.token_out,
                amount_in_s: &a.amount_in_s,
                units: &a.units,
                slippage_bps: a.slippage_bps,
                evm,
                from_addr,
            },
        )
        .await;
    }

    Keystore::release_lock(lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_err(ToolError::new(
            "invalid_provider",
            "evm swaps must use provider=uniswap or provider=1inch",
        )),
    ))
}
