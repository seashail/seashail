use serde_json::{Map, Value};

// Standardize audit log shape. Fields may be null depending on the event type.
const REQUIRED_KEYS: [&str; 12] = [
    "ts",
    "tool",
    "wallet",
    "account_index",
    "chain",
    "usd_value",
    "usd_value_known",
    "policy_decision",
    "confirm_required",
    "confirm_result",
    "txid",
    "error_code",
];

pub fn normalize_entry(v: Value) -> Value {
    let mut obj = match v {
        Value::Object(m) => m,
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)) => {
            let mut m = Map::new();
            m.insert("raw".to_owned(), other);
            m
        }
    };

    // Ensure timestamp exists.
    if !obj.contains_key("ts") {
        obj.insert(
            "ts".to_owned(),
            Value::String(crate::keystore::utc_now_iso()),
        );
    }

    // Ensure required keys exist (null if unknown for the event).
    for k in REQUIRED_KEYS {
        if !obj.contains_key(k) {
            obj.insert(k.to_owned(), Value::Null);
        }
    }

    // Also standardize `result` (high-level outcome).
    // This makes it easy to filter audit events by outcome.
    if !obj.contains_key("result") {
        obj.insert("result".to_owned(), Value::Null);
    }

    Value::Object(obj)
}
