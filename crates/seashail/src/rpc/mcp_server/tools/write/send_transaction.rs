use crate::{
    amount,
    chains::{bitcoin as btc_chain, evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
    financial_math,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};
use alloy::{primitives::U256, rpc::types::TransactionRequest};
use eyre::Context as _;
use serde_json::json;
use std::str::FromStr as _;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::helpers::{
    evm_addr_for_account, is_native_token, resolve_wallet_and_account, solana_fallback_urls,
    u128_to_u256, u128_to_u64,
};
use super::super::key_loading::{load_bitcoin_privkey, load_evm_signer, load_solana_keypair};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::common::summarize_sim_error;
use super::HandlerCtx;

/// Parsed send transaction parameters common across chains.
struct SendParams<'a> {
    lock: std::fs::File,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    effective_policy: &'a crate::policy::Policy,
    chain: &'a str,
    to: &'a str,
    token: &'a str,
    amount: &'a str,
    units: &'a str,
}

/// Build, sign, and broadcast the Bitcoin transaction. Returns (txid, `fee_sats`, `from_addr`).
async fn bitcoin_build_sign_send<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    to: &str,
    amount_sats: u64,
) -> eyre::Result<(String, u64, String)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mode = effective_network_mode(ctx.shared, ctx.conn);
    let from_addr = if mode == crate::config::NetworkMode::Testnet {
        w.bitcoin_addresses_testnet
            .get(idx as usize)
            .cloned()
            .unwrap_or_default()
    } else {
        w.bitcoin_addresses_mainnet
            .get(idx as usize)
            .cloned()
            .unwrap_or_default()
    };
    if from_addr.is_empty() {
        eyre::bail!("wallet has no bitcoin address for this account");
    }

    let base = if mode == crate::config::NetworkMode::Testnet {
        ctx.shared.cfg.http.bitcoin_api_base_url_testnet.clone()
    } else {
        ctx.shared.cfg.http.bitcoin_api_base_url_mainnet.clone()
    };
    let network = if mode == crate::config::NetworkMode::Testnet {
        bitcoin::Network::Testnet
    } else {
        bitcoin::Network::Bitcoin
    };
    let btc =
        btc_chain::BitcoinChain::new(&base).map_err(|e| eyre::eyre!("bitcoin config: {e:#}"))?;

    let to_addr = bitcoin::Address::<bitcoin::address::NetworkUnchecked>::from_str(to)
        .context("parse bitcoin address")?
        .require_network(network)
        .context("bitcoin address network mismatch")?;

    let utxos = btc.list_utxos(&from_addr).await?;
    let fee_rate = btc.fee_rate_sats_per_vb().await?;

    let privkey = load_bitcoin_privkey(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let signed = btc_chain::build_and_sign_p2wpkh_send(
        &secp,
        network,
        &privkey,
        &to_addr,
        amount_sats,
        fee_rate,
        utxos,
    )?;
    let txid = btc.broadcast_tx_hex(&signed.tx_hex).await?;
    Ok((txid, signed.fee_sats, from_addr))
}

/// Parse bitcoin amount and compute its USD value.
async fn bitcoin_parse_amount_and_usd<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    amount: &str,
    units: &str,
) -> eyre::Result<(u64, f64)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let amount_sats = if units == "base" {
        u128_to_u64(amount::parse_amount_base_u128(amount)?)?
    } else {
        u128_to_u64(amount::parse_amount_ui_to_base_u128(amount, 8)?)?
    };
    let usd_value = {
        ctx.shared.ensure_db().await;
        let db = ctx.shared.db();
        let btc_price = price::native_token_price_usd_cached("bitcoin", &ctx.shared.cfg, db)
            .await?
            .usd;
        financial_math::token_base_to_usd(u128::from(amount_sats), 8, btc_price)
    };
    Ok((amount_sats, usd_value))
}

/// Check OFAC SDN blocklist for bitcoin. Returns an error response if blocked.
async fn bitcoin_check_ofac<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    to: &str,
    usd_value: f64,
    enable_ofac: bool,
) -> Option<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if enable_ofac && ctx.shared.ofac_sdn_contains_bitcoin(to).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": "send_transaction", "wallet": w.name,
          "account_index": idx, "chain": "bitcoin", "usd_value": usd_value,
          "usd_value_known": true, "policy_decision": null, "confirm_required": false,
          "confirm_result": null, "txid": null, "error_code": "ofac_sdn_blocked",
          "result": "blocked_ofac_sdn", "to": to,
        }));
        return Some(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        ));
    }
    None
}

/// Record tx history and audit log for a bitcoin send, then build the success response.
struct BitcoinSendRecord<'a> {
    ctx_req_id: &'a serde_json::Value,
    ks: &'a Keystore,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    to: &'a str,
    amount_sats: u64,
    usd_value: f64,
    txid: &'a str,
    fee_sats: u64,
    from_addr: &'a str,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

fn bitcoin_record_and_respond(r: &BitcoinSendRecord<'_>) -> eyre::Result<JsonRpcResponse> {
    r.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
      "type": "send", "chain": "bitcoin", "wallet": r.w.name,
      "account_index": r.idx, "token": "native",
      "amount_sats": r.amount_sats.to_string(), "usd_value": r.usd_value,
      "txid": r.txid, "fee_sats": r.fee_sats, "to": r.to,
    }))?;
    let _audit_log = r.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": "send_transaction", "wallet": r.w.name,
      "account_index": r.idx, "chain": "bitcoin", "usd_value": r.usd_value,
      "usd_value_known": true, "policy_decision": r.outcome.policy_decision,
      "confirm_required": r.outcome.confirm_required, "confirm_result": r.outcome.confirm_result,
      "daily_used_usd": r.outcome.daily_used_usd, "forced_confirm": r.outcome.forced_confirm,
      "txid": r.txid, "error_code": null, "result": "broadcasted", "to": r.to,
    }));
    Ok(ok(
        r.ctx_req_id.clone(),
        tool_ok(json!({
          "chain": "bitcoin", "txid": r.txid, "usd_value": r.usd_value,
          "fee_sats": r.fee_sats, "from": r.from_addr, "to": r.to
        })),
    ))
}

async fn handle_bitcoin_send<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: SendParams<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let SendParams {
        lock,
        w,
        idx,
        effective_policy,
        to,
        amount,
        units,
        ..
    } = p;
    let token = "native";
    if !is_native_token(token) {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "unsupported_token",
                "bitcoin only supports native BTC transfers (token must be native)",
            )),
        ));
    }

    let (amount_sats, usd_value) = bitcoin_parse_amount_and_usd(ctx, amount, units).await?;

    if let Some(blocked) = bitcoin_check_ofac(
        ctx,
        w,
        idx,
        to,
        usd_value,
        effective_policy.enable_ofac_sdn.get(),
    )
    .await
    {
        Keystore::release_lock(lock)?;
        return Ok(blocked);
    }

    let summary = format!("SEND BTC: {amount_sats} sats to {to}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "send_transaction",
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: WriteOp::Send,
            chain: "bitcoin",
            usd_value,
            usd_value_known: true,
            force_confirm: true,
            slippage_bps: None,
            to_address: Some(to),
            contract: None,
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

    let (txid, fee_sats, from_addr) = bitcoin_build_sign_send(ctx, w, idx, to, amount_sats).await?;
    let resp = bitcoin_record_and_respond(&BitcoinSendRecord {
        ctx_req_id: &ctx.req_id,
        ks: &ctx.shared.ks,
        w,
        idx,
        to,
        amount_sats,
        usd_value,
        txid: &txid,
        fee_sats,
        from_addr: &from_addr,
        outcome: &outcome,
    })?;
    Keystore::release_lock(lock)?;
    Ok(resp)
}

/// Check solana blocklists (OFAC SDN + scam). Returns an error response if blocked.
async fn solana_send_check_blocklists<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    to: &str,
    to_pk: solana_sdk::pubkey::Pubkey,
    usd_value: f64,
    enable_ofac: bool,
) -> Option<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if enable_ofac && ctx.shared.ofac_sdn_contains_solana(to_pk).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": "send_transaction", "wallet": w.name,
          "account_index": idx, "chain": "solana", "usd_value": usd_value,
          "usd_value_known": true, "policy_decision": null, "confirm_required": false,
          "confirm_result": null, "txid": null, "error_code": "ofac_sdn_blocked",
          "result": "blocked_ofac_sdn", "to": to,
        }));
        return Some(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        ));
    }
    if ctx.shared.scam_blocklist_contains_solana(to_pk).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": "send_transaction", "wallet": w.name,
          "account_index": idx, "chain": "solana", "usd_value": usd_value,
          "usd_value_known": true, "policy_decision": null, "confirm_required": false,
          "confirm_result": null, "txid": null, "error_code": "scam_address_blocked",
          "result": "blocked_scam_blocklist", "to": to,
        }));
        return Some(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient is blocked by the scam address blocklist",
            )),
        ));
    }
    None
}

/// Resolve USD value for a Solana send (native SOL or SPL token).
async fn solana_send_resolve_usd<R, W>(
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

/// Record tx history and audit log for a solana send, then build the success response.
struct SolanaSendRecord<'a> {
    ctx_req_id: &'a serde_json::Value,
    ks: &'a Keystore,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    to: &'a str,
    token: &'a str,
    amount_base: u64,
    usd_value: f64,
    sig: &'a solana_sdk::signature::Signature,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

fn solana_send_record_and_respond(r: &SolanaSendRecord<'_>) -> eyre::Result<JsonRpcResponse> {
    r.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
      "type": "send", "chain": "solana", "wallet": r.w.name,
      "account_index": r.idx, "to": r.to,
      "token": if is_native_token(r.token) { "native" } else { r.token },
      "amount_base": r.amount_base.to_string(), "usd_value": r.usd_value,
      "signature": r.sig.to_string()
    }))?;
    let _audit_log = r.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": "send_transaction", "wallet": r.w.name,
      "account_index": r.idx, "chain": "solana", "usd_value": r.usd_value,
      "usd_value_known": true, "policy_decision": r.outcome.policy_decision,
      "confirm_required": r.outcome.confirm_required, "confirm_result": r.outcome.confirm_result,
      "daily_used_usd": r.outcome.daily_used_usd, "forced_confirm": r.outcome.forced_confirm,
      "txid": r.sig.to_string(), "error_code": null, "result": "broadcasted",
      "signature": r.sig.to_string()
    }));
    Ok(ok(
        r.ctx_req_id.clone(),
        tool_ok(
            json!({ "chain": "solana", "signature": r.sig.to_string(), "usd_value": r.usd_value }),
        ),
    ))
}

async fn handle_solana_send<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    p: SendParams<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let SendParams {
        lock,
        w,
        idx,
        effective_policy,
        chain,
        to,
        token,
        amount,
        units,
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
    let mint_decimals = if is_native_token(token) {
        9_u8
    } else {
        let mint = SolanaChain::parse_pubkey(token)?;
        sol.get_mint_decimals(mint)
            .await
            .context("get spl mint decimals")?
    };
    let amount_base = if units == "base" {
        u128_to_u64(amount::parse_amount_base_u128(amount)?)?
    } else {
        u128_to_u64(amount::parse_amount_ui_to_base_u128(
            amount,
            u32::from(mint_decimals),
        )?)?
    };

    let usd_value = solana_send_resolve_usd(ctx, &sol, token, amount_base).await?;

    let to_pk = SolanaChain::parse_pubkey(to)?;
    if let Some(blocked) = solana_send_check_blocklists(
        ctx,
        w,
        idx,
        to,
        to_pk,
        usd_value,
        effective_policy.enable_ofac_sdn.get(),
    )
    .await
    {
        Keystore::release_lock(lock)?;
        return Ok(blocked);
    }

    let summary = format!("SEND on Solana to {to}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "send_transaction",
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: WriteOp::Send,
            chain,
            usd_value,
            usd_value_known: true,
            force_confirm: false,
            slippage_bps: None,
            to_address: Some(to),
            contract: None,
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

    let kp = load_solana_keypair(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    let sig = if is_native_token(token) {
        sol.send_sol(&kp, to_pk, amount_base).await?
    } else {
        let mint = SolanaChain::parse_pubkey(token)?;
        sol.send_spl(&kp, to_pk, mint, amount_base).await?
    };

    let resp = solana_send_record_and_respond(&SolanaSendRecord {
        ctx_req_id: &ctx.req_id,
        ks: &ctx.shared.ks,
        w,
        idx,
        to,
        token,
        amount_base,
        usd_value,
        sig: &sig,
        outcome: &outcome,
    })?;
    Keystore::release_lock(lock)?;
    Ok(resp)
}

/// EVM send parameters parsed and ready.
struct EvmSendParams {
    lock: std::fs::File,
    chain: String,
    to: String,
    token: String,
    amount: String,
    units: String,
}

/// Check EVM blocklists (OFAC SDN + scam). Returns an error response if blocked.
async fn evm_send_check_blocklists<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    chain: &str,
    to: &str,
    to_addr: alloy::primitives::Address,
    enable_ofac: bool,
) -> Option<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if enable_ofac && ctx.shared.ofac_sdn_contains_evm(to_addr).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": "send_transaction", "wallet": w.name,
          "account_index": idx, "chain": chain, "usd_value": 0.0_f64,
          "usd_value_known": false, "policy_decision": null, "confirm_required": false,
          "confirm_result": null, "txid": null, "error_code": "ofac_sdn_blocked",
          "result": "blocked_ofac_sdn", "to": to,
        }));
        return Some(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        ));
    }
    if ctx.shared.scam_blocklist_contains_evm(to_addr).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": "send_transaction", "wallet": w.name,
          "account_index": idx, "chain": chain, "usd_value": 0.0_f64,
          "usd_value_known": false, "policy_decision": null, "confirm_required": false,
          "confirm_result": null, "txid": null, "error_code": "scam_address_blocked",
          "result": "blocked_scam_blocklist", "to": to,
        }));
        return Some(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient is blocked by the scam address blocklist",
            )),
        ));
    }
    None
}

/// Build EVM transfer transaction and compute USD value.
struct SendTokenAmount<'a> {
    token: &'a str,
    amount: &'a str,
    units: &'a str,
}

async fn evm_build_send_tx<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    evm: &EvmChain,
    chain: &str,
    a: SendTokenAmount<'_>,
    from: alloy::primitives::Address,
    to_addr: alloy::primitives::Address,
) -> eyre::Result<(TransactionRequest, U256, f64, bool)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if is_native_token(a.token) {
        let amount_base = if a.units == "base" {
            crate::chains::evm::parse_u256_dec(a.amount)?
        } else {
            u128_to_u256(amount::parse_amount_ui_to_base_u128(a.amount, 18)?)
        };
        let usd = {
            ctx.shared.ensure_db().await;
            let db = ctx.shared.db();
            price::native_token_price_usd_cached(chain, &ctx.shared.cfg, db)
                .await?
                .usd
        };
        let usd_value = financial_math::token_base_to_usd(
            crate::chains::evm::u256_low_u128(amount_base),
            18,
            usd,
        );
        let tx = EvmChain::build_native_transfer(from, to_addr, amount_base);
        Ok((tx, amount_base, usd_value, true))
    } else {
        let token_addr = EvmChain::parse_address(a.token)?;
        let (decimals, _sym) = evm.get_erc20_metadata(token_addr).await?;
        let amount_base = if a.units == "base" {
            crate::chains::evm::parse_u256_dec(a.amount)?
        } else {
            u128_to_u256(amount::parse_amount_ui_to_base_u128(
                a.amount,
                u32::from(decimals),
            )?)
        };
        let (mut usd_value, mut usd_known) = (0.0_f64, false);
        if let Some(u) = &evm.uniswap {
            if token_addr == u.usdc {
                usd_value = financial_math::token_base_to_usd(
                    crate::chains::evm::u256_low_u128(amount_base),
                    6,
                    1.0,
                );
            } else {
                usd_value = {
                    ctx.shared.ensure_db().await;
                    let db = ctx.shared.db();
                    price::evm_token_price_usd_cached(
                        evm,
                        &ctx.shared.cfg,
                        token_addr,
                        amount_base,
                        50,
                        db,
                    )
                    .await?
                    .usd
                };
            }
            usd_known = true;
        }
        let tx = evm.build_erc20_transfer(from, token_addr, to_addr, amount_base)?;
        Ok((tx, amount_base, usd_value, usd_known))
    }
}

/// Record tx history and audit log for an EVM send, then build the success response.
struct EvmSendRecord<'a> {
    ctx_req_id: &'a serde_json::Value,
    ks: &'a Keystore,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    chain: &'a str,
    to: &'a str,
    token: &'a str,
    amount_base: U256,
    usd_value: f64,
    usd_known: bool,
    tx_hash: alloy::primitives::B256,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

fn evm_send_record_and_respond(r: &EvmSendRecord<'_>) -> eyre::Result<JsonRpcResponse> {
    r.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": "send",
      "chain": r.chain, "wallet": r.w.name, "account_index": r.idx, "to": r.to,
      "token": if is_native_token(r.token) { "native" } else { r.token },
      "amount_base": r.amount_base.to_string(), "usd_value": r.usd_value,
      "tx_hash": format!("{:#x}", r.tx_hash)
    }))?;
    let _audit_log = r.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": "send_transaction", "wallet": r.w.name,
      "account_index": r.idx, "chain": r.chain, "usd_value": r.usd_value,
      "usd_value_known": r.usd_known, "policy_decision": r.outcome.policy_decision,
      "confirm_required": r.outcome.confirm_required, "confirm_result": r.outcome.confirm_result,
      "daily_used_usd": r.outcome.daily_used_usd, "forced_confirm": r.outcome.forced_confirm,
      "txid": format!("{:#x}", r.tx_hash), "error_code": null, "result": "broadcasted",
      "tx_hash": format!("{:#x}", r.tx_hash)
    }));
    Ok(ok(
        r.ctx_req_id.clone(),
        tool_ok(
            json!({ "chain": r.chain, "tx_hash": format!("{:#x}", r.tx_hash), "usd_value": r.usd_value }),
        ),
    ))
}

/// Handle EVM send transaction logic (blocklists, build tx, simulate, sign, broadcast).
async fn handle_evm_send<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    effective_policy: &crate::policy::Policy,
    p: EvmSendParams,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let EvmSendParams {
        lock,
        chain,
        to,
        token,
        amount,
        units,
    } = p;
    let rpc_url = ctx
        .shared
        .cfg
        .rpc
        .evm_rpc_urls
        .get(chain.as_str())
        .ok_or_else(|| eyre::eyre!("unknown evm chain: {chain}"))?
        .clone();
    let chain_id = *ctx
        .shared
        .cfg
        .rpc
        .evm_chain_ids
        .get(chain.as_str())
        .ok_or_else(|| eyre::eyre!("missing evm chain id: {chain}"))?;
    let mut evm = EvmChain::for_name(&chain, chain_id, &rpc_url, &ctx.shared.cfg.http);
    if let Some(fb) = ctx.shared.cfg.rpc.evm_fallback_rpc_urls.get(chain.as_str()) {
        evm.fallback_rpc_urls.clone_from(fb);
    }
    let from = evm_addr_for_account(w, idx)?;
    let to_addr = EvmChain::parse_address(&to)?;

    if let Some(blocked) = evm_send_check_blocklists(
        ctx,
        w,
        idx,
        &chain,
        &to,
        to_addr,
        effective_policy.enable_ofac_sdn.get(),
    )
    .await
    {
        Keystore::release_lock(lock)?;
        return Ok(blocked);
    }

    let (tx, amount_base, usd_value, usd_known) = evm_build_send_tx(
        ctx,
        &evm,
        &chain,
        SendTokenAmount {
            token: token.as_str(),
            amount: amount.as_str(),
            units: units.as_str(),
        },
        from,
        to_addr,
    )
    .await?;

    let summary = format!("SEND on {chain} to {to}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "send_transaction",
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op: WriteOp::Send,
            chain: &chain,
            usd_value,
            usd_value_known: usd_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: Some(&to),
            contract: None,
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

    if let Err(e) = evm.simulate_tx_strict(&tx).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(), "tool": "send_transaction", "wallet": w.name,
          "account_index": idx, "chain": chain, "usd_value": usd_value,
          "usd_value_known": usd_known, "policy_decision": outcome.policy_decision,
          "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result,
          "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm,
          "txid": null, "error_code": "simulation_failed", "result": "simulation_failed",
        }));
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "send_transaction"),
            )),
        ));
    }

    let wallet = load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    let tx_hash = evm.send_tx(wallet, tx).await?;
    let resp = evm_send_record_and_respond(&EvmSendRecord {
        ctx_req_id: &ctx.req_id,
        ks: &ctx.shared.ks,
        w,
        idx,
        chain: &chain,
        to: &to,
        token: &token,
        amount_base,
        usd_value,
        usd_known,
        tx_hash,
        outcome: &outcome,
    })?;
    Keystore::release_lock(lock)?;
    Ok(resp)
}

pub async fn handle_ctx<R, W>(ctx: &mut HandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(ctx.shared, &ctx.args)?;
    let (effective_policy, _) = ctx.shared.cfg.policy_for_wallet(Some(w.name.as_str()));
    let chain = ctx
        .args
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let to = ctx
        .args
        .get("to")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let token = ctx
        .args
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("native")
        .to_owned();
    let amount = ctx
        .args
        .get("amount")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let units = ctx
        .args
        .get("amount_units")
        .and_then(|v| v.as_str())
        .unwrap_or("ui")
        .to_owned();

    if chain.is_empty() || to.is_empty() || amount.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "missing chain/to/amount")),
        ));
    }

    if chain == "bitcoin" {
        return handle_bitcoin_send(
            ctx,
            SendParams {
                lock,
                w: &w,
                idx,
                effective_policy: &effective_policy,
                chain: &chain,
                to: &to,
                token: &token,
                amount: &amount,
                units: &units,
            },
        )
        .await;
    }
    if chain == "solana" {
        return handle_solana_send(
            ctx,
            SendParams {
                lock,
                w: &w,
                idx,
                effective_policy: &effective_policy,
                chain: &chain,
                to: &to,
                token: &token,
                amount: &amount,
                units: &units,
            },
        )
        .await;
    }

    // EVM
    handle_evm_send(
        ctx,
        &w,
        idx,
        &effective_policy,
        EvmSendParams {
            lock,
            chain,
            to,
            token,
            amount,
            units,
        },
    )
    .await
}
