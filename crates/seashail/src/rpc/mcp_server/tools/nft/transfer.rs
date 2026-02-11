use eyre::Context as _;
use serde_json::{json, Value};
use tokio::io::BufReader;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, resolve_wallet_and_account, solana_fallback_urls,
};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::common::summarize_sim_error;
use crate::chains::{evm::EvmChain, solana::SolanaChain};
use crate::errors::ToolError;
use crate::keystore::{utc_now_iso, Keystore};
use crate::policy_engine::WriteOp;

const USD_ZERO: f64 = 0.0_f64;

pub async fn handle<R, W>(
    req_id: Value,
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
    let lock = shared.ks.acquire_write_lock()?;
    let res = handle_locked(req_id, args, shared, conn, stdin, stdout).await;
    let release_res = Keystore::release_lock(lock);

    match (res, release_res) {
        (Ok(v), Ok(())) => Ok(v),
        (Ok(_), Err(e)) | (Err(e), Ok(())) => Err(e),
        (Err(e), Err(re)) => Err(e.wrap_err(format!("failed to release keystore lock: {re:#}"))),
    }
}

struct TransferCtx<'a, R, W> {
    shared: &'a mut SharedState,
    conn: &'a mut ConnState,
    stdin: &'a mut tokio::io::Lines<BufReader<R>>,
    stdout: &'a mut W,
    wallet: &'a crate::wallet::WalletRecord,
    account_index: u32,
}

async fn handle_locked<R, W>(
    req_id: Value,
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
    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let chain = args.get("chain").and_then(Value::as_str).unwrap_or("");
    let to = args.get("to").and_then(Value::as_str).unwrap_or("");
    if chain.is_empty() || to.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing chain/to")),
        ));
    }

    let mut ctx = TransferCtx {
        shared,
        conn,
        stdin,
        stdout,
        wallet: &w,
        account_index: idx,
    };
    if chain == "solana" {
        return handle_solana(req_id, &args, &mut ctx, to).await;
    }

    handle_evm(req_id, &args, &mut ctx, chain, to).await
}

async fn handle_solana<R, W>(
    req_id: Value,
    args: &Value,
    ctx: &mut TransferCtx<'_, R, W>,
    to: &str,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mint = args.get("mint").and_then(Value::as_str).unwrap_or("");
    if mint.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing mint")),
        ));
    }

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

    let to_pk = SolanaChain::parse_pubkey(to)?;
    let mint_pk = SolanaChain::parse_pubkey(mint)?;

    if ctx.shared.scam_blocklist_contains_solana(to_pk).await
        || ctx.shared.scam_blocklist_contains_solana(mint_pk).await
    {
        let _audit_log = ctx.shared.ks.append_audit_log(&audit_blocked_scam_solana(
            &ctx.wallet.name,
            ctx.account_index,
            to,
            mint,
        ));
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient/mint is blocked by the scam address blocklist",
            )),
        ));
    }

    let summary = format!("TRANSFER NFT on Solana mint {mint} to {to}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "transfer_nft",
            wallet: Some(ctx.wallet.name.as_str()),
            account_index: Some(ctx.account_index),
            op: WriteOp::TransferNft,
            chain: "solana",
            usd_value: USD_ZERO,
            usd_value_known: false,
            force_confirm: false,
            slippage_bps: None,
            to_address: Some(to),
            contract: Some(mint),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => return Ok(ok(req_id, tool_err(te))),
    };

    let kp = super::super::key_loading::load_solana_keypair(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        ctx.wallet,
        ctx.account_index,
    )
    .await?;
    let sig = sol.send_spl(&kp, to_pk, mint_pk, 1).await?;

    ctx.shared.ks.append_tx_history(&tx_history_solana(
        &ctx.wallet.name,
        ctx.account_index,
        mint,
        to,
        &sig,
    ))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&audit_broadcasted_solana(
        &ctx.wallet.name,
        ctx.account_index,
        mint,
        to,
        &sig,
        &outcome,
    ));

    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": "solana", "signature": sig.to_string() })),
    ))
}

async fn handle_evm<R, W>(
    req_id: Value,
    args: &Value,
    ctx: &mut TransferCtx<'_, R, W>,
    chain: &str,
    to: &str,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    // EVM (ERC-721 safeTransferFrom)
    let contract = args.get("contract").and_then(Value::as_str).unwrap_or("");
    let token_id_s = args.get("token_id").and_then(Value::as_str).unwrap_or("");
    if contract.is_empty() || token_id_s.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "missing contract/token_id for EVM NFT transfer",
            )),
        ));
    }

    let evm = evm_chain_for(ctx.shared, chain)?;
    let from = evm_addr_for_account(ctx.wallet, ctx.account_index)?;
    let to_addr = EvmChain::parse_address(to)?;
    let contract_addr = EvmChain::parse_address(contract)?;
    let token_id = crate::chains::evm::parse_u256_dec(token_id_s).context("parse token_id")?;

    if let Some(resp) = evm_blocklist_guard(
        req_id.clone(),
        ctx,
        chain,
        to,
        contract,
        to_addr,
        contract_addr,
    )
    .await?
    {
        return Ok(resp);
    }

    let outcome =
        match confirm_transfer_nft_evm(&req_id, ctx, chain, to, contract, token_id_s).await? {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

    let tx = EvmChain::build_erc721_safe_transfer_from(from, contract_addr, to_addr, token_id);
    if let Err(e) = evm.simulate_tx_strict(&tx).await {
        let _audit_log = ctx.shared.ks.append_audit_log(&audit_simulation_failed_evm(
            &ctx.wallet.name,
            ctx.account_index,
            chain,
            &outcome,
        ));
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, "transfer_nft"),
            )),
        ));
    }

    let wallet = super::super::key_loading::load_evm_signer(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        ctx.wallet,
        ctx.account_index,
    )
    .await?;
    let tx_hash = evm.send_tx(wallet, tx).await?;

    ctx.shared.ks.append_tx_history(&tx_history_evm(
        &ctx.wallet.name,
        ctx.account_index,
        chain,
        contract,
        token_id_s,
        to,
        tx_hash,
    ))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&audit_broadcasted_evm(
        &ctx.wallet.name,
        ctx.account_index,
        chain,
        tx_hash,
        &outcome,
    ));

    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": chain, "tx_hash": format!("{tx_hash:#x}") })),
    ))
}

fn evm_chain_for(shared: &SharedState, chain: &str) -> eyre::Result<EvmChain> {
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

async fn evm_blocklist_guard<R, W>(
    req_id: Value,
    ctx: &mut TransferCtx<'_, R, W>,
    chain: &str,
    to: &str,
    contract: &str,
    to_addr: alloy::primitives::Address,
    contract_addr: alloy::primitives::Address,
) -> eyre::Result<Option<JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if ctx.shared.scam_blocklist_contains_evm(to_addr).await
        || ctx.shared.scam_blocklist_contains_evm(contract_addr).await
    {
        let _audit_log = ctx.shared.ks.append_audit_log(&audit_blocked_scam_evm(
            &ctx.wallet.name,
            ctx.account_index,
            chain,
            to,
            contract,
        ));
        return Ok(Some(ok(
            req_id,
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient/contract is blocked by the scam address blocklist",
            )),
        )));
    }
    Ok(None)
}

async fn confirm_transfer_nft_evm<R, W>(
    req_id: &Value,
    ctx: &mut TransferCtx<'_, R, W>,
    chain: &str,
    to: &str,
    contract: &str,
    token_id_s: &str,
) -> eyre::Result<Result<super::super::policy_confirm::WriteConfirmOutcome, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let summary =
        format!("TRANSFER NFT on {chain} contract {contract} token_id {token_id_s} to {to}");
    match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: "transfer_nft",
            wallet: Some(ctx.wallet.name.as_str()),
            account_index: Some(ctx.account_index),
            op: WriteOp::TransferNft,
            chain,
            usd_value: USD_ZERO,
            usd_value_known: false,
            force_confirm: false,
            slippage_bps: None,
            to_address: Some(to),
            contract: Some(contract),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => Ok(Ok(v)),
        Err(te) => Ok(Err(ok(req_id.clone(), tool_err(te)))),
    }
}

fn audit_blocked_scam_solana(wallet: &str, account_index: u32, to: &str, mint: &str) -> Value {
    json!({
      "ts": utc_now_iso(),
      "tool": "transfer_nft",
      "wallet": wallet,
      "account_index": account_index,
      "chain": "solana",
      "usd_value": USD_ZERO,
      "usd_value_known": false,
      "policy_decision": null,
      "confirm_required": false,
      "confirm_result": null,
      "txid": null,
      "error_code": "scam_address_blocked",
      "result": "blocked_scam_blocklist",
      "to": to,
      "mint": mint
    })
}

fn audit_blocked_scam_evm(
    wallet: &str,
    account_index: u32,
    chain: &str,
    to: &str,
    contract: &str,
) -> Value {
    json!({
      "ts": utc_now_iso(),
      "tool": "transfer_nft",
      "wallet": wallet,
      "account_index": account_index,
      "chain": chain,
      "usd_value": USD_ZERO,
      "usd_value_known": false,
      "policy_decision": null,
      "confirm_required": false,
      "confirm_result": null,
      "txid": null,
      "error_code": "scam_address_blocked",
      "result": "blocked_scam_blocklist",
      "to": to,
      "contract": contract
    })
}

fn audit_simulation_failed_evm(
    wallet: &str,
    account_index: u32,
    chain: &str,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) -> Value {
    json!({
      "ts": utc_now_iso(),
      "tool": "transfer_nft",
      "wallet": wallet,
      "account_index": account_index,
      "chain": chain,
      "usd_value": USD_ZERO,
      "usd_value_known": false,
      "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result,
      "forced_confirm": outcome.forced_confirm,
      "daily_used_usd": outcome.daily_used_usd,
      "txid": null,
      "error_code": "simulation_failed",
      "result": "simulation_failed",
    })
}

fn audit_broadcasted_solana(
    wallet: &str,
    account_index: u32,
    mint: &str,
    to: &str,
    sig: &solana_sdk::signature::Signature,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) -> Value {
    json!({
      "ts": utc_now_iso(),
      "tool": "transfer_nft",
      "wallet": wallet,
      "account_index": account_index,
      "chain": "solana",
      "usd_value": USD_ZERO,
      "usd_value_known": false,
      "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result,
      "forced_confirm": outcome.forced_confirm,
      "daily_used_usd": outcome.daily_used_usd,
      "txid": sig.to_string(),
      "error_code": null,
      "result": "broadcasted",
      "signature": sig.to_string(),
      "to": to,
      "mint": mint
    })
}

fn audit_broadcasted_evm(
    wallet: &str,
    account_index: u32,
    chain: &str,
    tx_hash: alloy::primitives::B256,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) -> Value {
    json!({
      "ts": utc_now_iso(),
      "tool": "transfer_nft",
      "wallet": wallet,
      "account_index": account_index,
      "chain": chain,
      "usd_value": USD_ZERO,
      "usd_value_known": false,
      "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result,
      "forced_confirm": outcome.forced_confirm,
      "daily_used_usd": outcome.daily_used_usd,
      "txid": format!("{tx_hash:#x}"),
      "error_code": null,
      "result": "broadcasted",
      "tx_hash": format!("{tx_hash:#x}"),
    })
}

fn tx_history_solana(
    wallet: &str,
    account_index: u32,
    mint: &str,
    to: &str,
    sig: &solana_sdk::signature::Signature,
) -> Value {
    json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "nft_transfer",
      "chain": "solana",
      "wallet": wallet,
      "account_index": account_index,
      "mint": mint,
      "to": to,
      "amount_base": "1",
      "usd_value": USD_ZERO,
      "signature": sig.to_string()
    })
}

fn tx_history_evm(
    wallet: &str,
    account_index: u32,
    chain: &str,
    contract: &str,
    token_id_s: &str,
    to: &str,
    tx_hash: alloy::primitives::B256,
) -> Value {
    json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "nft_transfer",
      "chain": chain,
      "wallet": wallet,
      "account_index": account_index,
      "contract": contract,
      "token_id": token_id_s,
      "to": to,
      "usd_value": USD_ZERO,
      "tx_hash": format!("{tx_hash:#x}")
    })
}
