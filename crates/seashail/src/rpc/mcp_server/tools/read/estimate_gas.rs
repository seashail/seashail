use crate::{
    amount,
    chains::{evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
};
use alloy::primitives::U256;
use alloy::rpc::types::TransactionRequest;
use bincode::Options as _;
use eyre::Context as _;
use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{
    evm_addr_for_account, is_native_token, resolve_wallet_and_account, sol_pubkey_for_account,
    solana_fallback_urls, u128_to_u256, u128_to_u64, MAX_REMOTE_TX_BYTES,
};

/// Bundled parameters for send-transaction gas estimation.
struct SendEstimateParams<'a> {
    req_id: Value,
    args: &'a Value,
    chain: &'a str,
    to: &'a str,
    token: &'a str,
    amount: &'a str,
    units: &'a str,
}

/// Bundled parameters for swap gas estimation.
struct SwapEstimateParams<'a> {
    req_id: Value,
    chain: &'a str,
    token_in: &'a str,
    token_out: &'a str,
    amount_in: &'a str,
    units: &'a str,
    slippage_bps: u32,
}

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    if op.is_empty() || chain.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing op or chain")),
        ));
    }

    if op == "send_transaction" {
        return estimate_send_transaction(req_id, &args, chain, shared, conn).await;
    }

    if op == "swap_tokens" {
        return estimate_swap_tokens(req_id, &args, chain, shared, conn).await;
    }

    Ok(ok(
        req_id,
        tool_err(ToolError::new("invalid_request", "unknown op")),
    ))
}

async fn estimate_send_transaction(
    req_id: Value,
    args: &Value,
    chain: &str,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("native");
    let amount = args.get("amount").and_then(|v| v.as_str()).unwrap_or("");
    let units = args
        .get("amount_units")
        .and_then(|v| v.as_str())
        .unwrap_or("ui");

    let params = SendEstimateParams {
        req_id,
        args,
        chain,
        to,
        token,
        amount,
        units,
    };

    if chain == "solana" {
        return estimate_send_solana(params, shared, conn).await;
    }

    estimate_send_evm(params, shared).await
}

async fn estimate_send_solana(
    p: SendEstimateParams<'_>,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let SendEstimateParams {
        req_id,
        args,
        chain,
        to,
        token,
        amount,
        units,
    } = p;
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
    let to_pk = SolanaChain::parse_pubkey(to)?;
    let (w, idx) = resolve_wallet_and_account(shared, args)?;
    let from_pk = sol_pubkey_for_account(&w, idx)?;
    let ixs = if is_native_token(token) {
        let lamports = if units == "base" {
            u128_to_u64(amount::parse_amount_base_u128(amount)?)?
        } else {
            u128_to_u64(amount::parse_amount_ui_to_base_u128(amount, 9)?)?
        };
        let from_addr = solana_address::Address::new_from_array(from_pk.to_bytes());
        let to_addr = solana_address::Address::new_from_array(to_pk.to_bytes());
        vec![solana_system_interface::instruction::transfer(
            &from_addr, &to_addr, lamports,
        )]
    } else {
        let mint = SolanaChain::parse_pubkey(token)?;
        let decimals = sol
            .get_mint_decimals(mint)
            .await
            .context("get mint decimals")?;
        let amount_base = if units == "base" {
            u128_to_u64(amount::parse_amount_base_u128(amount)?)?
        } else {
            u128_to_u64(amount::parse_amount_ui_to_base_u128(
                amount,
                u32::from(decimals),
            )?)?
        };

        let from_ata = spl_associated_token_account::get_associated_token_address(&from_pk, &mint);
        let to_ata = spl_associated_token_account::get_associated_token_address(&to_pk, &mint);
        let mut ixs = vec![];
        if sol.get_account(&to_ata).await.is_err() {
            ixs.push(
                spl_associated_token_account::instruction::create_associated_token_account(
                    &from_pk,
                    &to_pk,
                    &mint,
                    &spl_token::id(),
                ),
            );
        }
        ixs.push(
            spl_token::instruction::transfer_checked(
                &spl_token::id(),
                &from_ata,
                &mint,
                &to_ata,
                &from_pk,
                &[],
                amount_base,
                decimals,
            )
            .context("build spl transfer")?,
        );
        ixs
    };
    // `Message::new` uses a default (zero) recent blockhash which can cause
    // `getFeeForMessage` to fail on local validators. Use a fresh blockhash.
    let bh = sol
        .get_latest_blockhash()
        .await
        .context("get latest blockhash")?;
    let msg = solana_sdk::message::Message::new_with_blockhash(&ixs, Some(&from_pk), &bh);
    // `getFeeForMessage` is not supported on some local validator versions.
    // Fall back to a conservative static fee-per-signature value (best-effort).
    let fee = match sol.get_fee_for_message_legacy(&msg).await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "get_fee_for_message failed; falling back to static lamports_per_signature"
            );
            let lamports_per_signature = 5_000_u64;
            u64::from(msg.header.num_required_signatures).saturating_mul(lamports_per_signature)
        }
    };
    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": chain, "fee_lamports": fee })),
    ))
}

async fn estimate_send_evm(
    p: SendEstimateParams<'_>,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let SendEstimateParams {
        req_id,
        args,
        chain,
        to,
        token,
        amount,
        units,
    } = p;
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
        evm.fallback_rpc_urls = fb.clone();
    }
    let (w, idx) = resolve_wallet_and_account(shared, args)?;
    let from = evm_addr_for_account(&w, idx)?;
    let to_addr = EvmChain::parse_address(to)?;

    let mut tx: TransactionRequest;
    if is_native_token(token) {
        let wei: U256 = if units == "base" {
            crate::chains::evm::parse_u256_dec(amount)?
        } else {
            // native assumes 18 decimals
            u128_to_u256(amount::parse_amount_ui_to_base_u128(amount, 18)?)
        };
        tx = EvmChain::build_native_transfer(from, to_addr, wei);
    } else {
        let token_addr = EvmChain::parse_address(token)?;
        let (decimals, _sym) = evm.get_erc20_metadata(token_addr).await?;
        let val: U256 = if units == "base" {
            crate::chains::evm::parse_u256_dec(amount)?
        } else {
            u128_to_u256(amount::parse_amount_ui_to_base_u128(
                amount,
                u32::from(decimals),
            )?)
        };
        tx = evm.build_erc20_transfer(from, token_addr, to_addr, val)?;
    }
    tx.from = Some(from);
    let gas = evm.estimate_tx_gas(&tx).await?;
    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": chain, "gas": gas.to_string() })),
    ))
}

async fn estimate_swap_tokens(
    req_id: Value,
    args: &Value,
    chain: &str,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let token_in = args.get("token_in").and_then(|v| v.as_str()).unwrap_or("");
    let token_out = args.get("token_out").and_then(|v| v.as_str()).unwrap_or("");
    let amount_in = args.get("amount_in").and_then(|v| v.as_str()).unwrap_or("");
    let units = args
        .get("amount_units")
        .and_then(|v| v.as_str())
        .unwrap_or("ui");
    let slippage_bps = args
        .get("slippage_bps")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(100);
    let provider_raw = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");
    let provider = if provider_raw == "auto" {
        if chain == "solana" {
            "jupiter"
        } else {
            "uniswap"
        }
    } else {
        provider_raw
    };

    let params = SwapEstimateParams {
        req_id,
        chain,
        token_in,
        token_out,
        amount_in,
        units,
        slippage_bps,
    };

    if chain == "solana" {
        return estimate_swap_solana(params, args, provider, shared, conn).await;
    }

    estimate_swap_evm(params, args, provider, shared).await
}

async fn estimate_swap_solana(
    p: SwapEstimateParams<'_>,
    args: &Value,
    provider: &str,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let SwapEstimateParams {
        req_id,
        chain,
        token_in,
        token_out,
        amount_in,
        units,
        slippage_bps,
    } = p;
    if provider != "jupiter" {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_provider",
                "solana swaps must use provider=jupiter",
            )),
        ));
    }
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
    let (w, idx) = resolve_wallet_and_account(shared, args)?;
    let owner = sol_pubkey_for_account(&w, idx)?;

    // Jupiter expects lamports-style base units.
    let mint_in = if is_native_token(token_in) {
        "So11111111111111111111111111111111111111112".to_owned()
    } else {
        token_in.to_owned()
    };
    let mint_in_pk = SolanaChain::parse_pubkey(&mint_in)?;
    let decimals = sol
        .get_mint_decimals(mint_in_pk)
        .await
        .context("get mint decimals")?;
    let amt_u64 = if units == "base" {
        u128_to_u64(amount::parse_amount_base_u128(amount_in)?)?
    } else {
        u128_to_u64(amount::parse_amount_ui_to_base_u128(
            amount_in,
            u32::from(decimals),
        )?)?
    };
    let out_mint = if is_native_token(token_out) {
        "So11111111111111111111111111111111111111112"
    } else {
        token_out
    };
    let quote = sol
        .jupiter_quote(&mint_in, out_mint, amt_u64, slippage_bps)
        .await?;
    let tx_bytes = sol.jupiter_swap_tx(quote, owner).await?;
    let vt: solana_sdk::transaction::VersionedTransaction = bincode::DefaultOptions::new()
        .with_limit(MAX_REMOTE_TX_BYTES)
        .deserialize(&tx_bytes)
        .context("deserialize versioned tx")?;
    let msg = vt.message;
    let fee = solana_fee_from_versioned_message(&sol, msg).await;
    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": chain, "fee_lamports": fee })),
    ))
}

async fn solana_fee_from_versioned_message(
    sol: &SolanaChain,
    msg: solana_sdk::message::VersionedMessage,
) -> u64 {
    match msg {
        solana_sdk::message::VersionedMessage::Legacy(m) => {
            match sol.get_fee_for_message_legacy(&m).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        "get_fee_for_message failed; falling back to static lamports_per_signature"
                    );
                    let lamports_per_signature = 5_000_u64;
                    u64::from(m.header.num_required_signatures)
                        .saturating_mul(lamports_per_signature)
                }
            }
        }
        solana_sdk::message::VersionedMessage::V0(m) => {
            match sol.get_fee_for_message_v0(&m).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        "get_fee_for_message failed; falling back to static lamports_per_signature"
                    );
                    let lamports_per_signature = 5_000_u64;
                    u64::from(m.header.num_required_signatures)
                        .saturating_mul(lamports_per_signature)
                }
            }
        }
    }
}

async fn estimate_swap_evm(
    p: SwapEstimateParams<'_>,
    args: &Value,
    provider: &str,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let SwapEstimateParams {
        req_id,
        chain,
        token_in,
        token_out,
        amount_in,
        units,
        slippage_bps,
    } = p;
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
        evm.fallback_rpc_urls = fb.clone();
    }
    let (w, idx) = resolve_wallet_and_account(shared, args)?;
    let from = evm_addr_for_account(&w, idx)?;

    if provider == "uniswap" {
        let sp = SwapEstimateParams {
            req_id,
            chain,
            token_in,
            token_out,
            amount_in,
            units,
            slippage_bps,
        };
        return estimate_swap_uniswap(sp, from, &evm).await;
    }

    if provider == "1inch" {
        let sp = SwapEstimateParams {
            req_id,
            chain,
            token_in,
            token_out,
            amount_in,
            units,
            slippage_bps,
        };
        return estimate_swap_1inch(sp, from, &evm, shared).await;
    }

    Ok(ok(
        req_id,
        tool_err(ToolError::new(
            "invalid_provider",
            "evm swaps must use provider=uniswap or provider=1inch",
        )),
    ))
}

async fn estimate_swap_uniswap(
    p: SwapEstimateParams<'_>,
    from: alloy::primitives::Address,
    evm: &EvmChain,
) -> eyre::Result<JsonRpcResponse> {
    let SwapEstimateParams {
        req_id,
        chain,
        token_in,
        token_out,
        amount_in,
        units,
        slippage_bps,
    } = p;
    let Some(u) = &evm.uniswap else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "uniswap_unavailable",
                "uniswap not configured for this chain",
            )),
        ));
    };
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

    let decimals_in = if native_in {
        18_u32
    } else {
        u32::from(evm.get_erc20_metadata(token_in_addr).await?.0)
    };
    let amt_in_u256: U256 = if units == "base" {
        crate::chains::evm::parse_u256_dec(amount_in)?
    } else {
        u128_to_u256(amount::parse_amount_ui_to_base_u128(
            amount_in,
            decimals_in,
        )?)
    };

    // Pick best fee tier.
    let fees = [500_u32, 3000_u32, 10_000_u32];
    let mut best = None;
    for fee in fees {
        let Ok(out) = evm
            .quote_uniswap_exact_in(token_in_addr, token_out_addr, amt_in_u256, fee)
            .await
        else {
            continue;
        };
        let is_better = best.as_ref().map_or(true, |(b, _)| out > *b);
        if is_better {
            best = Some((out, fee));
        }
    }
    let (out, fee) = best.ok_or_else(|| eyre::eyre!("no uniswap quote"))?;
    let min_out = out
        .checked_mul(U256::from(10_000_u64 - u64::from(slippage_bps)))
        .ok_or_else(|| eyre::eyre!("overflow"))?
        / U256::from(10_000_u64);

    let swap_req = crate::chains::evm::UniswapSwapRequest {
        from,
        token_in: token_in_addr,
        token_out: token_out_addr,
        amount_in: amt_in_u256,
        amount_out_min: min_out,
        fee,
        native_in,
        native_out,
    };
    let tx = evm.build_uniswap_swap_tx(&swap_req)?;
    let gas = evm.estimate_tx_gas(&tx).await?;
    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": chain, "provider": "uniswap", "gas": gas.to_string() })),
    ))
}

async fn estimate_swap_1inch(
    p: SwapEstimateParams<'_>,
    from: alloy::primitives::Address,
    evm: &EvmChain,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let SwapEstimateParams {
        req_id,
        chain,
        token_in,
        token_out,
        amount_in,
        units,
        slippage_bps,
    } = p;
    if shared
        .cfg
        .http
        .oneinch_api_key
        .as_ref()
        .map_or(true, |k| k.trim().is_empty())
    {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "missing_api_key",
                "1inch is optional and requires an API key (set http.oneinch_api_key in config.toml)",
            )),
        ));
    }

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

    let decimals_in = if native_in {
        18_u32
    } else {
        u32::from(evm.get_erc20_metadata(token_in_addr).await?.0)
    };
    let amt_in_u256: U256 = if units == "base" {
        crate::chains::evm::parse_u256_dec(amount_in)?
    } else {
        u128_to_u256(amount::parse_amount_ui_to_base_u128(
            amount_in,
            decimals_in,
        )?)
    };

    let (tx, _expected_out) = evm
        .oneinch_swap_tx(
            from,
            token_in_addr,
            token_out_addr,
            amt_in_u256,
            slippage_bps,
        )
        .await?;
    let gas = evm.estimate_tx_gas(&tx).await?;
    Ok(ok(
        req_id,
        tool_ok(json!({ "chain": chain, "provider": "1inch", "gas": gas.to_string() })),
    ))
}
