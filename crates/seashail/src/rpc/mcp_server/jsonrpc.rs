use crate::errors::ToolError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Value::is_null", default)]
    data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

pub fn ok(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

pub fn err(id: Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
            data: Value::Null,
        }),
    }
}

pub fn tool_ok(payload: Value) -> Value {
    let text = payload.to_string();
    drop(payload);
    json!({
      "content": [{ "type": "text", "text": text }],
      "isError": false
    })
}

pub fn tool_err(tool_error: ToolError) -> Value {
    let text = serde_json::to_string(&tool_error).unwrap_or_else(|_e| {
        "{\"code\":\"error\",\"message\":\"failed to serialize error\"}".into()
    });
    drop(tool_error);
    json!({
      "content": [{ "type": "text", "text": text }],
      "isError": true
    })
}
