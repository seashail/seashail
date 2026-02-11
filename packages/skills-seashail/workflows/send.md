# Workflow: External Send (Solana, EVM, Bitcoin)

Goal: transfer funds to a non-Seashail address safely.

Inputs (must be explicit):

1. `chain`
2. `wallet`, `account_index` (or confirm active wallet/account)
3. destination address (`to` for Solana/EVM; `to` for Bitcoin as well)
4. `token` and `amount` (Bitcoin supports `token=native` only)
5. `amount_units` (UI vs base)

Steps:

1. Wallet reality-check: `list_wallets`, then `get_wallet_info` for the sender address if needed.
2. Policy check: `get_policy` and confirm `enable_send`, allowlisting posture (`send_allow_any`), and per-tx and daily USD caps.
3. If allowlisting is required and the destination is not approved, stop and ask the user whether they want to allowlist the address (do not proceed implicitly).
4. Optional fee preview: `estimate_gas` (Solana/EVM).
5. Execute: `send_transaction`.
6. If elicitation appears, present the confirmation summary and ask the user to approve or decline.
7. Verify:
   1. `get_transaction_history` includes the send.
   2. `get_balance` decreased by approximately amount plus fees.

Bitcoin-specific notes:

1. Bitcoin sends always require confirmation.
2. BTC uses Blockstream-compatible HTTP endpoints; if they are not configured, ask the user to configure them (or run in a mode where defaults exist).

Failure handling:

1. `policy_send_disabled` or `policy_*`: ask the user whether to change policy, then `update_policy`.
2. `ofac_sdn_blocked` or scam blocklist blocks: treat as hard stop and inform the user.
3. `invalid_request`: re-collect missing fields (chain, to, amount, units).
