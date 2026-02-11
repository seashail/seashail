mod active;
mod add_account;
mod create_wallet;
mod create_wallet_pool;
mod deposit_info;
mod import_wallet;
mod info;
mod list;
mod shares;

use crate::{
    chains::evm::EvmChain,
    errors::SeashailError,
    keystore::{utc_now_iso, Keystore},
};
use secrecy::SecretString;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::BufReader;

use super::super::elicitation::elicit_form;
use super::super::jsonrpc::{err, JsonRpcResponse};
use super::super::{ConnState, SharedState};

pub struct WalletHandlerCtx<'a, R, W> {
    pub req_id: Value,
    pub args: Value,
    pub shared: &'a mut SharedState,
    pub conn: &'a mut ConnState,
    pub stdin: &'a mut tokio::io::Lines<BufReader<R>>,
    pub stdout: &'a mut W,
}

fn evm_usdc_identifier(shared: &SharedState, chain: &str) -> Option<String> {
    let rpc_url = shared.cfg.rpc.evm_rpc_urls.get(chain)?;
    let chain_id = shared.cfg.rpc.evm_chain_ids.get(chain)?;
    let mut evm = EvmChain::for_name(chain, *chain_id, rpc_url, &shared.cfg.http);
    if let Some(fb) = shared.cfg.rpc.evm_fallback_rpc_urls.get(chain) {
        evm.fallback_rpc_urls.clone_from(fb);
    }
    evm.uniswap.as_ref().map(|u| format!("{:?}", u.usdc))
}

pub(super) async fn create_wallet_from_passphrase<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    lock: std::fs::File,
    name: String,
    passphrase: String,
) -> eyre::Result<crate::wallet::WalletInfo>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let pass = SecretString::new(passphrase.into());

    let salt = shared.ks.ensure_passphrase_salt(&mut shared.cfg)?;
    let key = crate::keystore::crypto::derive_passphrase_key(&pass, &salt)?;
    shared.session.set(
        key,
        Duration::from_secs(shared.cfg.passphrase_session_seconds),
    );

    let (info, backup_share3) = shared.ks.create_generated_wallet(name, key)?;

    // Show-once backup flow (share 3 is encrypted on disk; this display is a convenience).
    let tail = backup_share3
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let backup_schema = json!({
      "type": "object",
      "properties": {
        "confirm_tail": { "type": "string", "title": "Type the last 6 characters to confirm you saved it", "minLength": 6_u32, "maxLength": 6_u32 },
        "ack": { "type": "boolean", "title": "I understand losing 2+ shares means permanent loss of funds", "default": false }
      },
      "required": ["confirm_tail", "ack"]
    });
    let msg = format!(
        "Offline backup share (Share 3) for wallet `{}`. Store it offline. It will not be shown again unless you explicitly export shares.\n\nSHARE3_BASE64:\n{}\n",
        info.name, backup_share3
    );
    let backup_res = elicit_form(
        conn,
        stdin,
        stdout,
        &msg,
        backup_schema,
        Duration::from_secs(5 * 60),
    )
    .await?;
    if backup_res.action != "accept" {
        Keystore::release_lock(lock)?;
        return Err(SeashailError::UserDeclined.into());
    }
    let confirm_tail = backup_res
        .content
        .get("confirm_tail")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ack = backup_res
        .content
        .get("ack")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !ack || confirm_tail != tail {
        Keystore::release_lock(lock)?;
        return Err(SeashailError::BackupNotConfirmed.into());
    }

    // Disclaimers (Section 11)
    let disc_schema = json!({
      "type": "object",
      "properties": { "accept": { "type": "boolean", "title": "I acknowledge the Seashail disclaimers", "default": false } },
      "required": ["accept"]
    });
    let disc_msg = "Disclaimers: Seashail is provided as-is, non-custodial, no financial advice, irreversible transactions, loss of shares/passphrase can cause permanent loss of funds.";
    let disc_res = elicit_form(
        conn,
        stdin,
        stdout,
        disc_msg,
        disc_schema,
        Duration::from_secs(5 * 60),
    )
    .await?;
    if disc_res.action != "accept"
        || disc_res
            .content
            .get("accept")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        Keystore::release_lock(lock)?;
        return Err(SeashailError::UserDeclined.into());
    }

    shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "wallet_created",
      "wallet": info.name,
      "wallet_kind": "generated"
    }))?;

    Keystore::release_lock(lock)?;
    Ok(info)
}

pub async fn handle<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match tool_name {
        "list_wallets" => list::handle(req_id, shared),
        "get_wallet_info" => info::handle(req_id, &args, shared),
        "get_deposit_info" => deposit_info::handle(req_id, &args, shared, conn),
        "set_active_wallet" => active::handle(req_id, &args, shared),
        "add_account" => {
            let mut ctx = WalletHandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            add_account::handle(&mut ctx).await
        }
        "create_wallet" => {
            let mut ctx = WalletHandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            create_wallet::handle(&mut ctx).await
        }
        "create_wallet_pool" => {
            let mut ctx = WalletHandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            create_wallet_pool::handle(&mut ctx).await
        }
        "import_wallet" => {
            let mut ctx = WalletHandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            import_wallet::handle(&mut ctx).await
        }
        "export_shares" | "rotate_shares" => {
            let mut ctx = WalletHandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            shares::handle(tool_name, &mut ctx).await
        }
        _ => Ok(err(req_id, -32601, "unknown tool")),
    }
}
