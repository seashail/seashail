use crate::errors::SeashailError;
use eyre::Context as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};
use tokio::io::BufReader;

use super::{state::ConnState, transport::write_frame, SharedState};

#[derive(Debug, Deserialize)]
struct JsonRpcClientResponse {
    jsonrpc: String,
    id: Value,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: Value,
}

#[derive(Debug, Deserialize)]
pub struct ElicitResult {
    pub action: String,
    #[serde(default)]
    pub content: BTreeMap<String, Value>,
}

pub async fn elicit_form<R, W>(
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    message: &str,
    requested_schema: Value,
    timeout: Duration,
) -> eyre::Result<ElicitResult>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let id = json!(conn.next_server_id());
    let req = json!({
      "jsonrpc": "2.0",
      "id": id,
      "method": "elicitation/create",
      "params": {
        "mode": "form",
        "message": message,
        "requestedSchema": requested_schema
      }
    });

    write_frame(stdout, &req).await?;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let next = tokio::time::timeout_at(deadline, stdin.next_line())
            .await
            .context("elicitation timeout")??;
        let Some(line) = next else {
            eyre::bail!("stdin closed while awaiting elicitation response");
        };

        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Ignore incoming requests/notifications; we only care about the response to our id.
        if v.get("method").is_some() {
            continue;
        }

        let resp: JsonRpcClientResponse = match serde_json::from_value(v) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if resp.jsonrpc != "2.0" {
            continue;
        }
        if resp.id != id {
            continue;
        }
        if !resp.error.is_null() {
            eyre::bail!("elicitation response error: {}", resp.error);
        }
        let result: ElicitResult =
            serde_json::from_value(resp.result).context("parse elicitation result")?;
        return Ok(result);
    }
}

pub async fn ensure_unlocked<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<[u8; 32]>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    if let Some(k) = shared.session.get() {
        return Ok(*k);
    }

    // Headless mode: env var.
    if let Ok(pw) = std::env::var("SEASHAIL_PASSPHRASE") {
        let pass = SecretString::new(pw.into());
        let salt = shared.ks.ensure_passphrase_salt(&mut shared.cfg)?;
        let key = crate::keystore::crypto::derive_passphrase_key(&pass, &salt)?;
        shared.session.set(
            key,
            Duration::from_secs(shared.cfg.passphrase_session_seconds),
        );
        return Ok(key);
    }

    // Interactive: elicit via form.
    let schema = json!({
      "type": "object",
      "properties": {
        "passphrase": { "type": "string", "title": "Passphrase", "minLength": 1_u64 }
      },
      "required": ["passphrase"]
    });

    let mut res = elicit_form(
        conn,
        stdin,
        stdout,
        "Enter your Seashail passphrase to unlock signing for the next session window.",
        schema,
        Duration::from_secs(5 * 60),
    )
    .await?;

    if res.action != "accept" {
        return Err(SeashailError::UserDeclined.into());
    }

    let passphrase = match res.content.remove("passphrase") {
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => return Err(SeashailError::PassphraseRequired.into()),
    };
    let pass = SecretString::new(passphrase.into());

    let salt = shared.ks.ensure_passphrase_salt(&mut shared.cfg)?;
    let key = crate::keystore::crypto::derive_passphrase_key(&pass, &salt)?;

    shared.session.set(
        key,
        Duration::from_secs(shared.cfg.passphrase_session_seconds),
    );
    Ok(key)
}
