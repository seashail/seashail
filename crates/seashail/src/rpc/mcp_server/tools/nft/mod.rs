mod common;
mod inventory;
mod trade;
mod transfer;

use serde_json::Value;

use super::super::jsonrpc::{err, JsonRpcResponse};
use super::super::{ConnState, SharedState};

pub async fn handle<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<tokio::io::BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match tool_name {
        "get_nft_inventory" => inventory::handle(req_id, args, shared, conn).await,
        "transfer_nft" => transfer::handle(req_id, args, shared, conn, stdin, stdout).await,
        "buy_nft" | "sell_nft" | "bid_nft" => {
            trade::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }
        _ => Ok(err(req_id, -32601, "unknown tool")),
    }
}
