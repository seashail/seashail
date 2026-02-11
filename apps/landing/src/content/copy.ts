/** Hero section copy for all landing page variants. */
export const hero = {
  headline: "Agent-native trading infrastructure",
  headlineAccent: "for crypto",
  subheadline:
    "A self-hosted binary that lets AI agents trade across DeFi without ever seeing your private keys.",
} as const;

/** Problem section copy describing the security gap. */
export const problem = {
  heading: "Your agent should never see your private key.",
  body: "When you give an AI agent your private key, you give it unlimited access to every asset in your wallet. One prompt injection, one compromised plugin, one hallucination \u2014 and your funds are gone.",
  incident:
    "The OpenClaw incident proved it: countless malicious skills discovered stealing private keys, prompt injection attacks draining wallets, and a single CVE enabling remote code execution with operator-level access.",
  highlight: "Your agent should never see your private key.",
} as const;

/** Solution section copy for the security boundary explanation. */
export const solution = {
  heading: "A security boundary, not a wrapper",
  subheading:
    "Seashail sits between the agent and your keys. The agent talks MCP. The binary handles everything else.",
  features: [
    {
      title: "Shamir's secret sharing",
      description:
        "Keys split into 2-of-3 shares at creation. No single point of compromise. Key bio shares stored separately.",
    },
    {
      title: "Policy engine",
      description:
        "Every transaction passes through configurable rules before signing. Per-transaction limits, daily caps, allowlists.",
    },
    {
      title: "MCP protocol",
      description:
        "Agents connect through stdio only. Structured, auditable tool calls. No raw key access, ever.",
    },
    {
      title: "Zeroize on use",
      description:
        "Key material decrypted only during signing, then immediately zeroed. No key data persists in memory.",
    },
  ],
} as const;

/** Architecture section copy for the data flow diagram. */
export const architecture = {
  heading: "How it works",
  description:
    "One binary. No servers. No HTTP. No external dependencies. Everything runs locally on your machine.",
  layers: [
    {
      label: "AI Agent",
      detail: "OpenClaw, Claude Code, Claude Desktop, Codex, any MCP client",
    },
    {
      label: "MCP Server (stdio)",
      detail: "Structured tool calls, auditable traffic",
    },
    {
      label: "Policy Engine",
      detail: "Rules, limits, approvals before signing",
    },
    {
      label: "Encrypted Wallet",
      detail: "Shamir shares, zeroize, AES-256-GCM",
    },
    { label: "Transaction Signer", detail: "Sign and broadcast, nothing else" },
    {
      label: "DeFi Protocols",
      detail: "Jupiter, Hyperliquid, 1inch, and more",
    },
  ],
} as const;

/** Trading surface section listing supported DeFi categories. */
export const tradingSurface = {
  heading: "Full DeFi surface area",
  subheading:
    "One binary, every major protocol. No API keys. No exchange accounts. Reuse your KYC.",
  categories: [
    {
      name: "Spot Trading",
      protocols: "Jupiter, 1inch, Uniswap",
      description: "Swap any token via DEX aggregators.",
    },
    {
      name: "Perpetuals",
      protocols: "Hyperliquid, Jupiter Perps",
      description: "Leveraged longs and shorts on crypto.",
    },
    {
      name: "NFTs",
      protocols: "Magic Eden, Tensor, Blur, OpenSea",
      description: "Buy, sell, bid, and manage collections.",
    },
    {
      name: "Predictions",
      protocols: "Polymarket",
      description: "Trade on real-world event outcomes.",
    },
    {
      name: "Lending",
      protocols: "Aave, Compound, Kamino, Marginfi",
      description: "Supply, borrow, and manage collateral.",
    },
    {
      name: "Staking",
      protocols: "Lido, Jito, EigenLayer, Marinade",
      description: "Stake and manage validator positions.",
    },
  ],
} as const;

/** Agent compatibility section with supported agent platforms. */
export const agentCompat = {
  heading: "Works with your agent",
  subheading:
    "Seashail exposes an MCP server over stdio. Any agent that speaks MCP can trade.",
  agents: [
    { name: "OpenClaw", status: "Full support" },
    { name: "Claude Code", status: "Full support" },
    { name: "Claude Desktop", status: "Full support" },
    { name: "Codex", status: "Full support" },
    { name: "Cursor", status: "Full support" },
    { name: "GitHub Copilot", status: "Full support" },
    { name: "Windsurf", status: "Full support" },
    { name: "Cline", status: "Full support" },
    { name: "Any MCP Client", status: "Full support" },
  ],
} as const;

/** Security model section describing protection mechanisms. */
export const security = {
  heading: "Security model",
  subheading:
    "Seashail allows rules to protect and what is shared. Read the code. Verify it.",
  features: [
    {
      title: "Encrypted at rest",
      description:
        "AES-256-GCM via libsodium. Keys/shares encrypted before touching disk.",
    },
    {
      title: "Shamir 2-of-3",
      description:
        "No single storage location holds a complete key. Machine, backup, and recovery shares.",
    },
    {
      title: "Zeroize",
      description:
        "Key material erased from memory immediately after signing. Uses the zeroize crate.",
    },
    {
      title: "Policy engine",
      description:
        "Per-transaction limits, daily caps, address allowlists. Configurable per wallet.",
    },
    {
      title: "Tiered approval",
      description:
        "Auto-approve low-risk transactions. Human confirmation above thresholds.",
    },
    {
      title: "Session expiry",
      description:
        "Passphrase sessions expire automatically. Configurable timeout.",
    },
    {
      title: "No secrets in logs",
      description:
        "Private keys, shares, and passphrases never appear in logs. Verified by E2E tests.",
    },
    {
      title: "Single binary, pure Rust",
      description:
        "Built entirely in Rust for memory safety without garbage collection. No dependencies, no sidecars, no runtime extensions. One auditable binary.",
    },
  ],
} as const;

/** Open source section copy. */
export const openSource = {
  heading: "Open source. Verifiable.",
  license: "Apache 2.0",
  points: [
    "Entire binary built in Rust",
    "Full source code on GitHub",
    "Reproducible builds from source",
    "Signed release binaries with SHA256",
    "Open cryptography: no proprietary algorithms",
    "Community contributions welcome",
  ],
} as const;

/** Call-to-action section copy. */
export const cta = {
  heading: "Start trading in 5 minutes",
  subheading:
    "Install the binary, fund a wallet, connect your agent. That is all it takes.",
} as const;
