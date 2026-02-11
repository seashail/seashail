use serde_json::Value;

#[must_use]
pub(super) fn parse_usd_value(args: &Value) -> (f64, bool) {
    // If present, usd_value always makes it "known".
    let usd_value = args.get("usd_value").and_then(Value::as_f64).or_else(|| {
        args.get("usd_value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.trim().parse::<f64>().ok())
    });
    usd_value.map_or((0.0_f64, false), |v| (v, true))
}

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

#[must_use]
pub(super) fn get_asset_obj(args: &Value) -> Option<&Value> {
    args.get("asset").and_then(|v| v.as_object().map(|_| v))
}

#[must_use]
pub(super) fn get_str_in_args_or_asset<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    if let Some(v) = get_str(args, key) {
        return Some(v);
    }
    let asset = get_asset_obj(args)?;
    asset
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn is_loopback_http(url: &str) -> bool {
    fn host_prefix_ok(s: &str, prefix: &str) -> bool {
        if !s.starts_with(prefix) {
            return false;
        }
        matches!(s.as_bytes().get(prefix.len()), None | Some(b':' | b'/'))
    }
    let u = url.trim();
    host_prefix_ok(u, "http://127.0.0.1")
        || host_prefix_ok(u, "http://localhost")
        || host_prefix_ok(u, "http://[::1]")
}

fn ensure_https_or_loopback(url: &str, name: &str) -> Result<(), String> {
    let u = url.trim();
    if u.starts_with("https://") || is_loopback_http(u) {
        return Ok(());
    }
    Err(format!(
        "{name} must use https (or http://localhost for local testing)"
    ))
}

/// Validate the defi adapter base URL, build a client, GET `path` with `query`, and return
/// the parsed JSON body. Returns `Err(ToolError)` for any recoverable failure.
pub(super) async fn defi_adapter_fetch(
    base_url_opt: Option<&String>,
    path: &str,
    query: &[(&str, &str)],
) -> Result<serde_json::Value, crate::errors::ToolError> {
    let base_url = base_url_opt
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            crate::errors::ToolError::new(
                "defi_adapter_not_configured",
                "defi_adapter_base_url is not configured (configure http.defi_adapter_base_url)",
            )
        })?;
    if let Err(msg) = ensure_https_or_loopback(&base_url, "defi_adapter_base_url") {
        return Err(crate::errors::ToolError::new("invalid_config", msg));
    }
    let url = format!("{}/{path}", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(2_000))
        .build()
        .map_err(|e| {
            crate::errors::ToolError::new("defi_adapter_error", format!("build http client: {e:#}"))
        })?;
    let resp = client.get(url).query(query).send().await.map_err(|e| {
        crate::errors::ToolError::new("defi_adapter_error", format!("fetch {path}: {e:#}"))
    })?;
    if !resp.status().is_success() {
        return Err(crate::errors::ToolError::new(
            "defi_adapter_error",
            format!("defi adapter http {}", resp.status()),
        ));
    }
    resp.json().await.map_err(|e| {
        crate::errors::ToolError::new("defi_adapter_error", format!("decode {path} json: {e:#}"))
    })
}

pub(super) fn summarize_sim_error(e: &eyre::Report, label: &str) -> String {
    // Keep errors compact for agent UX; detailed traces belong in logs.
    let s = format!("{e:#}");
    if s.len() > 600 {
        format!("{label}: simulation failed (truncated)")
    } else {
        format!("{label}: {s}")
    }
}
