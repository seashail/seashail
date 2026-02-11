use eyre::{Context as _, ContextCompat as _};
use sha2::{Digest as _, Sha256};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::str::FromStr as _;

// Jupiter Perps (mainnet) constants derived from Jupiter Perps docs/examples.
// Note: This integration submits "position request" instructions which are executed by keepers.
pub const PROGRAM_ID_MAINNET: &str = "PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu";
pub const EVENT_AUTHORITY_MAINNET: &str = "37hJBDnntwqhGbK7L6M1bLyvccj4u55CCUiLPdYkiqBN";
pub const JLP_POOL_MAINNET: &str = "5BUwFW4nRbftYTDMbgxykoFWqWHPzahFSNAaaaJtVKsq";

pub const CUSTODY_SOL_MAINNET: &str = "7xS2gz2bTp3fwCC7knJvUWTEU9Tycczu6VhJYKgi1wdz";
pub const CUSTODY_ETH_MAINNET: &str = "AQCGyheWPLeo6Qp9WpYS9m3Qj479t7R636N9ey1rEjEn";
pub const CUSTODY_BTC_MAINNET: &str = "5Pv3gM9JrFFH883SWAhvJC9RPYmo8UNxuFtv5bMMALkm";
pub const CUSTODY_USDC_MAINNET: &str = "G18jKKXQwBbrHeiK3C9MRXhkHsLHf7XgCSisykV46EZa";

// Canonical Solana USDC mint on mainnet.
pub const USDC_MINT_MAINNET: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub const fn as_anchor_discriminant(self) -> u8 {
        // Anchor enum variants for Side: None=0, Long=1, Short=2
        match self {
            Self::Long => 1,
            Self::Short => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PositionAccount {
    pub owner: Pubkey,
    pub pool: Pubkey,
    pub custody: Pubkey,
    pub collateral_custody: Pubkey,
    pub open_time: i64,
    pub update_time: i64,
    pub side: u8,
    pub price: u64,
    pub size_usd: u64,
    pub collateral_usd: u64,
    pub realised_pnl_usd: i64,
    pub cumulative_interest_snapshot: u128,
    pub locked_amount: u64,
    pub bump: u8,
}

pub fn parse_pubkey(s: &str) -> eyre::Result<Pubkey> {
    Pubkey::from_str(s.trim()).context("parse pubkey")
}

pub fn custody_for_market_mainnet(symbol: &str) -> Option<&'static str> {
    match symbol.trim().to_ascii_uppercase().as_str() {
        "SOL" => Some(CUSTODY_SOL_MAINNET),
        "BTC" => Some(CUSTODY_BTC_MAINNET),
        "ETH" => Some(CUSTODY_ETH_MAINNET),
        _ => None,
    }
}

pub fn perpetuals_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"perpetuals"], program_id)
}

pub fn position_pda(
    program_id: &Pubkey,
    owner: &Pubkey,
    pool: &Pubkey,
    custody: &Pubkey,
    collateral_custody: &Pubkey,
    side: Side,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"position",
            owner.as_ref(),
            pool.as_ref(),
            custody.as_ref(),
            collateral_custody.as_ref(),
            &[side.as_anchor_discriminant()],
        ],
        program_id,
    )
}

pub fn position_request_pda(
    program_id: &Pubkey,
    position: &Pubkey,
    counter: u64,
    request_change: u8, // Increase=1, Decrease=2
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"position_request",
            position.as_ref(),
            &counter.to_le_bytes(),
            &[request_change],
        ],
        program_id,
    )
}

fn anchor_sighash(ix_name: &str) -> eyre::Result<[u8; 8]> {
    let mut hasher = Sha256::new();
    hasher.update(format!("global:{ix_name}").as_bytes());
    let h = hasher.finalize();
    let mut out = [0_u8; 8];
    let first8 = h.as_slice().get(..8).context("sha256 truncated")?;
    out.copy_from_slice(first8);
    Ok(out)
}

fn anchor_account_discriminator(account_name: &str) -> eyre::Result<[u8; 8]> {
    let mut hasher = Sha256::new();
    hasher.update(format!("account:{account_name}").as_bytes());
    let h = hasher.finalize();
    let mut out = [0_u8; 8];
    let first8 = h.as_slice().get(..8).context("sha256 truncated")?;
    out.copy_from_slice(first8);
    Ok(out)
}

fn push_u64_le(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_option_u64(out: &mut Vec<u8>, v: Option<u64>) {
    match v {
        None => out.push(0),
        Some(x) => {
            out.push(1);
            push_u64_le(out, x);
        }
    }
}

fn push_option_bool(out: &mut Vec<u8>, v: Option<bool>) {
    match v {
        None => out.push(0),
        Some(b) => {
            out.push(1);
            out.push(u8::from(b));
        }
    }
}

fn push_side(out: &mut Vec<u8>, side: Side) {
    out.push(side.as_anchor_discriminant());
}

#[derive(Debug, Clone)]
pub struct IncreaseParams {
    pub size_usd_delta: u64,
    pub collateral_token_delta: u64,
    pub side: Side,
    pub price_slippage: u64,
    pub jupiter_minimum_out: Option<u64>,
    pub counter: u64,
}

#[derive(Debug, Clone)]
pub struct DecreaseParams {
    pub collateral_usd_delta: u64,
    pub size_usd_delta: u64,
    pub price_slippage: u64,
    pub jupiter_minimum_out: Option<u64>,
    pub entire_position: Option<bool>,
    pub counter: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct IncreasePositionMarketRequestAccounts {
    pub program_id: Pubkey,
    pub owner: Pubkey,
    pub funding_account: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub position_request: Pubkey,
    pub position_request_ata: Pubkey,
    pub custody: Pubkey,
    pub collateral_custody: Pubkey,
    pub input_mint: Pubkey,
    pub referral: Option<Pubkey>,
}

#[derive(Debug, Clone, Copy)]
pub struct DecreasePositionMarketRequestAccounts {
    pub program_id: Pubkey,
    pub owner: Pubkey,
    pub receiving_account: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub position_request: Pubkey,
    pub position_request_ata: Pubkey,
    pub custody: Pubkey,
    pub collateral_custody: Pubkey,
    pub desired_mint: Pubkey,
    pub referral: Option<Pubkey>,
}

pub fn build_create_increase_position_market_request_ix(
    accts: &IncreasePositionMarketRequestAccounts,
    params: &IncreaseParams,
) -> eyre::Result<Instruction> {
    let (perpetuals, _b) = perpetuals_pda(&accts.program_id);
    let event_auth = parse_pubkey(EVENT_AUTHORITY_MAINNET).context("event authority")?;
    let system_program = Pubkey::new_from_array([0_u8; 32]);

    let mut accounts = vec![
        AccountMeta::new(accts.owner, true),
        AccountMeta::new(accts.funding_account, false),
        AccountMeta::new_readonly(perpetuals, false),
        AccountMeta::new_readonly(accts.pool, false),
        AccountMeta::new(accts.position, false),
        AccountMeta::new(accts.position_request, false),
        AccountMeta::new(accts.position_request_ata, false),
        AccountMeta::new_readonly(accts.custody, false),
        AccountMeta::new_readonly(accts.collateral_custody, false),
        AccountMeta::new_readonly(accts.input_mint, false),
    ];
    if let Some(r) = accts.referral {
        accounts.push(AccountMeta::new_readonly(r, false));
    }
    accounts.extend_from_slice(&[
        AccountMeta::new_readonly(spl_token::id(), false),
        AccountMeta::new_readonly(spl_associated_token_account::id(), false),
        AccountMeta::new_readonly(system_program, false),
        AccountMeta::new_readonly(event_auth, false),
        AccountMeta::new_readonly(accts.program_id, false),
    ]);

    let mut data = Vec::with_capacity(8 + 64);
    data.extend_from_slice(&anchor_sighash("createIncreasePositionMarketRequest")?);
    push_u64_le(&mut data, params.size_usd_delta);
    push_u64_le(&mut data, params.collateral_token_delta);
    push_side(&mut data, params.side);
    push_u64_le(&mut data, params.price_slippage);
    push_option_u64(&mut data, params.jupiter_minimum_out);
    push_u64_le(&mut data, params.counter);

    Ok(Instruction {
        program_id: accts.program_id,
        accounts,
        data,
    })
}

pub fn build_create_decrease_position_market_request_ix(
    accts: &DecreasePositionMarketRequestAccounts,
    params: &DecreaseParams,
) -> eyre::Result<Instruction> {
    let (perpetuals, _b) = perpetuals_pda(&accts.program_id);
    let event_auth = parse_pubkey(EVENT_AUTHORITY_MAINNET).context("event authority")?;
    let system_program = Pubkey::new_from_array([0_u8; 32]);

    let mut accounts = vec![
        AccountMeta::new(accts.owner, true),
        AccountMeta::new(accts.receiving_account, false),
        AccountMeta::new_readonly(perpetuals, false),
        AccountMeta::new_readonly(accts.pool, false),
        AccountMeta::new_readonly(accts.position, false),
        AccountMeta::new(accts.position_request, false),
        AccountMeta::new(accts.position_request_ata, false),
        AccountMeta::new_readonly(accts.custody, false),
        AccountMeta::new_readonly(accts.collateral_custody, false),
        AccountMeta::new_readonly(accts.desired_mint, false),
    ];
    if let Some(r) = accts.referral {
        accounts.push(AccountMeta::new_readonly(r, false));
    }
    accounts.extend_from_slice(&[
        AccountMeta::new_readonly(spl_token::id(), false),
        AccountMeta::new_readonly(spl_associated_token_account::id(), false),
        AccountMeta::new_readonly(system_program, false),
        AccountMeta::new_readonly(event_auth, false),
        AccountMeta::new_readonly(accts.program_id, false),
    ]);

    let mut data = Vec::with_capacity(8 + 80);
    data.extend_from_slice(&anchor_sighash("createDecreasePositionMarketRequest")?);
    push_u64_le(&mut data, params.collateral_usd_delta);
    push_u64_le(&mut data, params.size_usd_delta);
    push_u64_le(&mut data, params.price_slippage);
    push_option_u64(&mut data, params.jupiter_minimum_out);
    push_option_bool(&mut data, params.entire_position);
    push_u64_le(&mut data, params.counter);

    Ok(Instruction {
        program_id: accts.program_id,
        accounts,
        data,
    })
}

fn take_bytes<const N: usize>(data: &[u8], i: &mut usize) -> eyre::Result<[u8; N]> {
    let end = i.saturating_add(N);
    let Some(slice) = data.get(*i..end) else {
        eyre::bail!("position decode: truncated");
    };
    let mut out = [0_u8; N];
    out.copy_from_slice(slice);
    *i = end;
    Ok(out)
}

pub fn decode_position_account(data: &[u8]) -> eyre::Result<PositionAccount> {
    let disc = anchor_account_discriminator("Position")?;
    let Some(prefix) = data.get(..8) else {
        eyre::bail!("unexpected account discriminator for Position");
    };
    if prefix != disc {
        eyre::bail!("unexpected account discriminator for Position");
    }
    let mut i = 8_usize;

    let owner = Pubkey::new_from_array(take_bytes::<32>(data, &mut i)?);
    let pool = Pubkey::new_from_array(take_bytes::<32>(data, &mut i)?);
    let custody = Pubkey::new_from_array(take_bytes::<32>(data, &mut i)?);
    let collateral_custody = Pubkey::new_from_array(take_bytes::<32>(data, &mut i)?);
    let open_time = i64::from_le_bytes(take_bytes::<8>(data, &mut i)?);
    let update_time = i64::from_le_bytes(take_bytes::<8>(data, &mut i)?);
    let side = take_bytes::<1>(data, &mut i)?[0];
    let price = u64::from_le_bytes(take_bytes::<8>(data, &mut i)?);
    let size_usd = u64::from_le_bytes(take_bytes::<8>(data, &mut i)?);
    let collateral_usd = u64::from_le_bytes(take_bytes::<8>(data, &mut i)?);
    let realised_pnl_usd = i64::from_le_bytes(take_bytes::<8>(data, &mut i)?);
    let cumulative_interest_snapshot = u128::from_le_bytes(take_bytes::<16>(data, &mut i)?);
    let locked_amount = u64::from_le_bytes(take_bytes::<8>(data, &mut i)?);
    let bump = take_bytes::<1>(data, &mut i)?[0];

    Ok(PositionAccount {
        owner,
        pool,
        custody,
        collateral_custody,
        open_time,
        update_time,
        side,
        price,
        size_usd,
        collateral_usd,
        realised_pnl_usd,
        cumulative_interest_snapshot,
        locked_amount,
        bump,
    })
}
