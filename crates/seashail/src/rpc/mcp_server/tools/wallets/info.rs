use serde_json::Value;

use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::super::super::SharedState;
use crate::errors::SeashailError;

pub fn handle(req_id: Value, args: &Value, shared: &SharedState) -> eyre::Result<JsonRpcResponse> {
    let name = args.get("wallet").and_then(|v| v.as_str()).unwrap_or("");
    let info = if name.is_empty() {
        let (w, _idx) = shared
            .ks
            .get_active_wallet()?
            .ok_or_else(|| SeashailError::WalletNotFound("active".into()))?;
        shared.ks.get_wallet_info(&w.name)?
    } else {
        shared.ks.get_wallet_info(name)?
    };
    Ok(ok(req_id, tool_ok(serde_json::json!(info))))
}
