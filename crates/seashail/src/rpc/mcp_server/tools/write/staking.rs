use alloy::primitives::Address;
use alloy::rpc::types::TransactionRequest;
use alloy::sol;
use eyre::Context as _;
use serde_json::{json, Value};

use crate::{
    amount,
    chains::{evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
    financial_math,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::helpers::{
    evm_addr_for_account, resolve_wallet_and_account, sol_pubkey_for_account, solana_fallback_urls,
    u128_to_u256, u128_to_u64,
};
use super::super::key_loading::{load_evm_signer, load_solana_keypair};
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::common::{summarize_sim_error, wait_for_allowance};
use super::HandlerCtx;

sol! {
    #[sol(rpc)]
    #[allow(clippy::used_underscore_binding)]
    contract ILidoSteth {
        function submit(address _referral) external payable returns (uint256);
    }
}

sol! {
    #[sol(rpc)]
    #[allow(clippy::used_underscore_binding)]
    contract ILidoWithdrawalQueue {
        function requestWithdrawals(uint256[] _amounts, address _owner) external returns (uint256[] requestIds);
    }
}

const LIDO_STETH: &str = "0xae7ab96520de3a18e5e111b5eaab095312d7fe84";
const LIDO_WITHDRAWAL_QUEUE: &str = "0x889edc2edab5f40e902b864ad4d7ade8e412f9b1";

const JITOSOL_MINT: &str = "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse_amount_base_for_staking(
    amount_s: &str,
    units: &str,
    decimals: u8,
) -> Result<u128, ToolError> {
    if units == "base" {
        amount::parse_amount_base_u128(amount_s)
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))
    } else {
        amount::parse_amount_ui_to_base_u128(amount_s, u32::from(decimals))
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))
    }
}

fn parse_u128_to_u64(v: u128) -> Result<u64, ToolError> {
    u128_to_u64(v)
        .map_err(|e| ToolError::new("invalid_request", format!("amount too large: {e:#}")))
}

/// Handle Jito staking on Solana via Jupiter swap.
async fn handle_jito_stake<R, W>(
    tool_name: &str,
    ctx: &mut HandlerCtx<'_, R, W>,
    lock: std::fs::File,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    effective_policy: &crate::policy::Policy,
    op: WriteOp,
    history_type: &str,
    amount_s: &str,
    units: &str,
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
    let owner = sol_pubkey_for_account(w, idx)?;

    let (mint_in, mint_out) = if tool_name == "stake_tokens" {
        (WSOL_MINT, JITOSOL_MINT)
    } else {
        (JITOSOL_MINT, WSOL_MINT)
    };

    let mint_in_pk = match SolanaChain::parse_pubkey(mint_in) {
        Ok(v) => v,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new(
                    "invalid_request",
                    format!("invalid mint: {e:#}"),
                )),
            ));
        }
    };
    let decimals_in = sol
        .get_mint_decimals(mint_in_pk)
        .await
        .context("get mint decimals")?;
    let base_u128 = match parse_amount_base_for_staking(amount_s, units, decimals_in) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };
    let amt_in = match parse_u128_to_u64(base_u128) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    ctx.shared.ensure_db().await;
    let db = ctx.shared.db();
    let usd_value = if mint_in == WSOL_MINT {
        let p = price::native_token_price_usd_cached("solana", &ctx.shared.cfg, db).await?;
        financial_math::lamports_to_usd(amt_in, p.usd)
    } else {
        price::solana_token_price_usd_cached(
            &sol,
            &ctx.shared.cfg,
            mint_in,
            USDC_MINT,
            amt_in,
            50,
            db,
        )
        .await?
        .usd
    };

    let summary = format!("Jito stake via Jupiter swap on Solana: {mint_in} -> {mint_out}");
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op,
            chain: "solana",
            usd_value,
            usd_value_known: true,
            force_confirm: effective_policy.require_user_confirm_for_remote_tx.get(),
            slippage_bps: None,
            to_address: None,
            contract: Some("jito"),
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
        .jupiter_quote(mint_in, mint_out, amt_in, 100)
        .await
        .context("jupiter quote")?;
    let tx_bytes = sol.jupiter_swap_tx(quote, owner).await?;
    let kp = load_solana_keypair(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, w, idx).await?;
    let sig = sol.sign_and_send_versioned(&kp, &tx_bytes).await?;

    ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(), "type": history_type,
      "chain": "solana", "wallet": w.name, "account_index": idx, "usd_value": usd_value,
      "signature": sig.to_string(), "protocol": "jito", "mint_in": mint_in,
      "mint_out": mint_out, "amount_in_base": amt_in.to_string(),
    }))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": tool_name, "wallet": w.name, "account_index": idx,
      "chain": "solana", "usd_value": usd_value, "usd_value_known": true,
      "policy_decision": outcome.policy_decision, "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result, "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm, "txid": sig.to_string(),
      "error_code": null, "result": "broadcasted"
    }));

    Keystore::release_lock(lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(
            json!({ "chain": "solana", "protocol": "jito", "signature": sig.to_string(), "usd_value": usd_value }),
        ),
    ))
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
    let (w, idx) = resolve_wallet_and_account(ctx.shared, &ctx.args)?;
    let (effective_policy, _) = ctx.shared.cfg.policy_for_wallet(Some(w.name.as_str()));

    let chain = arg_str(&ctx.args, "chain").unwrap_or("");
    let protocol = arg_str(&ctx.args, "protocol").unwrap_or("");
    let protocol = if protocol.is_empty() {
        if chain == "solana" {
            "jito"
        } else {
            "lido"
        }
    } else {
        protocol
    };

    let amount_s = arg_str(&ctx.args, "amount").unwrap_or("").to_owned();
    let units = arg_str(&ctx.args, "amount_units")
        .unwrap_or("ui")
        .to_owned();
    if chain.is_empty() || amount_s.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "missing chain/amount")),
        ));
    }
    if amount_s.eq_ignore_ascii_case("max") {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "staking native paths do not support amount=max (provide an explicit amount, or use tx envelope fallback)",
            )),
        ));
    }

    let op = if tool_name == "stake_tokens" {
        WriteOp::Stake
    } else {
        WriteOp::Unstake
    };
    let history_type = if tool_name == "stake_tokens" {
        "stake"
    } else {
        "unstake"
    };

    if chain == "solana" && protocol == "jito" {
        return handle_jito_stake(
            tool_name,
            ctx,
            lock,
            &w,
            idx,
            &effective_policy,
            op,
            history_type,
            &amount_s,
            &units,
        )
        .await;
    }

    if chain == "ethereum" && protocol == "lido" {
        let amount_base_u128 = match parse_amount_base_for_staking(&amount_s, &units, 18) {
            Ok(v) => v,
            Err(te) => {
                Keystore::release_lock(lock)?;
                return Ok(ok(ctx.req_id.clone(), tool_err(te)));
            }
        };
        let amount_wei = u128_to_u256(amount_base_u128);
        if amount_wei.is_zero() {
            Keystore::release_lock(lock)?;
            return Ok(ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new("invalid_request", "amount must be > 0")),
            ));
        }

        let rpc_url = ctx
            .shared
            .cfg
            .rpc
            .evm_rpc_urls
            .get(chain)
            .ok_or_else(|| eyre::eyre!("unknown evm chain: {chain}"))?
            .clone();
        let chain_id = *ctx
            .shared
            .cfg
            .rpc
            .evm_chain_ids
            .get(chain)
            .ok_or_else(|| eyre::eyre!("missing evm chain id: {chain}"))?;
        let mut evm = EvmChain::for_name(chain, chain_id, &rpc_url, &ctx.shared.cfg.http);
        if let Some(fb) = ctx.shared.cfg.rpc.evm_fallback_rpc_urls.get(chain) {
            evm.fallback_rpc_urls.clone_from(fb);
        }

        let from_addr = evm_addr_for_account(&w, idx)?;

        ctx.shared.ensure_db().await;
        let db = ctx.shared.db();
        let eth_price = price::native_token_price_usd_cached(chain, &ctx.shared.cfg, db).await?;
        let usd_value = financial_math::token_base_to_usd(amount_base_u128, 18, eth_price.usd);

        let steth_addr = EvmChain::parse_address(LIDO_STETH).context("parse steth address")?;
        let queue_addr =
            EvmChain::parse_address(LIDO_WITHDRAWAL_QUEUE).context("parse withdrawal queue")?;

        if tool_name == "stake_tokens" {
            let steth = ILidoSteth::new(steth_addr, evm.provider()?);
            let call = steth.submit(Address::ZERO);
            let data: alloy::primitives::Bytes = call.calldata().clone();
            let tx = TransactionRequest {
                from: Some(from_addr),
                to: Some(steth_addr.into()),
                value: Some(amount_wei),
                input: data.into(),
                ..Default::default()
            };

            let summary = format!("Lido stake (stETH submit) on Ethereum: amount_eth={amount_s}");
            let outcome = match maybe_confirm_write(
                ctx.shared,
                ctx.conn,
                ctx.stdin,
                ctx.stdout,
                &WriteConfirmRequest {
                    tool: tool_name,
                    wallet: Some(w.name.as_str()),
                    account_index: Some(idx),
                    op,
                    chain: "ethereum",
                    usd_value,
                    usd_value_known: true,
                    force_confirm: false,
                    slippage_bps: None,
                    to_address: Some(LIDO_STETH),
                    contract: Some(LIDO_STETH),
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
                  "ts": utc_now_iso(),
                  "tool": tool_name,
                  "wallet": w.name,
                  "account_index": idx,
                  "chain": "ethereum",
                  "usd_value": usd_value,
                  "usd_value_known": true,
                  "policy_decision": outcome.policy_decision,
                  "confirm_required": outcome.confirm_required,
                  "confirm_result": outcome.confirm_result,
                  "daily_used_usd": outcome.daily_used_usd,
                  "forced_confirm": outcome.forced_confirm,
                  "txid": null,
                  "error_code": "simulation_failed",
                  "result": "simulation_failed",
                  "protocol": "lido",
                }));
                Keystore::release_lock(lock)?;
                return Ok(ok(
                    ctx.req_id.clone(),
                    tool_err(ToolError::new(
                        "simulation_failed",
                        summarize_sim_error(&e, "lido stake"),
                    )),
                ));
            }

            let signer =
                load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &w, idx).await?;
            let txid = evm.send_tx(signer, tx).await.context("send lido tx")?;
            let txid_s = format!("{txid:#x}");

            ctx.shared.ks.append_tx_history(&json!({
              "ts": utc_now_iso(),
              "day": Keystore::current_utc_day_key(),
              "type": history_type,
              "chain": "ethereum",
              "wallet": w.name,
              "account_index": idx,
              "protocol": "lido",
              "contract": LIDO_STETH,
              "amount_wei": amount_wei.to_string(),
              "usd_value": usd_value,
              "txid": txid_s,
            }))?;
            let _audit_log = ctx.shared.ks.append_audit_log(&json!({
              "ts": utc_now_iso(),
              "tool": tool_name,
              "wallet": w.name,
              "account_index": idx,
              "chain": "ethereum",
              "usd_value": usd_value,
              "usd_value_known": true,
              "policy_decision": outcome.policy_decision,
              "confirm_required": outcome.confirm_required,
              "confirm_result": outcome.confirm_result,
              "daily_used_usd": outcome.daily_used_usd,
              "forced_confirm": outcome.forced_confirm,
              "txid": txid_s,
              "error_code": null,
              "result": "broadcasted"
            }));

            Keystore::release_lock(lock)?;
            return Ok(ok(
                ctx.req_id.clone(),
                tool_ok(json!({
                  "chain": "ethereum",
                  "protocol": "lido",
                  "txid": txid_s,
                  "usd_value": usd_value
                })),
            ));
        }

        // Unstake (withdrawal request). This is asynchronous on Lido.
        // Initiation: approve stETH to the WithdrawalQueue and call requestWithdrawals.
        let steth = crate::chains::evm::IERC20::new(steth_addr, evm.provider()?);
        let allowance = steth
            .allowance(from_addr, queue_addr)
            .call()
            .await
            .context("read steth allowance")?;
        if allowance < amount_wei {
            let approve_tx = evm
                .build_erc20_approve(from_addr, steth_addr, queue_addr, amount_wei)
                .context("build steth approve tx")?;
            if let Err(e) = evm.simulate_tx_strict(&approve_tx).await {
                Keystore::release_lock(lock)?;
                return Ok(ok(
                    ctx.req_id.clone(),
                    tool_err(ToolError::new(
                        "simulation_failed",
                        summarize_sim_error(&e, "approve (lido)"),
                    )),
                ));
            }
            let signer =
                load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &w, idx).await?;
            let approve_txid = evm
                .send_tx(signer, approve_tx)
                .await
                .context("send approve")?;
            if !wait_for_allowance(&evm, steth_addr, from_addr, queue_addr, amount_wei).await {
                Keystore::release_lock(lock)?;
                return Ok(ok(
                    ctx.req_id.clone(),
                    tool_err(ToolError::new(
                        "approval_failed",
                        "approve transaction did not result in sufficient allowance in time",
                    )),
                ));
            }
            ctx.shared.ks.append_tx_history(&json!({
              "ts": utc_now_iso(),
              "day": Keystore::current_utc_day_key(),
              "type": "approve",
              "chain": "ethereum",
              "wallet": w.name,
              "account_index": idx,
              "protocol": "lido",
              "token": LIDO_STETH,
              "spender": LIDO_WITHDRAWAL_QUEUE,
              "amount_base": amount_wei.to_string(),
              "usd_value": 0.0_f64,
              "txid": format!("{approve_txid:#x}"),
            }))?;
        }

        let queue = ILidoWithdrawalQueue::new(queue_addr, evm.provider()?);
        let expected_ids = queue
            .requestWithdrawals(vec![amount_wei], from_addr)
            .call()
            .await
            .unwrap_or_default();
        let call = queue.requestWithdrawals(vec![amount_wei], from_addr);
        let data: alloy::primitives::Bytes = call.calldata().clone();
        let tx = TransactionRequest {
            from: Some(from_addr),
            to: Some(queue_addr.into()),
            input: data.into(),
            ..Default::default()
        };

        let summary =
            format!("Lido unstake (request withdrawal) on Ethereum: stETH amount={amount_s}");
        let outcome = match maybe_confirm_write(
            ctx.shared,
            ctx.conn,
            ctx.stdin,
            ctx.stdout,
            &WriteConfirmRequest {
                tool: tool_name,
                wallet: Some(w.name.as_str()),
                account_index: Some(idx),
                op,
                chain: "ethereum",
                usd_value,
                usd_value_known: true,
                force_confirm: false,
                slippage_bps: None,
                to_address: Some(LIDO_WITHDRAWAL_QUEUE),
                contract: Some(LIDO_WITHDRAWAL_QUEUE),
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
            Keystore::release_lock(lock)?;
            return Ok(ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new(
                    "simulation_failed",
                    summarize_sim_error(&e, "withdraw (lido)"),
                )),
            ));
        }
        let signer = load_evm_signer(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &w, idx).await?;
        let txid = evm.send_tx(signer, tx).await.context("send withdraw")?;
        let txid_s = format!("{txid:#x}");

        ctx.shared.ks.append_tx_history(&json!({
          "ts": utc_now_iso(),
          "day": Keystore::current_utc_day_key(),
          "type": history_type,
          "chain": "ethereum",
          "wallet": w.name,
          "account_index": idx,
          "protocol": "lido",
          "contract": LIDO_WITHDRAWAL_QUEUE,
          "amount_base": amount_wei.to_string(),
          "usd_value": usd_value,
          "txid": txid_s,
        }))?;
        let _audit_log = ctx.shared.ks.append_audit_log(&json!({
          "ts": utc_now_iso(),
          "tool": tool_name,
          "wallet": w.name,
          "account_index": idx,
          "chain": "ethereum",
          "usd_value": usd_value,
          "usd_value_known": true,
          "policy_decision": outcome.policy_decision,
          "confirm_required": outcome.confirm_required,
          "confirm_result": outcome.confirm_result,
          "daily_used_usd": outcome.daily_used_usd,
          "forced_confirm": outcome.forced_confirm,
          "txid": txid_s,
          "error_code": null,
          "result": "broadcasted"
        }));

        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_ok(json!({
              "chain": "ethereum",
              "protocol": "lido",
              "status": "pending",
              "txid": txid_s,
              "request_ids": expected_ids.iter().map(std::string::ToString::to_string).collect::<Vec<_>>(),
              "next_action": "Monitor Lido WithdrawalQueue claimability in the Lido UI or via on-chain reads.",
              "usd_value": usd_value
            })),
        ));
    }

    Keystore::release_lock(lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_err(ToolError::new(
            "invalid_request",
            "unsupported chain/protocol for native staking (supported: solana+jito, ethereum+lido)",
        )),
    ))
}
