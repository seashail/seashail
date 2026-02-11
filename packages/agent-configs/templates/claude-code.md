# Claude Code

Claude Code supports MCP servers via the `claude` CLI.

Add Seashail:

```bash
claude mcp add seashail seashail mcp
```

Testnet mode (optional):

```bash
claude mcp add seashail-testnet seashail mcp --network testnet
```

Notes:

- This creates an MCP server named `seashail` (or `seashail-testnet`) that launches the Seashail binary over stdio.
- If your Seashail install isn't on `PATH`, use the absolute path to the binary.

