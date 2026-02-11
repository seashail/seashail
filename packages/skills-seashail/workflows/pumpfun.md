# Workflow: pump.fun (Discovery + Buy/Sell)

pump.fun is a high-risk surface. The default stance is small size, strict caps, and strict mint identity checks.

Inputs (must be explicit):

1. `wallet`, `account_index` (or a wallet pool plan)
2. max spend per buy and per time window
3. mint identity confirmation rule (never by symbol alone)

Steps:

1. Policy check: `get_policy` and confirm pump.fun is enabled and caps are understood.
2. If multi-wallet is desired, run `create_wallet_pool` then `fund_wallets`.
3. Discovery:
   1. `pumpfun_list_new_coins` to get candidate mints.
   2. For each candidate, `pumpfun_get_coin_info` and verify the mint matches the intended coin.
4. Execution:
   1. `pumpfun_buy` with an explicit small amount per wallet/account.
   2. Handle confirmation if elicited.
5. Exit:
   1. `pumpfun_sell` with explicit sizing.
6. Verify:
   1. `get_transaction_history` shows the buy/sell.
   2. `get_balance` reflects expected SOL movement.

Failure handling:

1. `policy_pumpfun_disabled` or pump.fun caps: stop and ask the user whether to change policy.
2. `simulation_failed`: treat as a hard stop; assume the envelope or mint identity is wrong.
