use serde_json::{json, Value};
use std::time::Duration;

use super::super::super::elicitation::{elicit_form, ensure_unlocked};
use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::WalletHandlerCtx;
use crate::errors::{SeashailError, ToolError};
use crate::keystore::Keystore;
use crate::wallet::WalletKind;

fn tail6(s: &str) -> String {
    s.chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

pub async fn handle<R, W>(
    tool_name: &str,
    ctx: &mut WalletHandlerCtx<'_, R, W>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match tool_name {
        "export_shares" => export_shares(ctx).await,
        "rotate_shares" => rotate_shares(ctx).await,
        _ => Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "unknown tool")),
        )),
    }
}

async fn export_shares<R, W>(ctx: &mut WalletHandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let name = ctx
        .args
        .get("wallet")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let w = ctx
        .shared
        .ks
        .get_wallet_by_name(name)?
        .ok_or_else(|| SeashailError::WalletNotFound(name.to_owned()))?;
    if w.kind != WalletKind::Generated {
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("not_generated", "wallet is not generated")),
        ));
    }

    let key = ensure_unlocked(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout).await?;
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let plan = match ctx.shared.ks.plan_rotate_shares(&w.id, &key) {
        Ok(p) => p,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Err(e);
        }
    };
    let share3 = plan.share3_base64.clone();
    let tail = tail6(&share3);

    let schema = json!({
      "type": "object",
      "properties": {
        "confirm_tail": { "type": "string", "title": "Type the last 6 characters to confirm you saved it", "minLength": 6_u32, "maxLength": 6_u32 },
        "ack": { "type": "boolean", "title": "I understand losing 2+ shares means permanent loss of funds", "default": false }
      },
      "required": ["confirm_tail", "ack"]
    });
    let msg = format!(
        "Offline backup share (Share 3) for wallet `{}`. Store it offline. It will not be shown again; requesting shares again will rotate them.\n\nSHARE3_BASE64:\n{}\n",
        w.name, share3
    );
    let res = match elicit_form(
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &msg,
        schema,
        Duration::from_secs(5 * 60),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Err(e);
        }
    };
    if res.action != "accept" {
        Keystore::release_lock(lock)?;
        return Err(SeashailError::UserDeclined.into());
    }
    let confirm_tail = res
        .content
        .get("confirm_tail")
        .and_then(Value::as_str)
        .unwrap_or("");
    let ack = res
        .content
        .get("ack")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !ack || confirm_tail != tail {
        Keystore::release_lock(lock)?;
        return Err(SeashailError::BackupNotConfirmed.into());
    }

    if let Err(e) = ctx.shared.ks.commit_rotate_shares(&w.id, &plan) {
        Keystore::release_lock(lock)?;
        return Err(e);
    }
    Keystore::release_lock(lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({ "ok": true, "wallet": w.name, "status": "share_exported" })),
    ))
}

async fn rotate_shares<R, W>(ctx: &mut WalletHandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let name = ctx
        .args
        .get("wallet")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let w = ctx
        .shared
        .ks
        .get_wallet_by_name(name)?
        .ok_or_else(|| SeashailError::WalletNotFound(name.to_owned()))?;
    if w.kind != WalletKind::Generated {
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("not_generated", "wallet is not generated")),
        ));
    }

    let key = ensure_unlocked(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout).await?;
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let plan = match ctx.shared.ks.plan_rotate_shares(&w.id, &key) {
        Ok(p) => p,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Err(e);
        }
    };
    let share3 = plan.share3_base64.clone();
    let tail = tail6(&share3);

    let schema = json!({
      "type": "object",
      "properties": {
        "confirm_tail": { "type": "string", "title": "Type the last 6 characters to confirm you saved it", "minLength": 6_u32, "maxLength": 6_u32 },
        "ack": { "type": "boolean", "title": "I understand losing 2+ shares means permanent loss of funds", "default": false }
      },
      "required": ["confirm_tail", "ack"]
    });
    let msg = format!(
        "Offline backup share (Share 3) for wallet `{}` after rotating shares. Store it offline. It will not be shown again.\n\nSHARE3_BASE64:\n{}\n",
        w.name, share3
    );
    let res = match elicit_form(
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &msg,
        schema,
        Duration::from_secs(5 * 60),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Err(e);
        }
    };
    if res.action != "accept" {
        Keystore::release_lock(lock)?;
        return Err(SeashailError::UserDeclined.into());
    }
    let confirm_tail = res
        .content
        .get("confirm_tail")
        .and_then(Value::as_str)
        .unwrap_or("");
    let ack = res
        .content
        .get("ack")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !ack || confirm_tail != tail {
        Keystore::release_lock(lock)?;
        return Err(SeashailError::BackupNotConfirmed.into());
    }

    if let Err(e) = ctx.shared.ks.commit_rotate_shares(&w.id, &plan) {
        Keystore::release_lock(lock)?;
        return Err(e);
    }
    Keystore::release_lock(lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({ "ok": true, "wallet": w.name, "status": "shares_rotated" })),
    ))
}
