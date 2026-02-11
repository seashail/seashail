# OpenClaw

Recommended (first-class): install the Seashail OpenClaw plugin:

```bash
seashail openclaw install
```

This uses OpenClaw's official plugin mechanism so Seashail tools show up as native OpenClaw tools.

If your OpenClaw build supports MCP stdio servers directly, configure a server named `seashail` with:

- Command: `seashail`
- Args: `mcp`

Testnet mode (optional):

- Command: `seashail`
- Args: `mcp --network testnet`

If your OpenClaw build does not support MCP natively, use an MCP bridge tool that can run stdio MCP servers (for example, a bridge skill that proxies MCP tool calls to a local stdio process), and point it at `seashail mcp`.
