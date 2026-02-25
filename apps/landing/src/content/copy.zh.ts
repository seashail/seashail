/** Hero 区域文案 — 简体中文。 */
export const hero = {
  headline: "智能体原生交易基础设施",
  headlineAccent: "为加密货币打造",
  subheadline:
    "一个自托管的二进制程序，让 AI 智能体在 DeFi 中进行交易，同时永远无法接触你的私钥。",
} as const;

/** 问题区域文案 — 描述安全风险。 */
export const problem = {
  heading: "你的智能体永远不应看到你的私钥。",
  body: "当你把私钥交给 AI 智能体时，你就赋予了它对钱包中所有资产的无限访问权限。一次提示注入、一个被入侵的插件、一次幻觉——你的资金就没了。",
  incident:
    "OpenClaw 事件已经证明了这一点：无数恶意技能被发现窃取私钥，提示注入攻击导致钱包被清空，一个 CVE 漏洞就能以运营者级别的权限实现远程代码执行。",
  highlight: "你的智能体永远不应看到你的私钥。",
} as const;

/** 解决方案区域文案 — 安全边界说明。 */
export const solution = {
  heading: "安全边界，而非封装",
  subheading:
    "Seashail 位于智能体和你的密钥之间。智能体通过 MCP 通信，二进制程序处理其余一切。",
  features: [
    {
      title: "沙米尔秘密分享",
      description:
        "密钥在创建时被拆分为 2-of-3 分片。没有单一泄露点。密钥分片分别存储。",
      docPath: "/guides/security-model",
    },
    {
      title: "策略引擎",
      description:
        "每笔交易在签名前都会通过可配置的规则。支持单笔限额、每日上限、白名单。",
      docPath: "/guides/policy-and-approvals",
    },
    {
      title: "MCP 协议",
      description:
        "智能体仅通过 stdio 连接。结构化、可审计的工具调用。永远无法直接访问密钥。",
      docPath: "/guides/agents",
    },
    {
      title: "使用后清零",
      description:
        "密钥材料仅在签名时解密，随后立即清零。内存中不保留任何密钥数据。",
      docPath: "/guides/security-model",
    },
  ],
} as const;

/** 架构区域文案 — 数据流图说明。 */
export const architecture = {
  heading: "工作原理",
  description:
    "一个二进制程序。无服务器。无 HTTP。无外部依赖。一切都在你的本地机器上运行。",
  layers: [
    {
      label: "AI 智能体",
      detail: "OpenClaw、Claude Code、Claude Desktop、Codex 及任何 MCP 客户端",
    },
    {
      label: "MCP 服务器 (stdio)",
      detail: "结构化工具调用，可审计的通信流量",
    },
    {
      label: "策略引擎",
      detail: "规则、限额、签名前审批",
    },
    {
      label: "加密钱包",
      detail: "沙米尔分片、内存清零、AES-256-GCM",
    },
    { label: "交易签名器", detail: "签名并广播，仅此而已" },
    {
      label: "DeFi 协议",
      detail: "Jupiter、Hyperliquid、1inch 等",
    },
  ],
} as const;

/** 交易范围区域文案 — 支持的 DeFi 类别列表。 */
export const tradingSurface = {
  heading: "完整的 DeFi 交易范围",
  subheading:
    "一个二进制程序，覆盖所有主流协议。无需 API 密钥。无需交易所账户。复用你的 KYC。",
  categories: [
    {
      name: "现货交易",
      protocols: "Jupiter、1inch、Uniswap",
      description: "通过 DEX 聚合器兑换任意代币。",
      docPath: "/guides/swapping",
    },
    {
      name: "永续合约",
      protocols: "Hyperliquid、Jupiter Perps",
      description: "加密货币的杠杆做多和做空。",
      docPath: "/guides/perps",
    },
    {
      name: "NFT",
      protocols: "Magic Eden、Tensor、Blur、OpenSea",
      description: "买入、卖出、竞价和管理收藏品。",
      docPath: "/guides/nfts",
    },
    {
      name: "预测市场",
      protocols: "Polymarket",
      description: "基于真实事件结果进行交易。",
      docPath: "/guides/predictions",
    },
    {
      name: "借贷",
      protocols: "Aave、Compound、Kamino、Marginfi",
      description: "存入、借出和管理抵押品。",
      docPath: "/guides/defi",
    },
    {
      name: "质押",
      protocols: "Lido、Jito、EigenLayer、Marinade",
      description: "质押和管理验证者仓位。",
      docPath: "/guides/defi",
    },
  ],
} as const;

/** 智能体兼容性区域文案。 */
export const agentCompat = {
  heading: "兼容你的智能体",
  subheading:
    "Seashail 通过 stdio 暴露 MCP 服务器。任何支持 MCP 的智能体都可以进行交易。",
  agents: [
    {
      name: "OpenClaw",
      status: "完全支持",
      docPath: "/guides/agents/openclaw",
    },
    {
      name: "Claude Code",
      status: "完全支持",
      docPath: "/guides/agents/claude-code",
    },
    {
      name: "Claude Desktop",
      status: "完全支持",
      docPath: "/guides/agents/claude-desktop",
    },
    { name: "Codex", status: "完全支持", docPath: "/guides/agents/codex" },
    {
      name: "Cursor",
      status: "完全支持",
      docPath: "/guides/agents/cursor",
    },
    {
      name: "GitHub Copilot",
      status: "完全支持",
      docPath: "/guides/agents/github-copilot",
    },
    {
      name: "Windsurf",
      status: "完全支持",
      docPath: "/guides/agents/windsurf",
    },
    { name: "Cline", status: "完全支持", docPath: "/guides/agents/cline" },
    {
      name: "任何 MCP 客户端",
      status: "完全支持",
      docPath: "/guides/agents/any-mcp-client",
    },
  ],
} as const;

/** 安全模型区域文案。 */
export const security = {
  heading: "安全模型",
  subheading: "Seashail 让规则决定保护什么、共享什么。阅读源码，自行验证。",
  features: [
    {
      title: "静态加密",
      description:
        "通过 libsodium 实现 AES-256-GCM 加密。密钥和分片在写入磁盘前即已加密。",
      docPath: "/guides/security-model",
    },
    {
      title: "沙米尔 2-of-3",
      description:
        "没有任何单一存储位置持有完整密钥。机器分片、备份分片和恢复分片分开存储。",
      docPath: "/guides/security-model",
    },
    {
      title: "内存清零",
      description:
        "密钥材料在签名后立即从内存中擦除。使用 zeroize crate 实现。",
      docPath: "/guides/security-model",
    },
    {
      title: "策略引擎",
      description: "单笔交易限额、每日上限、地址白名单。可按钱包配置。",
      docPath: "/guides/policy-and-approvals",
    },
    {
      title: "分级审批",
      description: "低风险交易自动批准。超过阈值的交易需要人工确认。",
      docPath: "/guides/policy-and-approvals",
    },
    {
      title: "会话过期",
      description: "密码短语会话自动过期。超时时间可配置。",
      docPath: "/guides/security-model",
    },
    {
      title: "日志无秘密",
      description:
        "私钥、分片和密码短语永远不会出现在日志中。通过端到端测试验证。",
      docPath: "/guides/security-model",
    },
    {
      title: "单一二进制，纯 Rust",
      description:
        "完全使用 Rust 构建，具备无垃圾回收的内存安全。无依赖、无边车进程、无运行时扩展。一个可审计的二进制程序。",
      docPath: "/guides/security-model",
    },
  ],
} as const;

/** 开源区域文案。 */
export const openSource = {
  heading: "开源。可验证。",
  license: "Apache 2.0",
  points: [
    "整个二进制程序用 Rust 构建",
    "完整源代码托管在 GitHub",
    "支持从源码可复现构建",
    "签名发布的二进制文件附带 SHA256 校验",
    "开放密码学：无专有算法",
    "欢迎社区贡献",
  ],
} as const;

/** 行动号召区域文案。 */
export const cta = {
  heading: "5 分钟开始交易",
  subheading: "安装二进制程序，为钱包充值，连接你的智能体。就这么简单。",
} as const;

/** 共享 UI 字符串 — 简体中文。 */
export const ui = {
  goToDocs: "查看文档",
  github: "GitHub",
  languageSwitcherLabel: "语言",
  theProblemHeading: "关键问题",
} as const;
