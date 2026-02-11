use crate::{
    config::NetworkMode,
    errors::{SeashailError, ToolError},
    keystore::Keystore,
    paths::SeashailPaths,
};
use eyre::Context as _;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tracing::warn;

mod elicitation;
mod jsonrpc;
mod state;
mod tools;
mod transport;

pub use jsonrpc::{err, ok, tool_err, JsonRpcResponse};
pub use state::{ConnState, SharedState};
pub use tools::{handle_tools_call, list_tools_result};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcNotification {
    jsonrpc: String,
}

fn handle_initialize(
    req_id: Value,
    params: &Value,
    shared: &SharedState,
    conn: &mut ConnState,
) -> eyre::Result<JsonRpcResponse> {
    if let Some(m) = params
        .get("seashail_network_override")
        .and_then(|override_v| override_v.as_str())
        .and_then(state::parse_network_mode)
    {
        conn.network_override = Some(m);
    }
    // Ensure a default wallet exists so agents can immediately query addresses/balances.
    shared
        .ks
        .ensure_default_wallet()
        .context("ensure default wallet")?;
    Ok(ok(
        req_id,
        json!({
          "protocolVersion": "2025-06-18",
          "serverInfo": { "name": "seashail", "version": env!("CARGO_PKG_VERSION") },
          "capabilities": { "tools": {}, "elicitation": { "form": {} } }
        }),
    ))
}

pub async fn run(network_override: Option<NetworkMode>) -> eyre::Result<()> {
    let paths = SeashailPaths::discover()?;
    let ks = Keystore::open(paths)?;
    // Standalone mode should default to a per-process DB file so multiple processes can run.
    let mut shared = SharedState::new(ks, false)?;
    let mut conn = ConnState::new();

    if let Some(m) = network_override {
        // Session-only override; do not persist.
        conn.network_override = Some(m);
    }

    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = stdin.next_line().await? {
        if line.len() > crate::rpc::server::MAX_JSONRPC_LINE_BYTES {
            break;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "invalid json on stdin");
                continue;
            }
        };

        // Ignore notifications (no "id").
        if v.get("id").is_none() {
            if let Ok(note) = serde_json::from_value::<JsonRpcNotification>(v.clone()) {
                if note.jsonrpc == "2.0" {
                    continue;
                }
            }
        }

        let req: JsonRpcRequest = match serde_json::from_value(v) {
            Ok(parsed_req) => parsed_req,
            Err(e) => {
                warn!(error = %e, "failed to parse jsonrpc request");
                continue;
            }
        };

        if req.jsonrpc != "2.0" {
            transport::write_frame(&mut stdout, &err(req.id, -32600, "invalid jsonrpc version"))
                .await?;
            continue;
        }

        let resp = match req.method.as_str() {
            "initialize" => handle_initialize(req.id, &req.params, &shared, &mut conn)?,
            "ping" => ok(req.id, json!({})),
            "tools/list" => ok(req.id, list_tools_result()),
            "tools/call" => {
                let name = req
                    .params
                    .get("name")
                    .and_then(|name_v| name_v.as_str())
                    .unwrap_or("");
                let args = req.params.get("arguments").cloned().unwrap_or(Value::Null);
                let id = req.id.clone();
                match handle_tools_call(
                    id.clone(),
                    name,
                    args,
                    &mut shared,
                    &mut conn,
                    &mut stdin,
                    &mut stdout,
                )
                .await
                {
                    Ok(tool_resp) => tool_resp,
                    Err(e) => {
                        if let Some(se) = e.downcast_ref::<SeashailError>() {
                            ok(id, tool_err(ToolError::from(se.clone())))
                        } else {
                            let te = ToolError::new("internal_error", format!("{e:#}"));
                            ok(id, tool_err(te))
                        }
                    }
                }
            }
            _ => err(req.id, -32601, "method not found"),
        };

        transport::write_frame(&mut stdout, &resp).await?;
    }

    Ok(())
}
