# Agent Config Templates

Static, copy-pasteable MCP configuration templates for popular agents/editors.

> See the [main README](../../README.md) for full project documentation, or the [Agent Integration guide](https://seashail.com/docs/guides/agents) for step-by-step setup instructions for each agent.

## How It Works

These templates configure an MCP stdio server named `seashail` that runs:

```bash
seashail mcp
```

`seashail mcp` is a thin stdio proxy that forwards requests to a singleton local `seashail daemon` (it autostarts the daemon if needed). This allows multiple agent clients to share one wallet state and one in-memory passphrase session safely. See the [Architecture docs](https://seashail.com/docs/reference/architecture) for details.

## Included Templates

| Agent | Template Path | Testnet Template |
|-------|---------------|-----------------|
| Cursor | `templates/cursor/mcp.json` | `templates/cursor/mcp.testnet.json` |
| VS Code / GitHub Copilot | `templates/vscode/mcp.json` | `templates/vscode/mcp.testnet.json` |
| Windsurf | `templates/windsurf/mcp_config.json` | `templates/windsurf/mcp_config.testnet.json` |
| Claude Desktop | `templates/claude-desktop/claude_desktop_config.json` | `templates/claude-desktop/claude_desktop_config.testnet.json` |
| Continue | `templates/continue/mcp.json` | `templates/continue/mcp.testnet.json` |
| JetBrains | `templates/jetbrains/mcp.json` | `templates/jetbrains/mcp.testnet.json` |

## One-Click Install (Recommended)

Instead of manually copying templates, use the CLI:

```bash
seashail agent list                           # List supported targets
seashail agent install cursor                 # Install mainnet config
seashail agent install cursor --network testnet  # Install testnet config
seashail agent install claude-desktop
seashail agent install vscode
seashail agent install windsurf
```

If `seashail` is not on your `PATH` (e.g., running a dev build from `./target/debug/seashail`), installed templates will point at the currently-running binary via an absolute path.

## Testnet Mode

Testnet templates run:

```bash
seashail mcp --network testnet
```

This is a per-session override and does not change your persisted `config.toml` unless you explicitly call the MCP tool `set_network_mode` or edit config. See the [Network Mode Guide](https://seashail.com/docs/guides/network-mode) for details.

## Generic MCP Stdio Config

For agents not listed above, use the generic shape:

```json
{
  "mcpServers": {
    "seashail": { "command": "seashail", "args": ["mcp"] }
  }
}
```

Or with the "no install" npx wrapper:

```json
{
  "mcpServers": {
    "seashail": { "command": "npx", "args": ["-y", "@seashail/mcp", "--"] }
  }
}
```

## Related

- [Main README](../../README.md) — Full project documentation
- [`packages/mcp/`](../mcp/) — `@seashail/mcp` npx wrapper
- [`python/`](../../python/) — `seashail-mcp` uvx wrapper
- [Agent Integration Guide](https://seashail.com/docs/guides/agents) — Step-by-step setup for all agents
- [CLI Reference](https://seashail.com/docs/reference/cli) — `seashail agent` subcommands
