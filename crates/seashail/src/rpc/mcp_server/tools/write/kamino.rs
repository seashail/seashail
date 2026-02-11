use base64::Engine as _;
use eyre::Context as _;
use serde_json::{json, Value};
use tokio::io::BufReader;

use crate::{
    amount,
    chains::solana::SolanaChain,
    errors::ToolError,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::super::{ConnState, SharedState};
use super::super::helpers::{is_native_token, resolve_wallet_and_account, sol_pubkey_for_account};
use super::super::helpers::{solana_fallback_urls, u128_to_u64};
use super::super::key_loading::load_solana_keypair;
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::{parse_usd_value, summarize_sim_error};

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

const KAMINO_LEND_PROGRAM: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
const KAMINO_SCOPE_PROGRAM: &str = "HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ";

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
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

fn ensure_https_or_loopback(url: &str, name: &str) -> eyre::Result<()> {
    let u = url.trim();
    if u.starts_with("https://") || is_loopback_http(u) {
        return Ok(());
    }
    eyre::bail!("{name} must use https (or http://localhost for local testing)");
}

fn default_allowed_program_ids() -> eyre::Result<Vec<solana_sdk::pubkey::Pubkey>> {
    // Allowlisting is strict: every invoked program id in the remote-constructed transaction
    // must be in this list, or the tx is refused.
    //
    // Keep this list conservative and stable. Users can always fall back to tx envelopes.
    let ids = [
        // Common programs
        "11111111111111111111111111111111", // system
        "ComputeBudget111111111111111111111111111111",
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", // SPL Token
        "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb", // Token-2022
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", // ATA
        "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr", // Memo
        "AddressLookupTab1e1111111111111111111111111", // Address Lookup Table
        // Kamino programs
        KAMINO_LEND_PROGRAM,
        KAMINO_SCOPE_PROGRAM,
    ];
    ids.iter()
        .copied()
        .map(SolanaChain::parse_pubkey)
        .collect::<Result<Vec<_>, _>>()
        .context("parse kamino allowlist program ids")
}

fn base58_32_bytes(s: &str) -> eyre::Result<[u8; 32]> {
    let v = bs58::decode(s)
        .into_vec()
        .with_context(|| format!("base58 decode pubkey: {s}"))?;
    if v.len() != 32 {
        eyre::bail!("expected 32 bytes for pubkey");
    }
    let mut out = [0_u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

fn find_unique_offset(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut first: Option<usize> = None;
    for (i, w) in haystack.windows(needle.len()).enumerate() {
        if w == needle {
            if first.is_some() {
                return None; // not unique
            }
            first = Some(i);
        }
    }
    first
}

fn discover_mint_offset(reserves: &[(String, Vec<u8>)]) -> eyre::Result<usize> {
    // Discover the "liquidity mint" offset by searching for a known stablecoin mint.
    let mut chosen_offset: Option<usize> = None;
    for known in [USDC_MINT, USDT_MINT] {
        let known_bytes = base58_32_bytes(known)?;
        for (_reserve_pk, data) in reserves {
            if let Some(off) = find_unique_offset(data, &known_bytes) {
                chosen_offset = Some(off);
                break;
            }
        }
        if chosen_offset.is_some() {
            break;
        }
    }
    if let Some(off) = chosen_offset {
        return Ok(off);
    }
    // As a last resort, try WSOL but require uniqueness within the account blob.
    let known_bytes = base58_32_bytes(WSOL_MINT)?;
    for (_reserve_pk, data) in reserves {
        if let Some(o) = find_unique_offset(data, &known_bytes) {
            return Ok(o);
        }
    }
    eyre::bail!("failed to locate reserve mint offset")
}

/// Parse reserves JSON from Kamino API into (pubkey, data) pairs.
fn parse_reserves_from_json(v: &Value, market: &str) -> eyre::Result<Vec<(String, Vec<u8>)>> {
    let arr = v
        .as_array()
        .ok_or_else(|| eyre::eyre!("invalid reserves json"))?;
    let mut reserves: Vec<(String, Vec<u8>)> = vec![];
    for it in arr {
        if it.get("market").and_then(Value::as_str) != Some(market) {
            continue;
        }
        let Some(rs) = it.get("reserves").and_then(Value::as_array) else {
            continue;
        };
        for r in rs {
            let Some(pk) = r.get("pubkey").and_then(Value::as_str) else {
                continue;
            };
            let Some(data_b64) = r.get("data").and_then(Value::as_str) else {
                continue;
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_b64)
                .context("decode reserve data b64")?;
            reserves.push((pk.to_owned(), bytes));
        }
    }
    Ok(reserves)
}

/// Build a mint->reserve map from reserve account data and the discovered offset.
fn build_mint_reserve_map(
    reserves: &[(String, Vec<u8>)],
    off: usize,
) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for (reserve_pk, data) in reserves {
        let Some(mint_bytes) = data.get(off..off + 32) else {
            continue;
        };
        let mint_s = bs58::encode(mint_bytes).into_string();
        map.insert(mint_s, Value::String(reserve_pk.clone()));
    }
    map
}

async fn kamino_reserve_for_mint(
    shared: &mut SharedState,
    base_url: &str,
    market: &str,
    mint: &str,
) -> eyre::Result<Option<String>> {
    shared.ensure_db().await;
    let cache_key = format!("kamino:reserve_map:{market}");
    if let Some(db) = shared.db() {
        if let Ok(now) = crate::db::Db::now_ms() {
            if let Ok(Some(row)) = db.get_json_if_fresh(&cache_key, now).await {
                if let Ok(v) = serde_json::from_str::<Value>(&row.json) {
                    if let Some(reserve) = v.get(mint).and_then(Value::as_str) {
                        return Ok(Some(reserve.to_owned()));
                    }
                }
            }
        }
    }

    ensure_https_or_loopback(base_url, "kamino_api_base_url")?;
    let url = format!(
        "{}/kamino-market/reserves/account-data",
        base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .context("build http client")?;
    let resp = client
        .get(url)
        .query(&[("markets", market)])
        .send()
        .await
        .context("kamino reserves request")?;
    if !resp.status().is_success() {
        eyre::bail!("kamino reserves http {}", resp.status());
    }
    let v: Value = resp.json().await.context("kamino reserves json")?;

    let reserves = parse_reserves_from_json(&v, market)?;
    if reserves.is_empty() {
        return Ok(None);
    }
    let off = discover_mint_offset(&reserves)?;
    let map = build_mint_reserve_map(&reserves, off);

    if let Some(db) = shared.db() {
        if let Ok(now) = crate::db::Db::now_ms() {
            let stale_at = now.saturating_add(15 * 60 * 1000);
            drop(
                db.upsert_json(
                    &cache_key,
                    &Value::Object(map.clone()).to_string(),
                    now,
                    stale_at,
                )
                .await,
            );
        }
    }

    Ok(map.get(mint).and_then(Value::as_str).map(ToOwned::to_owned))
}

fn parse_kamino_amount(amount_s: &str, units: &str, decimals: u8) -> Result<u128, ToolError> {
    if units == "base" {
        amount::parse_amount_base_u128(amount_s)
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))
    } else {
        amount::parse_amount_ui_to_base_u128(amount_s, u32::from(decimals))
            .map_err(|e| ToolError::new("invalid_request", format!("invalid amount: {e:#}")))
    }
}

fn kamino_u128_to_u64(v: u128) -> Result<u64, ToolError> {
    u128_to_u64(v)
        .map_err(|e| ToolError::new("invalid_request", format!("amount too large: {e:#}")))
}

fn kamino_format_amount_ui(base: u128, decimals: u8) -> Result<String, ToolError> {
    amount::format_amount_base_to_ui_string(base, u32::from(decimals))
        .map_err(|e| ToolError::new("invalid_request", format!("format amount: {e:#}")))
}

/// Fetch the remote transaction from Kamino KTX API, sign, and broadcast.
async fn kamino_fetch_sign_broadcast<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    sol: &SolanaChain,
    owner: &solana_sdk::pubkey::Pubkey,
    base_url: &str,
    endpoint: &str,
    market: &str,
    reserve: &str,
    amount_ui: &str,
) -> eyre::Result<solana_sdk::signature::Signature>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    ensure_https_or_loopback(base_url, "kamino_api_base_url")?;

    let url = format!("{}/ktx/klend/{endpoint}", base_url.trim_end_matches('/'));
    let body = json!({
      "wallet": owner.to_string(),
      "market": market,
      "reserve": reserve,
      "amount": amount_ui,
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build http client")?;
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .context("kamino ktx request")?;
    let status = resp.status();
    let v: Value = resp.json().await.context("kamino ktx json")?;
    if !status.is_success() {
        eyre::bail!("kamino ktx http {status}: {v}");
    }

    let tx_b64 = v
        .get("transaction")
        .and_then(Value::as_str)
        .or_else(|| v.get("transactionB64").and_then(Value::as_str))
        .or_else(|| v.get("tx").and_then(Value::as_str))
        .ok_or_else(|| eyre::eyre!("missing transaction in kamino ktx response"))?;
    let tx_bytes = base64::engine::general_purpose::STANDARD
        .decode(tx_b64)
        .context("decode kamino transaction b64")?;

    let allowed = default_allowed_program_ids()?;
    let kp = load_solana_keypair(shared, conn, stdin, stdout, w, idx).await?;
    sol.sign_and_send_versioned_allowlist(&kp, &tx_bytes, &allowed)
        .await
        .map_err(|e| eyre::eyre!(summarize_sim_error(&e, "kamino tx")))
}

/// Resolve the Kamino reserve for a given mint.
async fn kamino_resolve_reserve(
    shared: &mut SharedState,
    mint: &str,
    market: &str,
    reserve_s: Option<&str>,
) -> Result<String, ToolError> {
    if let Some(r) = reserve_s {
        return Ok(r.to_owned());
    }
    let base_url = shared.cfg.http.kamino_api_base_url.trim().to_owned();
    match kamino_reserve_for_mint(shared, &base_url, market, mint).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Err(ToolError::new(
            "kamino_reserve_not_found",
            "failed to resolve Kamino reserve for this token mint; provide `reserve` explicitly",
        )),
        Err(e) => Err(ToolError::new(
            "kamino_reserve_lookup_failed",
            format!("resolve reserve failed: {e:#}"),
        )),
    }
}

/// Validate the incoming kamino args and return early-exit errors or parsed values.
fn validate_kamino_args<'a>(
    args: &'a Value,
    default_market: &str,
) -> Result<(&'a str, Option<&'a str>, &'a str, &'a str, String), ToolError> {
    let chain = arg_str(args, "chain").unwrap_or("");
    if chain != "solana" {
        return Err(ToolError::new(
            "invalid_request",
            "kamino handler requires chain=solana",
        ));
    }
    let protocol = arg_str(args, "protocol").unwrap_or("kamino");
    if protocol != "kamino" {
        return Err(ToolError::new("invalid_request", "protocol must be kamino"));
    }
    let token_s = arg_str(args, "token").unwrap_or("");
    let reserve_s = arg_str(args, "reserve");
    let amount_s = arg_str(args, "amount").unwrap_or("");
    let units = arg_str(args, "amount_units").unwrap_or("ui");
    let market = arg_str(args, "market")
        .unwrap_or(default_market)
        .trim()
        .to_owned();
    if token_s.is_empty() || amount_s.is_empty() {
        return Err(ToolError::new("invalid_request", "missing token/amount"));
    }
    if amount_s.eq_ignore_ascii_case("max") {
        return Err(ToolError::new(
            "invalid_request",
            "kamino native path does not support amount=max (provide an explicit amount, or use tx envelope fallback)",
        ));
    }
    Ok((token_s, reserve_s, amount_s, units, market))
}

/// Map tool name to (`WriteOp`, `history_type`, `action_label`, `endpoint`).
fn kamino_op_for_tool(
    tool_name: &str,
) -> Result<(WriteOp, &'static str, &'static str, &'static str), ToolError> {
    match tool_name {
        "lend_tokens" => Ok((WriteOp::Lend, "lend", "deposit", "deposit")),
        "withdraw_lending" => Ok((
            WriteOp::WithdrawLending,
            "withdraw_lending",
            "withdraw",
            "withdraw",
        )),
        "borrow_tokens" => Ok((WriteOp::Borrow, "borrow", "borrow", "borrow")),
        "repay_borrow" => Ok((WriteOp::RepayBorrow, "repay_borrow", "repay", "repay")),
        _ => Err(ToolError::new("invalid_request", "unknown tool")),
    }
}

/// Compute USD value for a Kamino operation.
async fn kamino_resolve_usd(
    shared: &mut SharedState,
    sol: &SolanaChain,
    token_s: &str,
    mint: &str,
    base_u64: u64,
    args: &Value,
) -> eyre::Result<(f64, bool)> {
    let usd_value = if is_native_token(token_s) {
        let sol_amt = crate::financial_math::token_base_to_usd(u128::from(base_u64), 9, 1.0_f64);
        let usd = {
            shared.ensure_db().await;
            let db = shared.db();
            price::native_token_price_usd_cached("solana", &shared.cfg, db)
                .await?
                .usd
        };
        crate::financial_math::mul_f64(usd, sol_amt)
    } else {
        shared.ensure_db().await;
        let db = shared.db();
        price::solana_token_price_usd_cached(sol, &shared.cfg, mint, USDC_MINT, base_u64, 50, db)
            .await?
            .usd
    };
    let (arg_usd, arg_usd_known) = parse_usd_value(args);
    if arg_usd_known {
        Ok((arg_usd, true))
    } else {
        Ok((usd_value, true))
    }
}

/// Record tx history and audit log for a Kamino operation, then build the success response.
fn kamino_record_and_respond(
    req_id: Value,
    ks: &Keystore,
    w: &crate::wallet::WalletRecord,
    idx: u32,
    tool_name: &str,
    history_type: &str,
    action_label: &str,
    usd_value: f64,
    usd_value_known: bool,
    sig: &solana_sdk::signature::Signature,
    market: &str,
    reserve: &str,
    mint: &str,
    amount_ui: &str,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
) -> eyre::Result<JsonRpcResponse> {
    ks.append_tx_history(&json!({
      "ts": utc_now_iso(), "day": Keystore::current_utc_day_key(),
      "type": history_type, "chain": "solana", "wallet": w.name,
      "account_index": idx, "usd_value": usd_value,
      "signature": sig.to_string(), "protocol": "kamino",
      "market": market, "reserve": reserve, "mint": mint
    }))?;
    let _audit_log = ks.append_audit_log(&json!({
      "ts": utc_now_iso(), "tool": tool_name, "wallet": w.name,
      "account_index": idx, "chain": "solana", "usd_value": usd_value,
      "usd_value_known": usd_value_known, "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required, "confirm_result": outcome.confirm_result,
      "daily_used_usd": outcome.daily_used_usd, "forced_confirm": outcome.forced_confirm,
      "txid": sig.to_string(), "error_code": null, "result": "broadcasted"
    }));
    Ok(ok(
        req_id,
        tool_ok(json!({
          "chain": "solana", "protocol": "kamino", "action": action_label,
          "signature": sig.to_string(), "usd_value": usd_value,
          "market": market, "reserve": reserve, "mint": mint, "amount_ui": amount_ui
        })),
    ))
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
    let lock = shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(shared, &args)?;
    let (effective_policy, _) = shared.cfg.policy_for_wallet(Some(w.name.as_str()));

    let (token_s, reserve_s, amount_s, units, market) =
        match validate_kamino_args(&args, &shared.cfg.http.kamino_default_lend_market) {
            Ok(v) => v,
            Err(te) => {
                Keystore::release_lock(lock)?;
                return Ok(ok(req_id, tool_err(te)));
            }
        };

    let mode = effective_network_mode(shared, conn);
    let sol = SolanaChain::new_with_fallbacks(
        &shared.cfg.rpc.solana_rpc_url,
        solana_fallback_urls(shared, mode),
        &shared.cfg.http.jupiter_base_url,
        shared.cfg.http.jupiter_api_key.as_deref(),
        shared.cfg.rpc.solana_default_compute_unit_limit,
        shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );
    let owner = sol_pubkey_for_account(&w, idx)?;
    let mint = if is_native_token(token_s) {
        WSOL_MINT
    } else {
        token_s
    };
    let mint_pk = SolanaChain::parse_pubkey(mint)?;
    let decimals = sol.get_mint_decimals(mint_pk).await?;

    let base_u128 = match parse_kamino_amount(amount_s, units, decimals) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };
    let base_u64 = match kamino_u128_to_u64(base_u128) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };
    let reserve = match kamino_resolve_reserve(shared, mint, market.as_str(), reserve_s).await {
        Ok(r) => r,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };
    let amount_ui = if units == "base" {
        match kamino_format_amount_ui(base_u128, decimals) {
            Ok(s) => s,
            Err(te) => {
                Keystore::release_lock(lock)?;
                return Ok(ok(req_id, tool_err(te)));
            }
        }
    } else {
        amount_s.to_owned()
    };

    let (usd_value, usd_value_known) =
        kamino_resolve_usd(shared, &sol, token_s, mint, base_u64, &args).await?;

    let (op, history_type, action_label, endpoint) = match kamino_op_for_tool(tool_name) {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };

    let summary = format!(
        "Kamino Lend {action_label} on Solana: token={mint} amount={amount_ui} market={market}"
    );
    let outcome = match maybe_confirm_write(
        shared,
        conn,
        stdin,
        stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(w.name.as_str()),
            account_index: Some(idx),
            op,
            chain: "solana",
            usd_value,
            usd_value_known,
            force_confirm: effective_policy.require_user_confirm_for_remote_tx.get(),
            slippage_bps: None,
            to_address: None,
            contract: Some("kamino"),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(req_id, tool_err(te)));
        }
    };

    let base_url = shared.cfg.http.kamino_api_base_url.trim().to_owned();
    let sig = match kamino_fetch_sign_broadcast(
        shared, conn, stdin, stdout, &w, idx, &sol, &owner, &base_url, endpoint, &market, &reserve,
        &amount_ui,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            Keystore::release_lock(lock)?;
            return Ok(ok(
                req_id,
                tool_err(ToolError::new("kamino_tx_failed", format!("{e:#}"))),
            ));
        }
    };

    let resp = kamino_record_and_respond(
        req_id,
        &shared.ks,
        &w,
        idx,
        tool_name,
        history_type,
        action_label,
        usd_value,
        usd_value_known,
        &sig,
        &market,
        &reserve,
        mint,
        &amount_ui,
        &outcome,
    )?;
    Keystore::release_lock(lock)?;
    Ok(resp)
}
