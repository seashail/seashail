use crate::policy::Policy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SOLANA_MAINNET_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
pub const SOLANA_DEVNET_RPC_URL: &str = "https://api.devnet.solana.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    /// Binance public API base URL (keyless). Used for USD prices.
    pub binance_base_url: String,
    /// Jupiter Swap API base URL. Used for Solana quotes and swaps.
    pub jupiter_base_url: String,
    /// Optional Jupiter API key (x-api-key). Some tiers/hosts require this; Seashail supports keyless usage
    /// where Jupiter permits it (typically with reduced rate limits).
    pub jupiter_api_key: Option<String>,
    /// 1inch Swap API v6 base URL. Note: 1inch currently requires an API key.
    pub oneinch_base_url: String,
    /// Optional 1inch API key. If unset, 1inch integration is disabled and swaps should use Uniswap.
    pub oneinch_api_key: Option<String>,

    /// Hyperliquid API base URL (mainnet).
    pub hyperliquid_base_url_mainnet: String,
    /// Hyperliquid API base URL (testnet).
    pub hyperliquid_base_url_testnet: String,

    /// Bitcoin HTTP API base URL (mainnet).
    ///
    /// Uses Blockstream-compatible endpoints (e.g. blockstream.info).
    pub bitcoin_api_base_url_mainnet: String,
    /// Bitcoin HTTP API base URL (testnet).
    pub bitcoin_api_base_url_testnet: String,

    /// Optional marketplace adapter base URL for Blur (EVM NFTs).
    ///
    /// This is intentionally a Seashail-owned "tx construction" endpoint (see docs). Seashail
    /// will fetch an on-chain tx envelope from this URL and then apply strict pre-sign checks,
    /// simulation, and forced confirmation before broadcasting.
    pub blur_adapter_base_url: Option<String>,
    /// Optional marketplace adapter base URL for Magic Eden (Solana NFTs).
    pub magic_eden_adapter_base_url: Option<String>,
    /// Optional marketplace adapter base URL for `OpenSea` (EVM NFTs).
    pub opensea_adapter_base_url: Option<String>,
    /// Optional `OpenSea` API key. If an `OpenSea` adapter base URL is configured, this key may be
    /// required depending on the adapter implementation.
    pub opensea_api_key: Option<String>,
    /// Optional marketplace adapter base URL for Tensor (Solana NFTs).
    pub tensor_adapter_base_url: Option<String>,

    /// Optional pump.fun adapter base URL.
    ///
    /// If set, Seashail can fetch discovery data and Solana tx envelopes from a loopback/https
    /// adapter endpoint.
    pub pumpfun_adapter_base_url: Option<String>,

    /// Optional `DeFi` adapter base URL.
    ///
    /// If set, Seashail can fetch tx envelopes for complex multi-step protocols (bridge/lending/
    /// staking/liquidity/prediction).
    pub defi_adapter_base_url: Option<String>,

    /// Polymarket API base URLs.
    ///
    /// These defaults allow users who import their Polymarket wallet to trade immediately without
    /// additional configuration.
    pub polymarket_clob_base_url: String,
    pub polymarket_data_base_url: String,
    pub polymarket_gamma_base_url: String,
    /// Geoblock host used by the Polymarket CLOB client (separate from the CLOB base URL).
    pub polymarket_geoblock_base_url: String,

    /// Wormholescan API base URL.
    ///
    /// Used to fetch signed VAAs for Wormhole transfers. Keyless by default.
    pub wormholescan_api_base_url: String,

    /// Kamino API base URL (Solana `DeFi`: Kamino Lend).
    ///
    /// Used for data endpoints (market/reserve discovery, obligations) and KTX transaction endpoints.
    pub kamino_api_base_url: String,
    /// Default Kamino Lend market pubkey to use when agents omit `market`.
    pub kamino_default_lend_market: String,

    /// Default Marginfi group pubkey to use when agents omit `group`.
    pub marginfi_default_group: String,

    /// Optional signed scam-address blocklist URL. If set, Seashail will
    /// periodically fetch and verify this blocklist, and block sends to listed recipients.
    ///
    /// Must be `https`, except `<http://localhost>` / `<http://127.0.0.1>` / `<http://[::1]>` for local testing.
    pub scam_blocklist_url: Option<String>,
    /// Optional expected Ed25519 pubkey for the scam blocklist (base64, 32 bytes).
    /// If unset, the envelope's pubkey is used.
    pub scam_blocklist_pubkey_b64: Option<String>,
    /// Refresh interval for scam blocklist fetches (seconds).
    pub scam_blocklist_refresh_seconds: u64,

    /// Optional OFAC SDN list URL. If set, Seashail will periodically fetch and cache
    /// the list and block transactions to listed addresses.
    pub ofac_sdn_url: Option<String>,
    /// Refresh interval for OFAC SDN fetches (seconds).
    pub ofac_sdn_refresh_seconds: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            binance_base_url: "https://api.binance.com".into(),
            // Jupiter consolidated quote+swap under /swap/v1. This base URL should end in /swap/v1.
            jupiter_base_url: "https://api.jup.ag/swap/v1".into(),
            jupiter_api_key: None,
            oneinch_base_url: "https://api.1inch.dev/swap/v6.0".into(),
            oneinch_api_key: None,

            hyperliquid_base_url_mainnet: "https://api.hyperliquid.xyz".into(),
            hyperliquid_base_url_testnet: "https://api.hyperliquid-testnet.xyz".into(),

            bitcoin_api_base_url_mainnet: "https://blockstream.info/api".into(),
            bitcoin_api_base_url_testnet: "https://blockstream.info/testnet/api".into(),

            blur_adapter_base_url: None,
            magic_eden_adapter_base_url: None,
            opensea_adapter_base_url: None,
            opensea_api_key: None,
            tensor_adapter_base_url: None,
            pumpfun_adapter_base_url: None,
            defi_adapter_base_url: None,

            polymarket_clob_base_url: "https://clob.polymarket.com".into(),
            polymarket_data_base_url: "https://data-api.polymarket.com".into(),
            polymarket_gamma_base_url: "https://gamma-api.polymarket.com".into(),
            polymarket_geoblock_base_url: "https://polymarket.com".into(),

            wormholescan_api_base_url: "https://api.wormholescan.io/v1".into(),

            kamino_api_base_url: "https://api.kamino.finance".into(),
            // Kamino examples/docs commonly use this as the main market.
            kamino_default_lend_market: "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF".into(),

            // Marginfi "Main" group (mainnet) used by the official app.
            marginfi_default_group: "4qp6Fx6Bg6s3d3f2mBSumNptAFjLJ23EWUeHuxS2a2k".into(),

            scam_blocklist_url: None,
            scam_blocklist_pubkey_b64: None,
            scam_blocklist_refresh_seconds: 6 * 60 * 60,

            ofac_sdn_url: None,
            ofac_sdn_refresh_seconds: 24 * 60 * 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcConfig {
    /// Solana RPC endpoint URL.
    pub solana_rpc_url: String,
    /// Additional Solana mainnet RPC endpoints to try if the primary fails.
    pub solana_fallback_rpc_urls_mainnet: Vec<String>,
    /// Additional Solana devnet RPC endpoints to try if the primary fails.
    pub solana_fallback_rpc_urls_devnet: Vec<String>,
    /// Optional default Solana compute unit limit for native-built transactions.
    ///
    /// If set, Seashail prepends a `ComputeBudget` `setComputeUnitLimit` instruction when it
    /// constructs Solana transactions locally (not for remote-provided tx bytes).
    pub solana_default_compute_unit_limit: Option<u32>,
    /// Optional default Solana compute unit price (priority fee) in micro-lamports per compute unit
    /// for native-built transactions.
    ///
    /// If set, Seashail prepends a `ComputeBudget` `setComputeUnitPrice` instruction when it
    /// constructs Solana transactions locally (not for remote-provided tx bytes).
    pub solana_default_compute_unit_price_micro_lamports: Option<u64>,
    /// EVM RPC endpoints keyed by chain name.
    pub evm_rpc_urls: BTreeMap<String, String>,
    /// EVM fallback RPC endpoints keyed by chain name.
    pub evm_fallback_rpc_urls: BTreeMap<String, Vec<String>>,
    /// EVM chain IDs keyed by chain name.
    pub evm_chain_ids: BTreeMap<String, u64>,
}

/// A single EVM chain definition used by the table-driven [`RpcConfig::default()`].
struct EvmChainDef {
    name: &'static str,
    rpc_url: &'static str,
    chain_id: u64,
    fallbacks: &'static [&'static str],
}

/// Insert all entries from a chain definition table into the RPC maps.
fn populate_evm_chains(
    table: &[EvmChainDef],
    urls: &mut BTreeMap<String, String>,
    fallbacks: &mut BTreeMap<String, Vec<String>>,
    ids: &mut BTreeMap<String, u64>,
) {
    for def in table {
        urls.insert(def.name.into(), def.rpc_url.into());
        ids.insert(def.name.into(), def.chain_id);
        fallbacks.insert(
            def.name.into(),
            def.fallbacks.iter().map(|&s| s.into()).collect(),
        );
    }
}

/// Default EVM mainnet chain definitions.
const EVM_MAINNETS: &[EvmChainDef] = &[
    EvmChainDef {
        name: "ethereum",
        rpc_url: "https://eth.llamarpc.com",
        chain_id: 1,
        fallbacks: &[
            "https://ethereum-rpc.publicnode.com",
            "https://rpc.ankr.com/eth",
            "https://cloudflare-eth.com",
        ],
    },
    EvmChainDef {
        name: "base",
        rpc_url: "https://base.llamarpc.com",
        chain_id: 8453,
        fallbacks: &[
            "https://mainnet.base.org",
            "https://base-rpc.publicnode.com",
            "https://rpc.ankr.com/base",
        ],
    },
    EvmChainDef {
        name: "arbitrum",
        rpc_url: "https://arbitrum.llamarpc.com",
        chain_id: 42161,
        fallbacks: &[
            "https://arb1.arbitrum.io/rpc",
            "https://arbitrum-rpc.publicnode.com",
            "https://rpc.ankr.com/arbitrum",
        ],
    },
    EvmChainDef {
        name: "optimism",
        rpc_url: "https://optimism.llamarpc.com",
        chain_id: 10,
        fallbacks: &[
            "https://mainnet.optimism.io",
            "https://optimism-rpc.publicnode.com",
            "https://rpc.ankr.com/optimism",
        ],
    },
    EvmChainDef {
        name: "polygon",
        rpc_url: "https://polygon.llamarpc.com",
        chain_id: 137,
        fallbacks: &[
            "https://polygon-rpc.com",
            "https://polygon-bor-rpc.publicnode.com",
            "https://rpc.ankr.com/polygon",
        ],
    },
    EvmChainDef {
        name: "bnb",
        rpc_url: "https://bsc.llamarpc.com",
        chain_id: 56,
        fallbacks: &[
            "https://bsc-dataseed.binance.org",
            "https://bsc-rpc.publicnode.com",
            "https://rpc.ankr.com/bsc",
        ],
    },
    EvmChainDef {
        name: "avalanche",
        rpc_url: "https://avalanche-c-chain.llamarpc.com",
        chain_id: 43114,
        fallbacks: &[
            "https://api.avax.network/ext/bc/C/rpc",
            "https://avalanche-c-chain-rpc.publicnode.com",
            "https://rpc.ankr.com/avalanche",
        ],
    },
    EvmChainDef {
        name: "monad",
        rpc_url: "https://rpc.monad.xyz",
        chain_id: 143,
        fallbacks: &[
            "https://monad-rpc.synergynodes.com",
            "https://143.rpc.thirdweb.com",
        ],
    },
];

/// Default EVM testnet chain definitions.
const EVM_TESTNETS: &[EvmChainDef] = &[
    EvmChainDef {
        name: "sepolia",
        rpc_url: "https://rpc.sepolia.org",
        chain_id: 11_155_111,
        fallbacks: &[
            "https://ethereum-sepolia-rpc.publicnode.com",
            "https://rpc.ankr.com/eth_sepolia",
        ],
    },
    EvmChainDef {
        name: "base-sepolia",
        rpc_url: "https://sepolia.base.org",
        chain_id: 84532,
        fallbacks: &[
            "https://base-sepolia-rpc.publicnode.com",
            "https://rpc.ankr.com/base_sepolia",
        ],
    },
    EvmChainDef {
        name: "arbitrum-sepolia",
        rpc_url: "https://sepolia-rollup.arbitrum.io/rpc",
        chain_id: 421_614,
        fallbacks: &[
            "https://arbitrum-sepolia-rpc.publicnode.com",
            "https://rpc.ankr.com/arbitrum_sepolia",
        ],
    },
    EvmChainDef {
        name: "optimism-sepolia",
        rpc_url: "https://sepolia.optimism.io",
        chain_id: 11_155_420,
        fallbacks: &[
            "https://optimism-sepolia-rpc.publicnode.com",
            "https://rpc.ankr.com/optimism_sepolia",
        ],
    },
    EvmChainDef {
        name: "polygon-amoy",
        rpc_url: "https://rpc-amoy.polygon.technology",
        chain_id: 80002,
        fallbacks: &[
            "https://polygon-amoy-bor-rpc.publicnode.com",
            "https://rpc.ankr.com/polygon_amoy",
        ],
    },
    EvmChainDef {
        name: "bnb-testnet",
        rpc_url: "https://data-seed-prebsc-1-s1.bnbchain.org:8545",
        chain_id: 97,
        fallbacks: &[
            "https://data-seed-prebsc-2-s1.bnbchain.org:8545",
            "https://data-seed-prebsc-1-s2.bnbchain.org:8545",
            "https://bsc-testnet-rpc.publicnode.com",
        ],
    },
    EvmChainDef {
        name: "avalanche-fuji",
        rpc_url: "https://api.avax-test.network/ext/bc/C/rpc",
        chain_id: 43113,
        fallbacks: &[
            "https://avalanche-fuji-c-chain-rpc.publicnode.com",
            "https://rpc.ankr.com/avalanche_fuji",
        ],
    },
    EvmChainDef {
        name: "monad-testnet",
        rpc_url: "https://testnet-rpc.monad.xyz",
        chain_id: 10143,
        fallbacks: &["https://10143.rpc.thirdweb.com"],
    },
];

impl Default for RpcConfig {
    fn default() -> Self {
        let mut evm_rpc_urls = BTreeMap::new();
        let mut evm_fallback_rpc_urls: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut evm_chain_ids = BTreeMap::new();

        populate_evm_chains(
            EVM_MAINNETS,
            &mut evm_rpc_urls,
            &mut evm_fallback_rpc_urls,
            &mut evm_chain_ids,
        );
        populate_evm_chains(
            EVM_TESTNETS,
            &mut evm_rpc_urls,
            &mut evm_fallback_rpc_urls,
            &mut evm_chain_ids,
        );

        Self {
            solana_rpc_url: SOLANA_MAINNET_RPC_URL.into(),
            solana_fallback_rpc_urls_mainnet: vec![
                "https://solana-rpc.publicnode.com".into(),
                "https://rpc.ankr.com/solana".into(),
                "https://solana.drpc.org".into(),
            ],
            solana_fallback_rpc_urls_devnet: vec!["https://rpc.ankr.com/solana_devnet".into()],
            solana_default_compute_unit_limit: None,
            solana_default_compute_unit_price_micro_lamports: None,
            evm_rpc_urls,
            evm_fallback_rpc_urls,
            evm_chain_ids,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SeashailConfig {
    pub policy: Policy,
    /// Optional per-wallet policy overrides keyed by wallet name.
    ///
    /// Tools accept `wallet` to view/update these overrides.
    #[serde(default)]
    pub policy_overrides_by_wallet: BTreeMap<String, Policy>,
    pub rpc: RpcConfig,
    pub http: HttpConfig,

    /// Network mode controls which chains are used by default (when a tool omits `chain`/`chains`)
    /// and provides agent-facing guidance. Chains can still be selected explicitly by name.
    ///
    /// This is the successor to `testnet_mode`. If unset, Seashail falls back to `testnet_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_mode: Option<NetworkMode>,

    /// Base64 salt used for Argon2id passphrase derivation (shared across wallets).
    /// Generated on first wallet creation.
    pub passphrase_salt_b64: Option<String>,

    /// Passphrase session lifetime (seconds). Default 30 minutes.
    pub passphrase_session_seconds: u64,

    /// Local price cache TTL for "native token USD" lookups (seconds).
    pub price_cache_ttl_seconds_native: u64,

    /// Local price cache TTL for token->USDC quote-based lookups (seconds).
    pub price_cache_ttl_seconds_quote: u64,

    /// Legacy. Prefer `network_mode`.
    #[serde(default, skip_serializing)]
    pub testnet_mode: bool,
}

impl Default for SeashailConfig {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            policy_overrides_by_wallet: BTreeMap::new(),
            rpc: RpcConfig::default(),
            http: HttpConfig::default(),
            network_mode: Some(NetworkMode::Mainnet),
            passphrase_salt_b64: None,
            passphrase_session_seconds: 30 * 60,
            price_cache_ttl_seconds_native: 30,
            price_cache_ttl_seconds_quote: 10,
            testnet_mode: false,
        }
    }
}

impl SeashailConfig {
    pub fn effective_network_mode(&self) -> NetworkMode {
        self.network_mode.unwrap_or(if self.testnet_mode {
            NetworkMode::Testnet
        } else {
            NetworkMode::Mainnet
        })
    }

    pub fn policy_for_wallet(&self, wallet: Option<&str>) -> (Policy, bool) {
        if let Some(w) = wallet.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(p) = self.policy_overrides_by_wallet.get(w) {
                return (p.clone(), true);
            }
        }
        (self.policy.clone(), false)
    }

    pub fn default_chains_for_mode(&self, mode: NetworkMode) -> Vec<String> {
        let mut out = vec!["solana".to_owned()];

        for k in self.rpc.evm_rpc_urls.keys() {
            let is_testnet = is_evm_testnet_chain_name(k);
            match mode {
                NetworkMode::Mainnet => {
                    if !is_testnet {
                        out.push(k.clone());
                    }
                }
                NetworkMode::Testnet => {
                    if is_testnet {
                        out.push(k.clone());
                    }
                }
            }
        }
        out
    }
}

pub fn is_evm_testnet_chain_name(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    n.contains("testnet")
        || n.contains("sepolia")
        || n.contains("goerli")
        || n.contains("holesky")
        || n.contains("mumbai")
        || n.contains("amoy")
        || n.contains("fuji")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_chains_respects_network_mode() {
        let cfg = SeashailConfig {
            network_mode: Some(NetworkMode::Mainnet),
            ..Default::default()
        };
        let mainnet = cfg.default_chains_for_mode(cfg.effective_network_mode());
        assert!(mainnet.contains(&"solana".to_owned()));
        assert!(mainnet.contains(&"ethereum".to_owned()));
        assert!(!mainnet.contains(&"sepolia".to_owned()));
        assert!(!mainnet.contains(&"arbitrum-sepolia".to_owned()));

        let cfg_testnet = SeashailConfig {
            network_mode: Some(NetworkMode::Testnet),
            ..Default::default()
        };
        let testnet = cfg_testnet.default_chains_for_mode(cfg_testnet.effective_network_mode());
        assert!(testnet.contains(&"solana".to_owned()));
        assert!(testnet.contains(&"sepolia".to_owned()));
        assert!(testnet.contains(&"base-sepolia".to_owned()));
        assert!(testnet.contains(&"arbitrum-sepolia".to_owned()));
        assert!(testnet.contains(&"optimism-sepolia".to_owned()));
        assert!(testnet.contains(&"polygon-amoy".to_owned()));
        assert!(testnet.contains(&"bnb-testnet".to_owned()));
        assert!(testnet.contains(&"avalanche-fuji".to_owned()));
        assert!(testnet.contains(&"monad-testnet".to_owned()));
        assert!(!testnet.contains(&"ethereum".to_owned()));
    }

    #[test]
    fn legacy_testnet_mode_is_respected_when_network_mode_unset() {
        let cfg = SeashailConfig {
            network_mode: None,
            testnet_mode: true,
            ..Default::default()
        };
        assert_eq!(cfg.effective_network_mode(), NetworkMode::Testnet);
    }
}
