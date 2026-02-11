use alloy::network::TransactionBuilder as _;
use base64::Engine as _;
use eyre::Context as _;
use serde_json::{json, Value};
use tokio::io::BufReader;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, resolve_wallet_and_account, sol_pubkey_for_account, solana_fallback_urls,
};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::common::{
    get_asset_obj, get_str_in_args_or_asset, parse_usd_value, summarize_sim_error,
};
use crate::chains::{evm::EvmChain, solana::SolanaChain};
use crate::errors::ToolError;
use crate::keystore::{utc_now_iso, Keystore};
use crate::policy_engine::WriteOp;

/// Classify an adapter error message into a user-facing error code and message.
fn classify_adapter_error(err_msg: &str) -> (&'static str, &str) {
    if err_msg.starts_with("missing_api_key:") {
        (
            "missing_api_key",
            err_msg.trim_start_matches("missing_api_key:").trim(),
        )
    } else if err_msg.contains("adapter not configured") {
        (
            "marketplace_unavailable",
            "marketplace adapter not configured",
        )
    } else {
        ("marketplace_error", "marketplace adapter request failed")
    }
}

/// Map a tool name to its NFT history type string.
fn nft_history_type(tool_name: &str) -> &'static str {
    match tool_name {
        "buy_nft" => "nft_buy",
        "sell_nft" => "nft_sell",
        "bid_nft" => "nft_bid",
        _ => "nft",
    }
}

/// Extract `allowed_program_ids` from args or asset, preferring adapter-provided ones.
fn collect_allowed_programs(args: &Value, adapter_ids: Vec<String>) -> Vec<String> {
    if !adapter_ids.is_empty() {
        return adapter_ids;
    }
    let extract = |val: &Value| -> Vec<String> {
        val.get("allowed_program_ids")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|elem| elem.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut ids = extract(args);
    if ids.is_empty() {
        if let Some(asset) = get_asset_obj(args) {
            ids = extract(asset);
        }
    }
    ids
}

/// Common fields for NFT trade audit logging and recording.
struct NftRecordCtx<'a> {
    tool_name: &'a str,
    w_name: &'a str,
    idx: u32,
    chain: &'a str,
    marketplace: &'a str,
    usd_value: f64,
    usd_value_known: bool,
}

/// Record tx history and audit log for a completed Solana NFT trade, then respond.
fn solana_nft_record_and_respond(
    shared: &SharedState,
    lock: std::fs::File,
    r: &NftRecordCtx<'_>,
    sig: &solana_sdk::signature::Signature,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
    req_id: Value,
) -> eyre::Result<JsonRpcResponse> {
    let ty = nft_history_type(r.tool_name);
    shared.ks.append_tx_history(&json!({
        "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": ty,
        "chain": "solana", "wallet": r.w_name, "account_index": r.idx,
        "marketplace": r.marketplace, "usd_value": r.usd_value,
        "usd_value_known": r.usd_value_known, "signature": sig.to_string()
    }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(), "tool": r.tool_name, "wallet": r.w_name, "account_index": r.idx,
        "chain": "solana", "marketplace": r.marketplace, "usd_value": r.usd_value,
        "usd_value_known": r.usd_value_known, "policy_decision": outcome.policy_decision,
        "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result,
        "forced_confirm": outcome.forced_confirm, "daily_used_usd": outcome.daily_used_usd,
        "txid": sig.to_string(), "error_code": null, "result": "broadcasted",
        "signature": sig.to_string(),
    }));
    Keystore::release_lock(lock)?;
    Ok(ok(
        req_id,
        tool_ok(
            json!({ "chain": "solana", "marketplace": r.marketplace, "signature": sig.to_string() }),
        ),
    ))
}

/// EVM-specific result data for recording an NFT trade.
struct EvmNftResult<'a> {
    to_s: &'a str,
    value_wei_s: &'a str,
    tx_hash: alloy::primitives::B256,
    outcome: &'a super::super::policy_confirm::WriteConfirmOutcome,
}

/// Record tx history and audit log for a completed EVM NFT trade, then respond.
fn evm_nft_record_and_respond(
    shared: &SharedState,
    lock: std::fs::File,
    r: &NftRecordCtx<'_>,
    e: &EvmNftResult<'_>,
    req_id: Value,
) -> eyre::Result<JsonRpcResponse> {
    let EvmNftResult {
        to_s,
        value_wei_s,
        tx_hash,
        outcome,
    } = e;
    let ty = nft_history_type(r.tool_name);
    shared.ks.append_tx_history(&json!({
        "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": ty,
        "chain": r.chain, "wallet": r.w_name, "account_index": r.idx,
        "marketplace": r.marketplace, "to": to_s, "value_wei": value_wei_s,
        "usd_value": r.usd_value, "usd_value_known": r.usd_value_known,
        "tx_hash": format!("{:#x}", tx_hash)
    }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(), "tool": r.tool_name, "wallet": r.w_name, "account_index": r.idx,
        "chain": r.chain, "marketplace": r.marketplace, "usd_value": r.usd_value,
        "usd_value_known": r.usd_value_known, "policy_decision": outcome.policy_decision,
        "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result,
        "forced_confirm": outcome.forced_confirm, "daily_used_usd": outcome.daily_used_usd,
        "txid": format!("{:#x}", tx_hash), "error_code": null, "result": "broadcasted",
        "tx_hash": format!("{:#x}", tx_hash),
    }));
    Keystore::release_lock(lock)?;
    Ok(ok(
        req_id,
        tool_ok(
            json!({ "chain": r.chain, "marketplace": r.marketplace, "tx_hash": format!("{:#x}", tx_hash) }),
        ),
    ))
}

/// Audit-log a scam blocklist rejection for an EVM NFT trade.
fn evm_nft_blocklist_audit(shared: &SharedState, r: &NftRecordCtx<'_>, to_s: &str) {
    let _audit_log = shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(), "tool": r.tool_name, "wallet": r.w_name, "account_index": r.idx,
        "chain": r.chain, "usd_value": r.usd_value, "usd_value_known": r.usd_value_known,
        "policy_decision": null, "confirm_required": false, "confirm_result": null,
        "txid": null, "error_code": "scam_address_blocked",
        "result": "blocked_scam_blocklist", "to": to_s
    }));
}

/// Audit-log a simulation failure for an EVM NFT trade.
fn evm_nft_sim_fail_audit(
    shared: &SharedState,
    r: &NftRecordCtx<'_>,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
    to_s: &str,
) {
    let _audit_log = shared.ks.append_audit_log(&json!({
        "ts": utc_now_iso(), "tool": r.tool_name, "wallet": r.w_name, "account_index": r.idx,
        "chain": r.chain, "marketplace": r.marketplace, "usd_value": r.usd_value,
        "usd_value_known": r.usd_value_known, "policy_decision": outcome.policy_decision,
        "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result,
        "forced_confirm": outcome.forced_confirm, "daily_used_usd": outcome.daily_used_usd,
        "txid": null, "error_code": "simulation_failed", "result": "simulation_failed", "to": to_s
    }));
}

/// Validated NFT trade context shared by Solana and EVM handlers.
struct NftTradeCtx<'a> {
    req_id: Value,
    tool_name: &'a str,
    args: &'a Value,
    lock: std::fs::File,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    marketplace: String,
    op: WriteOp,
    usd_value: f64,
    usd_value_known: bool,
}

struct SolanaTxEnvelopeResult {
    tx_b64: String,
    adapter_ids: Vec<String>,
    usd_value: f64,
    usd_value_known: bool,
}

async fn resolve_solana_nft_envelope(
    shared: &SharedState,
    nft: &NftTradeCtx<'_>,
) -> Result<SolanaTxEnvelopeResult, ToolError> {
    let mut tx_b64 = get_str_in_args_or_asset(nft.args, "tx_b64")
        .unwrap_or_default()
        .to_owned();
    let mut result = SolanaTxEnvelopeResult {
        tx_b64: String::new(),
        adapter_ids: Vec::new(),
        usd_value: nft.usd_value,
        usd_value_known: nft.usd_value_known,
    };
    if tx_b64.is_empty() {
        if let Some(asset) = get_asset_obj(nft.args).cloned() {
            let from_pk = sol_pubkey_for_account(nft.w, nft.idx)
                .map_err(|e| ToolError::new("invalid_request", format!("{e:#}")))?;
            match crate::marketplace_adapter::fetch_solana_tx_envelope(
                &shared.cfg.http,
                &nft.marketplace,
                nft.tool_name,
                &from_pk.to_string(),
                asset,
            )
            .await
            {
                Ok(env) => {
                    tx_b64 = env.tx_b64;
                    if !result.usd_value_known {
                        if let Some(v) = env.usd_value {
                            result.usd_value = v;
                            result.usd_value_known = true;
                        }
                    }
                    result.adapter_ids = env.allowed_program_ids;
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    let (code, human) = classify_adapter_error(&msg);
                    return Err(ToolError::new(code, human));
                }
            }
        }
        if tx_b64.is_empty() {
            return Err(ToolError::new("invalid_request", "missing tx_b64"));
        }
    }
    result.tx_b64 = tx_b64;
    Ok(result)
}

/// Handle an NFT trade on Solana (buy/sell/bid via agent-supplied or adapter envelope).
async fn handle_solana_nft<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    nft: NftTradeCtx<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let env = match resolve_solana_nft_envelope(shared, &nft).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(nft.lock)?;
            return Ok(ok(nft.req_id, tool_err(te)));
        }
    };
    let NftTradeCtx {
        req_id,
        tool_name,
        args,
        lock,
        w,
        idx,
        marketplace,
        op,
        ..
    } = nft;
    let allowed_strs = collect_allowed_programs(args, env.adapter_ids);
    if allowed_strs.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "missing allowed_program_ids",
            )),
        ));
    }
    let allow: Vec<solana_sdk::pubkey::Pubkey> = allowed_strs
        .iter()
        .map(|s| SolanaChain::parse_pubkey(s))
        .collect::<Result<Vec<_>, _>>()?;
    let tx_bytes = base64::engine::general_purpose::STANDARD
        .decode(env.tx_b64.as_str())
        .context("decode tx_b64")?;

    let summary = format!(
        "{} NFT on Solana marketplace {} (remote tx)",
        tool_name.to_uppercase(),
        marketplace
    );
    let outcome = match maybe_confirm_write(
        shared,
        conn,
        stdin,
        stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op,
            chain: "solana",
            usd_value: env.usd_value,
            usd_value_known: env.usd_value_known,
            force_confirm: true,
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
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };

    let mode = effective_network_mode(shared, conn);
    let sol = SolanaChain::new_with_fallbacks(
        &shared.cfg.rpc.solana_rpc_url,
        solana_fallback_urls(shared, mode),
        &shared.cfg.http.jupiter_base_url,
        shared.cfg.http.jupiter_api_key.as_deref(),
        shared.cfg.rpc.solana_default_compute_unit_limit,
        shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );
    let kp =
        super::super::key_loading::load_solana_keypair(shared, conn, stdin, stdout, w, idx).await?;
    let sig = sol
        .sign_and_send_versioned_allowlist(&kp, &tx_bytes, &allow)
        .await?;

    let r = NftRecordCtx {
        tool_name,
        w_name: &w.name,
        idx,
        chain: "solana",
        marketplace: &marketplace,
        usd_value: env.usd_value,
        usd_value_known: env.usd_value_known,
    };
    solana_nft_record_and_respond(shared, lock, &r, &sig, &outcome, req_id)
}

/// Resolved EVM NFT envelope data (to, data, `value_wei`, `usd_value`).
struct EvmNftEnvelope {
    to_s: String,
    data_s: String,
    value_wei_s: String,
    usd_value: f64,
    usd_value_known: bool,
}

struct ResolveEnvelopeCtx<'a> {
    tool_name: &'a str,
    marketplace: &'a str,
    chain: &'a str,
    from: alloy::primitives::Address,
    usd_value: f64,
    usd_value_known: bool,
}

/// Resolve the EVM NFT envelope fields: parse args or fetch from adapter.
async fn resolve_evm_nft_envelope(
    shared: &SharedState,
    args: &Value,
    ctx: &ResolveEnvelopeCtx<'_>,
) -> Result<EvmNftEnvelope, ToolError> {
    let mut to_s = get_str_in_args_or_asset(args, "to")
        .unwrap_or_default()
        .to_owned();
    let mut data_s = get_str_in_args_or_asset(args, "data")
        .unwrap_or("0x")
        .to_owned();
    let mut value_wei_s = get_str_in_args_or_asset(args, "value_wei")
        .unwrap_or("0")
        .to_owned();
    let mut usd = ctx.usd_value;
    let mut usd_known = ctx.usd_value_known;

    if to_s.is_empty() {
        if let Some(asset) = get_asset_obj(args).cloned() {
            match crate::marketplace_adapter::fetch_evm_tx_envelope(
                &shared.cfg.http,
                ctx.marketplace,
                ctx.tool_name,
                ctx.chain,
                &format!("{:#x}", ctx.from),
                asset,
            )
            .await
            {
                Ok(env) => {
                    to_s = env.to;
                    data_s = env.data;
                    if !env.value_wei.trim().is_empty() {
                        value_wei_s = env.value_wei;
                    }
                    if !usd_known {
                        if let Some(v) = env.usd_value {
                            usd = v;
                            usd_known = true;
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    let (code, human) = classify_adapter_error(&msg);
                    return Err(ToolError::new(code, human));
                }
            }
        }
    }
    if to_s.is_empty() {
        return Err(ToolError::new("invalid_request", "missing to"));
    }
    Ok(EvmNftEnvelope {
        to_s,
        data_s,
        value_wei_s,
        usd_value: usd,
        usd_value_known: usd_known,
    })
}

/// Build the EVM typed transaction from the resolved envelope.
fn build_evm_nft_tx(
    from: alloy::primitives::Address,
    to_addr: alloy::primitives::Address,
    data_s: &str,
    value_wei_s: &str,
) -> eyre::Result<alloy::rpc::types::TransactionRequest> {
    let data = if data_s.trim().is_empty() || data_s.trim() == "0x" {
        alloy::primitives::Bytes::from(Vec::<u8>::new())
    } else {
        let t = data_s
            .trim()
            .strip_prefix("0x")
            .unwrap_or_else(|| data_s.trim());
        alloy::primitives::Bytes::from(hex::decode(t).context("decode data hex")?)
    };
    let value_wei = crate::chains::evm::parse_u256_dec(value_wei_s).context("parse value_wei")?;
    let tx_req = alloy::rpc::types::TransactionRequest::default()
        .with_from(from)
        .with_to(to_addr)
        .with_input(data)
        .with_value(value_wei);
    Ok(tx_req)
}

struct EvmNftSetup {
    evm: EvmChain,
    from: alloy::primitives::Address,
    env: EvmNftEnvelope,
    to_addr: alloy::primitives::Address,
}

async fn setup_evm_nft(
    shared: &SharedState,
    args: &Value,
    chain: &str,
    resolve_ctx: &ResolveEnvelopeCtx<'_>,
) -> Result<EvmNftSetup, ToolError> {
    let rpc_url = shared
        .cfg
        .rpc
        .evm_rpc_urls
        .get(chain)
        .ok_or_else(|| ToolError::new("invalid_request", format!("unknown evm chain: {chain}")))?
        .clone();
    let chain_id = *shared.cfg.rpc.evm_chain_ids.get(chain).ok_or_else(|| {
        ToolError::new("invalid_request", format!("missing evm chain id: {chain}"))
    })?;
    let mut evm = EvmChain::for_name(chain, chain_id, &rpc_url, &shared.cfg.http);
    if let Some(fb) = shared.cfg.rpc.evm_fallback_rpc_urls.get(chain) {
        evm.fallback_rpc_urls = fb.clone();
    }
    let from = resolve_ctx.from;
    let env = resolve_evm_nft_envelope(shared, args, resolve_ctx).await?;
    let to_addr = EvmChain::parse_address(env.to_s.as_str())
        .map_err(|e| ToolError::new("invalid_request", format!("{e:#}")))?;
    Ok(EvmNftSetup {
        evm,
        from,
        env,
        to_addr,
    })
}

/// Handle an NFT trade on an EVM chain (buy/sell/bid via agent-supplied or adapter envelope).
async fn handle_evm_nft<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    chain: &str,
    nft: NftTradeCtx<'_>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let NftTradeCtx {
        req_id,
        tool_name,
        args,
        lock,
        w,
        idx,
        marketplace,
        op,
        usd_value,
        usd_value_known,
    } = nft;
    let from_addr = evm_addr_for_account(w, idx).unwrap_or_default();
    let resolve_ctx = ResolveEnvelopeCtx {
        tool_name,
        marketplace: marketplace.as_str(),
        chain,
        from: from_addr,
        usd_value,
        usd_value_known,
    };
    let setup = match setup_evm_nft(shared, args, chain, &resolve_ctx).await {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };
    let EvmNftSetup { env, to_addr, .. } = &setup;
    let rec = NftRecordCtx {
        tool_name,
        w_name: &w.name,
        idx,
        chain,
        marketplace: &marketplace,
        usd_value: env.usd_value,
        usd_value_known: env.usd_value_known,
    };
    if shared.scam_blocklist_contains_evm(*to_addr).await {
        evm_nft_blocklist_audit(shared, &rec, &env.to_s);
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient/contract is blocked by the scam address blocklist",
            )),
        ));
    }
    let tx = build_evm_nft_tx(setup.from, *to_addr, &env.data_s, &env.value_wei_s)?;
    let summary = format!(
        "{} NFT on {} marketplace {} (remote tx to {})",
        tool_name.to_uppercase(),
        chain,
        marketplace,
        env.to_s.as_str()
    );
    let outcome = match maybe_confirm_write(
        shared,
        conn,
        stdin,
        stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op,
            chain,
            usd_value: env.usd_value,
            usd_value_known: env.usd_value_known,
            force_confirm: true,
            slippage_bps: None,
            to_address: Some(env.to_s.as_str()),
            contract: Some(env.to_s.as_str()),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };
    if let Err(e) = setup.evm.simulate_tx_strict(&tx).await {
        evm_nft_sim_fail_audit(shared, &rec, &outcome, &env.to_s);
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, tool_name),
            )),
        ));
    }
    let wallet =
        super::super::key_loading::load_evm_signer(shared, conn, stdin, stdout, w, idx).await?;
    let tx_hash = setup.evm.send_tx(wallet, tx).await?;
    let evm_result = EvmNftResult {
        to_s: &env.to_s,
        value_wei_s: &env.value_wei_s,
        tx_hash,
        outcome: &outcome,
    };
    evm_nft_record_and_respond(shared, lock, &rec, &evm_result, req_id)
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
    let lock = shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    let marketplace = args
        .get("marketplace")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if chain.is_empty() || marketplace.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "missing chain/marketplace",
            )),
        ));
    }
    let op = match tool_name {
        "buy_nft" => WriteOp::BuyNft,
        "sell_nft" => WriteOp::SellNft,
        "bid_nft" => WriteOp::BidNft,
        _ => {
            Keystore::release_lock(lock)?;
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("invalid_request", "unknown tool")),
            ));
        }
    };
    let (usd_value, usd_value_known) = parse_usd_value(&args);
    let chain_owned = chain.to_owned();

    let nft = NftTradeCtx {
        req_id,
        tool_name,
        args: &args,
        lock,
        w: &w,
        idx,
        marketplace: marketplace.to_owned(),
        op,
        usd_value,
        usd_value_known,
    };

    if chain == "solana" {
        handle_solana_nft(shared, conn, stdin, stdout, nft).await
    } else {
        handle_evm_nft(shared, conn, stdin, stdout, &chain_owned, nft).await
    }
}
