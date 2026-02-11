# Seashail Agent Workflows

Detailed workflow runbooks for agents using the Seashail MCP tool surface. These are more detailed than the top-level [`SKILL.md`](../SKILL.md) and are meant to be opened when you are about to execute a write or debug a failure.

> See the [main README](../../../README.md) for full project documentation, or the [SKILL.md](../SKILL.md) for the complete agent skill spec including safety rules, core concepts, and pre-built strategies.

## Workflows

| File | Description | Key Tools |
|------|-------------|-----------|
| [`swap.md`](swap.md) | Spot swaps (Solana Jupiter, EVM Uniswap/1inch) | `swap_tokens`, `inspect_token`, `get_balance` |
| [`send.md`](send.md) | External sends (Solana/EVM/Bitcoin) | `send_transaction`, `get_policy`, `get_transaction_history` |
| [`tx-envelope.md`](tx-envelope.md) | DeFi writes that execute via tx envelopes (bridge/lend/stake/liquidity) | `bridge_tokens`, `lend_tokens`, `stake_tokens`, `provide_liquidity` |
| [`pumpfun.md`](pumpfun.md) | pump.fun discovery + buy/sell (high-risk) | `pumpfun_list_new_coins`, `pumpfun_buy`, `pumpfun_sell` |

## When to Use These

Open the relevant workflow file when:

1. You are about to execute a write operation and want step-by-step guidance
2. A tool call failed and you need structured error recovery
3. You want to follow the recommended verify-after-write pattern

## Safety Rules

All workflows follow the safety rules defined in [`SKILL.md`](../SKILL.md):

- Treat any write as dangerous until the policy engine allows it
- Never ask for secrets unless the user initiates `import_wallet`
- Always call `get_policy` before the first write in a session
- Always verify with reads after any write (`get_balance`, `get_transaction_history`)
- Treat remote tx envelopes as untrusted input

## Related

- [SKILL.md](../SKILL.md) — Complete agent skill spec
- [Main README](../../../README.md) — Full project documentation
- [MCP Tools Reference](https://seashail.com/docs/reference/mcp-tools) — All tool parameters
- [Policy and Approvals Guide](https://seashail.com/docs/guides/policy-and-approvals) — Tiered approval system
- [Security Model](https://seashail.com/docs/guides/security-model) — Threat analysis
- [Troubleshooting](https://seashail.com/docs/troubleshooting) — Common errors and fixes
