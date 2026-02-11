use serde_json::{json, Value};

fn network_tool_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "get_network_mode", "description": "Get current network mode (mainnet or testnet). This affects default chain selection when tools omit `chain`/`chains`.", "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false } }),
        json!({ "name": "set_network_mode", "description": "Set network mode (mainnet or testnet) and persist to config.toml. Optionally also switches Solana RPC between the default mainnet and devnet endpoints.", "inputSchema": {
          "type": "object",
          "properties": {
            "mode": { "type": "string", "enum": ["mainnet", "testnet"] },
            "apply_default_solana_rpc": { "type": "boolean", "default": true }
          },
          "required": ["mode"],
          "additionalProperties": false
        }}),
        json!({ "name": "get_capabilities", "description": "Describe supported chains, protocol surfaces, and which optional integrations require API keys or extra configuration.", "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false } }),
        json!({ "name": "get_testnet_faucet_links", "description": "Get official faucet links for supported testnets (no keys required). This is informational only.", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string", "description": "solana, sepolia, base-sepolia, arbitrum-sepolia, optimism-sepolia, polygon-amoy, bnb-testnet, avalanche-fuji, monad-testnet" },
            "address": { "type": "string", "description": "Optional address to paste into faucet sites." }
          },
          "required": ["chain"],
          "additionalProperties": false
        }}),
        json!({ "name": "configure_rpc", "description": "Set a custom RPC endpoint for a chain.", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string" },
            "url": { "type": "string", "minLength": 1, "description": "Primary RPC URL." },
            "fallback_urls": { "type": "array", "items": { "type": "string", "minLength": 1 }, "description": "Optional fallback RPC URLs to try if the primary fails." },
            "mode": { "type": "string", "enum": ["mainnet", "testnet"], "description": "For Solana only: which network's fallback list to update. If omitted, uses the effective network mode." }
          },
          "required": ["chain", "url"],
          "additionalProperties": false
        }}),
    ]
}

fn read_token_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "inspect_token", "description": "Read-only token inspection (authorities/decimals/supply) to help evaluate risk for newly launched tokens.", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string", "description": "solana or an EVM chain name." },
            "token": { "type": "string", "description": "native, or a token mint (Solana) / contract address (EVM)." }
          },
          "required": ["chain", "token"],
          "additionalProperties": false
        }}),
        json!({ "name": "get_defi_yield_pools", "description": "Fetch and filter DeFi yield pool metadata (best-effort) for agent research. Read-only; does not execute transactions.", "inputSchema": {
          "type": "object",
          "properties": {
            "chains": { "type": "array", "items": { "type": "string" }, "description": "Optional chain filter (matches upstream chain names, e.g. Ethereum, Arbitrum, Base, Solana)." },
            "query": { "type": "string", "description": "Optional substring filter applied to project/symbol (case-insensitive)." },
            "min_tvl_usd": { "type": "number", "minimum": 0, "default": 10_000_000, "description": "Filter out small pools." },
            "min_apy": { "type": "number", "default": 0, "description": "Minimum APY (percent)." },
            "stablecoin_only": { "type": "boolean", "default": true, "description": "If true, only include pools flagged as stablecoin-focused by the upstream dataset." },
            "exclude_il_risk": { "type": "boolean", "default": true, "description": "If true, exclude pools with IL risk flagged by the upstream dataset." },
            "max_results": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "get_token_price", "description": "Current USD price estimate for a token.", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string", "description": "solana or an EVM chain name." },
            "token": { "type": "string", "description": "native, or a token mint (Solana) / contract address (EVM)." }
          },
          "required": ["chain", "token"],
          "additionalProperties": false
        }}),
    ]
}

fn read_portfolio_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "get_balance", "description": "Query token balances for a wallet.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string", "description": "solana, bitcoin, or an EVM chain name (ethereum, base, arbitrum, optimism, polygon, bnb, avalanche, monad, sepolia, base-sepolia, arbitrum-sepolia, optimism-sepolia, polygon-amoy, bnb-testnet, avalanche-fuji, monad-testnet). If omitted, returns default chains based on network mode." },
            "tokens": { "type": "array", "items": { "type": "string" }, "description": "Optional token addresses/mints to query. If omitted, returns native + a small default set." }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "get_portfolio", "description": "Aggregate portfolio view with USD values across all wallets.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallets": { "type": "array", "items": { "type": "string" }, "description": "Optional list of wallet names. If omitted, uses all wallets." },
            "chains": { "type": "array", "items": { "type": "string" }, "description": "Optional list of chains. If omitted, uses default chains." },
            "tokens": { "type": "object", "description": "Optional per-chain token list to include in addition to native. Keys are chain names. Values are arrays of token identifiers (Solana mint or EVM contract address).", "additionalProperties": { "type": "array", "items": { "type": "string" } } },
            "include_history": { "type": "boolean", "default": false, "description": "If true, persist a portfolio snapshot and return recent snapshot totals + simple P&L deltas." },
            "history_limit": { "type": "integer", "minimum": 1, "maximum": 365, "default": 30, "description": "Number of snapshot totals to return when include_history=true." },
            "include_health": { "type": "boolean", "default": false, "description": "If true, attach latest persisted position/health snapshots from monitoring surfaces (perps/lending/prediction) when available." }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "estimate_gas", "description": "Estimate gas/fees for a proposed send or swap.", "inputSchema": {
          "type": "object",
          "properties": {
            "op": { "type": "string", "enum": ["send_transaction", "swap_tokens"] },
            "chain": { "type": "string" },
            "to": { "type": "string" },
            "token": { "type": "string" },
            "amount": { "type": "string" },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
            "token_in": { "type": "string" },
            "token_out": { "type": "string" },
            "amount_in": { "type": "string" },
            "slippage_bps": { "type": "integer", "minimum": 0, "maximum": 5000, "default": 100 },
            "provider": { "type": "string", "enum": ["auto", "jupiter", "uniswap", "1inch"], "default": "auto" }
          },
          "required": ["op", "chain"],
          "additionalProperties": false
        }}),
        json!({ "name": "get_transaction_history", "description": "Return locally tracked transaction history (with optional filtering).", "inputSchema": {
          "type": "object",
          "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 50 },
            "wallet": { "type": "string", "description": "Optional wallet name to filter." },
            "chain": { "type": "string", "description": "Optional chain name to filter (solana, ethereum, base, ...)." },
            "type": { "type": "string", "description": "Optional event type to filter (send, swap, approve, airdrop, wallet_created, wallet_imported)." },
            "since_ts": { "type": "string", "description": "Optional RFC3339 timestamp (inclusive)." },
            "until_ts": { "type": "string", "description": "Optional RFC3339 timestamp (inclusive)." }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "get_portfolio_analytics", "description": "Portfolio analytics computed from local transaction history: totals and USD volume breakdowns by type, chain, and day.", "inputSchema": {
          "type": "object",
          "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 500 },
            "wallet": { "type": "string", "description": "Optional wallet name to filter." },
            "chain": { "type": "string", "description": "Optional chain name to filter." },
            "type": { "type": "string", "description": "Optional event type to filter." },
            "since_ts": { "type": "string", "description": "Optional RFC3339 timestamp (inclusive)." },
            "until_ts": { "type": "string", "description": "Optional RFC3339 timestamp (inclusive)." },
            "snapshot_scope": { "type": "object", "description": "Optional scope for snapshot-based P&L, matching get_portfolio(include_history=true). If omitted, Seashail derives a best-effort scope from wallet/chain and default chains for the current network mode.", "properties": {
              "wallets": { "type": "array", "items": { "type": "string" } },
              "chains": { "type": "array", "items": { "type": "string" } }
            }, "additionalProperties": false }
          },
          "additionalProperties": false
        }}),
    ]
}

fn read_defi_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "get_lending_positions", "description": "Read-only lending/borrowing positions. Supports native reads for Aave v3 (EVM), Compound v3/Comet (EVM), Kamino (Solana), and Marginfi (Solana). Falls back to a configured DeFi adapter for other protocols.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string", "description": "solana or an EVM chain name." },
            "protocol": { "type": "string", "enum": ["auto", "aave", "compound", "kamino", "marginfi"], "description": "Optional protocol hint. If auto, defaults by chain (EVM: aave; Solana: kamino).", "default": "auto" },
            "pool_address": { "type": "string", "description": "Optional override for Aave v3 Pool address (useful for local testing/mocks)." },
            "comet_address": { "type": "string", "description": "Compound v3: optional override for the Comet market address. If omitted, Seashail uses a per-chain default for the USDC market when available." },
            "market": { "type": "string", "description": "Kamino: optional market pubkey override (defaults to http.kamino_default_lend_market)." },
            "group": { "type": "string", "description": "Marginfi: optional group pubkey override (defaults to http.marginfi_default_group)." },
            "marginfi_account": { "type": "string", "description": "Marginfi: optional marginfi_account pubkey to read. If omitted, Seashail uses the most recently created Marginfi account for this wallet/account (best-effort)." }
          },
          "required": ["chain"],
          "additionalProperties": false
        }}),
        json!({ "name": "get_bridge_status", "description": "Read-only bridge status. Wormhole uses Wormholescan (keyless) when bridge_id is a wormhole id; other providers use a configured DeFi adapter.", "inputSchema": {
          "type": "object",
          "properties": {
            "bridge_id": { "type": "string", "minLength": 1, "description": "Bridge operation id (use the initiating tx signature/txid)." },
            "bridge_provider": { "type": "string", "enum": ["wormhole", "layerzero"], "default": "wormhole" },
            "include_vaa_bytes": { "type": "boolean", "default": false, "description": "If true, include signed VAA bytes (base64) when available." }
          },
          "required": ["bridge_id"],
          "additionalProperties": false
        }}),
        json!({ "name": "pumpfun_list_new_coins", "description": "List recent pump.fun launches. Read-only.", "inputSchema": {
          "type": "object",
          "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20 },
            "program_id": { "type": "string", "minLength": 1, "description": "Optional Solana program id to scan when using the RPC fallback. Defaults to the pump.fun program id." }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "pumpfun_get_coin_info", "description": "Fetch pump.fun coin details. Read-only.", "inputSchema": {
          "type": "object",
          "properties": {
            "mint": { "type": "string", "minLength": 1 },
            "program_id": { "type": "string", "minLength": 1, "description": "Optional pump.fun program id override when using the RPC fallback. Defaults to the mainnet program id." }
          },
          "required": ["mint"],
          "additionalProperties": false
        }}),
    ]
}

fn read_tool_schemas() -> Vec<Value> {
    let mut schemas = read_token_schemas();
    schemas.extend(read_portfolio_schemas());
    schemas.extend(read_defi_schemas());
    schemas
}

fn wallet_tool_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "get_policy", "description": "View current transaction policy configuration. If `wallet` is provided, returns the effective policy for that wallet (global default or wallet override).", "inputSchema": {
          "type": "object",
          "properties": { "wallet": { "type": "string", "description": "Optional wallet name to view the effective policy for." } },
          "additionalProperties": false
        }}),
        json!({ "name": "update_policy", "description": "Update transaction policy rules. If `wallet` is provided, updates that wallet's policy override; otherwise updates the global default policy. Use `clear=true` to remove a wallet override.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "Optional wallet name to update the policy for." },
            "policy": { "type": "object", "description": "Policy object. Required unless `clear=true`." },
            "clear": { "type": "boolean", "default": false, "description": "If true, remove the wallet override policy. Only valid when `wallet` is provided." }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "list_wallets", "description": "List all wallets with names, types, accounts, and cached addresses. Seashail maintains a generated 'default' wallet; on first run it may be created the first time you call a wallet-dependent tool.", "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false } }),
        json!({ "name": "get_wallet_info", "description": "Get details for a specific wallet (cached public addresses only). Seashail maintains a generated 'default' wallet; on first run it may be created on-demand. For funding, prefer get_deposit_info (deposit address).", "inputSchema": {
          "type": "object",
          "properties": { "wallet": { "type": "string", "description": "If omitted, returns the active wallet." } },
          "additionalProperties": false
        }}),
        json!({ "name": "get_deposit_info", "description": "Get a deposit address for a wallet on a specific chain (address-only; no QR). If the generated 'default' wallet does not exist yet (fresh install), Seashail may create it on-demand before returning an address.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string", "description": "Chain to deposit on (solana, ethereum, base, arbitrum, optimism, polygon, bnb, avalanche, sepolia, base-sepolia, bnb-testnet). If omitted, uses the default chain for the current network mode." },
            "token": { "type": "string", "description": "Optional token hint for display (native, USDC, or a mint/contract address). This does not change the deposit address." }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "set_active_wallet", "description": "Set the active wallet and account index.", "inputSchema": {
          "type": "object",
          "properties": { "wallet": { "type": "string" }, "account_index": { "type": "integer", "minimum": 0 } },
          "required": ["wallet", "account_index"],
          "additionalProperties": false
        }}),
        json!({ "name": "add_account", "description": "Add a new account index to an existing BIP-44 wallet.", "inputSchema": {
          "type": "object",
          "properties": { "wallet": { "type": "string" } },
          "required": ["wallet"],
          "additionalProperties": false
        }}),
        json!({ "name": "create_wallet_pool", "description": "Create N managed spending accounts (new account indexes) under an existing wallet root. Requires passphrase unlock.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "count": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Number of new accounts to create." }
          },
          "required": ["count"],
          "additionalProperties": false
        }}),
        json!({ "name": "create_wallet", "description": "Create a generated wallet (Shamir 2-of-3). Requires user confirmation.", "inputSchema": {
          "type": "object",
          "properties": { "name": { "type": "string", "minLength": 1 } },
          "required": ["name"],
          "additionalProperties": false
        }}),
        json!({ "name": "import_wallet", "description": "Import an existing private key or mnemonic. Requires user confirmation. The secret is always requested via an interactive prompt (not via tool arguments).", "inputSchema": {
          "type": "object",
          "properties": {
            "name": { "type": "string", "minLength": 1 },
            "kind": { "type": "string", "enum": ["private_key", "mnemonic"] },
            "private_key_chain": { "type": "string", "enum": ["evm", "solana"] },
            "secret": { "type": "string", "description": "Deprecated. Leave unset; Seashail will prompt for the secret via an elicitation form.", "minLength": 1 }
          },
          "required": ["name", "kind"],
          "additionalProperties": false
        }}),
        json!({ "name": "export_shares", "description": "Export Shamir shares for a generated wallet (share2 + share3). In quickstart mode, Seashail can auto-unlock without prompting.", "inputSchema": {
          "type": "object",
          "properties": { "wallet": { "type": "string" } },
          "required": ["wallet"],
          "additionalProperties": false
        }}),
        json!({ "name": "rotate_shares", "description": "Regenerate all Shamir shares for a generated wallet. Requires passphrase.", "inputSchema": {
          "type": "object",
          "properties": { "wallet": { "type": "string" } },
          "required": ["wallet"],
          "additionalProperties": false
        }}),
    ]
}

fn prediction_tool_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "get_prediction_positions", "description": "Read-only Polymarket positions. Uses the Polymarket Data API (keyless read).", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string", "description": "EVM chain name (e.g. polygon).", "default": "polygon" },
            "protocol": { "type": "string", "enum": ["polymarket"], "description": "Protocol.", "default": "polymarket" }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "search_prediction_markets", "description": "Search Polymarket events/markets via the Polymarket Gamma API (keyless). Returns outcomes and CLOB token_id(s) for trading.", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string", "description": "polygon (or polygon-amoy for testing).", "default": "polygon" },
            "protocol": { "type": "string", "enum": ["polymarket"], "default": "polymarket" },
            "query": { "type": "string", "minLength": 1, "description": "Search query." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Max results per type." },
            "page": { "type": "integer", "minimum": 1, "default": 1 },
            "include_closed": { "type": "boolean", "default": false, "description": "If true, include closed markets in results." }
          },
          "required": ["query"],
          "additionalProperties": false
        }}),
        json!({ "name": "get_prediction_orderbook", "description": "Fetch Polymarket CLOB orderbook for a specific outcome token_id (keyless read).", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string", "description": "polygon (or polygon-amoy for testing).", "default": "polygon" },
            "protocol": { "type": "string", "enum": ["polymarket"], "default": "polymarket" },
            "token_id": { "type": "string", "minLength": 1, "description": "Polymarket outcome token id (CLOB token_id). Accepts decimal or 0x-prefixed hex." }
          },
          "required": ["token_id"],
          "additionalProperties": false
        }}),
    ]
}

fn perp_read_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "get_market_data", "description": "Read-only market data for perpetual venues (Hyperliquid and Jupiter Perps, best-effort).", "inputSchema": {
          "type": "object",
          "properties": {
            "provider": { "type": "string", "enum": ["hyperliquid", "jupiter_perps"], "default": "hyperliquid" },
            "market": { "type": "string", "description": "Optional market/coin symbol (e.g. BTC). If omitted, returns all markets." }
          },
          "additionalProperties": false
        }}),
        json!({ "name": "get_positions", "description": "Read-only open positions for perpetual venues (Hyperliquid and Jupiter Perps, best-effort).", "inputSchema": {
          "type": "object",
          "properties": {
            "provider": { "type": "string", "enum": ["hyperliquid", "jupiter_perps"], "default": "hyperliquid" },
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." }
          },
          "additionalProperties": false
        }}),
    ]
}

fn perp_write_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "open_perp_position", "description": "Open a perpetual position on Hyperliquid or Jupiter Perps (market requests only). Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "provider": { "type": "string", "enum": ["hyperliquid", "jupiter_perps"], "default": "hyperliquid" },
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "market": { "type": "string", "description": "Coin/market symbol (e.g. BTC)." },
            "side": { "type": "string", "enum": ["long", "short"] },
            "size": { "type": "string", "description": "Position size (USD or asset units depending on size_units)." },
            "size_units": { "type": "string", "enum": ["usd", "asset"], "default": "usd" },
            "leverage": { "type": "integer", "minimum": 1, "default": 1 },
            "order_type": { "type": "string", "enum": ["market", "limit"], "default": "market" },
            "limit_px": { "type": "string", "description": "Required when order_type=limit." },
            "slippage_bps": { "type": "integer", "minimum": 0, "maximum": 5000, "default": 50, "description": "Used for market orders as an aggressive limit price." }
          },
          "required": ["market", "side", "size"],
          "additionalProperties": false
        }}),
        json!({ "name": "close_perp_position", "description": "Close an existing perpetual position on Hyperliquid or Jupiter Perps. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "provider": { "type": "string", "enum": ["hyperliquid", "jupiter_perps"], "default": "hyperliquid" },
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "market": { "type": "string", "description": "Coin/market symbol (e.g. BTC)." },
            "side": { "type": "string", "enum": ["long", "short"], "description": "Optional hint; close direction is derived from current position." },
            "size": { "type": "string", "description": "Optional close size (USD or asset units). If omitted, closes full position." },
            "size_units": { "type": "string", "enum": ["usd", "asset"], "default": "asset" },
            "slippage_bps": { "type": "integer", "minimum": 0, "maximum": 5000, "default": 50 }
          },
          "required": ["market"],
          "additionalProperties": false
        }}),
        json!({ "name": "modify_perp_order", "description": "Modify a perp order on Hyperliquid (implemented as cancel + new order). Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "provider": { "type": "string", "enum": ["hyperliquid"], "default": "hyperliquid" },
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "oid": { "type": "integer", "minimum": 0, "description": "Order id to cancel." },
            "market": { "type": "string", "description": "Coin/market symbol (e.g. BTC)." },
            "side": { "type": "string", "enum": ["long", "short"] },
            "size": { "type": "string" },
            "size_units": { "type": "string", "enum": ["usd", "asset"], "default": "usd" },
            "leverage": { "type": "integer", "minimum": 1, "default": 1 },
            "limit_px": { "type": "string" }
          },
          "required": ["oid", "market", "side", "size", "limit_px"],
          "additionalProperties": false
        }}),
        json!({ "name": "place_limit_order", "description": "Place a limit order on a perp venue (Hyperliquid). Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "provider": { "type": "string", "enum": ["hyperliquid"], "default": "hyperliquid" },
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "market": { "type": "string", "description": "Coin/market symbol (e.g. BTC)." },
            "side": { "type": "string", "enum": ["long", "short"] },
            "size": { "type": "string" },
            "size_units": { "type": "string", "enum": ["usd", "asset"], "default": "usd" },
            "leverage": { "type": "integer", "minimum": 1, "default": 1 },
            "limit_px": { "type": "string" }
          },
          "required": ["market", "side", "size", "limit_px"],
          "additionalProperties": false
        }}),
    ]
}

fn perp_prediction_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "place_prediction", "description": "Place a Polymarket CLOB order. Order is constructed and signed locally; execution is via the Polymarket CLOB API. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string" },
            "account_index": { "type": "integer", "minimum": 0 },
            "chain": { "type": "string", "description": "polygon (or polygon-amoy for testing)." },
            "protocol": { "type": "string", "enum": ["polymarket"], "default": "polymarket" },
            "token_id": { "type": "string", "description": "Polymarket outcome token id (CLOB token_id). Accepts decimal or 0x-prefixed hex." },
            "side": { "type": "string", "enum": ["buy", "sell"] },
            "order_kind": { "type": "string", "enum": ["limit", "market"], "default": "limit", "description": "Order type. Limit requires price+size; market requires amount_usdc." },
            "price": { "type": "string", "description": "Limit price in USD per share (0.01 tick size typical)." },
            "size": { "type": "string", "description": "Limit size in shares." },
            "amount_usdc": { "type": "string", "description": "Market order notional in USDC." },
            "time_in_force": { "type": "string", "enum": ["gtc", "gtd", "fok", "fak"], "default": "gtc" },
            "post_only": { "type": "boolean", "default": false },
            "usd_value": { "type": "number" },
            "usd_value_known": { "type": "boolean", "default": false }
          },
          "required": ["chain", "token_id", "side"],
          "additionalProperties": false
        }}),
        json!({ "name": "close_prediction", "description": "Cancel an existing Polymarket CLOB order by order_id. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string" },
            "account_index": { "type": "integer", "minimum": 0 },
            "chain": { "type": "string", "description": "polygon (or polygon-amoy for testing)." },
            "protocol": { "type": "string", "enum": ["polymarket"], "default": "polymarket" },
            "order_id": { "type": "string", "minLength": 1, "description": "Polymarket CLOB order id to cancel." },
            "usd_value": { "type": "number" },
            "usd_value_known": { "type": "boolean", "default": false }
          },
          "required": ["chain", "order_id"],
          "additionalProperties": false
        }}),
    ]
}

fn perp_tool_schemas() -> Vec<Value> {
    let mut schemas = perp_read_schemas();
    schemas.extend(perp_write_schemas());
    schemas.extend(perp_prediction_schemas());
    schemas
}

fn nft_tool_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "get_nft_inventory", "description": "Read-only NFT inventory for a wallet/account. Supported chains: solana.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string", "description": "solana or an EVM chain name (limited support)." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "default": 200 }
          },
          "required": ["chain"],
          "additionalProperties": false
        }}),
        json!({ "name": "transfer_nft", "description": "Transfer an NFT (Solana mints; EVM ERC-721 safeTransferFrom). Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string" },
            "to": { "type": "string" },
            "mint": { "type": "string", "description": "Solana mint (required when chain=solana)." },
            "contract": { "type": "string", "description": "EVM contract address (required for EVM chains)." },
            "token_id": { "type": "string", "description": "EVM token id (required for EVM chains). Decimal string." }
          },
          "required": ["chain", "to"],
          "additionalProperties": false
        }}),
        json!({ "name": "buy_nft", "description": "Buy an NFT via a marketplace by executing a transaction envelope (agent-supplied or fetched from a configured marketplace adapter). EVM: to/data/value_wei; Solana: tx_b64 + allowed_program_ids. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string" },
            "marketplace": { "type": "string", "enum": ["blur", "magic_eden", "opensea", "tensor"] },
            "usd_value": { "type": "number", "description": "Best-effort USD value for policy enforcement. If omitted, treated as unknown." },
            "usd_value_known": { "type": "boolean", "default": false, "description": "Optional override. If usd_value is provided, this is treated as true." },
            "to": { "type": "string", "description": "EVM: transaction recipient/contract address." },
            "data": { "type": "string", "description": "EVM: 0x-prefixed calldata. Use 0x for empty." },
            "value_wei": { "type": "string", "description": "EVM: value in wei as a decimal string.", "default": "0" },
            "tx_b64": { "type": "string", "description": "Solana: base64-encoded VersionedTransaction bytes (unsigned; Seashail will sign)." },
            "allowed_program_ids": { "type": "array", "items": { "type": "string" }, "description": "Solana: allowlist of program IDs; every instruction program id must be in this list." },
            "asset": { "type": "object", "description": "Marketplace-specific asset identifier passed to marketplace adapters when fetching a tx envelope. You may also include tx_b64/to/data/value_wei fields inside this object." }
          },
          "required": ["chain", "marketplace"],
          "additionalProperties": false
        }}),
        json!({ "name": "sell_nft", "description": "Sell an NFT via a marketplace by executing a transaction envelope (agent-supplied or fetched from a configured marketplace adapter). EVM: to/data/value_wei; Solana: tx_b64 + allowed_program_ids. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string" },
            "marketplace": { "type": "string", "enum": ["blur", "magic_eden", "opensea", "tensor"] },
            "usd_value": { "type": "number", "description": "Best-effort USD value for policy enforcement. If omitted, treated as unknown." },
            "usd_value_known": { "type": "boolean", "default": false, "description": "Optional override. If usd_value is provided, this is treated as true." },
            "to": { "type": "string", "description": "EVM: transaction recipient/contract address." },
            "data": { "type": "string", "description": "EVM: 0x-prefixed calldata. Use 0x for empty." },
            "value_wei": { "type": "string", "description": "EVM: value in wei as a decimal string.", "default": "0" },
            "tx_b64": { "type": "string", "description": "Solana: base64-encoded VersionedTransaction bytes (unsigned; Seashail will sign)." },
            "allowed_program_ids": { "type": "array", "items": { "type": "string" }, "description": "Solana: allowlist of program IDs; every instruction program id must be in this list." },
            "asset": { "type": "object", "description": "Marketplace-specific asset identifier passed to marketplace adapters when fetching a tx envelope. You may also include tx_b64/to/data/value_wei fields inside this object." }
          },
          "required": ["chain", "marketplace"],
          "additionalProperties": false
        }}),
        json!({ "name": "bid_nft", "description": "Place a bid/offer for an NFT via a marketplace by executing a transaction envelope (agent-supplied or fetched from a configured marketplace adapter). EVM: to/data/value_wei; Solana: tx_b64 + allowed_program_ids. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string" },
            "marketplace": { "type": "string", "enum": ["blur", "magic_eden", "opensea", "tensor"] },
            "usd_value": { "type": "number", "description": "Best-effort USD value for policy enforcement. If omitted, treated as unknown." },
            "usd_value_known": { "type": "boolean", "default": false, "description": "Optional override. If usd_value is provided, this is treated as true." },
            "to": { "type": "string", "description": "EVM: transaction recipient/contract address." },
            "data": { "type": "string", "description": "EVM: 0x-prefixed calldata. Use 0x for empty." },
            "value_wei": { "type": "string", "description": "EVM: value in wei as a decimal string.", "default": "0" },
            "tx_b64": { "type": "string", "description": "Solana: base64-encoded VersionedTransaction bytes (unsigned; Seashail will sign)." },
            "allowed_program_ids": { "type": "array", "items": { "type": "string" }, "description": "Solana: allowlist of program IDs; every instruction program id must be in this list." },
            "asset": { "type": "object", "description": "Marketplace-specific asset identifier passed to marketplace adapters when fetching a tx envelope. You may also include tx_b64/to/data/value_wei fields inside this object." }
          },
          "required": ["chain", "marketplace"],
          "additionalProperties": false
        }}),
    ]
}

fn write_spot_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "request_airdrop", "description": "Request a Solana airdrop (devnet/testnet/local validators only).", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string", "enum": ["solana"], "default": "solana" },
            "address": { "type": "string", "description": "If omitted, uses the wallet's Solana address for the selected account." },
            "amount": { "type": "string", "description": "Amount of SOL to request." },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui", "description": "ui = SOL, base = lamports." }
          },
          "required": ["amount"],
          "additionalProperties": false
        }}),
        json!({ "name": "send_transaction", "description": "Transfer native tokens, ERC-20, or SPL tokens. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string" },
            "to": { "type": "string" },
            "token": { "type": "string", "description": "native (default) or token mint/contract address." },
            "amount": { "type": "string" },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" }
          },
          "required": ["chain", "to", "amount"],
          "additionalProperties": false
        }}),
        json!({ "name": "swap_tokens", "description": "Execute a token swap via Jupiter (Solana) or Uniswap/1inch (EVM). Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "chain": { "type": "string" },
            "token_in": { "type": "string" },
            "token_out": { "type": "string" },
            "amount_in": { "type": "string" },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
            "slippage_bps": { "type": "integer", "minimum": 0, "maximum": 5000, "default": 100 },
            "provider": { "type": "string", "enum": ["auto", "jupiter", "uniswap", "1inch"], "default": "auto" }
          },
          "required": ["chain", "token_in", "token_out", "amount_in"],
          "additionalProperties": false
        }}),
        json!({ "name": "transfer_between_wallets", "description": "Transfer tokens between Seashail-managed wallets/accounts. Internal transfers are policy-exempt by default.", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string", "description": "solana or an EVM chain name." },
            "token": { "type": "string", "description": "native (default) or token mint/contract address." },
            "amount": { "type": "string" },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
            "from_wallet": { "type": "string" },
            "from_account_index": { "type": "integer", "minimum": 0 },
            "to_wallet": { "type": "string" },
            "to_account_index": { "type": "integer", "minimum": 0 }
          },
          "required": ["chain", "amount", "from_wallet", "from_account_index", "to_wallet", "to_account_index"],
          "additionalProperties": false
        }}),
        json!({ "name": "fund_wallets", "description": "Distribute funds from one managed wallet/account to many managed wallets/accounts. Internal transfers are policy-exempt by default.", "inputSchema": {
          "type": "object",
          "properties": {
            "chain": { "type": "string", "description": "solana or an EVM chain name." },
            "token": { "type": "string", "description": "native (default) or token mint/contract address." },
            "amount_each": { "type": "string", "description": "Amount to send to each destination." },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
            "from_wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "from_account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the wallet's last active account index." },
            "destinations": { "type": "array", "items": { "type": "object", "properties": { "wallet": { "type": "string" }, "account_index": { "type": "integer", "minimum": 0 } }, "required": ["wallet", "account_index"], "additionalProperties": false } }
          },
          "required": ["chain", "amount_each", "destinations"],
          "additionalProperties": false
        }}),
        json!({ "name": "pumpfun_buy", "description": "Buy a pump.fun coin. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "mint": { "type": "string", "minLength": 1, "description": "Token mint (identity-checked)." },
            "amount_sol": { "type": "number", "minimum": 0, "description": "Exact SOL amount to spend." }
          },
          "required": ["mint", "amount_sol"],
          "additionalProperties": false
        }}),
        json!({ "name": "pumpfun_sell", "description": "Sell a pump.fun coin. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
            "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
            "mint": { "type": "string", "minLength": 1, "description": "Token mint (identity-checked)." },
            "percent": { "type": "number", "minimum": 0, "maximum": 100, "description": "Percent of holdings to sell." }
          },
          "required": ["mint", "percent"],
          "additionalProperties": false
        }}),
    ]
}

fn write_defi_bridge_schema() -> Value {
    json!({ "name": "bridge_tokens", "description": "Bridge tokens cross-chain. Native Wormhole token bridge execution on EVM and Solana when to_chain+token+amount are provided; otherwise falls back to a tx envelope. Requires policy approval.", "inputSchema": {
      "type": "object",
      "properties": {
        "wallet": { "type": "string", "description": "If omitted, uses the active wallet." },
        "account_index": { "type": "integer", "minimum": 0, "description": "If omitted, uses the active account index." },
        "chain": { "type": "string", "description": "Execution chain (solana or an EVM chain name)." },
        "bridge_provider": { "type": "string", "enum": ["wormhole", "layerzero"], "default": "wormhole", "description": "Bridge provider selection." },
        "to_chain": { "type": "string", "description": "Destination chain name (EVM chain name or solana)." },
        "token": { "type": "string", "description": "Token identifier. EVM execution chain: ERC-20 contract address. Solana execution chain: SPL mint pubkey." },
        "amount": { "type": "string", "description": "Amount to bridge (string). amount=max is not supported for Wormhole native path." },
        "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui", "description": "Amount units (ui uses token decimals; base uses raw base units)." },
        "recipient": { "type": "string", "description": "Optional recipient. EVM->EVM or Solana->EVM: EVM address. EVM->Solana: Solana owner pubkey (Seashail derives the ATA for the wrapped mint). Defaults to the same Seashail wallet on the destination chain." },
        "recipient_token_account": { "type": "string", "description": "EVM->Solana only: optional override for the exact destination SPL token account pubkey (must match the token account address embedded in the VAA). If omitted, Seashail uses the derived ATA." },
        "redeem": { "type": "boolean", "default": true, "description": "If true (default), Seashail will best-effort fetch the signed VAA and redeem on destination (requires destination fees: EVM gas or SOL)." },
        "token_bridge_address": { "type": "string", "description": "Optional override for source Wormhole token bridge contract (useful for local testing/mocks)." },
        "to_token_bridge_address": { "type": "string", "description": "Optional override for destination Wormhole token bridge contract (useful for local testing/mocks)." },
        "usd_value": { "type": "number", "description": "Best-effort USD value for policy enforcement. If omitted, treated as unknown." },
        "usd_value_known": { "type": "boolean", "default": false, "description": "Optional override. If usd_value is provided, this is treated as true." },
        "to": { "type": "string", "description": "EVM: transaction recipient/contract address." },
        "data": { "type": "string", "description": "EVM: 0x-prefixed calldata. Use 0x for empty." },
        "value_wei": { "type": "string", "description": "EVM: value in wei as a decimal string.", "default": "0" },
        "tx_b64": { "type": "string", "description": "Solana: base64-encoded VersionedTransaction bytes (unsigned; Seashail will sign)." },
        "allowed_program_ids": { "type": "array", "items": { "type": "string" }, "description": "Solana: allowlist of program IDs; every instruction program id must be in this list." },
        "asset": { "type": "object", "description": "Protocol-specific request object used by an optional adapter to construct a tx envelope." }
      },
      "required": ["chain"],
      "additionalProperties": false
    }})
}

fn schema_lend_tokens() -> Value {
    json!({ "name": "lend_tokens", "description": "Lend/supply tokens to a lending protocol. Native execution supported for EVM Aave v3 and EVM Compound v3 (Comet), and Solana Kamino/Marginfi when native params are provided; otherwise falls back to a tx envelope. Requires policy approval.", "inputSchema": {
      "type": "object",
      "properties": {
        "wallet": { "type": "string" },
        "account_index": { "type": "integer", "minimum": 0 },
        "chain": { "type": "string" },
        "protocol": { "type": "string", "enum": ["aave", "compound", "kamino", "marginfi"], "description": "Protocol selection. If omitted, defaults by chain (EVM: aave; Solana: kamino)." },
        "token": { "type": "string", "description": "Token identifier. EVM: ERC-20 contract address to supply. Solana: SPL mint pubkey." },
        "amount": { "type": "string", "description": "Amount to supply (string). For EVM native Aave/Compound paths, amount=max is not supported." },
        "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui", "description": "Amount units (ui uses token decimals; base uses raw base units)." },
        "pool_address": { "type": "string", "description": "Optional override for Aave v3 Pool address (useful for local testing/mocks)." },
        "comet_address": { "type": "string", "description": "Compound v3: optional override for the Comet market address. If omitted, Seashail uses a per-chain default for the USDC market when available." },
        "market": { "type": "string", "description": "Kamino: optional market pubkey override (defaults to http.kamino_default_lend_market)." },
        "group": { "type": "string", "description": "Marginfi: optional group pubkey override (defaults to http.marginfi_default_group)." },
        "usd_value": { "type": "number" },
        "usd_value_known": { "type": "boolean", "default": false },
        "to": { "type": "string" },
        "data": { "type": "string" },
        "value_wei": { "type": "string", "default": "0" },
        "tx_b64": { "type": "string" },
        "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
        "asset": { "type": "object" }
      },
      "required": ["chain"],
      "additionalProperties": false
    }})
}

fn schema_withdraw_lending() -> Value {
    json!({ "name": "withdraw_lending", "description": "Withdraw supplied tokens from a lending protocol. Native execution supported for EVM Aave v3 and EVM Compound v3 (Comet), and Solana Kamino/Marginfi when native params are provided; otherwise falls back to a tx envelope. Requires policy approval.", "inputSchema": {
      "type": "object",
      "properties": {
        "wallet": { "type": "string" },
        "account_index": { "type": "integer", "minimum": 0 },
        "chain": { "type": "string" },
        "protocol": { "type": "string", "enum": ["aave", "compound", "kamino", "marginfi"], "description": "Protocol selection. If omitted, defaults by chain (EVM: aave; Solana: kamino)." },
        "token": { "type": "string", "description": "Token identifier. EVM: ERC-20 contract address to withdraw. Solana: SPL mint pubkey." },
        "amount": { "type": "string", "description": "Amount to withdraw (string). For Aave, amount=max is supported but requires usd_value for policy. For Compound native path, amount=max is not supported." },
        "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
        "pool_address": { "type": "string", "description": "Optional override for Aave v3 Pool address (useful for local testing/mocks)." },
        "comet_address": { "type": "string", "description": "Compound v3: optional override for the Comet market address. If omitted, Seashail uses a per-chain default for the USDC market when available." },
        "market": { "type": "string", "description": "Kamino: optional market pubkey override (defaults to http.kamino_default_lend_market)." },
        "group": { "type": "string", "description": "Marginfi: optional group pubkey override (defaults to http.marginfi_default_group)." },
        "usd_value": { "type": "number" },
        "usd_value_known": { "type": "boolean", "default": false },
        "to": { "type": "string" },
        "data": { "type": "string" },
        "value_wei": { "type": "string", "default": "0" },
        "tx_b64": { "type": "string" },
        "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
        "asset": { "type": "object" }
      },
      "required": ["chain"],
      "additionalProperties": false
    }})
}

fn schema_borrow_tokens() -> Value {
    json!({ "name": "borrow_tokens", "description": "Borrow tokens from a lending protocol. Native execution supported for EVM Aave v3 and EVM Compound v3 (Comet) when native params are provided; otherwise falls back to a tx envelope. Requires policy approval.", "inputSchema": {
      "type": "object",
      "properties": {
        "wallet": { "type": "string" },
        "account_index": { "type": "integer", "minimum": 0 },
        "chain": { "type": "string" },
        "protocol": { "type": "string", "enum": ["aave", "compound", "kamino", "marginfi"], "description": "Protocol selection. If omitted, defaults by chain (EVM: aave; Solana: kamino)." },
        "token": { "type": "string", "description": "EVM: ERC-20 token contract address to borrow. For Compound v3 native path, this must be the market base token; you may pass token=\"base\" to mean baseToken()." },
        "amount": { "type": "string", "description": "Amount to borrow (string). For EVM native Aave/Compound paths, amount=max is not supported." },
        "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
        "interest_rate_mode": { "type": "string", "enum": ["variable", "stable"], "default": "variable", "description": "Aave interest rate mode for borrow/repay." },
        "pool_address": { "type": "string", "description": "Optional override for Aave v3 Pool address (useful for local testing/mocks)." },
        "comet_address": { "type": "string", "description": "Compound v3: optional override for the Comet market address. If omitted, Seashail uses a per-chain default for the USDC market when available." },
        "market": { "type": "string", "description": "Kamino: optional market pubkey override (defaults to http.kamino_default_lend_market)." },
        "group": { "type": "string", "description": "Marginfi: optional group pubkey override (defaults to http.marginfi_default_group)." },
        "usd_value": { "type": "number" },
        "usd_value_known": { "type": "boolean", "default": false },
        "to": { "type": "string" },
        "data": { "type": "string" },
        "value_wei": { "type": "string", "default": "0" },
        "tx_b64": { "type": "string" },
        "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
        "asset": { "type": "object" }
      },
      "required": ["chain"],
      "additionalProperties": false
    }})
}

fn schema_repay_borrow() -> Value {
    json!({ "name": "repay_borrow", "description": "Repay a borrow on a lending protocol. Native execution supported for EVM Aave v3 and EVM Compound v3 (Comet) when native params are provided; otherwise falls back to a tx envelope. Requires policy approval.", "inputSchema": {
      "type": "object",
      "properties": {
        "wallet": { "type": "string" },
        "account_index": { "type": "integer", "minimum": 0 },
        "chain": { "type": "string" },
        "protocol": { "type": "string", "enum": ["aave", "compound", "kamino", "marginfi"], "description": "Protocol selection. If omitted, defaults by chain (EVM: aave; Solana: kamino)." },
        "token": { "type": "string", "description": "EVM: ERC-20 token contract address to repay. For Compound v3 native path, this must be the market base token; you may pass token=\"base\" to mean baseToken()." },
        "amount": { "type": "string", "description": "Amount to repay (string). For Aave, amount=max is supported but requires usd_value for policy. For Compound native path, amount=max is not supported." },
        "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
        "interest_rate_mode": { "type": "string", "enum": ["variable", "stable"], "default": "variable", "description": "Aave interest rate mode for borrow/repay." },
        "pool_address": { "type": "string", "description": "Optional override for Aave v3 Pool address (useful for local testing/mocks)." },
        "comet_address": { "type": "string", "description": "Compound v3: optional override for the Comet market address. If omitted, Seashail uses a per-chain default for the USDC market when available." },
        "market": { "type": "string", "description": "Kamino: optional market pubkey override (defaults to http.kamino_default_lend_market)." },
        "group": { "type": "string", "description": "Marginfi: optional group pubkey override (defaults to http.marginfi_default_group)." },
        "usd_value": { "type": "number" },
        "usd_value_known": { "type": "boolean", "default": false },
        "to": { "type": "string" },
        "data": { "type": "string" },
        "value_wei": { "type": "string", "default": "0" },
        "tx_b64": { "type": "string" },
        "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
        "asset": { "type": "object" }
      },
      "required": ["chain"],
      "additionalProperties": false
    }})
}

fn write_defi_lending_schemas() -> Vec<Value> {
    vec![
        schema_lend_tokens(),
        schema_withdraw_lending(),
        schema_borrow_tokens(),
        schema_repay_borrow(),
    ]
}

fn write_defi_staking_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "stake_tokens", "description": "Stake/yield action. Native staking supported for Ethereum Lido and Solana Jito when `amount` is provided; otherwise executes a tx envelope. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string" },
            "account_index": { "type": "integer", "minimum": 0 },
            "chain": { "type": "string" },
            "protocol": { "type": "string", "enum": ["lido", "eigenlayer", "marinade", "jito"], "description": "Protocol selection. If omitted, defaults by chain (EVM: lido; Solana: jito)." },
            "token": { "type": "string", "description": "Token to stake. EVM Lido: native ETH (use token=native). Solana Jito: native SOL (token=native)." },
            "amount": { "type": "string", "description": "Amount to stake (string). Native staking paths require this." },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
            "slippage_bps": { "type": "integer", "minimum": 0, "maximum": 2000, "default": 100, "description": "Solana only (Jito via Jupiter swap): slippage in basis points." },
            "usd_value": { "type": "number" },
            "usd_value_known": { "type": "boolean", "default": false },
            "to": { "type": "string" },
            "data": { "type": "string" },
            "value_wei": { "type": "string", "default": "0" },
            "tx_b64": { "type": "string" },
            "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
            "asset": { "type": "object" }
          },
          "required": ["chain"],
          "additionalProperties": false
        }}),
        json!({ "name": "unstake_tokens", "description": "Unstake/yield action. Native unstaking supported for Ethereum Lido (withdrawal request) and Solana Jito when `amount` is provided; otherwise executes a tx envelope. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string" },
            "account_index": { "type": "integer", "minimum": 0 },
            "chain": { "type": "string" },
            "protocol": { "type": "string", "enum": ["lido", "eigenlayer", "marinade", "jito"], "description": "Protocol selection. If omitted, defaults by chain (EVM: lido; Solana: jito)." },
            "token": { "type": "string", "description": "Token to unstake. EVM Lido: stETH (default). Solana Jito: jitoSOL (default)." },
            "amount": { "type": "string", "description": "Amount to unstake (string). Native unstaking paths require this." },
            "amount_units": { "type": "string", "enum": ["ui", "base"], "default": "ui" },
            "request_id": { "type": "string", "description": "Optional async request id for Lido withdrawal claims. Native Lido unstake returns request_ids that can be used to claim once the withdrawal is finalized." },
            "slippage_bps": { "type": "integer", "minimum": 0, "maximum": 2000, "default": 100, "description": "Solana only (Jito via Jupiter swap): slippage in basis points." },
            "usd_value": { "type": "number" },
            "usd_value_known": { "type": "boolean", "default": false },
            "to": { "type": "string" },
            "data": { "type": "string" },
            "value_wei": { "type": "string", "default": "0" },
            "tx_b64": { "type": "string" },
            "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
            "asset": { "type": "object" }
          },
          "required": ["chain"],
          "additionalProperties": false
        }}),
    ]
}

fn write_defi_liquidity_schemas() -> Vec<Value> {
    vec![
        json!({ "name": "provide_liquidity", "description": "Provide liquidity to an AMM. Executes a tx envelope. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string" },
            "account_index": { "type": "integer", "minimum": 0 },
            "chain": { "type": "string" },
            "venue": { "type": "string", "enum": ["uniswap_lp", "orca_lp"], "description": "Venue selection. If omitted, defaults by chain (EVM: uniswap_lp; Solana: orca_lp)." },
            "usd_value": { "type": "number" },
            "usd_value_known": { "type": "boolean", "default": false },
            "to": { "type": "string" },
            "data": { "type": "string" },
            "value_wei": { "type": "string", "default": "0" },
            "tx_b64": { "type": "string" },
            "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
            "asset": { "type": "object" }
          },
          "required": ["chain"],
          "additionalProperties": false
        }}),
        json!({ "name": "remove_liquidity", "description": "Remove liquidity from an AMM. Executes a tx envelope. Requires policy approval.", "inputSchema": {
          "type": "object",
          "properties": {
            "wallet": { "type": "string" },
            "account_index": { "type": "integer", "minimum": 0 },
            "chain": { "type": "string" },
            "venue": { "type": "string", "enum": ["uniswap_lp", "orca_lp"], "description": "Venue selection. If omitted, defaults by chain (EVM: uniswap_lp; Solana: orca_lp)." },
            "usd_value": { "type": "number" },
            "usd_value_known": { "type": "boolean", "default": false },
            "to": { "type": "string" },
            "data": { "type": "string" },
            "value_wei": { "type": "string", "default": "0" },
            "tx_b64": { "type": "string" },
            "allowed_program_ids": { "type": "array", "items": { "type": "string" } },
            "asset": { "type": "object" }
          },
          "required": ["chain"],
          "additionalProperties": false
        }}),
    ]
}

fn write_defi_schemas() -> Vec<Value> {
    let mut schemas = vec![write_defi_bridge_schema()];
    schemas.extend(write_defi_lending_schemas());
    schemas.extend(write_defi_staking_schemas());
    schemas.extend(write_defi_liquidity_schemas());
    schemas
}

pub fn list_tools_result() -> Value {
    // Tool surface served via MCP.
    let mut tools = network_tool_schemas();
    tools.extend(read_tool_schemas());
    tools.extend(wallet_tool_schemas());
    tools.extend(prediction_tool_schemas());
    tools.extend(perp_tool_schemas());
    tools.extend(nft_tool_schemas());
    tools.extend(write_spot_schemas());
    tools.extend(write_defi_schemas());
    json!({ "tools": tools })
}
