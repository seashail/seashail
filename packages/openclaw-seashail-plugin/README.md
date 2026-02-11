# Seashail OpenClaw Plugin

An [OpenClaw](https://openclaw.ai) plugin that exposes Seashail tools as native OpenClaw agent tools by spawning a local `seashail mcp` subprocess.

> See the [main README](../../README.md) for full project documentation, or the [OpenClaw setup guide](https://seashail.com/docs/guides/agents/openclaw) for step-by-step instructions.

## Install

### Via Seashail CLI (Recommended)

```bash
seashail openclaw install
```

This installs the plugin, enables it, configures sandbox permissions, creates a default wallet if needed, and restarts the OpenClaw gateway.

Testnet mode:

```bash
seashail openclaw install --network testnet
```

### Via OpenClaw CLI

```bash
openclaw plugins install @seashail/seashail
openclaw plugins enable seashail
openclaw gateway restart
```

### Dev Install

```bash
seashail openclaw install --link --plugin ./packages/openclaw-seashail-plugin
```

## How It Works

- The plugin spawns `seashail mcp` as a subprocess using MCP stdio JSON-RPC transport (one JSON object per line)
- All Seashail MCP tools become available as native OpenClaw agent tools
- The plugin connects to the singleton `seashail daemon` for shared state (keystore, passphrase session, policy)

## Interactive Confirmation

Some Seashail actions require interactive confirmation and/or passphrase unlock:

- **Confirmation prompts** (policy-gated write operations) can be completed by calling the `seashail_resume` tool
- **Passphrase prompts** should generally be handled via an env var (see `passphraseEnvVar`), not by pasting secrets into chat

## CLI Flags

The `seashail openclaw install` command supports:

| Flag | Default | Description |
|------|---------|-------------|
| `--network` | `mainnet` | Session-only network mode |
| `--plugin` | Auto-detected | Plugin path or npm spec |
| `--link` | `false` | Link local plugin path instead of copying |
| `--restart-gateway` | `true` | Restart OpenClaw gateway after install |
| `--enable-in-sandbox` | `true` | Allow plugin in sandboxed agent mode |
| `--onboard-wallet` | `true` | Create default wallet if none exists |

See the [CLI Reference](https://seashail.com/docs/reference/cli#seashail-openclaw-install) for full details.

## Related

- [Main README](../../README.md) — Full project documentation
- [OpenClaw Setup Guide](https://seashail.com/docs/guides/agents/openclaw) — Full OpenClaw integration guide
- [Agent Config Templates](../agent-configs/) — Static config templates for other agents
- [Security Model](https://seashail.com/docs/guides/security-model) — How policy gating works
- [E2E Tests](../e2e/) — End-to-end tests including OpenClaw integration tests
