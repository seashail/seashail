use serde_json::{json, Value};

use super::super::super::elicitation::elicit_form;
use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::WalletHandlerCtx;
use crate::errors::SeashailError;
use crate::keystore::Keystore;

pub async fn handle<R, W>(ctx: &mut WalletHandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    // Serialize creates/imports across competing binaries.
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let name = ctx
        .args
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    // Passphrase entry
    let pass_schema = json!({
      "type": "object",
      "properties": {
        "passphrase": { "type": "string", "title": "Passphrase", "minLength": 8_u32, "description": "Used to encrypt Share 2 and imported wallets." }
      },
      "required": ["passphrase"]
    });
    let mut pass_res = elicit_form(
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        "Set a Seashail passphrase (stored nowhere; required to recover funds if machine is lost).",
        pass_schema,
        std::time::Duration::from_secs(5 * 60),
    )
    .await?;
    if pass_res.action != "accept" {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(SeashailError::UserDeclined.into()),
        ));
    }
    let passphrase = match pass_res.content.remove("passphrase") {
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => {
            Keystore::release_lock(lock)?;
            return Err(SeashailError::PassphraseRequired.into());
        }
    };

    let info = match super::create_wallet_from_passphrase(
        ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, lock, name, passphrase,
    )
    .await
    {
        Ok(i) => i,
        Err(e) => return Err(e),
    };

    Ok(ok(ctx.req_id.clone(), tool_ok(json!({ "wallet": info }))))
}
