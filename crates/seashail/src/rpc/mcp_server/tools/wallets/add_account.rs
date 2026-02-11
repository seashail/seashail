use serde_json::json;

use super::super::super::elicitation::ensure_unlocked;
use super::super::super::jsonrpc::{ok, tool_ok, JsonRpcResponse};
use super::WalletHandlerCtx;
use crate::keystore::Keystore;
use crate::wallet::WalletKind;

pub async fn handle<R, W>(ctx: &mut WalletHandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let name = ctx
        .args
        .get("wallet")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let w = ctx.shared.ks.get_wallet_by_name(name)?;
    let (info, new_index) = match w {
        Some(w)
            if w.kind == WalletKind::Generated
                && !ctx.shared.ks.generated_wallet_needs_passphrase(&w.id) =>
        {
            ctx.shared.ks.add_account_no_passphrase(name)?
        }
        _ => {
            let key = ensure_unlocked(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout).await?;
            ctx.shared.ks.add_account(name, &key)?
        }
    };
    Keystore::release_lock(lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({ "wallet": info, "new_account_index": new_index })),
    ))
}
