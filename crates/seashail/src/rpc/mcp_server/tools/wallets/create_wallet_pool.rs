use crate::{
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
};
use serde_json::{json, Value};

use super::super::super::elicitation::ensure_unlocked;
use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::WalletHandlerCtx;
use crate::wallet::WalletKind;

pub async fn handle<R, W>(ctx: &mut WalletHandlerCtx<'_, R, W>) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let lock = ctx.shared.ks.acquire_write_lock()?;
    let args = &ctx.args;

    // Default to the active wallet if `wallet` is omitted.
    let wallet_name = args.get("wallet").and_then(|v| v.as_str());
    let (w, _active_idx) = match wallet_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => (
            ctx.shared
                .ks
                .get_wallet_by_name(name)?
                .ok_or_else(|| crate::errors::SeashailError::WalletNotFound(name.to_owned()))?,
            0_u32,
        ),
        None => ctx
            .shared
            .ks
            .get_active_wallet()?
            .ok_or_else(|| crate::errors::SeashailError::WalletNotFound("active".into()))?,
    };

    let count = args.get("count").and_then(Value::as_u64).unwrap_or(0);
    let count_u32 = u32::try_from(count).unwrap_or(u32::MAX);
    if count_u32 == 0 {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "count must be >= 1")),
        ));
    }
    if count_u32 > 100 {
        Keystore::release_lock(lock)?;
        return Ok(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new(
                "invalid_request",
                "count too large (max 100)",
            )),
        ));
    }

    let needs_unlock =
        w.kind != WalletKind::Generated || ctx.shared.ks.generated_wallet_needs_passphrase(&w.id);
    // Unlock only if required (imported wallets, or generated wallets with passphrase-protected Share 2).
    let key = if needs_unlock {
        Some(ensure_unlocked(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout).await?)
    } else {
        None
    };

    // Create N new accounts under the selected wallet root.
    let mut created: Vec<Value> = vec![];
    for _ in 0..count_u32 {
        let result = match key.as_ref() {
            Some(k) => ctx.shared.ks.add_account(&w.name, k),
            None => ctx.shared.ks.add_account_no_passphrase(&w.name),
        };
        let (info, new_index) = match result {
            Ok(v) => v,
            Err(e) => {
                Keystore::release_lock(lock)?;
                return Ok(ok(
                    ctx.req_id.clone(),
                    tool_err(ToolError::new("wallet_pool_failed", e.to_string())),
                ));
            }
        };

        created.push(json!({
          "account_index": new_index,
          "evm_address": info.addresses.evm.get(new_index as usize).cloned().unwrap_or_default(),
          "solana_address": info.addresses.solana.get(new_index as usize).cloned().unwrap_or_default()
        }));
    }

    // Record a history event so strategies can reason about pools.
    ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "wallet_pool_created",
      "wallet": w.name,
      "count": count_u32
    }))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(),
      "tool": "create_wallet_pool",
      "wallet": w.name,
      "account_index": null,
      "chain": null,
      "usd_value": 0.0_f64,
      "usd_value_known": false,
      "policy_decision": null,
      "confirm_required": false,
      "confirm_result": null,
      "txid": null,
      "error_code": null,
      "result": "ok",
      "created_count": count_u32
    }));

    let updated = ctx.shared.ks.get_wallet_info(&w.name)?;

    Keystore::release_lock(lock)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
          "wallet": updated,
          "created_accounts": created
        })),
    ))
}
