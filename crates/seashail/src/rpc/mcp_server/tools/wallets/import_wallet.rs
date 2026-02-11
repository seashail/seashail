use serde_json::{json, Value};

use super::super::super::elicitation::{elicit_form, ensure_unlocked};
use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::helpers::decode_secret;
use super::WalletHandlerCtx;
use crate::errors::{SeashailError, ToolError};
use crate::keystore::{utc_now_iso, Keystore};
use crate::wallet::ImportedKind;

fn parse_import_kind(kind_s: &str) -> Result<ImportedKind, ToolError> {
    match kind_s {
        "mnemonic" => Ok(ImportedKind::Mnemonic),
        "private_key" => Ok(ImportedKind::PrivateKey),
        _ => Err(ToolError::new("invalid_kind", "invalid kind")),
    }
}

const fn secret_prompt_msg(kind: ImportedKind) -> &'static str {
    match kind {
        ImportedKind::Mnemonic => {
            "Paste your mnemonic (seed phrase). It will be encrypted locally at rest."
        }
        ImportedKind::PrivateKey => "Paste your private key. It will be encrypted locally at rest.",
    }
}

async fn confirm_import<R, W>(ctx: &mut WalletHandlerCtx<'_, R, W>) -> Result<(), JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let confirm_schema = json!({
      "type": "object",
      "properties": { "confirm": { "type": "boolean", "title": "I understand importing key material is sensitive", "default": false } },
      "required": ["confirm"]
    });
    let confirm_res = elicit_form(
        ctx.conn, ctx.stdin, ctx.stdout,
        "Confirm wallet import. You will provide sensitive key material to Seashail for local encryption at rest.",
        confirm_schema, std::time::Duration::from_secs(5 * 60),
    ).await.map_err(|e| ok(ctx.req_id.clone(), tool_err(ToolError::new("elicitation_error", e.to_string()))))?;
    if confirm_res.action != "accept"
        || confirm_res
            .content
            .get("confirm")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err(ok(
            ctx.req_id.clone(),
            tool_err(SeashailError::UserDeclined.into()),
        ));
    }
    Ok(())
}

async fn elicit_secret<R, W>(
    ctx: &mut WalletHandlerCtx<'_, R, W>,
    kind: ImportedKind,
) -> Result<zeroize::Zeroizing<String>, JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let secret_schema = json!({
      "type": "object",
      "properties": { "secret": { "type": "string", "title": "Secret", "minLength": 1_i32 } },
      "required": ["secret"]
    });
    let mut secret_res = elicit_form(
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        secret_prompt_msg(kind),
        secret_schema,
        std::time::Duration::from_secs(5 * 60),
    )
    .await
    .map_err(|e| {
        ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("elicitation_error", e.to_string())),
        )
    })?;
    if secret_res.action != "accept" {
        return Err(ok(
            ctx.req_id.clone(),
            tool_err(SeashailError::UserDeclined.into()),
        ));
    }
    if let Some(Value::String(s)) = secret_res.content.remove("secret") {
        Ok(zeroize::Zeroizing::new(s))
    } else {
        Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "missing secret")),
        ))
    }
}

pub async fn handle<R, W>(ctx: &mut WalletHandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let args = ctx.args.clone();
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let kind_s = args.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let chain_s = args.get("private_key_chain").and_then(|v| v.as_str());

    if name.trim().is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "missing name")),
        ));
    }

    let kind = match parse_import_kind(kind_s) {
        Ok(k) => k,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(e)));
        }
    };

    // Never accept secrets via tool arguments: those routinely end up in agent logs.
    if args
        .get("secret")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
    {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "secret_in_args_not_allowed",
                "do not pass key material in tool arguments; Seashail will prompt for it",
            )),
        ));
    }

    if let Err(resp) = confirm_import(ctx).await {
        Keystore::release_lock(lock)?;
        return Ok(resp);
    }

    let pass_key = ensure_unlocked(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout).await?;

    if kind == ImportedKind::PrivateKey && chain_s.is_none() {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "private_key_chain is required when kind=private_key",
            )),
        ));
    }

    let secret_s = match elicit_secret(ctx, kind).await {
        Ok(s) => s,
        Err(resp) => {
            Keystore::release_lock(lock)?;
            return Ok(resp);
        }
    };

    let decoded = decode_secret(kind, chain_s, secret_s.as_str())?;
    let info = ctx.shared.ks.import_wallet(name, kind, decoded, pass_key)?;

    ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "wallet_imported",
      "wallet": info.name,
      "wallet_kind": "imported"
    }))?;

    Keystore::release_lock(lock)?;
    Ok(ok(ctx.req_id.clone(), tool_ok(json!({ "wallet": info }))))
}
