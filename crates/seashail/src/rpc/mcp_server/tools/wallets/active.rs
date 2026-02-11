use serde_json::{json, Value};

use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::super::super::SharedState;

pub fn handle(req_id: Value, args: &Value, shared: &SharedState) -> eyre::Result<JsonRpcResponse> {
    let name = args.get("wallet").and_then(|v| v.as_str()).unwrap_or("");
    let idx = args
        .get("account_index")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    shared.ks.set_active_wallet(name, idx)?;
    Ok(ok(req_id, tool_ok(json!({ "ok": true }))))
}
