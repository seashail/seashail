use alloy::rpc::types::TransactionRequest;
use base64::Engine as _;
use eyre::Context as _;
use serde_json::{json, Value};
use tokio::io::BufReader;

use crate::chains::{evm::EvmChain, solana::SolanaChain};
use crate::errors::ToolError;
use crate::keystore::{utc_now_iso, Keystore};
use crate::policy_engine::WriteOp;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, resolve_wallet_and_account, sol_pubkey_for_account, solana_fallback_urls,
};
use super::super::key_loading::{load_evm_signer, load_solana_keypair};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::{
    get_asset_obj, get_str_in_args_or_asset, parse_usd_value, summarize_sim_error,
};

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Validate a protocol/provider arg against an allow-list.
fn validate_protocol(value: &str, allowed: &[&str], label: &str) -> Result<(), ToolError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(ToolError::new(
            "invalid_request",
            format!("{label} must be one of: {}", allowed.join(", ")),
        ))
    }
}

fn op_for_tool(
    tool_name: &str,
    chain: &str,
    args: &Value,
) -> Result<(WriteOp, &'static str, String), ToolError> {
    match tool_name {
        "bridge_tokens" => {
            let provider = arg_str(args, "bridge_provider")
                .or_else(|| arg_str(args, "provider"))
                .unwrap_or("wormhole");
            validate_protocol(provider, &["wormhole", "layerzero"], "bridge_provider")?;
            Ok((WriteOp::Bridge, "bridge", provider.to_owned()))
        }
        "lend_tokens" | "withdraw_lending" | "borrow_tokens" | "repay_borrow" => {
            let default = if chain == "solana" { "kamino" } else { "aave" };
            let protocol = arg_str(args, "protocol").unwrap_or(default);
            validate_protocol(
                protocol,
                &["aave", "compound", "kamino", "marginfi"],
                "protocol",
            )?;
            let (op, ht) = match tool_name {
                "lend_tokens" => (WriteOp::Lend, "lend"),
                "withdraw_lending" => (WriteOp::WithdrawLending, "withdraw_lending"),
                "borrow_tokens" => (WriteOp::Borrow, "borrow"),
                _ => (WriteOp::RepayBorrow, "repay_borrow"),
            };
            Ok((op, ht, protocol.to_owned()))
        }
        "stake_tokens" | "unstake_tokens" => {
            let default = if chain == "solana" { "jito" } else { "lido" };
            let protocol = arg_str(args, "protocol").unwrap_or(default);
            validate_protocol(
                protocol,
                &["lido", "eigenlayer", "marinade", "jito"],
                "protocol",
            )?;
            let is_stake = tool_name == "stake_tokens";
            let op = if is_stake {
                WriteOp::Stake
            } else {
                WriteOp::Unstake
            };
            let ht = if is_stake { "stake" } else { "unstake" };
            Ok((op, ht, protocol.to_owned()))
        }
        "provide_liquidity" | "remove_liquidity" => {
            let default = if chain == "solana" {
                "orca_lp"
            } else {
                "uniswap_lp"
            };
            let venue = arg_str(args, "venue")
                .or_else(|| arg_str(args, "protocol"))
                .unwrap_or(default);
            validate_protocol(venue, &["uniswap_lp", "orca_lp"], "venue")?;
            let is_provide = tool_name == "provide_liquidity";
            let op = if is_provide {
                WriteOp::ProvideLiquidity
            } else {
                WriteOp::RemoveLiquidity
            };
            let ht = if is_provide {
                "provide_liquidity"
            } else {
                "remove_liquidity"
            };
            Ok((op, ht, venue.to_owned()))
        }
        "place_prediction" | "close_prediction" => {
            let protocol = arg_str(args, "protocol").unwrap_or("polymarket");
            validate_protocol(protocol, &["polymarket"], "protocol")?;
            let is_place = tool_name == "place_prediction";
            let op = if is_place {
                WriteOp::PlacePrediction
            } else {
                WriteOp::ClosePrediction
            };
            let ht = if is_place {
                "prediction_place"
            } else {
                "prediction_close"
            };
            Ok((op, ht, protocol.to_owned()))
        }
        _ => Err(ToolError::new("invalid_request", "unknown tool")),
    }
}

fn collect_allowed_program_ids(args: &Value) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    if let Some(arr) = args.get("allowed_program_ids").and_then(|v| v.as_array()) {
        ids.extend(
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned),
        );
    }
    if ids.is_empty() {
        if let Some(asset) = get_asset_obj(args) {
            if let Some(arr) = asset.get("allowed_program_ids").and_then(|v| v.as_array()) {
                ids.extend(
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned),
                );
            }
        }
    }
    ids
}

/// Bundled parameters for Solana and EVM tx envelope handlers.
struct EnvelopeParams<'a> {
    req_id: Value,
    tool_name: &'a str,
    args: &'a Value,
    lock: std::fs::File,
    w: &'a crate::wallet::WalletRecord,
    idx: u32,
    op: WriteOp,
    history_type: &'a str,
    marketplace: &'a str,
    usd_value: f64,
    usd_value_known: bool,
}

/// Resolved Solana tx bytes and their allowed program ID pubkeys.
struct SolanaTxResolved {
    tx_b64: String,
    allowed: Vec<solana_sdk::pubkey::Pubkey>,
    usd_value: f64,
    usd_value_known: bool,
}

type SolanaTxResolvedResult = Result<SolanaTxResolved, Box<JsonRpcResponse>>;

/// Resolve inline Solana tx bytes and allowed program IDs from args.
fn resolve_solana_tx_inline(params: &EnvelopeParams<'_>) -> SolanaTxResolvedResult {
    let tx_b64 = get_str_in_args_or_asset(params.args, "tx_b64")
        .unwrap_or_default()
        .to_owned();
    let allowed_program_ids = collect_allowed_program_ids(params.args);
    finish_program_id_resolution(
        &params.req_id,
        tx_b64,
        allowed_program_ids,
        params.usd_value,
        params.usd_value_known,
    )
}

/// Validate and parse allowed program IDs, returning the resolved tx data or an error response.
fn finish_program_id_resolution(
    req_id: &Value,
    tx_b64: String,
    allowed_program_ids: Vec<String>,
    usd_value: f64,
    usd_value_known: bool,
) -> SolanaTxResolvedResult {
    if allowed_program_ids.is_empty() {
        return Err(Box::new(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing allowed_program_ids",
            )),
        )));
    }
    let allowed: Vec<solana_sdk::pubkey::Pubkey> = allowed_program_ids
        .into_iter()
        .filter_map(|s| SolanaChain::parse_pubkey(&s).ok())
        .collect();
    if allowed.is_empty() {
        return Err(Box::new(ok(
            req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "allowed_program_ids must be non-empty",
            )),
        )));
    }
    Ok(SolanaTxResolved {
        tx_b64,
        allowed,
        usd_value,
        usd_value_known,
    })
}

/// Resolve Solana tx bytes from adapter when `tx_b64` is not inline.
async fn resolve_solana_tx_via_adapter(
    params: &EnvelopeParams<'_>,
    shared: &SharedState,
) -> eyre::Result<SolanaTxResolvedResult> {
    let Some(asset) = get_asset_obj(params.args).cloned() else {
        return Ok(Err(Box::new(ok(
            params.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing tx_b64 (or provide asset for adapter)",
            )),
        ))));
    };
    let from_pk = sol_pubkey_for_account(params.w, params.idx)?;
    let env = match crate::marketplace_adapter::fetch_solana_tx_envelope(
        &shared.cfg.http,
        params.marketplace,
        params.tool_name,
        &from_pk.to_string(),
        asset,
    )
    .await
    {
        Ok(env) => env,
        Err(e) => {
            return Ok(Err(Box::new(ok(
                params.req_id.clone(),
                tool_err(ToolError::new("defi_adapter_error", format!("{e:#}"))),
            ))));
        }
    };

    let mut usd_value = params.usd_value;
    let mut usd_value_known = params.usd_value_known;
    if !usd_value_known {
        if let Some(v) = env.usd_value {
            usd_value = v;
            usd_value_known = true;
        }
    }

    let allowed_program_ids = if env.allowed_program_ids.is_empty() {
        collect_allowed_program_ids(params.args)
    } else {
        env.allowed_program_ids
    };

    Ok(finish_program_id_resolution(
        &params.req_id,
        env.tx_b64,
        allowed_program_ids,
        usd_value,
        usd_value_known,
    ))
}

/// Handle Solana tx envelope: resolve tx bytes, confirm, sign and broadcast.
async fn handle_solana_envelope<R, W>(
    params: EnvelopeParams<'_>,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let has_inline_tx = !get_str_in_args_or_asset(params.args, "tx_b64")
        .unwrap_or_default()
        .is_empty();

    let resolved = if has_inline_tx {
        match resolve_solana_tx_inline(&params) {
            Ok(r) => r,
            Err(resp) => {
                Keystore::release_lock(params.lock)?;
                return Ok(*resp);
            }
        }
    } else {
        match resolve_solana_tx_via_adapter(&params, shared).await? {
            Ok(r) => r,
            Err(resp) => {
                Keystore::release_lock(params.lock)?;
                return Ok(*resp);
            }
        }
    };

    let (effective_policy, _) = shared.cfg.policy_for_wallet(Some(params.w.name.as_str()));
    let summary = format!(
        "{} on Solana via tx envelope ({})",
        params.tool_name, params.marketplace
    );
    let outcome = match maybe_confirm_write(
        shared,
        conn,
        stdin,
        stdout,
        &WriteConfirmRequest {
            tool: params.tool_name,
            wallet: Some(params.w.name.as_str()),
            account_index: Some(params.idx),
            op: params.op,
            chain: "solana",
            usd_value: resolved.usd_value,
            usd_value_known: resolved.usd_value_known,
            force_confirm: effective_policy.require_user_confirm_for_remote_tx.get(),
            slippage_bps: None,
            to_address: None,
            contract: Some(params.marketplace),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(params.lock)?;
            return Ok(ok(params.req_id, tool_err(te)));
        }
    };

    let tx_bytes = base64::engine::general_purpose::STANDARD
        .decode(&resolved.tx_b64)
        .context("decode tx_b64")?;
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
    let kp = load_solana_keypair(shared, conn, stdin, stdout, params.w, params.idx).await?;
    let sig = sol
        .sign_and_send_versioned_allowlist(&kp, &tx_bytes, &resolved.allowed)
        .await?;

    shared.ks.append_tx_history(&json!({ "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": params.history_type, "chain": "solana", "wallet": params.w.name, "account_index": params.idx, "usd_value": resolved.usd_value, "signature": sig.to_string(), "protocol": params.marketplace }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": params.tool_name, "wallet": params.w.name, "account_index": params.idx, "chain": "solana", "usd_value": resolved.usd_value, "usd_value_known": resolved.usd_value_known, "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm, "txid": sig.to_string(), "error_code": null, "result": "broadcasted" }));

    Keystore::release_lock(params.lock)?;
    Ok(ok(
        params.req_id,
        tool_ok(
            json!({ "chain": "solana", "signature": sig.to_string(), "usd_value": resolved.usd_value }),
        ),
    ))
}

/// Resolved EVM tx parameters (to, data, `value_wei`) and updated USD value.
struct EvmTxResolved {
    to: String,
    data: String,
    value_wei: String,
    usd_value: f64,
    usd_value_known: bool,
}

/// Resolve EVM tx parameters from args or via the marketplace adapter.
async fn resolve_evm_tx_params(
    params: &EnvelopeParams<'_>,
    shared: &SharedState,
    chain: &str,
    from: alloy::primitives::Address,
) -> Result<Result<EvmTxResolved, JsonRpcResponse>, eyre::Report> {
    let mut to = get_str_in_args_or_asset(params.args, "to")
        .unwrap_or_default()
        .to_owned();
    let mut data = get_str_in_args_or_asset(params.args, "data")
        .unwrap_or("0x")
        .to_owned();
    let mut value_wei = get_str_in_args_or_asset(params.args, "value_wei")
        .unwrap_or("0")
        .to_owned();
    let mut usd_value = params.usd_value;
    let mut usd_value_known = params.usd_value_known;

    if to.trim().is_empty() && get_asset_obj(params.args).is_some() {
        let asset = get_asset_obj(params.args)
            .cloned()
            .unwrap_or_else(|| json!({}));
        let env = match crate::marketplace_adapter::fetch_evm_tx_envelope(
            &shared.cfg.http,
            params.marketplace,
            params.tool_name,
            chain,
            &format!("{from:#x}"),
            asset,
        )
        .await
        {
            Ok(env) => env,
            Err(e) => {
                return Ok(Err(ok(
                    params.req_id.clone(),
                    tool_err(ToolError::new("defi_adapter_error", format!("{e:#}"))),
                )));
            }
        };
        to = env.to;
        data = env.data;
        value_wei = env.value_wei;
        if !usd_value_known {
            if let Some(v) = env.usd_value {
                usd_value = v;
                usd_value_known = true;
            }
        }
    }

    if to.trim().is_empty() {
        return Ok(Err(ok(
            params.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "missing to (or provide asset for adapter)",
            )),
        )));
    }

    Ok(Ok(EvmTxResolved {
        to,
        data,
        value_wei,
        usd_value,
        usd_value_known,
    }))
}

/// Check EVM address against scam blocklist and OFAC SDN list.
async fn check_evm_blocklists(
    params: &EnvelopeParams<'_>,
    shared: &mut SharedState,
    chain: &str,
    to: &str,
    to_addr: alloy::primitives::Address,
    usd_value: f64,
    usd_value_known: bool,
) -> Option<JsonRpcResponse> {
    if shared.scam_blocklist_contains_evm(to_addr).await {
        let _audit_log = shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": params.tool_name, "wallet": params.w.name, "account_index": params.idx, "chain": chain, "usd_value": usd_value, "usd_value_known": usd_value_known, "policy_decision": null, "confirm_required": false, "confirm_result": null, "txid": null, "error_code": "scam_address_blocked", "result": "blocked_scam_blocklist", "to": to }));
        return Some(ok(
            params.req_id.clone(),
            tool_err(ToolError::new(
                "scam_address_blocked",
                "recipient is blocked by the scam address blocklist",
            )),
        ));
    }
    let (effective_policy, _) = shared.cfg.policy_for_wallet(Some(params.w.name.as_str()));
    if effective_policy.enable_ofac_sdn.get() && shared.ofac_sdn_contains_evm(to_addr).await {
        let _audit_log = shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": params.tool_name, "wallet": params.w.name, "account_index": params.idx, "chain": chain, "usd_value": usd_value, "usd_value_known": usd_value_known, "policy_decision": null, "confirm_required": false, "confirm_result": null, "txid": null, "error_code": "ofac_sdn_blocked", "result": "blocked_ofac_sdn", "to": to }));
        return Some(ok(
            params.req_id.clone(),
            tool_err(ToolError::new(
                "ofac_sdn_blocked",
                "recipient is blocked by the OFAC SDN list",
            )),
        ));
    }
    None
}

/// Build the EVM transaction request from resolved parameters.
fn build_evm_typed_tx(
    from: alloy::primitives::Address,
    to_addr: alloy::primitives::Address,
    data: &str,
    value_wei: &str,
) -> TransactionRequest {
    let value = crate::chains::evm::parse_u256_dec(value_wei).unwrap_or_default();
    TransactionRequest {
        from: Some(from),
        to: Some(to_addr.into()),
        input: alloy::primitives::Bytes::from(
            hex::decode(data.trim_start_matches("0x")).unwrap_or_default(),
        )
        .into(),
        value: Some(value),
        ..Default::default()
    }
}

/// Handle EVM tx envelope: resolve tx params, blocklist check, confirm, simulate, sign and broadcast.
async fn handle_evm_envelope<R, W>(
    params: EnvelopeParams<'_>,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    chain: &str,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
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

    let from = evm_addr_for_account(params.w, params.idx)?;
    let resolved = match resolve_evm_tx_params(&params, shared, chain, from).await? {
        Ok(r) => r,
        Err(resp) => {
            Keystore::release_lock(params.lock)?;
            return Ok(resp);
        }
    };

    let to_addr = EvmChain::parse_address(&resolved.to)?;
    if let Some(blocked_resp) = check_evm_blocklists(
        &params,
        shared,
        chain,
        &resolved.to,
        to_addr,
        resolved.usd_value,
        resolved.usd_value_known,
    )
    .await
    {
        Keystore::release_lock(params.lock)?;
        return Ok(blocked_resp);
    }

    let tx = build_evm_typed_tx(from, to_addr, &resolved.data, &resolved.value_wei);
    let (effective_policy, _) = shared.cfg.policy_for_wallet(Some(params.w.name.as_str()));
    let summary = format!(
        "{} on {chain} via tx envelope ({})",
        params.tool_name, params.marketplace
    );
    let outcome = match maybe_confirm_write(
        shared,
        conn,
        stdin,
        stdout,
        &WriteConfirmRequest {
            tool: params.tool_name,
            wallet: Some(params.w.name.as_str()),
            account_index: Some(params.idx),
            op: params.op,
            chain,
            usd_value: resolved.usd_value,
            usd_value_known: resolved.usd_value_known,
            force_confirm: effective_policy.require_user_confirm_for_remote_tx.get(),
            slippage_bps: None,
            to_address: Some(&resolved.to),
            contract: Some(&resolved.to),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(params.lock)?;
            return Ok(ok(params.req_id, tool_err(te)));
        }
    };

    if let Err(e) = evm.simulate_tx_strict(&tx).await {
        let _audit_log = shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": params.tool_name, "wallet": params.w.name, "account_index": params.idx, "chain": chain, "usd_value": resolved.usd_value, "usd_value_known": resolved.usd_value_known, "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm, "txid": null, "error_code": "simulation_failed", "result": "blocked_simulation" }));
        Keystore::release_lock(params.lock)?;
        return Ok(ok(
            params.req_id,
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, params.tool_name),
            )),
        ));
    }

    // Fail-closed on `estimateGas` as well. Some providers are inconsistent about surfacing
    // reverts via `eth_call`, but will still fail `estimateGas` for reverting transactions.
    if let Err(e) = evm.estimate_tx_gas_strict(&tx).await {
        let _audit_log = shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": params.tool_name, "wallet": params.w.name, "account_index": params.idx, "chain": chain, "usd_value": resolved.usd_value, "usd_value_known": resolved.usd_value_known, "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm, "txid": null, "error_code": "simulation_failed", "result": "blocked_simulation" }));
        Keystore::release_lock(params.lock)?;
        return Ok(ok(
            params.req_id,
            tool_err(ToolError::new(
                "simulation_failed",
                summarize_sim_error(&e, params.tool_name),
            )),
        ));
    }

    let signer = load_evm_signer(shared, conn, stdin, stdout, params.w, params.idx).await?;
    let txid = match evm.send_tx(signer, tx).await {
        Ok(txid) => txid,
        Err(e) => {
            // Treat gas estimation failures as simulation failures (fail-closed).
            // This commonly occurs when a call would revert.
            let msg = format!("{e:#}").to_ascii_lowercase();
            if msg.contains("estimate gas") || msg.contains("execution reverted") {
                let _audit_log = shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": params.tool_name, "wallet": params.w.name, "account_index": params.idx, "chain": chain, "usd_value": resolved.usd_value, "usd_value_known": resolved.usd_value_known, "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm, "txid": null, "error_code": "simulation_failed", "result": "blocked_simulation" }));
                Keystore::release_lock(params.lock)?;
                return Ok(ok(
                    params.req_id,
                    tool_err(ToolError::new(
                        "simulation_failed",
                        summarize_sim_error(&e, params.tool_name),
                    )),
                ));
            }
            return Err(e);
        }
    };

    shared.ks.append_tx_history(&json!({ "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": params.history_type, "chain": chain, "wallet": params.w.name, "account_index": params.idx, "usd_value": resolved.usd_value, "txid": format!("{txid:#x}"), "protocol": params.marketplace, "to": resolved.to }))?;
    let _audit_log = shared.ks.append_audit_log(&json!({ "ts": utc_now_iso(), "tool": params.tool_name, "wallet": params.w.name, "account_index": params.idx, "chain": chain, "usd_value": resolved.usd_value, "usd_value_known": resolved.usd_value_known, "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm, "txid": format!("{txid:#x}"), "error_code": null, "result": "broadcasted" }));

    Keystore::release_lock(params.lock)?;
    Ok(ok(
        params.req_id,
        tool_ok(
            json!({ "chain": chain, "txid": format!("{txid:#x}"), "usd_value": resolved.usd_value }),
        ),
    ))
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
    if chain.trim().is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing chain")),
        ));
    }

    let (op, history_type, marketplace) = match op_for_tool(tool_name, chain, &args) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };

    let (usd_value, usd_value_known) = parse_usd_value(&args);

    let params = EnvelopeParams {
        req_id,
        tool_name,
        args: &args,
        lock,
        w: &w,
        idx,
        op,
        history_type,
        marketplace: &marketplace,
        usd_value,
        usd_value_known,
    };

    if chain == "solana" {
        return handle_solana_envelope(params, shared, conn, stdin, stdout).await;
    }

    handle_evm_envelope(params, shared, conn, stdin, stdout, chain).await
}
