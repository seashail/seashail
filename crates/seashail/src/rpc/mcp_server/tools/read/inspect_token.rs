use crate::{
    chains::{evm::EvmChain, solana::SolanaChain},
    errors::ToolError,
};
use serde_json::{json, Value};
use solana_sdk::program_pack::Pack as _;
use spl_token::solana_program::program_option::COption;
use spl_token::state::Mint;

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{evm_native_symbol, is_native_token, solana_fallback_urls};

pub async fn handle(
    req_id: Value,
    args: Value,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    let token = args.get("token").and_then(|v| v.as_str()).unwrap_or("");
    if chain.is_empty() || token.is_empty() {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "inspect_token requires: chain, token",
            )),
        ));
    }

    if chain == "solana" {
        return inspect_solana(req_id, token, shared, conn).await;
    }

    inspect_evm(req_id, chain, token, shared).await
}

async fn inspect_solana(
    req_id: Value,
    token: &str,
    shared: &SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    if is_native_token(token) {
        return Ok(ok(
            req_id,
            tool_ok(json!({
              "chain": "solana",
              "token": "native",
              "kind": "native",
              "symbol": "SOL",
              "decimals": 9,
              "warnings": [],
            })),
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
    let Ok(mint_pk) = SolanaChain::parse_pubkey(token) else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "invalid Solana mint")),
        ));
    };

    let Ok(acc) = sol.get_account(&mint_pk).await else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("token_not_found", "mint account not found")),
        ));
    };

    let mut warnings: Vec<&'static str> = vec![];
    if acc.owner != spl_token::id() {
        warnings.push("mint_not_owned_by_spl_token_program");
    }

    let Ok(mint) = Mint::unpack(&acc.data) else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_token",
                "account is not a valid SPL mint",
            )),
        ));
    };

    let mint_authority = match mint.mint_authority {
        COption::Some(k) => Some(k.to_string()),
        COption::None => None,
    };
    let freeze_authority = match mint.freeze_authority {
        COption::Some(k) => Some(k.to_string()),
        COption::None => None,
    };
    if mint_authority.is_some() {
        warnings.push("mint_authority_present");
    }
    if freeze_authority.is_some() {
        warnings.push("freeze_authority_present");
    }
    if !mint.is_initialized {
        warnings.push("mint_not_initialized");
    }

    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": "solana",
          "token": token,
          "kind": "spl",
          "decimals": mint.decimals,
          "supply_base": mint.supply.to_string(),
          "mint_authority": mint_authority,
          "freeze_authority": freeze_authority,
          "warnings": warnings,
        })),
    ))
}

async fn inspect_evm(
    req_id: Value,
    chain: &str,
    token: &str,
    shared: &SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let Some(rpc_url) = shared.cfg.rpc.evm_rpc_urls.get(chain).cloned() else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("unsupported_chain", "unknown EVM chain")),
        ));
    };
    let Some(chain_id) = shared.cfg.rpc.evm_chain_ids.get(chain).copied() else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new("unsupported_chain", "missing EVM chain id")),
        ));
    };

    if is_native_token(token) {
        return Ok(ok(
            req_id,
            tool_ok(json!({
              "chain": chain,
              "token": "native",
              "kind": "native",
              "symbol": evm_native_symbol(chain),
              "decimals": 18,
              "warnings": [],
            })),
        ));
    }

    let Ok(token_addr) = EvmChain::parse_address(token) else {
        return Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "invalid EVM token address",
            )),
        ));
    };

    let mut evm = EvmChain::for_name(chain, chain_id, &rpc_url, &shared.cfg.http);
    if let Some(fb) = shared.cfg.rpc.evm_fallback_rpc_urls.get(chain) {
        evm.fallback_rpc_urls.clone_from(fb);
    }
    let code = evm.get_contract_code(token_addr).await?;
    if code.0.is_empty() {
        return Ok(ok(
            req_id,
            tool_ok(json!({
              "chain": chain,
              "token": token,
              "kind": "evm_address",
              "is_contract": false,
              "warnings": ["no_contract_code_at_address"],
            })),
        ));
    }

    let mut warnings: Vec<&'static str> = vec![];

    let impl_slot = EvmChain::eip1967_implementation_slot();
    let impl_val: alloy::primitives::B256 = evm.get_storage_at(token_addr, impl_slot).await?;
    let impl_bytes = impl_val.as_slice();
    let proxy_detected = impl_bytes.iter().any(|b| *b != 0);
    let implementation = proxy_detected.then(|| {
        let mut addr = [0_u8; 20];
        if let Some(b) = impl_bytes.get(12..32) {
            addr.copy_from_slice(b);
        }
        let impl_addr = alloy::primitives::Address::from(addr);
        warnings.push("eip1967_proxy_detected");
        format!("{impl_addr:#x}")
    });

    // Try to read ERC-20 metadata; if it fails, return best-effort contract info.
    if let Ok((decimals, symbol, name, supply)) = evm.get_erc20_details(token_addr).await {
        Ok(ok(
            req_id,
            tool_ok(json!({
              "chain": chain,
              "token": token,
              "kind": "erc20",
              "is_contract": true,
              "decimals": decimals,
              "symbol": symbol,
              "name": name,
              "total_supply_base": supply.to_string(),
              "proxy": { "eip1967_detected": proxy_detected, "implementation": implementation },
              "warnings": warnings,
            })),
        ))
    } else {
        warnings.push("erc20_metadata_unavailable");
        Ok(ok(
            req_id,
            tool_ok(json!({
              "chain": chain,
              "token": token,
              "kind": "evm_contract",
              "is_contract": true,
              "proxy": { "eip1967_detected": proxy_detected, "implementation": implementation },
              "warnings": warnings,
            })),
        ))
    }
}
