mod helpers;
mod key_loading;
mod network;
mod nft;
mod perps;
mod policy;
mod policy_confirm;
mod read;
mod schema;
mod value_helpers;
mod wallets;
mod write;

pub use schema::list_tools_result;

use serde_json::Value;
use tokio::io::BufReader;

use super::jsonrpc::{err, ok, tool_err, JsonRpcResponse};
use super::{ConnState, SharedState};
use crate::errors::ToolError;

fn tool_triggers_first_run_setup(tool_name: &str) -> bool {
    // Trigger first-run setup on first agent interaction with wallet-dependent tools.
    // We intentionally exclude informational/config-only tools.
    matches!(
        tool_name,
        "get_balance"
            | "get_portfolio"
            | "get_portfolio_analytics"
            | "estimate_gas"
            | "get_market_data"
            | "get_positions"
            | "list_wallets"
            | "get_wallet_info"
            | "get_deposit_info"
            | "set_active_wallet"
            | "add_account"
            | "create_wallet_pool"
            | "export_shares"
            | "rotate_shares"
            | "request_airdrop"
            | "send_transaction"
            | "swap_tokens"
            | "transfer_between_wallets"
            | "fund_wallets"
            | "pumpfun_buy"
            | "pumpfun_sell"
            | "bridge_tokens"
            | "lend_tokens"
            | "withdraw_lending"
            | "borrow_tokens"
            | "repay_borrow"
            | "get_lending_positions"
            | "stake_tokens"
            | "unstake_tokens"
            | "provide_liquidity"
            | "remove_liquidity"
            | "place_prediction"
            | "close_prediction"
            | "get_prediction_positions"
            | "open_perp_position"
            | "close_perp_position"
            | "modify_perp_order"
            | "place_limit_order"
            | "get_nft_inventory"
            | "transfer_nft"
            | "buy_nft"
            | "sell_nft"
            | "bid_nft"
    )
}

pub async fn handle_tools_call<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    // On first run, create a generated wallet via elicitation before servicing wallet-dependent tools.
    if tool_triggers_first_run_setup(tool_name) && shared.ks.list_wallets()?.is_empty() {
        // Non-interactive: auto-create a machine-bound default wallet so the agent can immediately
        // show deposit addresses and balances. Users can opt into portable recovery (passphrase +
        // Share 3) later via `export_shares` / `rotate_shares`.
        shared.ks.ensure_default_wallet()?;
    }

    // Write DeFi tools: require explicit non-empty chain to avoid surprising defaults.
    if matches!(
        tool_name,
        "bridge_tokens"
            | "lend_tokens"
            | "withdraw_lending"
            | "borrow_tokens"
            | "repay_borrow"
            | "stake_tokens"
            | "unstake_tokens"
            | "provide_liquidity"
            | "remove_liquidity"
            | "place_prediction"
            | "close_prediction"
    ) {
        let chain = args.get("chain").and_then(|v| v.as_str()).unwrap_or("");
        if chain.trim().is_empty() {
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("invalid_request", "missing chain")),
            ));
        }
    }

    match tool_name {
        // Network/config tools
        "get_network_mode"
        | "set_network_mode"
        | "get_capabilities"
        | "get_testnet_faucet_links"
        | "configure_rpc" => network::handle(req_id, tool_name, args, shared, conn),

        // Policy tools
        "get_policy" | "update_policy" => policy::handle(req_id, tool_name, &args, shared),

        // Read-only tools
        "inspect_token"
        | "get_defi_yield_pools"
        | "get_balance"
        | "get_token_price"
        | "estimate_gas"
        | "get_portfolio"
        | "get_portfolio_analytics"
        | "get_transaction_history"
        | "pumpfun_list_new_coins"
        | "pumpfun_get_coin_info"
        | "get_lending_positions"
        | "get_prediction_positions"
        | "get_bridge_status" => {
            read::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }

        // Perpetuals tools
        "get_market_data"
        | "get_positions"
        | "open_perp_position"
        | "close_perp_position"
        | "modify_perp_order"
        | "place_limit_order" => {
            perps::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }

        // NFT tools
        "get_nft_inventory" | "transfer_nft" | "buy_nft" | "sell_nft" | "bid_nft" => {
            nft::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }

        // Wallet management tools
        "list_wallets" | "get_wallet_info" | "get_deposit_info" | "set_active_wallet"
        | "add_account" | "create_wallet_pool" | "create_wallet" | "import_wallet"
        | "export_shares" | "rotate_shares" => {
            wallets::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }

        // Write/signer tools
        "request_airdrop"
        | "send_transaction"
        | "swap_tokens"
        | "transfer_between_wallets"
        | "fund_wallets"
        | "pumpfun_buy"
        | "pumpfun_sell"
        | "bridge_tokens"
        | "lend_tokens"
        | "withdraw_lending"
        | "borrow_tokens"
        | "repay_borrow"
        | "stake_tokens"
        | "unstake_tokens"
        | "provide_liquidity"
        | "remove_liquidity"
        | "place_prediction"
        | "close_prediction" => {
            write::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }

        _ => Ok(err(req_id, -32601, "unknown tool")),
    }
}
