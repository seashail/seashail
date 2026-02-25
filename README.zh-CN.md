# Seashail

> **[English](README.md)** | 简体中文

Agent 原生、自托管的加密货币交易基础设施。

Seashail 是一个本地二进制程序，通过 stdio 暴露 [MCP](https://modelcontextprotocol.io/) 服务器。Agent 可以查询余额、执行交易和管理去中心化金融（DeFi）仓位，同时 Seashail 通过策略引擎进行管控，并将密钥材料加密存储。Agent 永远无法接触到私钥。

**[完整文档](https://seashail.com/docs)** | **[安装](#安装)** | **[快速开始](#快速开始)** | **[支持的链](#支持的链)** | **[MCP 工具](#mcp-工具)** | **[安全模型](#安全模型)**

## 功能特性

- **60+ MCP 工具** — 现货兑换、发送、跨链桥接、DeFi（借贷、质押、流动性）、永续合约、非同质化代币（NFT）、预测市场、Pump.fun
- **策略引擎** — 单笔交易和每日美元价值上限、滑点限制、杠杆上限、白名单、操作开关、分级审批（自动审批 / 用户确认 / 硬性拒绝）
- **非托管密钥存储** — 生成钱包采用沙米尔秘密分享（2-of-3），导入钱包使用 AES-256-GCM 加密，密码短语会话支持可配置 TTL
- **多 agent 安全** — 代理/守护进程架构允许多个 agent（Claude Desktop、Cursor、Codex 等）共享同一密钥存储和密码短语会话，不会出现脑裂问题
- **多链** — Solana、Ethereum、Base、Arbitrum、Optimism、Polygon、BNB Chain、Avalanche、Monad、Bitcoin（+ 可配置的 EVM 链和测试网）
- **Agent 无关** — 适用于任何支持 MCP 的客户端；为 10+ agent/编辑器提供一键配置模板
- **防诈骗保护** — 可选的签名诈骗地址黑名单、OFAC SDN 检查、收款人/合约白名单
- **验证** — 发布资产使用 Sigstore Cosign 签名，生成 SBOM（SPDX-JSON）

## 安装

Seashail 通过 GitHub Releases 分发，也可以从源码构建。你的 agent 通过 `seashail mcp` 以 MCP stdio 服务器方式运行它。

### macOS / Linux（安装脚本）

```bash
curl -fsSL https://seashail.com/install | sh
```

如果安装后找不到 `seashail`，请将默认安装目录添加到你的 shell `PATH`：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Windows (PowerShell)

如果你有 WSL，可以在 WSL 中运行 macOS/Linux 安装脚本。否则：

```powershell
irm https://seashail.com/install.ps1 | iex
```

### 免安装选项

如果你不想安装二进制文件，可以在你的 agent MCP 配置中引用以下包装器。它们会自动安装并运行 Seashail。

**npx (Node.js)** — 参见 [`packages/mcp/`](packages/mcp/)：

- Command: `npx`
- Args: `-y @seashail/mcp --`

**uvx (Python)** — 参见 [`python/`](python/)：

- Command: `uvx`
- Args: `seashail-mcp`

### OpenClaw 用户

```bash
seashail openclaw install
```

或通过 OpenClaw CLI：

```bash
openclaw plugins install @seashail/seashail
openclaw plugins enable seashail
openclaw gateway restart
```

详见 [OpenClaw 设置指南](https://seashail.com/docs/guides/agents/openclaw)。

### 验证安装

```bash
seashail doctor
```

此命令检查常见问题（缺少依赖、权限、路径配置）。报告不含任何敏感信息，可安全粘贴到 issue 中。详见 [CLI 参考](#cli-参考)。

## 快速开始

### 1. 安装 Seashail

参见上方[安装](#安装)。

### 2. 连接你的 Agent

你的 agent 会自动启动 Seashail —— 你只需要告诉它如何启动。选择你的 agent：

**OpenClaw** ([文档](https://docs.openclaw.ai/tools/plugin))：

```bash
seashail openclaw install
```

**Claude Code** ([文档](https://code.claude.com/docs/en/mcp))：

```bash
claude mcp add seashail -- seashail mcp
```

**Claude Desktop** ([文档](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop))：

```bash
seashail agent install claude-desktop
```

**Codex** ([文档](https://developers.openai.com/codex/mcp))：

添加到 `~/.codex/config.toml`：

```toml
[mcp_servers.seashail]
command = "seashail"
args = ["mcp"]
```

**Cursor** ([文档](https://docs.cursor.com/context/model-context-protocol))：

```bash
seashail agent install cursor
```

**VS Code / GitHub Copilot** ([文档](https://code.visualstudio.com/docs/copilot/customization/mcp-servers))：

```bash
seashail agent install vscode
```

**Windsurf** ([文档](https://docs.windsurf.com/windsurf/cascade/mcp))：

```bash
seashail agent install windsurf
```

**Cline、Continue、JetBrains 或任何 MCP 客户端** — 参见下方 [Agent 集成](#agent-集成)或[完整 agent 设置指南](https://seashail.com/docs/guides/agents)。

> **测试网模式：** 在以上任何命令中添加 `--network testnet`。例如：`seashail agent install cursor --network testnet` 或 `claude mcp add seashail-testnet -- seashail mcp --network testnet`。

### 3. 首次运行

首次连接时，Seashail 会自动创建默认钱包（EVM、Solana、Bitcoin 地址），你的 agent 可以立即查询余额 —— 无需任何提示。

要创建具有更强恢复选项的钱包，可以让你的 agent 调用 `create_wallet`。这会使用 MCP 交互确认来：

- 设置密码短语（至少 8 个字符，不存储在任何地方）
- 显示并确认离线备份密钥分片（沙米尔秘密分享 2-of-3）
- 接受免责声明

密钥材料永远不会离开 Seashail 进程。Agent 只能接收工具输出（余额、报价、交易哈希）。

### 4. 验证

在你的 agent 中尝试：

- `get_capabilities` — 健全性检查：查看已配置的 RPC、兑换后端、链
- `list_wallets` — 查看你的钱包和地址
- `get_balance` — 查看余额

## 架构

Seashail 使用代理-守护进程架构，允许多个 agent 安全地共享单一密钥存储和策略状态。

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

**为什么采用代理 + 守护进程架构？**

1. **多 agent 共享状态** — 在 Claude Desktop 中创建的钱包会出现在 Cursor 中；密码短语只需输入一次即可在所有 agent 中解锁；策略变更全局生效
2. **并发访问安全** — 独占文件锁防止并发写入导致密钥存储损坏
3. **密码短语会话缓存** — 解锁一次，所有 agent 即可使用（可配置 TTL，默认 1 小时）

详见[架构文档](https://seashail.com/docs/reference/architecture)，了解数据流和各层职责。

## 支持的链

| 链               | 标识符              | 类型                                       |
| ---------------- | ------------------- | ------------------------------------------ |
| Solana           | `solana`            | 主网 + Devnet                              |
| Ethereum         | `ethereum`          | 主网                                       |
| Base             | `base`              | 主网                                       |
| Arbitrum         | `arbitrum`          | 主网                                       |
| Optimism         | `optimism`          | 主网                                       |
| Polygon          | `polygon`           | 主网                                       |
| BNB Chain        | `bnb`               | 主网                                       |
| Avalanche        | `avalanche`         | 主网                                       |
| Monad            | `monad`             | 主网                                       |
| Bitcoin          | `bitcoin`           | 主网 + 测试网（BIP-84 原生 SegWit）        |
| Sepolia          | `sepolia`           | 测试网                                     |
| Base Sepolia     | `base-sepolia`      | 测试网                                     |
| Arbitrum Sepolia | `arbitrum-sepolia`  | 测试网                                     |
| Optimism Sepolia | `optimism-sepolia`  | 测试网                                     |
| Polygon Amoy     | `polygon-amoy`      | 测试网                                     |
| BNB Testnet      | `bnb-testnet`       | 测试网                                     |
| Avalanche Fuji   | `avalanche-fuji`    | 测试网                                     |
| Monad Testnet    | `monad-testnet`     | 测试网                                     |

可通过 `config.toml` 添加自定义 EVM 链。使用 `get_capabilities` 查看你实例上已配置的内容。

详见[链文档](https://seashail.com/docs/reference/chains)，了解链标识符、网络模式默认值和 RPC 配置。

## MCP 工具

所有工具通过 `seashail mcp` 以 MCP stdio 方式提供。要了解各链支持情况，请调用 `get_capabilities`。

### 网络和 RPC

| 工具                        | 描述                                 |
| --------------------------- | ------------------------------------ |
| `get_network_mode`          | 查看当前主网/测试网模式             |
| `set_network_mode`          | 切换网络模式（持久化）              |
| `configure_rpc`             | 覆盖 RPC 端点                       |
| `get_testnet_faucet_links`  | 获取测试网水龙头链接                |
| `get_capabilities`          | 发现链、集成和功能面                |

### 读取工具

| 工具                       | 描述                                           |
| -------------------------- | ---------------------------------------------- |
| `inspect_token`            | 查询代币详情（符号、精度、地址）              |
| `get_defi_yield_pools`     | 发现跨协议的收益机会                          |
| `get_balance`              | 查看某条链上的代币余额                        |
| `get_portfolio`            | 多链投资组合概览                              |
| `get_token_price`          | 获取代币的美元价格                            |
| `estimate_gas`             | 估算操作的 Gas 费用                           |
| `get_transaction_history`  | 查看钱包的近期交易                            |
| `get_portfolio_analytics`  | 投资组合分析和追踪                            |
| `get_bridge_status`        | 追踪跨链桥转账状态                            |

### 钱包工具

| 工具                        | 描述                                  |
| --------------------------- | ------------------------------------- |
| `list_wallets`              | 列出所有钱包                         |
| `get_wallet_info`           | 获取钱包地址和详情                   |
| `get_deposit_info`          | 获取某条链/代币的存款地址            |
| `set_active_wallet`         | 设置工具调用的默认钱包               |
| `add_account`               | 添加 BIP-44 账户索引                 |
| `create_wallet`             | 创建新钱包（沙米尔秘密分享 2-of-3） |
| `import_wallet`             | 导入已有密钥/助记词                  |
| `export_shares`             | 导出沙米尔秘密分享备份密钥分片       |
| `rotate_shares`             | 轮换沙米尔秘密分享密钥分片           |
| `create_wallet_pool`        | 创建托管钱包池                       |
| `transfer_between_wallets`  | 在钱包之间内部转账                   |
| `fund_wallets`              | 在钱包池中分配资金                   |

### 写操作工具（发送、兑换、跨链桥接）

| 工具                | 描述                                                    |
| ------------------- | ------------------------------------------------------- |
| `request_airdrop`   | 请求 SOL 空投（仅限 devnet/测试网）                    |
| `send_transaction`  | 发送原生代币或同质化代币                                |
| `swap_tokens`       | 兑换代币（Solana 上用 Jupiter，EVM 上用 Uniswap/1inch）|
| `bridge_tokens`     | 跨链桥接代币（Wormhole、LayerZero）                    |

### DeFi 工具

| 工具                     | 描述                                                             |
| ------------------------ | ---------------------------------------------------------------- |
| `lend_tokens`            | 向借贷协议供应代币（Aave、Kamino、Compound、Marginfi）          |
| `withdraw_lending`       | 提取供应的代币 + 利息                                           |
| `borrow_tokens`          | 基于抵押品借款                                                  |
| `repay_borrow`           | 偿还借款                                                        |
| `get_lending_positions`  | 查看借贷仓位                                                    |
| `stake_tokens`           | 质押获取流动性质押衍生品（Lido、Jito）                          |
| `unstake_tokens`         | 将衍生品赎回为原生代币                                          |
| `provide_liquidity`      | 向自动做市商（AMM）池添加代币（Uniswap LP、Orca LP）           |
| `remove_liquidity`       | 从 AMM 池中提取                                                 |

### 永续合约工具

| 工具                   | 描述                                                   |
| ---------------------- | ------------------------------------------------------ |
| `get_market_data`      | 获取市场价格、资金费率                                |
| `get_positions`        | 查看已开永续合约仓位                                  |
| `open_perp_position`   | 开设杠杆仓位（Hyperliquid、Jupiter Perps）            |
| `close_perp_position`  | 平仓（全部或部分）                                    |
| `place_limit_order`    | 下限价单（Hyperliquid）                               |
| `modify_perp_order`    | 修改已有限价单（Hyperliquid）                         |

### NFT 工具

| 工具                 | 描述                                                                    |
| -------------------- | ----------------------------------------------------------------------- |
| `get_nft_inventory`  | 列出钱包中的 NFT（Solana）                                            |
| `transfer_nft`       | 转移 NFT（Solana + EVM）                                              |
| `buy_nft`            | 通过交易市场工具封装购买 NFT（Blur、Magic Eden、OpenSea、Tensor）     |
| `sell_nft`           | 通过交易市场工具封装出售/挂单 NFT                                      |
| `bid_nft`            | 通过交易市场工具封装出价/报价                                          |

### 预测市场工具

| 工具                         | 描述                                |
| ---------------------------- | ----------------------------------- |
| `search_prediction_markets`  | 搜索 Polymarket 事件               |
| `get_prediction_orderbook`   | 查看中央限价订单簿（CLOB）深度     |
| `get_prediction_positions`   | 查看已开预测仓位                   |
| `place_prediction`           | 在 Polymarket 上下 CLOB 订单      |
| `close_prediction`           | 取消已有订单                       |

### Pump.fun 工具

| 工具                      | 描述                               |
| ------------------------- | ---------------------------------- |
| `pumpfun_list_new_coins`  | 发现近期发行的 meme 币            |
| `pumpfun_get_coin_info`   | 获取代币详细信息                   |
| `pumpfun_buy`             | 用 SOL 购买 Pump.fun 代币        |
| `pumpfun_sell`            | 将 Pump.fun 代币卖回 SOL         |

### 策略工具

| 工具             | 描述                                     |
| ---------------- | ---------------------------------------- |
| `get_policy`     | 查看当前策略（全局或按钱包）            |
| `update_policy`  | 更新策略规则                             |

详见 [MCP 工具参考](https://seashail.com/docs/reference/mcp-tools)，了解完整参数详情和各工具的参考页面。

## 安全模型

Seashail 假定 agent 进程可能是恶意的或已被攻破的。二进制程序本身是安全边界。

### 密钥存储

- **生成的钱包**使用沙米尔秘密分享（2-of-3）：密钥分片 1 用机器密钥加密，密钥分片 2 用机器密钥（或密码短语以便移植）加密，密钥分片 3 仅显示一次作为离线备份
- **导入的钱包**使用 AES-256-GCM 静态加密，密钥由密码短语派生（Argon2id + HKDF 子密钥）
- 签名后密钥材料会从内存中 zeroize

### 策略引擎

每个写操作都受以下规则约束：

- **单笔交易美元上限**（`max_usd_per_tx`）和**每日美元上限**（`max_usd_per_day`）
- **兑换滑点上限**（`max_slippage_bps`）
- **永续合约杠杆上限**（`max_leverage`、`max_usd_per_position`）
- **收款人白名单**（`send_allowlist`）和**合约白名单**（`contract_allowlist`）
- **操作开关**（可单独启用/禁用发送、兑换、跨链桥接、永续合约、NFT）
- **分级审批**（通过 MCP 交互确认）：
  - **自动审批** — 静默执行（低风险，在限额内）
  - **用户确认** — MCP 交互确认提示（超过自动审批阈值）
  - **硬性拒绝** — 拒绝（超过硬上限或违反策略）

### 威胁缓解

| 威胁                     | 缓解措施                                                                    |
| ------------------------ | --------------------------------------------------------------------------- |
| 恶意 agent               | 策略引擎 + 分级审批 + 白名单 + 操作开关                                   |
| 从日志窃取密钥           | MCP 交互确认（密钥不会出现在 agent 对话中）；工具 schema 拒绝密钥参数      |
| 脑裂状态                 | 独占文件锁；单例守护进程                                                   |
| 密码短语窃取             | 基于 TTL 的会话；过期后 zeroize；敏感缓冲区 mlock                          |
| 钓鱼/诈骗地址            | 签名诈骗地址黑名单；OFAC SDN 检查；收款人白名单                            |
| 超额消费                 | 单笔和每日美元上限；操作开关                                               |
| 未知美元价值利用         | `deny_unknown_usd_value`（默认失败关闭）                                    |
| 杠杆失控                 | `max_leverage` 和 `max_usd_per_position` 上限                               |

### 密钥托管对比

| 方面         | Seashail                   | 浏览器钱包               | 云端托管                 |
| ------------ | -------------------------- | ------------------------ | ------------------------ |
| 密钥位置     | 本地加密密钥存储           | 浏览器扩展               | 远程服务器               |
| Agent 访问   | 策略管控的 MCP 工具        | 直接签名                 | 使用提供商密钥的 API     |
| 恢复方式     | 沙米尔秘密分享 2-of-3 + 密码短语 | 助记词              | 提供商流程               |
| 风险模型     | Agent 受策略约束           | 用户逐笔验证             | 信任第三方               |

详见[安全模型文档](https://seashail.com/docs/guides/security-model)，了解完整威胁分析。

## 策略与审批

Seashail 在解密任何密钥材料之前，会根据策略评估每个写操作。

### 常用控制项

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

### 查看和更新

```
get_policy              View current policy (global or per-wallet)
update_policy           Replace policy rules
```

支持按钱包覆盖：`get_policy({ wallet })` 和 `update_policy({ wallet, policy })`。

详见[策略与审批指南](https://seashail.com/docs/guides/policy-and-approvals)和[策略工具参考](https://seashail.com/docs/reference/tools-policy)，了解全部 33+ 策略字段。

## 配置

### 路径

```bash
seashail paths
```

输出（JSON）：

```json
{
  "config_dir": "/Users/you/.config/seashail",
  "data_dir": "/Users/you/.local/share/seashail",
  "log_file": "/Users/you/.local/share/seashail/seashail.log.jsonl"
}
```

可通过 `SEASHAIL_CONFIG_DIR` 和 `SEASHAIL_DATA_DIR` 环境变量覆盖。

### 配置文件

`config.toml` 位于 `config_dir` 下：

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

### 网络模式

主网是默认模式。网络模式会影响工具省略 `chain`/`chains` 参数时的默认链选择。

- **持久化：** 在 `config.toml` 中设置 `network_mode = "testnet"` 或通过 MCP 调用 `set_network_mode`
- **会话级覆盖：** `seashail mcp --network testnet`
- **首次运行默认值：** `export SEASHAIL_NETWORK_MODE=testnet`（在 `config.toml` 创建之前）

详见[网络模式指南](https://seashail.com/docs/guides/network-mode)，了解 Solana RPC 默认值、空投和水龙头链接。

## CLI 参考

### seashail mcp

通过 stdio 运行 MCP 服务器（默认代理模式）。

```bash
seashail mcp                        # Proxy mode (recommended)
seashail mcp --network testnet      # Testnet override
seashail mcp --standalone           # No daemon sharing
```

### seashail daemon

运行单例守护进程（跨 MCP 客户端共享状态）。

```bash
seashail daemon                             # Run until terminated
seashail daemon --idle-exit-seconds 300     # Auto-exit after 5 min idle
```

通常不需要手动运行 —— `seashail mcp` 会自动启动守护进程。

### seashail agent

管理 agent 配置模板。

```bash
seashail agent list                         # List supported targets
seashail agent print cursor                 # Print config to stdout
seashail agent install cursor               # Install config
seashail agent install cursor --network testnet  # Testnet template
```

支持的目标：`cursor`、`vscode`、`windsurf`、`claude-desktop`。

### seashail openclaw install

安装 OpenClaw 插件集成。

```bash
seashail openclaw install
seashail openclaw install --network testnet
seashail openclaw install --link --plugin ./packages/openclaw-seashail-plugin  # Dev
```

### seashail doctor

自诊断报告（不含敏感信息，可安全粘贴到 issue 中）。

```bash
seashail doctor          # Human-readable
seashail doctor --json   # Machine-readable
```

### seashail paths

打印解析后的配置、数据和日志路径。

```bash
seashail paths
```

### seashail upgrade

升级到最新版本。

```bash
seashail upgrade              # Interactive
seashail upgrade --yes        # Non-interactive
seashail upgrade --yes --quiet  # Silent (for scripts)
```

详见 [CLI 参考文档](https://seashail.com/docs/reference/cli)，了解完整的命令行标志详情。

## Agent 集成

Seashail 适用于任何支持 MCP 的 agent。对于[快速开始](#快速开始)中未列出的 agent：

### 通用 MCP Stdio

配置你的 agent 运行：

- **Command:** `seashail`
- **Args:** `mcp`

如果你的 agent 使用 JSON 配置文件：

```json
{
  "mcpServers": {
    "seashail": { "command": "seashail", "args": ["mcp"] }
  }
}
```

或：

```json
{
  "servers": {
    "seashail": { "type": "stdio", "command": "seashail", "args": ["mcp"] }
  }
}
```

### 支持的 Agent

| Agent                    | 设置方式                                   | 文档                                                            |
| ------------------------ | ------------------------------------------ | --------------------------------------------------------------- |
| OpenClaw                 | `seashail openclaw install`                | [指南](https://seashail.com/docs/guides/agents/openclaw)        |
| Claude Code              | `claude mcp add seashail -- seashail mcp`  | [指南](https://seashail.com/docs/guides/agents/claude-code)     |
| Claude Desktop           | `seashail agent install claude-desktop`    | [指南](https://seashail.com/docs/guides/agents/claude-desktop)  |
| Codex                    | 编辑 `~/.codex/config.toml`               | [指南](https://seashail.com/docs/guides/agents/codex)           |
| Cursor                   | `seashail agent install cursor`            | [指南](https://seashail.com/docs/guides/agents/cursor)          |
| VS Code / GitHub Copilot | `seashail agent install vscode`           | [指南](https://seashail.com/docs/guides/agents/github-copilot)  |
| Windsurf                 | `seashail agent install windsurf`          | [指南](https://seashail.com/docs/guides/agents/windsurf)        |
| Cline                    | 手动 JSON 配置                             | [指南](https://seashail.com/docs/guides/agents/cline)           |
| Continue                 | 手动 JSON 配置                             | [指南](https://seashail.com/docs/guides/agents/continue)        |
| JetBrains                | 手动 JSON 配置                             | [指南](https://seashail.com/docs/guides/agents/jetbrains)       |
| 任何 MCP 客户端          | 通用 stdio 配置                            | [指南](https://seashail.com/docs/guides/agents/any-mcp-client)  |

静态配置模板也可在 [`packages/agent-configs/`](packages/agent-configs/) 中获取。

### 多客户端行为

多个 agent 通过单例守护进程安全共享相同的钱包、密码短语会话和策略计数器。每个 `seashail mcp` 进程都是一个轻量级代理。

- macOS/Linux：Unix socket 位于 `data_dir/seashail-mcp.sock`
- Windows：命名管道

## 指南

以下为摘要。详见[完整文档](https://seashail.com/docs)。

### 钱包与密钥存储

- **生成的钱包：** 沙米尔秘密分享 2-of-3（机器密钥分片 + 密码短语密钥分片 + 离线备份）
- **导入的钱包：** AES-256-GCM 加密，密钥由密码短语派生
- **密码短语会话：** 缓存在内存中，可配置 TTL；跨所有 MCP 客户端共享
- 工具：`create_wallet`、`import_wallet`、`list_wallets`、`get_wallet_info`、`get_deposit_info`、`export_shares`、`rotate_shares`

[钱包指南](https://seashail.com/docs/guides/wallets)

### 发送代币

`send_transaction` 可转账原生代币（SOL、ETH 等）或同质化代币（SPL、ERC-20）。Seashail 处理链特定机制（Solana 上的 ATA 创建、EVM 上的 ERC-20 授权），并在签名前验证地址。

[发送指南](https://seashail.com/docs/guides/sending)

### 兑换代币

`swap_tokens` 自动路由：Solana 上使用 Jupiter，EVM 上使用 Uniswap（或配置了 1inch 时使用 1inch）。滑点容差由策略强制执行。

[兑换指南](https://seashail.com/docs/guides/swapping)

### 跨链桥接代币

`bridge_tokens` 通过 Wormhole（默认）或 LayerZero 在链之间移动代币（Solana <-> EVM、EVM <-> EVM）。支持目标链自动赎回。使用 `get_bridge_status` 追踪进度。

[跨链桥接指南](https://seashail.com/docs/guides/bridging)

### DeFi 操作

四种 DeFi 基本操作，自动选择协议：

| 操作     | EVM 默认              | Solana 默认            |
| -------- | --------------------- | ---------------------- |
| 借贷     | Aave v3               | Kamino                 |
| 借款     | Aave v3、Compound v3  | Kamino、Marginfi       |
| 质押     | Lido (ETH -> stETH)   | Jito (SOL -> JitoSOL)  |
| 流动性   | Uniswap LP            | Orca LP                |

[DeFi 指南](https://seashail.com/docs/guides/defi)

### 永续合约交易

在两个场所进行杠杆永续合约交易：

| 场所           | 地址    | 测试网 | 订单类型       | 部分平仓 |
| -------------- | ------- | ------ | -------------- | -------- |
| Hyperliquid    | EVM     | 是     | 市价单 + 限价单 | 是       |
| Jupiter Perps  | Solana  | 否     | 仅市价单       | 否       |

策略控制：`enable_perps`、`max_leverage`、`max_usd_per_position`。

[永续合约指南](https://seashail.com/docs/guides/perps)

### NFT 操作

库存查询、直接转移和通过交易工具封装进行交易市场交易。

支持的交易市场：**Blur**（Ethereum）、**Magic Eden**（Solana + 跨链）、**OpenSea**（Ethereum）、**Tensor**（Solana）。

[NFT 指南](https://seashail.com/docs/guides/nfts)

### 预测市场

在 Polymarket（Polygon 上的 CLOB）上交易。搜索市场、查看订单簿、下限价/市价单、追踪仓位。

[预测市场指南](https://seashail.com/docs/guides/predictions)

### Pump.fun

在 Pump.fun（仅限 Solana）上发现、购买和出售 meme 币。联合曲线机制 —— 价格随需求变动。

[Pump.fun 指南](https://seashail.com/docs/guides/pumpfun)

### 链与充值

使用 `get_deposit_info` 获取存款地址。对于测试网，使用 `get_testnet_faucet_links`（EVM）或 `request_airdrop`（Solana devnet/测试网）。

[链与充值指南](https://seashail.com/docs/guides/chains-and-funding)

### 诈骗地址黑名单

可选的签名诈骗地址黑名单。通过 `config.toml` 启用。阻止向已知恶意地址的发送和 NFT 转移。获取失败时采用开放策略。

[诈骗地址黑名单指南](https://seashail.com/docs/guides/scam-blocklist)

## 验证

发布资产使用 Sigstore Cosign 签名。SBOM 使用 SPDX-JSON 格式生成。

```bash
cosign verify-blob \
  --certificate <asset>.crt \
  --signature <asset>.sig \
  <asset>
```

详见[验证文档](https://seashail.com/docs/reference/verification)。

## 故障排除

常见问题和解决方案：

| 错误                   | 原因                      | 解决方法                                      |
| ---------------------- | ------------------------- | --------------------------------------------- |
| `wallet_not_found`     | 钱包名称不存在            | 运行 `list_wallets`，检查拼写                |
| `passphrase_required`  | 会话已过期                | 在下次工具调用时重新输入密码短语              |
| `keystore_busy`        | 锁竞争                    | 等待并重试；检查是否有卡住的守护进程          |
| 策略硬性拒绝           | 超过美元上限              | 检查 `get_policy`，调整限额                  |
| 滑点超限               | 价格波动过大              | 增加 `max_slippage_bps` 或重试               |
| 操作已禁用             | 策略中开关已关闭          | 通过 `update_policy` 启用                    |
| 白名单拒绝             | 地址未被允许              | 添加到白名单或禁用                           |
| 未知美元价值           | 无定价数据                | 设置 `deny_unknown_usd_value: false` 或重试  |
| RPC 连接错误           | 网络/端点问题             | 检查连接，切换 RPC                           |
| 网络错误               | 主网/测试网混淆           | 检查 `get_network_mode`，切换                |

运行 `seashail doctor` 获取诊断报告。详见[故障排除文档](https://seashail.com/docs/troubleshooting)。

## 仓库结构

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

### 包 README

- [`packages/mcp/`](packages/mcp/) — `@seashail/mcp` npx 运行器
- [`packages/agent-configs/`](packages/agent-configs/) — 静态 MCP 配置模板
- [`packages/openclaw-seashail-plugin/`](packages/openclaw-seashail-plugin/) — OpenClaw 插件
- [`packages/e2e/`](packages/e2e/) — 端到端测试
- [`packages/skills-seashail/`](packages/skills-seashail/) — Agent Skills 规范和工作流
- [`python/`](python/) — `seashail-mcp` uvx 运行器

## 从源码构建

前置条件：`git`、`cargo`（Rust 工具链）、`bun`（用于 TypeScript 包）。

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

在你的 agent 中使用开发版本：

- **Command:** `./target/debug/seashail`
- **Args:** `mcp`

## 命令

```bash
bun install                  # Install dependencies
bun run build                # Build TypeScript packages
bun run test                 # Run tests
bun run check                # Type-check and lint

cargo build -p seashail      # Build Rust binary
cargo test -p seashail       # Run Rust tests
```

## 许可证

Apache-2.0
