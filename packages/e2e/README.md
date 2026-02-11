# E2E Tests

End-to-end tests for Seashail, running against real binaries (`seashail` and optionally `openclaw`).

> See the [main README](../../README.md) for full project documentation.

## What's Tested

- **Policy rejection and tiered approval boundaries** — verifies the policy engine correctly auto-approves, confirms, and hard-blocks transactions based on USD caps and limits
- **Passphrase session expiry behavior** — verifies that passphrase sessions expire after TTL and re-authentication is required
- **No secrets in logs** — verifies that private keys, Shamir shares, and passphrases never appear in tool responses or logs
- **Concurrency and single-writer guarantees** — verifies that the daemon's exclusive lock prevents split-brain state under concurrent access
- **OpenClaw plugin integration** — verifies Seashail tools work as native OpenClaw agent tools

## Prerequisites

- The `seashail` binary built from this repo (see [Building from Source](../../README.md#building-from-source))
- `bun` (package manager)
- For OpenClaw tests: `openclaw` CLI installed

## Run

```bash
bun test --max-concurrency=1
```

OpenClaw-focused suite:

```bash
bun run test:openclaw
```

## OpenClaw Agent Chat Test (Optional)

`src/openclaw-agent-chat.e2e.test.ts` exercises `openclaw agent` (actual chat turns) and verifies the agent uses Seashail tools by comparing returned deposit addresses against direct tool calls.

It runs by default when `openclaw` is installed.

### Auth Setup

- **CI:** set `OPENCLAW_E2E_ANTHROPIC_TOKEN` (the test pastes it into the harness state dir)
- **Local dev:** if you already have OpenClaw configured, the test copies your auth store into the harness state dir (no changes to `~/.openclaw`)

## Related

- [Main README](../../README.md) — Full project documentation
- [Security Model](https://seashail.com/docs/guides/security-model) — Threat model and policy engine details
- [Policy and Approvals](https://seashail.com/docs/guides/policy-and-approvals) — Tiered approval system
- [OpenClaw Plugin](../openclaw-seashail-plugin/) — OpenClaw plugin source
- [Architecture](https://seashail.com/docs/reference/architecture) — Proxy/daemon design
