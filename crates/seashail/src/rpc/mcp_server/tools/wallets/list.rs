use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::super::super::SharedState;

pub fn handle(req_id: Value, shared: &SharedState) -> eyre::Result<JsonRpcResponse> {
    let wallets = shared.ks.list_wallets()?;
    Ok(ok(req_id, tool_ok(json!({ "wallets": wallets }))))
}
