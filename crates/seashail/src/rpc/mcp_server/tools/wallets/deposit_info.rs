use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{is_native_token, resolve_wallet_and_account};
use crate::{
    chains::{evm::EvmChain, solana::SolanaChain},
    config::NetworkMode,
    errors::ToolError,
};

struct DepositAddr {
    address: String,
    chain_kind: &'static str,
    chain_id: Option<u64>,
}

fn resolve_deposit_address(
    shared: &SharedState,
    conn: &ConnState,
    chain: &str,
    w: &crate::wallet::WalletRecord,
    idx: u32,
) -> Result<DepositAddr, Value> {
    if chain == "solana" {
        let Some(addr) = w.solana_addresses.get(idx as usize) else {
            return Err(tool_err(ToolError::new(
                "missing_address",
                "wallet has no Solana address for this account",
            )));
        };
        Ok(DepositAddr {
            address: addr.clone(),
            chain_kind: "solana",
            chain_id: None,
        })
    } else if chain == "bitcoin" {
        let mode = effective_network_mode(shared, conn);
        let addr = if mode == NetworkMode::Testnet {
            w.bitcoin_addresses_testnet.get(idx as usize)
        } else {
            w.bitcoin_addresses_mainnet.get(idx as usize)
        };
        let Some(addr) = addr else {
            return Err(tool_err(ToolError::new(
                "missing_address",
                "wallet has no Bitcoin address for this account",
            )));
        };
        Ok(DepositAddr {
            address: addr.clone(),
            chain_kind: "bitcoin",
            chain_id: None,
        })
    } else {
        if !shared.cfg.rpc.evm_rpc_urls.contains_key(chain) {
            return Err(tool_err(ToolError::new(
                "unsupported_chain",
                "unknown/unsupported chain (not configured)",
            )));
        }
        let Some(addr) = w.evm_addresses.get(idx as usize) else {
            return Err(tool_err(ToolError::new(
                "missing_address",
                "wallet has no EVM address for this account",
            )));
        };
        let chain_id = shared.cfg.rpc.evm_chain_ids.get(chain).copied();
        Ok(DepositAddr {
            address: addr.clone(),
            chain_kind: "evm",
            chain_id,
        })
    }
}

struct TokenInfo {
    kind: &'static str,
    symbol: Option<String>,
    identifier: Option<String>,
}

fn resolve_token_info(
    shared: &SharedState,
    conn: &ConnState,
    chain: &str,
    chain_kind: &str,
    token_hint: &str,
) -> TokenInfo {
    let mut identifier: Option<String> = None;
    let mut kind = if is_native_token(token_hint) {
        "native"
    } else if chain_kind == "bitcoin" {
        "unknown"
    } else if chain_kind == "solana" {
        "spl"
    } else {
        "erc20"
    };
    let mut symbol: Option<String> = None;

    if !is_native_token(token_hint) {
        if token_hint.eq_ignore_ascii_case("usdc") {
            symbol = Some("USDC".to_owned());
            if chain_kind == "solana" {
                let mode = effective_network_mode(shared, conn);
                identifier = Some(
                    if mode == NetworkMode::Mainnet {
                        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
                    } else {
                        "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
                    }
                    .to_owned(),
                );
            } else {
                identifier = super::evm_usdc_identifier(shared, chain);
            }
        } else if chain_kind == "solana" {
            if SolanaChain::parse_pubkey(token_hint).is_ok() {
                identifier = Some(token_hint.to_owned());
            } else {
                kind = "unknown";
            }
        } else if EvmChain::parse_address(token_hint).is_ok() {
            identifier = Some(token_hint.to_owned());
        } else {
            kind = "unknown";
        }
    } else if chain_kind == "bitcoin" {
        symbol = Some("BTC".to_owned());
    } else if chain_kind == "solana" {
        symbol = Some("SOL".to_owned());
    } else if chain == "polygon" {
        symbol = Some("POL".to_owned());
    } else if chain == "bnb" || chain == "bnb-testnet" {
        symbol = Some("BNB".to_owned());
    } else if chain == "avalanche" {
        symbol = Some("AVAX".to_owned());
    } else {
        symbol = Some("ETH".to_owned());
    }

    TokenInfo {
        kind,
        symbol,
        identifier,
    }
}

pub fn handle(
    req_id: Value,
    args: &Value,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let (w, idx) = resolve_wallet_and_account(shared, args)?;

    let chain_raw = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    let chain = if chain_raw.trim().is_empty() {
        shared
            .cfg
            .default_chains_for_mode(effective_network_mode(shared, conn))
            .into_iter()
            .next()
            .unwrap_or_else(|| "solana".to_owned())
    } else {
        chain_raw.trim().to_owned()
    };

    let token_raw = args
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("native");
    let token_hint = token_raw.trim();

    // Resolve a deposit address. This never requires unlocking; it uses cached public addresses.
    let deposit = match resolve_deposit_address(shared, conn, &chain, &w, idx) {
        Ok(d) => d,
        Err(tool_result) => return Ok(ok(req_id, tool_result)),
    };

    let token = resolve_token_info(shared, conn, &chain, deposit.chain_kind, token_hint);

    let mut warnings = vec![
        "Only send assets on the selected chain/network. Sending from the wrong chain may be unrecoverable.".to_owned(),
        "Always verify the address on both sides before sending.".to_owned(),
    ];
    if token_hint.eq_ignore_ascii_case("usdc") && token.identifier.is_none() {
        warnings.push("USDC contract/mint is chain-specific. If in doubt, paste the address into a trusted explorer and confirm the USDC token contract/mint before sending.".to_owned());
    }

    Ok(ok(
        req_id,
        tool_ok(json!({
          "wallet": w.name,
          "account_index": idx,
          "chain": chain,
          "chain_kind": deposit.chain_kind,
          "chain_id": deposit.chain_id,
          "asset": {
            "token": if token_hint.is_empty() { "native" } else { token_hint },
            "kind": token.kind,
            "symbol": token.symbol,
            "identifier": token.identifier,
          },
          "address": deposit.address,
          "warnings": warnings
        })),
    ))
}
