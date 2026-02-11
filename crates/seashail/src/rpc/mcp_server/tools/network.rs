use crate::{
    chains::evm::EvmChain,
    config::{NetworkMode, SOLANA_DEVNET_RPC_URL, SOLANA_MAINNET_RPC_URL},
    errors::ToolError,
};
use serde_json::{json, Value};

use super::super::jsonrpc::{err, ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::state::{effective_network_mode, network_mode_str, parse_network_mode};
use super::super::{ConnState, SharedState};
use super::helpers::{oneinch_supported_chain, solana_fallback_urls};

fn handle_get_network_mode(
    req_id: Value,
    shared: &SharedState,
    conn: &ConnState,
) -> JsonRpcResponse {
    ok(
        req_id,
        tool_ok(json!({
          "effective": network_mode_str(effective_network_mode(shared, conn)),
          "configured": shared.cfg.network_mode.map(network_mode_str),
          "legacy_testnet_mode": shared.cfg.testnet_mode,
          "solana_rpc_url": shared.cfg.rpc.solana_rpc_url,
        })),
    )
}

fn handle_set_network_mode(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let mode_s = args.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    match parse_network_mode(mode_s) {
        None => Ok(ok(
            req_id,
            tool_err(ToolError::new(
                "invalid_request",
                "mode must be one of: mainnet, testnet",
            )),
        )),
        Some(mode) => {
            let apply_default_solana_rpc = args
                .get("apply_default_solana_rpc")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            shared.cfg.network_mode = Some(mode);
            shared.cfg.testnet_mode = mode == NetworkMode::Testnet;

            if apply_default_solana_rpc {
                if mode == NetworkMode::Testnet
                    && shared.cfg.rpc.solana_rpc_url == SOLANA_MAINNET_RPC_URL
                {
                    shared.cfg.rpc.solana_rpc_url = SOLANA_DEVNET_RPC_URL.into();
                }
                if mode == NetworkMode::Mainnet
                    && shared.cfg.rpc.solana_rpc_url == SOLANA_DEVNET_RPC_URL
                {
                    shared.cfg.rpc.solana_rpc_url = SOLANA_MAINNET_RPC_URL.into();
                }
            }

            shared.ks.save_config(&shared.cfg)?;
            Ok(ok(
                req_id,
                tool_ok(json!({
                "ok": true,
                "mode": network_mode_str(mode),
                "solana_rpc_url": shared.cfg.rpc.solana_rpc_url,
                  })),
            ))
        }
    }
}

/// Helper to check if an optional string config is non-empty.
fn opt_configured(opt: Option<&String>) -> bool {
    opt.is_some_and(|s| !s.trim().is_empty())
}

/// Marketplace adapter availability flags.
struct MarketplaceFlags {
    blur: bool,
    magic_eden: bool,
    opensea: bool,
}

/// API key and adapter configuration flags for services.
struct ServiceFlags {
    opensea_api_key: bool,
    tensor_adapter: bool,
    pumpfun_adapter: bool,
}

/// Defi and trading integration flags.
struct DefiServiceFlags {
    jupiter_api_key: bool,
    oneinch: bool,
    defi_adapter: bool,
}

/// Top-level config flags split into sub-groups to avoid excessive booleans.
struct ConfigFlags {
    marketplace: MarketplaceFlags,
    services: ServiceFlags,
    defi: DefiServiceFlags,
    polymarket: bool,
}

fn collect_config_flags(shared: &SharedState) -> ConfigFlags {
    let h = &shared.cfg.http;
    ConfigFlags {
        marketplace: MarketplaceFlags {
            blur: opt_configured(h.blur_adapter_base_url.as_ref()),
            magic_eden: opt_configured(h.magic_eden_adapter_base_url.as_ref()),
            opensea: opt_configured(h.opensea_adapter_base_url.as_ref()),
        },
        services: ServiceFlags {
            opensea_api_key: opt_configured(h.opensea_api_key.as_ref()),
            tensor_adapter: opt_configured(h.tensor_adapter_base_url.as_ref()),
            pumpfun_adapter: opt_configured(h.pumpfun_adapter_base_url.as_ref()),
        },
        defi: DefiServiceFlags {
            jupiter_api_key: opt_configured(h.jupiter_api_key.as_ref()),
            oneinch: opt_configured(h.oneinch_api_key.as_ref()),
            defi_adapter: opt_configured(h.defi_adapter_base_url.as_ref()),
        },
        polymarket: !h.polymarket_clob_base_url.trim().is_empty()
            && !h.polymarket_data_base_url.trim().is_empty()
            && !h.polymarket_gamma_base_url.trim().is_empty()
            && !h.polymarket_geoblock_base_url.trim().is_empty(),
    }
}

fn collect_evm_chains(shared: &SharedState, oneinch_configured: bool) -> Vec<Value> {
    let mut evm_chains = vec![];
    for (chain, url) in &shared.cfg.rpc.evm_rpc_urls {
        let chain_id = *shared.cfg.rpc.evm_chain_ids.get(chain).unwrap_or(&0);
        let mut evm = EvmChain::for_name(chain, chain_id, url, &shared.cfg.http);
        if let Some(fb) = shared.cfg.rpc.evm_fallback_rpc_urls.get(chain) {
            evm.fallback_rpc_urls.clone_from(fb);
        }
        evm_chains.push(json!({
          "chain": chain,
          "chain_id": chain_id,
          "rpc_url": url,
          "supports": {
            "send_transaction": true,
            "swap_uniswap": evm.uniswap.is_some(),
            "swap_1inch": oneinch_supported_chain(chain) && oneinch_configured,
            "nft_transfer": true,
            "nft_marketplace_tx_envelope": true
          }
        }));
    }
    evm_chains
}

fn build_services_json(shared: &SharedState, f: &ConfigFlags) -> Value {
    let solana_rpc_configured = !shared.cfg.rpc.solana_rpc_url.trim().is_empty();
    json!({
        "binance": { "requires_api_key": false, "configured": true },
        "hyperliquid": { "requires_api_key": false, "configured": true, "notes": "Hyperliquid perps are supported via their public API + signed actions." },
        "polymarket": {
          "requires_api_key": false,
          "configured": f.polymarket,
          "base_urls": {
            "clob": shared.cfg.http.polymarket_clob_base_url,
            "data": shared.cfg.http.polymarket_data_base_url,
            "gamma": shared.cfg.http.polymarket_gamma_base_url,
            "geoblock": shared.cfg.http.polymarket_geoblock_base_url
          },
          "notes": "Polymarket is generally permissionless. Trading may be geo-blocked from some locations (including the US) based on IP/jurisdiction."
        },
        "jupiter": {
          "requires_api_key": false,
          "optional_api_key_supported": true,
          "api_key_configured": f.defi.jupiter_api_key,
          "notes": "Jupiter hosts/tiers vary: some allow keyless usage (often with reduced rate limits) and some require an x-api-key. Configure http.jupiter_api_key if you hit rate limits or get 401/403 responses."
        },
        "uniswap": { "requires_api_key": false, "configured": true },
        "oneinch": { "requires_api_key": true, "configured": f.defi.oneinch },
        "scam_blocklist": {
          "opt_in": true,
          "configured": shared.cfg.http.scam_blocklist_url.as_ref().is_some_and(|u| !u.trim().is_empty()),
          "pubkey_pinned": shared.cfg.http.scam_blocklist_pubkey_b64.as_ref().is_some_and(|k| !k.trim().is_empty()),
          "refresh_seconds": shared.cfg.http.scam_blocklist_refresh_seconds
        },
        "nft_marketplace_adapters": {
          "blur": { "configured": f.marketplace.blur },
          "magic_eden": { "configured": f.marketplace.magic_eden },
          "opensea": { "configured": f.marketplace.opensea, "requires_api_key": true, "api_key_configured": f.services.opensea_api_key },
          "tensor": { "configured": f.services.tensor_adapter }
        },
        "pumpfun": {
          "configured": f.services.pumpfun_adapter || solana_rpc_configured,
          "notes": "pump.fun discovery works via Solana RPC by default. Execution (buy/sell) uses an optional adapter endpoint for tx envelopes (https/loopback only)."
        },
        "wormholescan": {
          "configured": !shared.cfg.http.wormholescan_api_base_url.trim().is_empty(),
          "base_url": shared.cfg.http.wormholescan_api_base_url,
          "notes": "Wormhole bridging uses Wormholescan to fetch signed VAAs (keyless) for status and best-effort redemption."
        },
        "defi_adapter": {
          "configured": f.defi.defi_adapter,
          "notes": "DeFi surfaces can execute agent-supplied tx envelopes. Some protocols also have native handlers (Aave v3 lending; Wormhole bridging) that do not require the adapter."
        },
        "ofac_sdn": {
          "configured": shared.cfg.http.ofac_sdn_url.as_ref().is_some_and(|u| !u.trim().is_empty()),
          "refresh_seconds": shared.cfg.http.ofac_sdn_refresh_seconds,
          "notes": "If configured and enabled by policy, Seashail blocks transactions to OFAC SDN-listed addresses."
        }
    })
}

fn build_surfaces_json(f: &ConfigFlags, solana_rpc_configured: bool) -> Value {
    json!({
        "spot": { "swap_tokens": true, "send_transaction": true },
        "perps": { "hyperliquid": true, "jupiter_perps": true },
        "prediction_markets": {
          "polymarket": { "execution": true, "positions": true, "configured": f.polymarket }
        },
        "nfts": {
          "inventory": true,
          "transfer": true,
          "marketplace_tx_envelope": true,
          "marketplace_adapters": {
            "blur": f.marketplace.blur,
            "magic_eden": f.marketplace.magic_eden,
            "opensea": f.marketplace.opensea && f.services.opensea_api_key,
            "tensor": f.services.tensor_adapter
          }
        },
        "pumpfun": {
          "discovery": f.services.pumpfun_adapter || solana_rpc_configured,
          "execution": f.services.pumpfun_adapter
        },
        "defi": {
          "bridge": { "execution": true, "status": true, "native_wormhole": true, "adapter_configured": f.defi.defi_adapter },
          "lending": { "execution": true, "positions": true, "native_aave_v3": true, "adapter_configured": f.defi.defi_adapter },
          "staking": { "execution": true, "native_lido": true, "native_jito": true, "adapter_configured": f.defi.defi_adapter },
          "liquidity": { "execution": true, "adapter_configured": f.defi.defi_adapter },
          "prediction": { "execution": true, "positions": true, "provider": "polymarket", "configured": f.polymarket }
        },
        "portfolio_analytics": {
          "history": true,
          "totals": true,
          "by_chain": true,
          "by_type": true,
          "by_day": true
        }
    })
}

fn build_chains_json(
    shared: &SharedState,
    effective: crate::config::NetworkMode,
    evm_chains: &[Value],
) -> Value {
    json!({
        "solana": {
          "rpc_url": shared.cfg.rpc.solana_rpc_url,
          "fallback_rpc_urls": solana_fallback_urls(shared, effective),
          "supports": {
            "send_transaction": true,
            "swap_jupiter": true,
            "perps_jupiter_perps": true,
            "nft_inventory": true,
            "nft_transfer": true,
            "nft_marketplace_tx_envelope": true,
            "request_airdrop": true
          }
        },
        "bitcoin": {
          "api_base_url_mainnet": shared.cfg.http.bitcoin_api_base_url_mainnet,
          "api_base_url_testnet": shared.cfg.http.bitcoin_api_base_url_testnet,
          "supports": {
            "get_balance": true,
            "send_transaction": true,
            "get_deposit_info": true
          },
          "notes": "Bitcoin support uses Blockstream-compatible HTTP endpoints. Seashail requires explicit user confirmation for Bitcoin sends."
        },
        "evm": evm_chains
    })
}

fn handle_get_capabilities(
    req_id: Value,
    shared: &SharedState,
    conn: &ConnState,
) -> JsonRpcResponse {
    let effective = effective_network_mode(shared, conn);
    let configured = shared.cfg.network_mode.map(network_mode_str);
    let f = collect_config_flags(shared);
    let evm_chains = collect_evm_chains(shared, f.defi.oneinch);
    let solana_rpc_configured = !shared.cfg.rpc.solana_rpc_url.trim().is_empty();

    ok(
        req_id,
        tool_ok(json!({
          "network_mode": {
            "effective": network_mode_str(effective),
            "configured": configured,
          },
          "services": build_services_json(shared, &f),
          "surfaces": build_surfaces_json(&f, solana_rpc_configured),
          "rpc_defaults": {
            "solana_mainnet": SOLANA_MAINNET_RPC_URL,
            "solana_devnet": SOLANA_DEVNET_RPC_URL
          },
          "chains": build_chains_json(shared, effective, &evm_chains),
          "kyc_wallets": {
            "imported_wallets_supported": true,
            "note": "Seashail can import existing keys/mnemonics for KYC-gated platforms, but KYC/identity verification is always handled outside Seashail."
          }
        })),
    )
}

fn handle_get_testnet_faucet_links(req_id: Value, args: &Value) -> JsonRpcResponse {
    let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    if chain.is_empty() {
        return ok(
            req_id,
            tool_err(ToolError::new("invalid_request", "missing chain")),
        );
    }
    let address = args.get("address").and_then(|v| v.as_str()).unwrap_or("");

    // Keep this list small and conservative. Prefer official landing pages over
    // third-party/aggregator sites to reduce phishing risk.
    let faucets: Option<Vec<Value>> = match chain {
        "solana" => vec![json!({
          "name": "Solana Faucet (Devnet/Testnet)",
          "url": "https://faucet.solana.com/",
          "notes": "For devnet/testnet wallets. For local validators/devnet/testnet you can also use Seashail `request_airdrop`."
        })]
        .into(),
        "sepolia" => vec![json!({
          "name": "Coinbase Developer Platform (Sepolia Faucet)",
          "url": "https://docs.cdp.coinbase.com/faucets/docs/welcome",
          "notes": "Follow the Sepolia faucet instructions on the CDP page."
        })]
        .into(),
        "base-sepolia" => vec![json!({
          "name": "Coinbase Developer Platform (Base Sepolia Faucet)",
          "url": "https://docs.cdp.coinbase.com/faucets/docs/welcome",
          "notes": "Follow the Base Sepolia faucet instructions on the CDP page."
        })]
        .into(),
        "arbitrum-sepolia" => vec![
            json!({
              "name": "Coinbase Developer Platform (Sepolia Faucet)",
              "url": "https://docs.cdp.coinbase.com/faucets/docs/welcome",
              "notes": "Get Sepolia ETH, then bridge to Arbitrum Sepolia if needed."
            }),
            json!({
              "name": "Arbitrum Bridge",
              "url": "https://bridge.arbitrum.io/",
              "notes": "Use the Arbitrum bridge to move Sepolia ETH to Arbitrum Sepolia."
            }),
        ]
        .into(),
        "optimism-sepolia" => vec![json!({
          "name": "Optimism Faucet (Superchain)",
          "url": "https://console.optimism.io/faucets",
          "notes": "Official faucet for OP Sepolia and other Superchain testnets."
        })]
        .into(),
        "polygon-amoy" => vec![json!({
          "name": "Polygon Faucet (Amoy)",
          "url": "https://faucet.polygon.technology/",
          "notes": "Select Amoy and request testnet POL for the provided address."
        })]
        .into(),
        "bnb-testnet" => vec![json!({
          "name": "BNB Chain Testnet Faucet",
          "url": "https://www.bnbchain.org/en/testnet-faucet",
          "notes": "Request BNB testnet funds for the provided address."
        })]
        .into(),
        "avalanche-fuji" => vec![json!({
          "name": "Avalanche Faucet (Fuji)",
          "url": "https://faucet.avax.network/",
          "notes": "Request AVAX on Fuji for the provided address."
        })]
        .into(),
        "monad-testnet" => vec![json!({
          "name": "Monad Testnet Faucet",
          "url": "https://faucet.monad.xyz/",
          "notes": "Request MON testnet funds for the provided address."
        })]
        .into(),
        _ => None,
    };

    match faucets {
        None => ok(
            req_id,
            tool_err(ToolError::new(
                "unsupported_chain",
                "unsupported chain for faucet links",
            )),
        ),
        Some(faucets) => ok(
            req_id,
            tool_ok(json!({
              "chain": chain,
              "address": if address.is_empty() { Value::Null } else { Value::String(address.to_owned()) },
              "faucets": faucets,
              "warnings": [
                "Only use official faucet links. Avoid ads and lookalike domains.",
                "Never paste private keys, mnemonics, or Shamir shares into any website.",
                "Faucets are rate-limited and may require captcha or account verification."
              ]
            })),
        ),
    }
}

fn handle_configure_rpc(
    req_id: Value,
    args: &Value,
    shared: &mut SharedState,
) -> eyre::Result<JsonRpcResponse> {
    let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let fallback_urls = args
        .get("fallback_urls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_owned()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<String>>()
        });
    let mode = args
        .get("mode")
        .and_then(|v| v.as_str())
        .and_then(parse_network_mode);

    shared
        .ks
        .configure_rpc(&mut shared.cfg, chain, url, fallback_urls, mode)?;
    Ok(ok(req_id, tool_ok(json!({ "ok": true }))))
}

pub fn handle(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &ConnState,
) -> eyre::Result<JsonRpcResponse> {
    let args_ref = &args;
    let resp: eyre::Result<JsonRpcResponse> = match tool_name {
        "get_network_mode" => Ok(handle_get_network_mode(req_id, shared, conn)),
        "set_network_mode" => handle_set_network_mode(req_id, args_ref, shared),
        "get_capabilities" => Ok(handle_get_capabilities(req_id, shared, conn)),
        "get_testnet_faucet_links" => Ok(handle_get_testnet_faucet_links(req_id, args_ref)),
        "configure_rpc" => handle_configure_rpc(req_id, args_ref, shared),
        _ => Ok(err(req_id, -32601, "unknown tool")),
    };

    drop(args);
    resp
}
