use crate::{
    errors::{SeashailError, ToolError},
    keystore::Keystore,
    policy_engine::{self, PolicyContext, WriteOp},
};
use serde_json::json;
use std::time::Duration;

use super::super::elicitation::elicit_form;
use super::super::{ConnState, SharedState};

#[derive(Debug, Clone)]
pub struct WriteConfirmOutcome {
    pub policy_decision: &'static str,
    pub confirm_required: bool,
    pub confirm_result: Option<&'static str>,
    pub forced_confirm: bool,
    pub daily_used_usd: f64,
}

pub struct WriteConfirmRequest<'a> {
    pub tool: &'a str,
    pub wallet: Option<&'a str>,
    pub account_index: Option<u32>,
    pub op: WriteOp,
    pub chain: &'a str,
    pub usd_value: f64,
    pub usd_value_known: bool,
    pub force_confirm: bool,
    pub slippage_bps: Option<u32>,
    pub to_address: Option<&'a str>,
    pub contract: Option<&'a str>,
    pub leverage: Option<u32>,
    pub summary: &'a str,
}

pub async fn maybe_confirm_write<R, W>(
    shared: &SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<tokio::io::BufReader<R>>,
    stdout: &mut W,
    req: &WriteConfirmRequest<'_>,
) -> Result<WriteConfirmOutcome, ToolError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let day = Keystore::current_utc_day_key();
    let used = shared
        .ks
        .daily_used_usd_filtered(&day, req.wallet)
        .map_err(|e| ToolError::new("internal_error", format!("{e:#}")))?;
    let (policy, _is_override) = shared.cfg.policy_for_wallet(req.wallet);
    let ctx = PolicyContext {
        op: req.op,
        chain: req.chain,
        usd_value: req.usd_value,
        usd_value_known: req.usd_value_known,
        daily_used_usd: used,
        slippage_bps: req.slippage_bps,
        to_address: req.to_address,
        contract: req.contract,
        leverage: req.leverage,
    };

    match policy_engine::evaluate(&policy, &ctx) {
        Ok(policy_engine::Approval::AutoApprove) if !req.force_confirm => Ok(WriteConfirmOutcome {
            policy_decision: "auto_approve",
            confirm_required: false,
            confirm_result: None,
            forced_confirm: false,
            daily_used_usd: used,
        }),
        Ok(_) => confirm_with_user(shared, conn, stdin, stdout, req, used).await,
        Err(te) => {
            audit_policy_blocked(shared, req, used, &te);
            Err(te)
        }
    }
}

async fn confirm_with_user<R, W>(
    shared: &SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<tokio::io::BufReader<R>>,
    stdout: &mut W,
    req: &WriteConfirmRequest<'_>,
    used: f64,
) -> Result<WriteConfirmOutcome, ToolError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let schema = json!({
      "type": "object",
      "properties": {
        "confirm": { "type": "boolean", "title": "Confirm transaction", "default": false }
      },
      "required": ["confirm"]
    });
    let usd_s = if req.usd_value_known {
        format!("{:.2}", req.usd_value)
    } else {
        "unknown".to_owned()
    };
    let msg = format!(
        "Seashail requires confirmation.\n\n{}\n\nUSD value: {}\nDaily used (UTC): {:.2}\nChain: {}\n",
        req.summary, usd_s, used, req.chain
    );
    let res = elicit_form(
        conn,
        stdin,
        stdout,
        &msg,
        schema,
        Duration::from_secs(5 * 60),
    )
    .await
    .map_err(|e| ToolError::new("internal_error", format!("{e:#}")))?;

    let confirmed = res.action == "accept"
        && res
            .content
            .get("confirm")
            .and_then(serde_json::Value::as_bool)
            == Some(true);
    if !confirmed {
        audit_user_declined(shared, req, used);
        return Err(SeashailError::UserDeclined.into());
    }

    Ok(WriteConfirmOutcome {
        policy_decision: "user_confirmed",
        confirm_required: true,
        confirm_result: Some("confirmed"),
        forced_confirm: req.force_confirm,
        daily_used_usd: used,
    })
}

fn audit_user_declined(shared: &SharedState, req: &WriteConfirmRequest<'_>, used: f64) {
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": crate::keystore::utc_now_iso(),
      "tool": req.tool,
      "wallet": req.wallet,
      "account_index": req.account_index,
      "chain": req.chain,
      "usd_value": req.usd_value,
      "usd_value_known": req.usd_value_known,
      "daily_used_usd": used,
      "policy_decision": "user_declined",
      "confirm_required": true,
      "confirm_result": "declined",
      "txid": null,
      "error_code": "user_declined",
      "result": "blocked_user_declined"
    }));
}

fn audit_policy_blocked(
    shared: &SharedState,
    req: &WriteConfirmRequest<'_>,
    used: f64,
    te: &ToolError,
) {
    let _audit_log = shared.ks.append_audit_log(&json!({
      "ts": crate::keystore::utc_now_iso(),
      "tool": req.tool,
      "wallet": req.wallet,
      "account_index": req.account_index,
      "chain": req.chain,
      "usd_value": req.usd_value,
      "usd_value_known": req.usd_value_known,
      "daily_used_usd": used,
      "policy_decision": "blocked",
      "confirm_required": false,
      "confirm_result": null,
      "txid": null,
      "error_code": te.code,
      "result": "blocked_policy"
    }));
}
