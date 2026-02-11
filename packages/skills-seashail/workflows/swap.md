# Workflow: Spot Swap

Goal: swap one token for another using Seashail with policy gating and identity checks.

Preconditions:

1. You know the chain (`solana` or an EVM chain name).
2. The wallet has enough balance of the input token.
3. Policy allows swaps and slippage is within caps.

Inputs (must be explicit):

1. `chain`
2. `wallet`, `account_index` (or confirm active wallet/account)
3. `token_in`, `token_out` canonical identities
4. `amount` and `amount_units`
5. `slippage_bps` (or rely on policy default)

Steps:

1. Wallet reality-check: `list_wallets`, then `get_wallet_info` for addresses if needed.
2. Token identity:
   1. `inspect_token` for `token_in`.
   2. `inspect_token` for `token_out`.
   3. If the user provided only symbols and multiple candidates exist, ask the user to confirm the exact address before proceeding.
3. Funds check: `get_balance` for `token_in`.
4. Policy check: `get_policy` and confirm `max_slippage_bps`, USD caps, and allowlisting posture.
5. Execute: `swap_tokens` with conservative `slippage_bps`.
6. If elicitation appears, present the confirmation summary (chain, from wallet/account, tokens, amounts, USD estimate, slippage).
7. Verify:
   1. `get_transaction_history` shows the swap entry.
   2. `get_balance` reflects the expected deltas.

Failure handling:

1. `policy_*`: do not loosen policy implicitly; ask the user if they want to adjust it, then `update_policy`.
2. `simulation_failed`: treat as a hard stop; re-check token identity, chain RPC config, and slippage.
3. `upstream_error` or `rpc_error`: retry once, then ask the user whether to configure RPC fallbacks via `configure_rpc`.
