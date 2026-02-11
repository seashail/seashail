# Seashail

Agent-native, self-hosted trading infrastructure for crypto.

Seashail is a local binary that exposes an [MCP](https://modelcontextprotocol.io/) server over stdio. Agents can query balances, execute trades, and manage DeFi positions while Seashail enforces a policy engine and keeps key material encrypted at rest. The agent never sees private keys.

**[Full Documentation](https://seashail.com/docs)** | **[Install](#install)** | **[Quickstart](#quickstart)** | **[Supported Chains](#supported-chains)** | **[MCP Tools](#mcp-tools)** | **[Security Model](#security-model)**

## Features

- **60+ MCP tools** — spot swaps, sends, bridging, DeFi (lending, staking, liquidity), perpetual futures, NFTs, prediction markets, pump.fun
- **Policy engine** — per-transaction and daily USD caps, slippage limits, leverage caps, allowlists, operation toggles, tiered approval (auto-approve / confirm / hard-block)
- **Non-custodial key storage** — Shamir 2-of-3 secret sharing for generated wallets, AES-256-GCM encryption for imported wallets, passphrase sessions with configurable TTL
- **Multi-agent safe** — proxy/daemon architecture lets multiple agents (Claude Desktop, Cursor, Codex, etc.) share one keystore and passphrase session without split-brain
- **Multi-chain** — Solana, Ethereum, Base, Arbitrum, Optimism, Polygon, BNB Chain, Avalanche, Monad, Bitcoin (+ configurable EVM chains and testnets)
- **Agent-agnostic** — works with any MCP-capable client; one-click templates for 10+ agents/editors
- **Scam protection** — optional signed scam-address blocklist, OFAC SDN checking, recipient/contract allowlists
- **Verification** — release assets signed with Sigstore Cosign, SBOM generation (SPDX-JSON)

## Install

Seashail is distributed via GitHub Releases and can also be built from source. Your agent runs it as an MCP stdio server via `seashail mcp`.

### macOS (Homebrew)

```bash
brew install seashail/seashail/seashail
```

### macOS / Linux (installer)

```bash
curl -fsSL https://seashail.com/install | sh
```

If `seashail` is not found after install, add the default install dir to your shell `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Windows (PowerShell)

If you have WSL, you can run the macOS/Linux installer inside WSL. Otherwise:

```powershell
irm https://seashail.com/install.ps1 | iex
```

### No-Install Options

If you don't want to install the binary, reference these wrappers in your agent's MCP config instead. They install and run Seashail automatically.

**npx (Node.js)** — see [`packages/mcp/`](packages/mcp/):

- Command: `npx`
- Args: `-y @seashail/mcp --`

**uvx (Python)** — see [`python/`](python/):

- Command: `uvx`
- Args: `seashail-mcp`

### OpenClaw Users

```bash
seashail openclaw install
```

Or via the OpenClaw CLI:

```bash
openclaw plugins install @seashail/seashail
openclaw plugins enable seashail
openclaw gateway restart
```

See the [OpenClaw setup guide](https://seashail.com/docs/guides/agents/openclaw) for details.

### Verify Installation

```bash
seashail doctor
```

This checks for common issues (missing dependencies, permissions, path configuration). The report contains no secrets and is safe to paste in issues. See the [CLI reference](#cli-reference) for details.

## Quickstart

### 1. Install Seashail

See [Install](#install) above.

### 2. Connect Your Agent

Your agent starts Seashail automatically — you just need to tell it how. Pick your agent:

**OpenClaw** ([docs](https://docs.openclaw.ai/tools/plugin)):

```bash
seashail openclaw install
```

**Claude Code** ([docs](https://code.claude.com/docs/en/mcp)):

```bash
claude mcp add seashail -- seashail mcp
```

**Claude Desktop** ([docs](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop)):

```bash
seashail agent install claude-desktop
```

**Codex** ([docs](https://developers.openai.com/codex/mcp)):

Add to `~/.codex/config.toml`:

```toml
[mcp_servers.seashail]
command = "seashail"
args = ["mcp"]
```

**Cursor** ([docs](https://docs.cursor.com/context/model-context-protocol)):

```bash
seashail agent install cursor
```

**VS Code / GitHub Copilot** ([docs](https://code.visualstudio.com/docs/copilot/customization/mcp-servers)):

```bash
seashail agent install vscode
```

**Windsurf** ([docs](https://docs.windsurf.com/windsurf/cascade/mcp)):

```bash
seashail agent install windsurf
```

**Cline, Continue, JetBrains, or any MCP client** — see [Agent Integration](#agent-integration) below or the [full agent setup guide](https://seashail.com/docs/guides/agents).

> **Testnet mode:** Add `--network testnet` to any of the above. For example: `seashail agent install cursor --network testnet` or `claude mcp add seashail-testnet -- seashail mcp --network testnet`.

### 3. First Run

On first connection, Seashail automatically creates a default wallet (EVM, Solana, Bitcoin addresses) so your agent can immediately query balances — no prompts required.

To create a wallet with stronger recovery options, ask your agent to call `create_wallet`. This uses MCP elicitation to:

- set a passphrase (min 8 characters, stored nowhere)
- show and confirm an offline backup share (Shamir 2-of-3)
- accept disclaimers

Key material never leaves the Seashail process. The agent only receives tool outputs (balances, quotes, tx hashes).

### 4. Verify

In your agent, try:

- `get_capabilities` — sanity check: see configured RPCs, swap backends, chains
- `list_wallets` — see your wallets and addresses
- `get_balance` — check balances

## Architecture

Seashail uses a proxy-daemon architecture to allow multiple agents to share a single keystore and policy state safely.

```
┌─────────────────┐
│   MCP Host      │  (Agent: Claude Desktop, Cursor, etc.)
│  (stdio client) │
└────────┬────────┘
         │ stdio (JSON-RPC over newline-delimited JSON)
         ▼
┌─────────────────┐
│  seashail mcp   │  Lightweight stdio proxy
│    (proxy)      │  - Injects network override params
└────────┬────────┘  - Auto-spawns daemon if needed
         │ Unix socket / named pipe / TCP loopback
         ▼
┌─────────────────┐
│ seashail daemon │  Singleton process
│                 │  - Holds exclusive keystore lock
│                 │  - Manages passphrase session
└────────┬────────┘  - Coordinates all MCP clients
         │
         ▼
┌─────────────────┐
│  MCP Server     │  Tool dispatch + elicitation
│  (in daemon)    │  - Policy evaluation
└────────┬────────┘  - Key decryption + signing
         │           - Transaction broadcast
         ▼
┌─────────────────┐
│  Policy Engine  │  Gates every write operation
│                 │  - Tiered approval (auto/confirm/block)
└────────┬────────┘  - USD caps, slippage, allowlists
         │
         ▼
┌─────────────────┐
│   Chain Layers  │  Chain-specific adapters
│  (EVM/Solana/   │  - RPC communication
│   Bitcoin)      │  - Transaction construction
└─────────────────┘  - Broadcast
```

**Why proxy + daemon?**

1. **Multiple agents sharing state** — wallet created in Claude Desktop appears in Cursor; passphrase entered once unlocks everywhere; policy changes apply globally
2. **Concurrent access safety** — exclusive filesystem lock prevents keystore corruption from concurrent writes
3. **Passphrase session caching** — unlock once, use from all agents (configurable TTL, default 1 hour)

See the [Architecture docs](https://seashail.com/docs/reference/architecture) for full details on the data flow and layer responsibilities.

## Supported Chains

| Chain | Identifier | Type |
|-------|-----------|------|
| Solana | `solana` | Mainnet + Devnet |
| Ethereum | `ethereum` | Mainnet |
| Base | `base` | Mainnet |
| Arbitrum | `arbitrum` | Mainnet |
| Optimism | `optimism` | Mainnet |
| Polygon | `polygon` | Mainnet |
| BNB Chain | `bnb` | Mainnet |
| Avalanche | `avalanche` | Mainnet |
| Monad | `monad` | Mainnet |
| Bitcoin | `bitcoin` | Mainnet + Testnet (BIP-84 native SegWit) |
| Sepolia | `sepolia` | Testnet |
| Base Sepolia | `base-sepolia` | Testnet |
| Arbitrum Sepolia | `arbitrum-sepolia` | Testnet |
| Optimism Sepolia | `optimism-sepolia` | Testnet |
| Polygon Amoy | `polygon-amoy` | Testnet |
| BNB Testnet | `bnb-testnet` | Testnet |
| Avalanche Fuji | `avalanche-fuji` | Testnet |
| Monad Testnet | `monad-testnet` | Testnet |

Custom EVM chains can be added via `config.toml`. Use `get_capabilities` to see what's configured on your instance.

See the [Chains docs](https://seashail.com/docs/reference/chains) for chain identifiers, network mode defaults, and RPC configuration.

## MCP Tools

All tools are served over MCP stdio via `seashail mcp`. For chain-by-chain support, call `get_capabilities`.

### Network and RPC

| Tool | Description |
|------|-------------|
| `get_network_mode` | Check current mainnet/testnet mode |
| `set_network_mode` | Switch network mode (persistent) |
| `configure_rpc` | Override RPC endpoints |
| `get_testnet_faucet_links` | Get faucet URLs for testnets |
| `get_capabilities` | Discover chains, integrations, and surfaces |

### Read Tools

| Tool | Description |
|------|-------------|
| `inspect_token` | Look up token details (symbol, decimals, address) |
| `get_defi_yield_pools` | Discover yield opportunities across protocols |
| `get_balance` | Check token balance on a chain |
| `get_portfolio` | Multi-chain portfolio overview |
| `get_token_price` | Get USD price for a token |
| `estimate_gas` | Estimate gas cost for an operation |
| `get_transaction_history` | Recent transactions for a wallet |
| `get_portfolio_analytics` | Portfolio analytics and tracking |
| `get_bridge_status` | Track a bridge transfer |

### Wallet Tools

| Tool | Description |
|------|-------------|
| `list_wallets` | List all wallets |
| `get_wallet_info` | Get wallet addresses and details |
| `get_deposit_info` | Get deposit address for a chain/token |
| `set_active_wallet` | Set the default wallet for tool calls |
| `add_account` | Add a BIP-44 account index |
| `create_wallet` | Create a new wallet (Shamir 2-of-3) |
| `import_wallet` | Import an existing key/mnemonic |
| `export_shares` | Export Shamir backup share |
| `rotate_shares` | Rotate Shamir shares |
| `create_wallet_pool` | Create a pool of managed wallets |
| `transfer_between_wallets` | Internal transfer between wallets |
| `fund_wallets` | Distribute funds across wallet pool |

### Write Tools (Send, Swap, Bridge)

| Tool | Description |
|------|-------------|
| `request_airdrop` | Request SOL airdrop (devnet/testnet only) |
| `send_transaction` | Send native or fungible tokens |
| `swap_tokens` | Swap tokens (Jupiter on Solana, Uniswap/1inch on EVM) |
| `bridge_tokens` | Bridge tokens cross-chain (Wormhole, LayerZero) |

### DeFi Tools

| Tool | Description |
|------|-------------|
| `lend_tokens` | Supply tokens to lending protocols (Aave, Kamino, Compound, Marginfi) |
| `withdraw_lending` | Withdraw supplied tokens + interest |
| `borrow_tokens` | Borrow against collateral |
| `repay_borrow` | Repay borrowed amounts |
| `get_lending_positions` | View lending/borrowing positions |
| `stake_tokens` | Stake for liquid staking derivatives (Lido, Jito) |
| `unstake_tokens` | Unstake derivatives back to native tokens |
| `provide_liquidity` | Add tokens to AMM pools (Uniswap LP, Orca LP) |
| `remove_liquidity` | Withdraw from AMM pools |

### Perps Tools

| Tool | Description |
|------|-------------|
| `get_market_data` | Get market prices, funding rates |
| `get_positions` | View open perpetual positions |
| `open_perp_position` | Open a leveraged position (Hyperliquid, Jupiter Perps) |
| `close_perp_position` | Close a position (full or partial) |
| `place_limit_order` | Place a limit order (Hyperliquid) |
| `modify_perp_order` | Modify an existing limit order (Hyperliquid) |

### NFT Tools

| Tool | Description |
|------|-------------|
| `get_nft_inventory` | List NFTs in wallet (Solana) |
| `transfer_nft` | Transfer an NFT (Solana + EVM) |
| `buy_nft` | Buy NFT via marketplace envelope (Blur, Magic Eden, OpenSea, Tensor) |
| `sell_nft` | Sell/list NFT via marketplace envelope |
| `bid_nft` | Place bid/offer via marketplace envelope |

### Prediction Market Tools

| Tool | Description |
|------|-------------|
| `search_prediction_markets` | Search Polymarket events |
| `get_prediction_orderbook` | View CLOB orderbook depth |
| `get_prediction_positions` | View open prediction positions |
| `place_prediction` | Place a CLOB order on Polymarket |
| `close_prediction` | Cancel an existing order |

### Pump.fun Tools

| Tool | Description |
|------|-------------|
| `pumpfun_list_new_coins` | Discover recently launched meme coins |
| `pumpfun_get_coin_info` | Get detailed coin info |
| `pumpfun_buy` | Buy a pump.fun token with SOL |
| `pumpfun_sell` | Sell pump.fun tokens back to SOL |

### Policy Tools

| Tool | Description |
|------|-------------|
| `get_policy` | View current policy (global or per-wallet) |
| `update_policy` | Update policy rules |

See the [MCP Tools Reference](https://seashail.com/docs/reference/mcp-tools) for full parameter details and the individual tool reference pages.

## Security Model

Seashail assumes the agent process may be malicious or compromised. The binary is the security boundary.

### Key Storage

- **Generated wallets** use Shamir Secret Sharing (2-of-3): Share 1 encrypted with machine secret, Share 2 encrypted with machine key (or passphrase for portability), Share 3 shown once as offline backup
- **Imported wallets** are encrypted at rest using AES-256-GCM with a passphrase-derived key (Argon2id + HKDF subkeys)
- Key material is zeroized from memory after signing

### Policy Engine

Every write operation is gated by:

- **Per-transaction USD caps** (`max_usd_per_tx`) and **daily USD caps** (`max_usd_per_day`)
- **Slippage caps** for swaps (`max_slippage_bps`)
- **Leverage caps** for perps (`max_leverage`, `max_usd_per_position`)
- **Recipient allowlisting** (`send_allowlist`) and **contract allowlisting** (`contract_allowlist`)
- **Operation toggles** (enable/disable sends, swaps, bridging, perps, NFTs individually)
- **Tiered approvals** via MCP elicitation:
  - **Auto-approve** — silent execution (low risk, within limits)
  - **User confirm** — MCP elicitation prompt (exceeds auto-approve threshold)
  - **Hard block** — rejection (exceeds hard cap or violates policy)

### Threat Mitigations

| Threat | Mitigation |
|--------|-----------|
| Malicious agent | Policy engine + tiered approvals + allowlists + operation toggles |
| Key theft from logs | MCP elicitation (keys never in agent conversation); tool schema rejects secret params |
| Split-brain state | Exclusive filesystem lock; singleton daemon |
| Passphrase theft | TTL-based sessions; zeroize on expiry; mlock on sensitive buffers |
| Phishing/scam addresses | Signed scam blocklist; OFAC SDN checking; recipient allowlists |
| Excessive spending | Per-tx and daily USD caps; operation toggles |
| Unknown USD value exploit | `deny_unknown_usd_value` (fail-closed by default) |
| Leverage explosion | `max_leverage` and `max_usd_per_position` caps |

### Key Custody Comparison

| Aspect | Seashail | Browser Wallet | Cloud Custody |
|--------|----------|---------------|---------------|
| Key location | Local encrypted keystore | Browser extension | Remote server |
| Agent access | Policy-gated MCP tools | Direct signing | API with provider keys |
| Recovery | Shamir 2-of-3 + passphrase | Seed phrase | Provider flow |
| Risk model | Agent constrained by policy | User verifies every tx | Trusted third party |

See the [Security Model docs](https://seashail.com/docs/guides/security-model) for the full threat analysis.

## Policy and Approvals

Seashail evaluates every write against policy before any key material is decrypted.

### Common Controls

```
max_usd_per_tx          Per-transaction USD cap
max_usd_per_day         Daily USD cap (resets UTC midnight)
max_slippage_bps        Swap slippage cap (basis points; 100 bps = 1%)
deny_unknown_usd_value  Fail-closed when USD value unknown (default: true)
send_allow_any          Allow sends to any address (vs allowlist-only)
send_allowlist          Explicit list of permitted send addresses
contract_allow_any      Allow interaction with any contract
contract_allowlist      Explicit list of permitted contracts
enable_send             Toggle sends on/off
enable_swap             Toggle swaps on/off
enable_bridge           Toggle bridging on/off
enable_perps            Toggle perps on/off
max_leverage            Cap leverage for perps
max_usd_per_position    Cap perps position size
enable_nft              Toggle NFT operations on/off
max_usd_per_nft_tx      Cap per-NFT-transaction value
```

### Viewing and Updating

```
get_policy              View current policy (global or per-wallet)
update_policy           Replace policy rules
```

Per-wallet overrides are supported: `get_policy({ wallet })` and `update_policy({ wallet, policy })`.

See the [Policy and Approvals Guide](https://seashail.com/docs/guides/policy-and-approvals) and the [Policy Tools Reference](https://seashail.com/docs/reference/tools-policy) for all 33+ policy fields.

## Configuration

### Paths

```bash
seashail paths
```

Output (JSON):

```json
{
  "config_dir": "/Users/you/.config/seashail",
  "data_dir": "/Users/you/.local/share/seashail",
  "log_file": "/Users/you/.local/share/seashail/seashail.log.jsonl"
}
```

Override with `SEASHAIL_CONFIG_DIR` and `SEASHAIL_DATA_DIR` environment variables.

### Config File

`config.toml` lives under `config_dir`:

```toml
network_mode = "mainnet" # or "testnet"

[rpc]
solana_rpc_url = "https://api.mainnet-beta.solana.com"

[http]
binance_base_url = "https://api.binance.com"
jupiter_base_url = "https://api.jup.ag/swap/v1"
# 1inch requires an API key. If unset, swaps use Uniswap on EVM.
# oneinch_api_key = "..."

# Hyperliquid (perps)
hyperliquid_base_url_mainnet = "https://api.hyperliquid.xyz"
hyperliquid_base_url_testnet = "https://api.hyperliquid-testnet.xyz"

# Scam blocklist (optional)
# scam_blocklist_url = "https://example.com/seashail/scam-blocklist.json"
# scam_blocklist_pubkey_b64 = "..."
# scam_blocklist_refresh_seconds = 21600
```

### Network Mode

Mainnet is the default. Network mode affects default chain selection when tools omit `chain`/`chains`.

- **Persistent:** set `network_mode = "testnet"` in `config.toml` or call `set_network_mode` over MCP
- **Per-session override:** `seashail mcp --network testnet`
- **First-run default:** `export SEASHAIL_NETWORK_MODE=testnet` (before `config.toml` is created)

See the [Network Mode Guide](https://seashail.com/docs/guides/network-mode) for Solana RPC defaults, airdrops, and faucet links.

## CLI Reference

### seashail mcp

Run the MCP server over stdio (proxy mode by default).

```bash
seashail mcp                        # Proxy mode (recommended)
seashail mcp --network testnet      # Testnet override
seashail mcp --standalone           # No daemon sharing
```

### seashail daemon

Run the singleton daemon (shared state across MCP clients).

```bash
seashail daemon                             # Run until terminated
seashail daemon --idle-exit-seconds 300     # Auto-exit after 5 min idle
```

You usually don't need to run this manually — `seashail mcp` auto-spawns the daemon.

### seashail agent

Manage agent config templates.

```bash
seashail agent list                         # List supported targets
seashail agent print cursor                 # Print config to stdout
seashail agent install cursor               # Install config
seashail agent install cursor --network testnet  # Testnet template
```

Supported targets: `cursor`, `vscode`, `windsurf`, `claude-desktop`.

### seashail openclaw install

Install the OpenClaw plugin integration.

```bash
seashail openclaw install
seashail openclaw install --network testnet
seashail openclaw install --link --plugin ./packages/openclaw-seashail-plugin  # Dev
```

### seashail doctor

Self-diagnostic report (contains no secrets, safe to paste in issues).

```bash
seashail doctor          # Human-readable
seashail doctor --json   # Machine-readable
```

### seashail paths

Print resolved config, data, and log paths.

```bash
seashail paths
```

### seashail upgrade

Upgrade to the latest version.

```bash
seashail upgrade              # Interactive
seashail upgrade --yes        # Non-interactive
seashail upgrade --yes --quiet  # Silent (for scripts)
```

See the [CLI Reference docs](https://seashail.com/docs/reference/cli) for full flag details.

## Agent Integration

Seashail works with any MCP-capable agent. For agents not listed in [Quickstart](#quickstart):

### Generic MCP Stdio

Configure your agent to run:

- **Command:** `seashail`
- **Args:** `mcp`

If your agent uses a JSON config file:

```json
{
  "mcpServers": {
    "seashail": { "command": "seashail", "args": ["mcp"] }
  }
}
```

Or:

```json
{
  "servers": {
    "seashail": { "type": "stdio", "command": "seashail", "args": ["mcp"] }
  }
}
```

### Supported Agents

| Agent | Setup | Docs |
|-------|-------|------|
| OpenClaw | `seashail openclaw install` | [Guide](https://seashail.com/docs/guides/agents/openclaw) |
| Claude Code | `claude mcp add seashail -- seashail mcp` | [Guide](https://seashail.com/docs/guides/agents/claude-code) |
| Claude Desktop | `seashail agent install claude-desktop` | [Guide](https://seashail.com/docs/guides/agents/claude-desktop) |
| Codex | Edit `~/.codex/config.toml` | [Guide](https://seashail.com/docs/guides/agents/codex) |
| Cursor | `seashail agent install cursor` | [Guide](https://seashail.com/docs/guides/agents/cursor) |
| VS Code / GitHub Copilot | `seashail agent install vscode` | [Guide](https://seashail.com/docs/guides/agents/github-copilot) |
| Windsurf | `seashail agent install windsurf` | [Guide](https://seashail.com/docs/guides/agents/windsurf) |
| Cline | Manual JSON config | [Guide](https://seashail.com/docs/guides/agents/cline) |
| Continue | Manual JSON config | [Guide](https://seashail.com/docs/guides/agents/continue) |
| JetBrains | Manual JSON config | [Guide](https://seashail.com/docs/guides/agents/jetbrains) |
| Any MCP Client | Generic stdio config | [Guide](https://seashail.com/docs/guides/agents/any-mcp-client) |

Static config templates are also available in [`packages/agent-configs/`](packages/agent-configs/).

### Multi-Client Behavior

Multiple agents safely share the same wallets, passphrase session, and policy counters via the singleton daemon. Each `seashail mcp` process is a lightweight proxy.

- macOS/Linux: Unix socket at `data_dir/seashail-mcp.sock`
- Windows: named pipe

## Guides

These are summarized below. See the [full documentation](https://seashail.com/docs) for complete details.

### Wallets and Key Storage

- **Generated wallets:** Shamir 2-of-3 secret sharing (machine share + passphrase share + offline backup)
- **Imported wallets:** AES-256-GCM encrypted with passphrase-derived key
- **Passphrase session:** cached in memory with configurable TTL; shared across all MCP clients
- Tools: `create_wallet`, `import_wallet`, `list_wallets`, `get_wallet_info`, `get_deposit_info`, `export_shares`, `rotate_shares`

[Wallets Guide](https://seashail.com/docs/guides/wallets)

### Sending Tokens

`send_transaction` transfers native tokens (SOL, ETH, etc.) or fungible tokens (SPL, ERC-20). Seashail handles chain-specific mechanics (ATA creation on Solana, ERC-20 approval on EVM) and validates addresses before signing.

[Sending Guide](https://seashail.com/docs/guides/sending)

### Swapping Tokens

`swap_tokens` routes automatically: Jupiter on Solana, Uniswap (or 1inch when configured) on EVM. Slippage tolerance is enforced by policy.

[Swapping Guide](https://seashail.com/docs/guides/swapping)

### Bridging Tokens

`bridge_tokens` moves tokens cross-chain (Solana <-> EVM, EVM <-> EVM) via Wormhole (default) or LayerZero. Auto-redeem on destination chain is supported. Track progress with `get_bridge_status`.

[Bridging Guide](https://seashail.com/docs/guides/bridging)

### DeFi Operations

Four DeFi primitives with protocol auto-selection:

| Operation | EVM Default | Solana Default |
|-----------|------------|----------------|
| Lending | Aave v3 | Kamino |
| Borrowing | Aave v3, Compound v3 | Kamino, Marginfi |
| Staking | Lido (ETH -> stETH) | Jito (SOL -> JitoSOL) |
| Liquidity | Uniswap LP | Orca LP |

[DeFi Guide](https://seashail.com/docs/guides/defi)

### Perps Trading

Leveraged perpetual futures on two venues:

| Venue | Address | Testnet | Orders | Partial Close |
|-------|---------|---------|--------|---------------|
| Hyperliquid | EVM | Yes | Market + Limit | Yes |
| Jupiter Perps | Solana | No | Market only | No |

Policy controls: `enable_perps`, `max_leverage`, `max_usd_per_position`.

[Perps Guide](https://seashail.com/docs/guides/perps)

### NFT Operations

Inventory reads, direct transfers, and marketplace trading via transaction envelopes.

Supported marketplaces: **Blur** (Ethereum), **Magic Eden** (Solana + cross-chain), **OpenSea** (Ethereum), **Tensor** (Solana).

[NFT Guide](https://seashail.com/docs/guides/nfts)

### Prediction Markets

Trade on Polymarket (CLOB on Polygon). Search markets, view orderbooks, place limit/market orders, track positions.

[Prediction Markets Guide](https://seashail.com/docs/guides/predictions)

### Pump.fun

Discover, buy, and sell meme coins on Pump.fun (Solana only). Bonding curve mechanics — price moves with demand.

[Pump.fun Guide](https://seashail.com/docs/guides/pumpfun)

### Chains and Funding

Get deposit addresses with `get_deposit_info`. For testnets, use `get_testnet_faucet_links` (EVM) or `request_airdrop` (Solana devnet/testnet).

[Chains and Funding Guide](https://seashail.com/docs/guides/chains-and-funding)

### Scam Blocklist

Optional signed scam-address blocklist. Opt-in via `config.toml`. Blocks sends and NFT transfers to known-bad addresses. Fail-open if fetch fails.

[Scam Blocklist Guide](https://seashail.com/docs/guides/scam-blocklist)

## Verification

Release assets are signed with Sigstore Cosign. SBOM generation uses SPDX-JSON.

```bash
cosign verify-blob \
  --certificate <asset>.crt \
  --signature <asset>.sig \
  <asset>
```

See the [Verification docs](https://seashail.com/docs/reference/verification) for details.

## Troubleshooting

Common issues and solutions:

| Error | Cause | Fix |
|-------|-------|-----|
| `wallet_not_found` | Wallet name doesn't exist | Run `list_wallets`, check spelling |
| `passphrase_required` | Session expired | Re-enter passphrase on next tool call |
| `keystore_busy` | Lock contention | Wait and retry; check for stuck daemon |
| Policy hard-block | Exceeds USD caps | Check `get_policy`, adjust limits |
| Slippage exceeded | Price moved too far | Increase `max_slippage_bps` or retry |
| Operation disabled | Toggle off in policy | Enable via `update_policy` |
| Allowlist rejection | Address not permitted | Add to allowlist or disable |
| Unknown USD value | No pricing data | Set `deny_unknown_usd_value: false` or retry |
| RPC connection error | Network/endpoint issue | Check connectivity, switch RPC |
| Wrong network | Mainnet/testnet confusion | Check `get_network_mode`, switch |

Run `seashail doctor` for a diagnostic report. See the [Troubleshooting docs](https://seashail.com/docs/troubleshooting) for full details.

## Repo Layout

```
apps/
  docs/                          # Fumadocs + Next.js documentation site
  landing/                       # Next.js + React + Tailwind CSS landing page

crates/
  seashail/                      # Rust binary (CLI + daemon + MCP stdio bridge)

packages/
  agent-configs/                 # Static MCP config templates for editors/agents
  config/                        # Shared TypeScript config
  e2e/                           # End-to-end test harness (Bun)
  mcp/                           # npx-runnable MCP server wrapper (@seashail/mcp)
  openclaw-seashail-plugin/      # OpenClaw plugin: exposes Seashail tools as native OpenClaw tools
  oxlint/                        # Custom Oxlint plugin
  shared/                        # Shared TypeScript types (TypeBox + Standard Schema)
  skills-seashail/               # SKILL.md + agent-facing docs (Agent Skills spec)
  web-theme/                     # Shared CSS theme package

python/                          # Python tooling (uvx runner)
reference/                       # Archived reference documentation
```

### Package READMEs

- [`packages/mcp/`](packages/mcp/) — `@seashail/mcp` npx runner
- [`packages/agent-configs/`](packages/agent-configs/) — Static MCP config templates
- [`packages/openclaw-seashail-plugin/`](packages/openclaw-seashail-plugin/) — OpenClaw plugin
- [`packages/e2e/`](packages/e2e/) — End-to-end tests
- [`packages/skills-seashail/`](packages/skills-seashail/) — Agent Skills spec and workflows
- [`python/`](python/) — `seashail-mcp` uvx runner

## Building from Source

Prerequisites: `git`, `cargo` (Rust toolchain), `bun` (for TypeScript packages).

```bash
git clone https://github.com/seashail/seashail.git
cd seashail
bun install
bun run build
bun run check

cargo build -p seashail
./target/debug/seashail --help
./target/debug/seashail doctor
```

To use the dev binary with your agent:

- **Command:** `./target/debug/seashail`
- **Args:** `mcp`

## Commands

```bash
bun install                  # Install dependencies
bun run build                # Build TypeScript packages
bun run test                 # Run tests
bun run check                # Type-check and lint

cargo build -p seashail      # Build Rust binary
cargo test -p seashail       # Run Rust tests
```

## License

Apache-2.0
