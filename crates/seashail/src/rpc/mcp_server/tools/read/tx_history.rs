use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::super::super::SharedState;

pub fn handle(req_id: Value, args: &Value, shared: &SharedState) -> eyre::Result<JsonRpcResponse> {
    let limit = args
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(50);
    let wallet = args.get("wallet").and_then(|v| v.as_str());
    let chain = args.get("chain").and_then(|v| v.as_str());
    let type_filter = args.get("type").and_then(|v| v.as_str());
    let since_ts = args.get("since_ts").and_then(|v| v.as_str());
    let until_ts = args.get("until_ts").and_then(|v| v.as_str());
    let items = shared.ks.read_tx_history_filtered(
        limit,
        wallet,
        chain,
        type_filter,
        since_ts,
        until_ts,
    )?;
    Ok(ok(req_id, tool_ok(json!({ "items": items }))))
}
