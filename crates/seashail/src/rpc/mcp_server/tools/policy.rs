use serde_json::{json, Value};

use super::super::jsonrpc::{err, ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::SharedState;
use super::helpers::parse_policy;
use crate::errors::{SeashailError, ToolError};

pub fn handle(
    req_id: Value,
    tool_name: &str,
    args: &Value,
    shared: &mut SharedState,
) -> eyre::Result<JsonRpcResponse> {
    match tool_name {
        "get_policy" => {
            let wallet = args.get("wallet").and_then(|v| v.as_str()).map(str::trim);
            if let Some(w) = wallet.filter(|s| !s.is_empty()) {
                if shared.ks.get_wallet_by_name(w)?.is_none() {
                    return Ok(ok(
                        req_id,
                        tool_err(ToolError::from(SeashailError::WalletNotFound(w.to_owned()))),
                    ));
                }
            }
            let (p, _is_override) = shared.cfg.policy_for_wallet(wallet);
            Ok(ok(req_id, tool_ok(json!(p))))
        }
        "update_policy" => {
            let wallet = args.get("wallet").and_then(|v| v.as_str()).map(str::trim);
            let clear = args
                .get("clear")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);

            if let Some(w) = wallet.filter(|s| !s.is_empty()) {
                if shared.ks.get_wallet_by_name(w)?.is_none() {
                    return Ok(ok(
                        req_id,
                        tool_err(ToolError::from(SeashailError::WalletNotFound(w.to_owned()))),
                    ));
                }
                if clear {
                    shared.cfg.policy_overrides_by_wallet.remove(w);
                    shared.ks.save_config(&shared.cfg)?;
                    return Ok(ok(req_id, tool_ok(json!({ "ok": true, "cleared": true }))));
                }
            } else if clear {
                return Ok(ok(
                    req_id,
                    tool_err(ToolError::new(
                        "invalid_request",
                        "clear=true requires wallet to be set",
                    )),
                ));
            }

            let policy_v = args.get("policy").cloned().unwrap_or(Value::Null);
            if policy_v.is_null() {
                return Ok(ok(
                    req_id,
                    tool_err(ToolError::new(
                        "invalid_request",
                        "missing policy (or set clear=true to remove a wallet override)",
                    )),
                ));
            }

            let p = match parse_policy(policy_v) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ok(
                        req_id,
                        tool_err(ToolError::new("invalid_policy", format!("{e:#}"))),
                    ));
                }
            };

            if let Some(w) = wallet.filter(|s| !s.is_empty()) {
                shared
                    .cfg
                    .policy_overrides_by_wallet
                    .insert(w.to_owned(), p);
            } else {
                shared.cfg.policy = p;
            }
            shared.ks.save_config(&shared.cfg)?;
            Ok(ok(req_id, tool_ok(json!({ "ok": true }))))
        }

        _ => Ok(err(req_id, -32601, "unknown tool")),
    }
}
