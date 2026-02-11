mod balance;
mod bridge_status;
mod defi_yield_pools;
mod estimate_gas;
mod inspect_token;
mod lending_positions;
mod portfolio;
mod portfolio_analytics;
mod prediction_markets;
mod prediction_positions;
mod pumpfun;
mod token_price;
mod tx_history;

use serde_json::Value;
use tokio::io::BufReader;

use super::super::jsonrpc::{err, JsonRpcResponse};
use super::super::{ConnState, SharedState};

pub async fn handle<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &ConnState,
    _stdin: &mut tokio::io::Lines<BufReader<R>>,
    _stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match tool_name {
        "get_defi_yield_pools" => defi_yield_pools::handle(req_id, args).await,
        "inspect_token" => inspect_token::handle(req_id, args, shared, conn).await,
        "get_balance" => balance::handle(req_id, args, shared, conn).await,
        "get_token_price" => token_price::handle(req_id, args, shared, conn).await,
        "estimate_gas" => estimate_gas::handle(req_id, args, shared, conn).await,
        "get_portfolio" => portfolio::handle(req_id, args, shared, conn).await,
        "get_portfolio_analytics" => portfolio_analytics::handle(req_id, &args, shared).await,
        "get_transaction_history" => tx_history::handle(req_id, &args, shared),
        "pumpfun_list_new_coins" | "pumpfun_get_coin_info" => {
            pumpfun::handle(req_id, tool_name, args, shared).await
        }
        "get_lending_positions" => lending_positions::handle(req_id, args, shared, conn).await,
        "get_prediction_positions" => {
            prediction_positions::handle(req_id, args, shared, conn).await
        }
        "search_prediction_markets" | "get_prediction_orderbook" => {
            prediction_markets::handle(req_id, tool_name, args, shared, conn).await
        }
        "get_bridge_status" => bridge_status::handle(req_id, args, shared).await,
        _ => Ok(err(req_id, -32601, "unknown tool")),
    }
}
