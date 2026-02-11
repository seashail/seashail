# Workflow: Tx-Envelope DeFi Writes

Applies to write tools that execute via a transaction envelope (often adapter-constructed), such as:

1. `bridge_tokens` (non-native paths, including LayerZero)
2. `lend_tokens`, `borrow_tokens`, `repay_borrow`, `withdraw_lending` (adapter paths)
3. `stake_tokens`, `unstake_tokens` (adapter paths)
4. `provide_liquidity`, `remove_liquidity`

Principle: envelope construction is untrusted input. Require explicit user intent, allowlisting, and post-write verification.

Inputs (must be explicit):

1. `chain`
2. `wallet`, `account_index` (or confirm active wallet/account)
3. provider/protocol/venue (as applicable)
4. amounts and spend bounds

Steps:

1. Discover configuration: `get_capabilities` and confirm the relevant surface is available.
2. Policy check: `get_policy` and confirm the surface toggle is enabled (bridge/lending/staking/liquidity), caps (`max_usd_per_*`) are compatible, and allowlisting posture for EVM contracts is compatible (built-in allowlist vs explicit allowlist; whether `contract_allow_any` is enabled).
3. Execute the write tool.
4. If elicitation appears, summarize chain and wallet/account, destination contract/program and provider, amount and USD estimate (or why unknown), and policy constraints that apply.
5. Verify:
   1. `get_transaction_history` shows the operation.
   2. For bridging: poll `get_bridge_status` until it reaches a terminal state.
   3. For lending: `get_lending_positions`.
   4. For perps/staking/liquidity: verify with the relevant read surface when available (or at minimum balances and history).

Failure handling:

1. `policy_contract_not_allowlisted`: stop and ask whether the user wants to allowlist the destination contract. Do not override implicitly.
2. `simulation_failed`: treat as a hard stop. For adapter envelopes, assume the envelope may be incorrect, mis-targeted, or unsafe.
3. `defi_adapter_not_configured`: ask the user to configure the adapter or to use a native path if available.
