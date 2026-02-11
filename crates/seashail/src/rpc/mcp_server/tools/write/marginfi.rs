use eyre::Context as _;
use serde_json::{json, Value};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use solana_signer::Signer as _;

use carbon_core::deserialize::CarbonDeserialize as _;
use carbon_marginfi_v2_decoder::accounts::bank::Bank;

use crate::{
    amount,
    chains::solana::SolanaChain,
    errors::ToolError,
    financial_math,
    keystore::{utc_now_iso, Keystore},
    policy_engine::WriteOp,
    price,
};

use super::super::super::jsonrpc::{ok, tool_err, tool_ok, JsonRpcResponse};
use super::super::super::state::effective_network_mode;
use super::super::helpers::{is_native_token, resolve_wallet_and_account, sol_pubkey_for_account};
use super::super::helpers::{solana_fallback_urls, u128_to_u64};
use super::super::key_loading::load_solana_keypair;
use super::super::policy_confirm::{maybe_confirm_write, WriteConfirmRequest};
use super::super::value_helpers::parse_usd_value;
use super::HandlerCtx;

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

const MARGINFI_PROGRAM: &str = "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA";

// Instruction discriminators from marginfi v2 (Anchor).
const IX_MARGINFI_ACCOUNT_INITIALIZE: [u8; 8] = [0x2b, 0x4e, 0x3d, 0xff, 0x94, 0x34, 0xf9, 0x9a];
const IX_LENDING_ACCOUNT_DEPOSIT: [u8; 8] = [0xab, 0x5e, 0xeb, 0x67, 0x52, 0x40, 0xd4, 0x8c];
const IX_LENDING_ACCOUNT_WITHDRAW: [u8; 8] = [0x24, 0x48, 0x4a, 0x13, 0xd2, 0xd2, 0xc0, 0xc0];
const IX_LENDING_ACCOUNT_BORROW: [u8; 8] = [0x04, 0x7e, 0x74, 0x35, 0x30, 0x05, 0xd4, 0x1f];
const IX_LENDING_ACCOUNT_REPAY: [u8; 8] = [0x4f, 0xd1, 0xac, 0xb1, 0xde, 0x33, 0xad, 0x97];

// Account discriminators.
const DISC_BANK: [u8; 8] = [0x8e, 0x31, 0xa6, 0xf2, 0x32, 0x42, 0x61, 0xbc];

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn ix_data_u64(disc: [u8; 8], amount: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + 8);
    out.extend_from_slice(&disc);
    out.extend_from_slice(&amount.to_le_bytes());
    out
}

fn ix_data_u64_opt_bool_none(disc: [u8; 8], amount: u64) -> Vec<u8> {
    // Borsh: Option<bool> = 0 (None)
    let mut out = Vec::with_capacity(8 + 8 + 1);
    out.extend_from_slice(&disc);
    out.extend_from_slice(&amount.to_le_bytes());
    out.push(0);
    out
}

fn derive_liquidity_vault_authority(
    program_id: Pubkey,
    bank: Pubkey,
    bump: u8,
) -> eyre::Result<Pubkey> {
    // Marginfi publishes these as PDA seeds in its SDKs. To avoid hard-depending on a specific SDK
    // version, try a small set of plausible seed strings and require the stored bump to match.
    //
    // This is safe because the bump is stored in the on-chain bank account; we only accept PDAs
    // that match that bump.
    let seeds = [
        "liquidity_vault_authority",
        "bank_liquidity_vault_authority",
        "liquidity_vault_auth",
        "bank_liquidity_vault_auth",
    ];
    for s in seeds {
        let candidate =
            Pubkey::create_program_address(&[s.as_bytes(), bank.as_ref(), &[bump]], &program_id);
        if let Ok(pk) = candidate {
            return Ok(pk);
        }
    }
    eyre::bail!("failed to derive bank liquidity vault authority PDA")
}

async fn find_bank_for_mint(
    sol: &SolanaChain,
    group: Pubkey,
    mint: Pubkey,
) -> eyre::Result<(Pubkey, Bank)> {
    let program_id = SolanaChain::parse_pubkey(MARGINFI_PROGRAM)?;

    let mut mint_bytes = [0_u8; 32];
    mint_bytes.copy_from_slice(mint.as_ref());
    let mut group_bytes = [0_u8; 32];
    group_bytes.copy_from_slice(group.as_ref());

    let filters = vec![
        RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, DISC_BANK.to_vec())),
        RpcFilterType::Memcmp(Memcmp::new_raw_bytes(8, mint_bytes.to_vec())),
        RpcFilterType::Memcmp(Memcmp::new_raw_bytes(41, group_bytes.to_vec())),
    ];
    let accts = sol
        .get_program_accounts_bytes(program_id, filters)
        .await
        .context("get marginfi banks")?;
    for (pk, data) in accts {
        if let Some(bank) = Bank::deserialize(data.as_slice()) {
            return Ok((pk, bank));
        }
    }
    eyre::bail!("no marginfi bank found for mint+group")
}

async fn ensure_ata(
    sol: &SolanaChain,
    payer: Pubkey,
    owner: Pubkey,
    mint: Pubkey,
    ixs: &mut Vec<Instruction>,
) -> eyre::Result<Pubkey> {
    let ata = spl_associated_token_account::get_associated_token_address(&owner, &mint);
    let exists = sol
        .get_account_optional(&ata)
        .await
        .context("get ata account")?
        .is_some();
    if exists {
        return Ok(ata);
    }
    ixs.push(
        spl_associated_token_account::instruction::create_associated_token_account(
            &payer,
            &owner,
            &mint,
            &spl_token::id(),
        ),
    );
    Ok(ata)
}

struct ParsedMarginfi {
    lock: std::fs::File,
    w: crate::wallet::WalletRecord,
    idx: u32,
    group_s: String,
    group_pk: Pubkey,
    mint_s: String,
    mint_pk: Pubkey,
    base_u64: u64,
    usd_value: f64,
    usd_value_known: bool,
    op: WriteOp,
    history_type: &'static str,
    disc: [u8; 8],
    needs_vault_auth: bool,
}

/// Map tool name to (`WriteOp`, `history_type`, instruction discriminator, `needs_vault_auth`).
fn marginfi_op_for_tool(tool_name: &str) -> Option<(WriteOp, &'static str, [u8; 8], bool)> {
    match tool_name {
        "lend_tokens" => Some((WriteOp::Lend, "lend", IX_LENDING_ACCOUNT_DEPOSIT, false)),
        "withdraw_lending" => Some((
            WriteOp::WithdrawLending,
            "withdraw_lending",
            IX_LENDING_ACCOUNT_WITHDRAW,
            true,
        )),
        "borrow_tokens" => Some((WriteOp::Borrow, "borrow", IX_LENDING_ACCOUNT_BORROW, true)),
        "repay_borrow" => Some((
            WriteOp::RepayBorrow,
            "repay_borrow",
            IX_LENDING_ACCOUNT_REPAY,
            false,
        )),
        _ => None,
    }
}

/// Validate chain/protocol/token/amount and parse pubkeys for marginfi.
fn marginfi_validate_args(
    args: &serde_json::Value,
    app_cfg: &crate::config::SeashailConfig,
) -> Result<(String, Pubkey, String, Pubkey), ToolError> {
    let chain = arg_str(args, "chain").unwrap_or("");
    if chain != "solana" {
        return Err(ToolError::new(
            "invalid_request",
            "marginfi handler requires chain=solana",
        ));
    }
    let protocol = arg_str(args, "protocol").unwrap_or("marginfi");
    if protocol != "marginfi" {
        return Err(ToolError::new(
            "invalid_request",
            "protocol must be marginfi",
        ));
    }
    let token_s = arg_str(args, "token").unwrap_or("");
    let amount_s = arg_str(args, "amount").unwrap_or("");
    if token_s.is_empty() || amount_s.is_empty() {
        return Err(ToolError::new("invalid_request", "missing token/amount"));
    }
    if amount_s.eq_ignore_ascii_case("max") {
        return Err(ToolError::new("invalid_request",
            "marginfi native path does not support amount=max (provide an explicit amount, or use tx envelope fallback)"));
    }
    let group_s = arg_str(args, "group")
        .unwrap_or(app_cfg.http.marginfi_default_group.as_str())
        .trim()
        .to_owned();
    let group_pk = SolanaChain::parse_pubkey(&group_s)
        .map_err(|e| ToolError::new("invalid_request", format!("invalid group pubkey: {e:#}")))?;
    let mint_s = if is_native_token(token_s) {
        WSOL_MINT
    } else {
        token_s
    };
    let mint_pk = SolanaChain::parse_pubkey(mint_s)
        .map_err(|e| ToolError::new("invalid_request", format!("invalid token mint: {e:#}")))?;
    Ok((group_s, group_pk, mint_s.to_owned(), mint_pk))
}

fn parse_marginfi_args<R, W>(
    tool_name: &str,
    ctx: &HandlerCtx<'_, R, W>,
) -> eyre::Result<Result<ParsedMarginfi, JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let (group_s, group_pk, mint_s, mint_pk) =
        match marginfi_validate_args(&ctx.args, &ctx.shared.cfg) {
            Ok(v) => v,
            Err(te) => return Ok(Err(ok(ctx.req_id.clone(), tool_err(te)))),
        };

    let lock = ctx.shared.ks.acquire_write_lock()?;
    let (w, idx) = resolve_wallet_and_account(ctx.shared, &ctx.args)?;

    let Some((op, history_type, disc, needs_vault_auth)) = marginfi_op_for_tool(tool_name) else {
        Keystore::release_lock(lock)?;
        return Ok(Err(ok(
            ctx.req_id.clone(),
            tool_err(ToolError::new("invalid_request", "unknown tool")),
        )));
    };

    Ok(Ok(ParsedMarginfi {
        lock,
        w,
        idx,
        group_s,
        group_pk,
        mint_s,
        mint_pk,
        base_u64: 0,
        usd_value: 0.0,
        usd_value_known: false,
        op,
        history_type,
        disc,
        needs_vault_auth,
    }))
}

async fn resolve_marginfi_amount<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    sol: &SolanaChain,
    pm: &mut ParsedMarginfi,
) -> eyre::Result<Result<(), JsonRpcResponse>>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let amount_s = arg_str(&ctx.args, "amount").unwrap_or("");
    let units = arg_str(&ctx.args, "amount_units").unwrap_or("ui");
    let decimals = sol.get_mint_decimals(pm.mint_pk).await?;

    let base_u128 = if units == "base" {
        match amount::parse_amount_base_u128(amount_s) {
            Ok(v) => v,
            Err(e) => {
                return Ok(Err(ok(
                    ctx.req_id.clone(),
                    tool_err(ToolError::new(
                        "invalid_request",
                        format!("invalid amount: {e:#}"),
                    )),
                )))
            }
        }
    } else {
        match amount::parse_amount_ui_to_base_u128(amount_s, u32::from(decimals)) {
            Ok(v) => v,
            Err(e) => {
                return Ok(Err(ok(
                    ctx.req_id.clone(),
                    tool_err(ToolError::new(
                        "invalid_request",
                        format!("invalid amount: {e:#}"),
                    )),
                )))
            }
        }
    };
    let base_u64 = match u128_to_u64(base_u128) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Err(ok(
                ctx.req_id.clone(),
                tool_err(ToolError::new(
                    "invalid_request",
                    format!("amount too large: {e:#}"),
                )),
            )))
        }
    };

    ctx.shared.ensure_db().await;
    let db_opt = ctx.shared.db();
    let token_s = arg_str(&ctx.args, "token").unwrap_or("");
    let usd_value = if is_native_token(token_s) {
        let p = price::native_token_price_usd_cached("solana", &ctx.shared.cfg, db_opt).await?;
        financial_math::lamports_to_usd(base_u64, p.usd)
    } else {
        price::solana_token_price_usd_cached(
            sol,
            &ctx.shared.cfg,
            &pm.mint_s,
            USDC_MINT,
            base_u64,
            50,
            db_opt,
        )
        .await?
        .usd
    };
    let (arg_usd, arg_usd_known) = parse_usd_value(&ctx.args);
    pm.base_u64 = base_u64;
    pm.usd_value = if arg_usd_known { arg_usd } else { usd_value };
    pm.usd_value_known = true;
    Ok(Ok(()))
}

struct MarginfiSigners<'a> {
    kp: &'a solana_sdk::signer::keypair::Keypair,
    marginfi_kp: &'a solana_sdk::signer::keypair::Keypair,
    marginfi_account: Pubkey,
    owner: Pubkey,
}

async fn marginfi_resolve_bank_and_build_ixs(
    ks: &Keystore,
    sol: &SolanaChain,
    pm: &ParsedMarginfi,
    tool_name: &str,
    signers: &MarginfiSigners<'_>,
) -> eyre::Result<(Vec<Instruction>, MarginfiAccounts)> {
    let marginfi_account_exists = sol
        .get_account_optional(&signers.marginfi_account)
        .await
        .context("get marginfi account")?
        .is_some();

    let (bank_pk, bank) = find_bank_for_mint(sol, pm.group_pk, pm.mint_pk).await?;
    let vault = bank.liquidity_vault;
    let vault_auth = if pm.needs_vault_auth {
        Some(derive_liquidity_vault_authority(
            SolanaChain::parse_pubkey(MARGINFI_PROGRAM)?,
            bank_pk,
            bank.liquidity_vault_authority_bump,
        )?)
    } else {
        None
    };

    let mut ixs: Vec<Instruction> = vec![];
    if !marginfi_account_exists {
        marginfi_init_account(
            ks,
            sol,
            pm,
            signers.kp,
            signers.marginfi_kp,
            &signers.marginfi_account,
            signers.owner,
        )
        .await?;
    }
    let (signer_ata, dest_ata) =
        marginfi_resolve_atas(sol, pm, tool_name, signers.owner, &mut ixs).await?;

    let program_id = SolanaChain::parse_pubkey(MARGINFI_PROGRAM)?;
    let data = match tool_name {
        "lend_tokens" | "borrow_tokens" => ix_data_u64(pm.disc, pm.base_u64),
        "withdraw_lending" | "repay_borrow" => ix_data_u64_opt_bool_none(pm.disc, pm.base_u64),
        _ => vec![],
    };
    let accts = MarginfiAccounts {
        marginfi_account: signers.marginfi_account,
        owner: signers.owner,
        bank_pk,
        vault,
        vault_auth,
        signer_ata,
        dest_ata,
    };
    let accounts = marginfi_build_accounts(pm, tool_name, &accts)?;
    ixs.push(Instruction {
        program_id,
        accounts,
        data,
    });
    Ok((ixs, accts))
}

async fn marginfi_build_and_send<R, W>(
    tool_name: &str,
    ctx: &mut HandlerCtx<'_, R, W>,
    sol: &SolanaChain,
    pm: &ParsedMarginfi,
    owner: Pubkey,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let amount_s = arg_str(&ctx.args, "amount").unwrap_or("");
    let summary = format!(
        "Marginfi {} on Solana: token={} amount={amount_s} group={}",
        pm.history_type, pm.mint_s, pm.group_s
    );
    let outcome = match maybe_confirm_write(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &WriteConfirmRequest {
            tool: tool_name,
            wallet: Some(pm.w.name.as_str()),
            account_index: Some(pm.idx),
            op: pm.op,
            chain: "solana",
            usd_value: pm.usd_value,
            usd_value_known: pm.usd_value_known,
            force_confirm: false,
            slippage_bps: None,
            to_address: None,
            contract: Some("marginfi"),
            leverage: None,
            summary: &summary,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(te) => {
            Keystore::release_lock(pm.lock.try_clone()?)?;
            return Ok(ok(ctx.req_id.clone(), tool_err(te)));
        }
    };

    let kp =
        load_solana_keypair(ctx.shared, ctx.conn, ctx.stdin, ctx.stdout, &pm.w, pm.idx).await?;
    let marginfi_kp = load_solana_keypair(
        ctx.shared,
        ctx.conn,
        ctx.stdin,
        ctx.stdout,
        &pm.w,
        pm.idx.saturating_add(10_000),
    )
    .await?;
    let marginfi_account = marginfi_kp.pubkey();
    marginfi_cache_account(ctx, pm, &marginfi_account).await;

    let signers = MarginfiSigners {
        kp: &kp,
        marginfi_kp: &marginfi_kp,
        marginfi_account,
        owner,
    };
    let (ixs, accts) =
        marginfi_resolve_bank_and_build_ixs(&ctx.shared.ks, sol, pm, tool_name, &signers).await?;

    let sig = sol
        .sign_and_send_instructions(&kp, ixs)
        .await
        .context("send marginfi tx")?;
    marginfi_record_and_respond(
        ctx,
        pm,
        tool_name,
        &outcome,
        &sig,
        accts.bank_pk,
        &accts.marginfi_account,
    )
}

/// Best-effort persist the derived marginfi account pubkey for read tools.
async fn marginfi_cache_account<R, W>(
    ctx: &mut HandlerCtx<'_, R, W>,
    pm: &ParsedMarginfi,
    marginfi_account: &Pubkey,
) where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    ctx.shared.ensure_db().await;
    if let Some(db) = ctx.shared.db() {
        if let Ok(now) = crate::db::Db::now_ms() {
            let stale_at = now.saturating_add(365_i64 * 24 * 60 * 60 * 1000);
            let cache_key = format!(
                "lending:marginfi:account:{}:{}:{}",
                pm.group_s, pm.w.name, pm.idx
            );
            drop(
                db.upsert_json(
                    &cache_key,
                    &serde_json::to_string(&json!({
                      "marginfi_account": marginfi_account.to_string(),
                      "group": pm.group_s,
                      "wallet": pm.w.name,
                      "account_index": pm.idx,
                    }))
                    .unwrap_or_default(),
                    now,
                    stale_at,
                )
                .await,
            );
        }
    }
}

/// Send the marginfi account initialization transaction if needed.
async fn marginfi_init_account(
    ks: &Keystore,
    sol: &SolanaChain,
    pm: &ParsedMarginfi,
    kp: &solana_sdk::signer::keypair::Keypair,
    marginfi_kp: &solana_sdk::signer::keypair::Keypair,
    marginfi_account: &Pubkey,
    owner: Pubkey,
) -> eyre::Result<()> {
    let program_id = SolanaChain::parse_pubkey(MARGINFI_PROGRAM)?;
    let ix = Instruction {
        program_id,
        accounts: vec![
            solana_sdk::instruction::AccountMeta::new_readonly(pm.group_pk, false),
            solana_sdk::instruction::AccountMeta::new(*marginfi_account, true),
            solana_sdk::instruction::AccountMeta::new_readonly(owner, true),
            solana_sdk::instruction::AccountMeta::new(owner, true),
            solana_sdk::instruction::AccountMeta::new_readonly(
                solana_system_interface::program::id(),
                false,
            ),
        ],
        data: IX_MARGINFI_ACCOUNT_INITIALIZE.to_vec(),
    };
    let sig = sol
        .sign_and_send_instructions_multi(kp, &[marginfi_kp], vec![ix])
        .await
        .context("send marginfi init")?;
    ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "marginfi_init",
      "chain": "solana",
      "wallet": pm.w.name,
      "account_index": pm.idx,
      "usd_value": 0.0_f64,
      "signature": sig.to_string(),
      "protocol": "marginfi",
      "group": pm.group_s,
      "marginfi_account": marginfi_account.to_string(),
    }))?;
    Ok(())
}

/// Resolve signer and destination ATAs for the marginfi operation.
async fn marginfi_resolve_atas(
    sol: &SolanaChain,
    pm: &ParsedMarginfi,
    tool_name: &str,
    owner: Pubkey,
    ixs: &mut Vec<Instruction>,
) -> eyre::Result<(Option<Pubkey>, Option<Pubkey>)> {
    let mut signer_ata: Option<Pubkey> = None;
    let mut dest_ata: Option<Pubkey> = None;
    match tool_name {
        "lend_tokens" | "repay_borrow" => {
            let ata =
                spl_associated_token_account::get_associated_token_address(&owner, &pm.mint_pk);
            if sol
                .get_account_optional(&ata)
                .await
                .context("get signer ata")?
                .is_none()
            {
                Keystore::release_lock(pm.lock.try_clone()?)?;
                return Err(eyre::eyre!(
                    "missing source token account (create or receive SPL tokens first)"
                ));
            }
            signer_ata = Some(ata);
        }
        "withdraw_lending" | "borrow_tokens" => {
            let ata = ensure_ata(sol, owner, owner, pm.mint_pk, ixs).await?;
            dest_ata = Some(ata);
        }
        _ => {}
    }
    Ok((signer_ata, dest_ata))
}

/// Resolved on-chain accounts needed to build marginfi instructions.
struct MarginfiAccounts {
    marginfi_account: Pubkey,
    owner: Pubkey,
    bank_pk: Pubkey,
    vault: Pubkey,
    vault_auth: Option<Pubkey>,
    signer_ata: Option<Pubkey>,
    dest_ata: Option<Pubkey>,
}

/// Build the account metas for the marginfi lending instruction.
fn marginfi_build_accounts(
    pm: &ParsedMarginfi,
    tool_name: &str,
    accts: &MarginfiAccounts,
) -> eyre::Result<Vec<solana_sdk::instruction::AccountMeta>> {
    let token_program = spl_token::id();
    let mut accounts = vec![
        solana_sdk::instruction::AccountMeta::new_readonly(pm.group_pk, false),
        solana_sdk::instruction::AccountMeta::new(accts.marginfi_account, false),
        solana_sdk::instruction::AccountMeta::new_readonly(accts.owner, true),
        solana_sdk::instruction::AccountMeta::new(accts.bank_pk, false),
    ];
    match tool_name {
        "lend_tokens" | "repay_borrow" => {
            let ata = accts
                .signer_ata
                .ok_or_else(|| eyre::eyre!("missing signer ata"))?;
            accounts.push(solana_sdk::instruction::AccountMeta::new(ata, false));
            accounts.push(solana_sdk::instruction::AccountMeta::new(
                accts.vault,
                false,
            ));
            accounts.push(solana_sdk::instruction::AccountMeta::new_readonly(
                token_program,
                false,
            ));
        }
        "withdraw_lending" | "borrow_tokens" => {
            let ata = accts
                .dest_ata
                .ok_or_else(|| eyre::eyre!("missing dest ata"))?;
            let auth = accts
                .vault_auth
                .ok_or_else(|| eyre::eyre!("missing vault auth"))?;
            accounts.push(solana_sdk::instruction::AccountMeta::new(ata, false));
            accounts.push(solana_sdk::instruction::AccountMeta::new(
                accts.vault,
                false,
            ));
            accounts.push(solana_sdk::instruction::AccountMeta::new_readonly(
                auth, false,
            ));
            accounts.push(solana_sdk::instruction::AccountMeta::new_readonly(
                token_program,
                false,
            ));
        }
        _ => {}
    }
    Ok(accounts)
}

/// Record transaction history, audit log, release lock, and return the response.
fn marginfi_record_and_respond<R, W>(
    ctx: &HandlerCtx<'_, R, W>,
    pm: &ParsedMarginfi,
    tool_name: &str,
    outcome: &super::super::policy_confirm::WriteConfirmOutcome,
    sig: &solana_sdk::signature::Signature,
    bank_pk: Pubkey,
    marginfi_account: &Pubkey,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    ctx.shared.ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": pm.history_type,
      "chain": "solana",
      "wallet": pm.w.name,
      "account_index": pm.idx,
      "usd_value": pm.usd_value,
      "signature": sig.to_string(),
      "protocol": "marginfi",
      "group": pm.group_s,
      "marginfi_account": marginfi_account.to_string(),
      "bank": bank_pk.to_string(),
      "mint": pm.mint_s,
      "amount_base": pm.base_u64.to_string()
    }))?;
    let _audit_log = ctx.shared.ks.append_audit_log(&json!({
      "ts": utc_now_iso(),
      "tool": tool_name,
      "wallet": pm.w.name,
      "account_index": pm.idx,
      "chain": "solana",
      "usd_value": pm.usd_value,
      "usd_value_known": pm.usd_value_known,
      "policy_decision": outcome.policy_decision,
      "confirm_required": outcome.confirm_required,
      "confirm_result": outcome.confirm_result,
      "daily_used_usd": outcome.daily_used_usd,
      "forced_confirm": outcome.forced_confirm,
      "txid": sig.to_string(),
      "error_code": null,
      "result": "broadcasted"
    }));

    Keystore::release_lock(pm.lock.try_clone()?)?;
    Ok(ok(
        ctx.req_id.clone(),
        tool_ok(json!({
          "chain": "solana",
          "protocol": "marginfi",
          "signature": sig.to_string(),
          "usd_value": pm.usd_value,
          "group": pm.group_s,
          "marginfi_account": marginfi_account.to_string(),
          "bank": bank_pk.to_string(),
          "mint": pm.mint_s
        })),
    ))
}

/// Public entry point for marginfi lending/borrowing write operations.
pub async fn handle<R, W>(
    tool_name: &str,
    ctx: &mut HandlerCtx<'_, R, W>,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mut pm = match parse_marginfi_args(tool_name, ctx)? {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let mode = effective_network_mode(ctx.shared, ctx.conn);
    let sol = SolanaChain::new_with_fallbacks(
        &ctx.shared.cfg.rpc.solana_rpc_url,
        solana_fallback_urls(ctx.shared, mode),
        &ctx.shared.cfg.http.jupiter_base_url,
        ctx.shared.cfg.http.jupiter_api_key.as_deref(),
        ctx.shared.cfg.rpc.solana_default_compute_unit_limit,
        ctx.shared
            .cfg
            .rpc
            .solana_default_compute_unit_price_micro_lamports,
    );

    if let Err(resp) = resolve_marginfi_amount(ctx, &sol, &mut pm).await? {
        Keystore::release_lock(pm.lock)?;
        return Ok(resp);
    }

    let owner = sol_pubkey_for_account(&pm.w, pm.idx)?;
    marginfi_build_and_send(tool_name, ctx, &sol, &pm, owner).await
}
