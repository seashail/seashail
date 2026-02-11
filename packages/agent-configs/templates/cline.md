# Cline (VS Code Extension)

Cline can connect to MCP servers over stdio.

Use the Cline UI:

1. Open Cline
2. Open MCP Servers
3. Add a server named `seashail`
4. Set:
   - Command: `seashail`
   - Args: `mcp`

Testnet mode (optional):

- Command: `seashail`
- Args: `mcp --network testnet`

If your environment supports file-based MCP config, you can also use the VS Code template:

- `packages/agent-configs/templates/vscode/mcp.json`
- `packages/agent-configs/templates/vscode/mcp.testnet.json`

