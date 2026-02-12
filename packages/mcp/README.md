# `@seashail/mcp`

Run the Seashail MCP server in one line via `npx`. This is a thin wrapper that ensures `seashail` is installed and runs `seashail mcp` for you.

> See the [main README](../../README.md) for full project documentation, or the [Install docs](https://seashail.com/docs/getting-started/install) for all installation methods.

## Usage

```bash
npx -y @seashail/mcp
```

Pass through args to `seashail mcp`:

```bash
npx -y @seashail/mcp -- --network testnet
```

## Use in Agent MCP Config

If you don't want to install the `seashail` binary globally, reference this package in your agent's MCP config instead:

```json
{
  "mcpServers": {
    "seashail": {
      "command": "npx",
      "args": ["-y", "@seashail/mcp", "--"]
    }
  }
}
```

Testnet:

```json
{
  "mcpServers": {
    "seashail-testnet": {
      "command": "npx",
      "args": ["-y", "@seashail/mcp", "--", "--network", "testnet"]
    }
  }
}
```

## How It Works

- If `seashail` is already installed, the wrapper runs it directly
- If `seashail` is not installed, the wrapper executes the hosted installer:
  - macOS/Linux: `curl -fsSL https://seashail.com/install | sh`
  - Windows (PowerShell): `irm https://seashail.com/install.ps1 | iex`
- After installation, it runs `seashail mcp` with any provided args

## Alternatives

- **Direct install (recommended):** `curl -fsSL https://seashail.com/install | sh` — see [Install docs](https://seashail.com/docs/getting-started/install)
- **Python wrapper:** `uvx seashail-mcp` — see [`python/`](../../python/)
- **One-click agent templates:** `seashail agent install <target>` — see [Agent Integration](../../README.md#agent-integration)

## Related

- [Main README](../../README.md) — Full project documentation
- [Agent Config Templates](../agent-configs/) — Static config templates for editors/agents
- [Install Docs](https://seashail.com/docs/getting-started/install) — All installation methods
- [Quickstart](https://seashail.com/docs/getting-started/quickstart) — First run guide
