use crate::{errors::ToolError, policy::Policy};
use eyre::Context as _;
use std::str::FromStr as _;

const ONEINCH_ROUTER: &str = "0x1111111254eeb25477b68fb85ed929f73a960582";
const UNISWAP_ROUTER: &str = "0x68b3465833fb72a70ecdf485e0e4c7bd8665fc45";
const UNISWAP_BASE_ROUTER: &str = "0x2626664c2603336e57b271c5c0b26f421741e481";
// LayerZero EndpointV2 is deployed at the same address across many EVM chains.
// We allowlist it globally to make LayerZero bridging work out-of-the-box when
// adapter envelopes target the endpoint directly.
const LAYERZERO_ENDPOINT_V2: &str = "0x1a44076050125825900e736c501f859c50fe728c";

// Best-effort built-in DeFi allowlist for "native" protocol handlers.
//
// Users can disable allowlisting by setting `contract_allow_any=true`, or can provide
// explicit `contract_allowlist` entries.
const AAVE_V3_POOL_ETHEREUM: &str = "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2";
const AAVE_V3_POOL_BASE: &str = "0xa238dd80c259a72e81d7e4664a9801593f98d1c5";
const AAVE_V3_POOL_ARBITRUM_OPTIMISM_POLYGON: &str = "0x794a61358d6845594f94dc1db02a252b5b4814ad";

const WORMHOLE_TOKEN_BRIDGE_ETHEREUM: &str = "0x3ee18b2214aff97000d974cf647e7c347e8fa585";
const WORMHOLE_TOKEN_BRIDGE_ARBITRUM: &str = "0x0b2402144bb366a632d14b83f244d2e0e21bd39c";
const WORMHOLE_TOKEN_BRIDGE_OPTIMISM: &str = "0x1d68124e65fafc907325e3edbf8c4d84499daa8b";
const WORMHOLE_TOKEN_BRIDGE_POLYGON: &str = "0x5a58505a96d1dbf8df91cb21b54419fc36e93fde";
const WORMHOLE_TOKEN_BRIDGE_BASE: &str = "0x8d2de8d2f73f1dfe8b72d0d8e9fffbcf7aac8aef";
const WORMHOLE_TOKEN_BRIDGE_BNB: &str = "0xb6f6d86a8f9879a9c87f643768d9efc38c1da6e7";
const WORMHOLE_TOKEN_BRIDGE_AVALANCHE: &str = "0x0e082f06ff657d94310cb8ce8b0d9a04541d8052";

// Compound v3 Comet (USDC markets). Source: compound-finance/comet deployments.
const COMPOUND_COMET_ETHEREUM: &str = "0xc3d688b66703497daa19211eedff47f25384cdc3";
const COMPOUND_COMET_BASE: &str = "0xb125e6687d4313864e53df431d5425969c15eb2f";
const COMPOUND_COMET_ARBITRUM: &str = "0x9c4ec768c28520b50860ea7a15bd7213a9ff58bf";
const COMPOUND_COMET_OPTIMISM: &str = "0x2e44e174f7d53f0212823acc11c01a11d58c5bcb";
const COMPOUND_COMET_POLYGON: &str = "0xf25212e676d1f7f89cd72ffee66158f541246445";

// Lido (Ethereum mainnet).
const LIDO_STETH: &str = "0xae7ab96520de3a18e5e111b5eaab095312d7fe84";
const LIDO_WITHDRAWAL_QUEUE: &str = "0x889edc2edab5f40e902b864ad4d7ade8e412f9b1";

// Testnets (Wormhole).
const WORMHOLE_TOKEN_BRIDGE_SEPOLIA: &str = "0xdb5492265f6038831e89f495670ff909ade94bd9";
const WORMHOLE_TOKEN_BRIDGE_ARBITRUM_SEPOLIA: &str = "0xc7a204bdbfe983fcd8d8e61d02b475d4073ff97e";
const WORMHOLE_TOKEN_BRIDGE_OPTIMISM_SEPOLIA: &str = "0x99737ec4b815d816c49a385943baf0380e75c0ac";
const WORMHOLE_TOKEN_BRIDGE_BASE_SEPOLIA: &str = "0x86f55a04690fde37c5c5f6d0ca379b2ed2f334f9";
const WORMHOLE_TOKEN_BRIDGE_POLYGON_AMOY: &str = "0xc7a204bdbfe983fcd8d8e61d02b475d4073ff97e";
const WORMHOLE_TOKEN_BRIDGE_BNB_TESTNET: &str = "0x9dcf9d205c9de35334d646bee44b2d2859712a09";
const WORMHOLE_TOKEN_BRIDGE_AVALANCHE_FUJI: &str = "0x61e44e506ca5659e6c0bba9b678586fa2d729756";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOp {
    Send,
    Swap,
    OpenPerpPosition,
    ClosePerpPosition,
    ModifyPerpOrder,
    PlaceLimitOrder,
    BuyNft,
    SellNft,
    TransferNft,
    BidNft,
    PumpfunBuy,
    PumpfunSell,
    Bridge,
    Lend,
    WithdrawLending,
    Borrow,
    RepayBorrow,
    Stake,
    Unstake,
    ProvideLiquidity,
    RemoveLiquidity,
    PlacePrediction,
    ClosePrediction,
    /// Transfers between Seashail-managed wallets/accounts.
    ///
    /// These are exempt by default (`policy.internal_transfers_exempt=true`), but can be made
    /// subject to global USD caps + tiered approval when the exemption is disabled.
    InternalTransfer,
}

pub const ALL_WRITE_OPS: &[WriteOp] = &[
    WriteOp::Send,
    WriteOp::Swap,
    WriteOp::OpenPerpPosition,
    WriteOp::ClosePerpPosition,
    WriteOp::ModifyPerpOrder,
    WriteOp::PlaceLimitOrder,
    WriteOp::BuyNft,
    WriteOp::SellNft,
    WriteOp::TransferNft,
    WriteOp::BidNft,
    WriteOp::PumpfunBuy,
    WriteOp::PumpfunSell,
    WriteOp::Bridge,
    WriteOp::Lend,
    WriteOp::WithdrawLending,
    WriteOp::Borrow,
    WriteOp::RepayBorrow,
    WriteOp::Stake,
    WriteOp::Unstake,
    WriteOp::ProvideLiquidity,
    WriteOp::RemoveLiquidity,
    WriteOp::PlacePrediction,
    WriteOp::ClosePrediction,
    WriteOp::InternalTransfer,
];

#[derive(Debug, Clone)]
pub struct PolicyContext<'a> {
    pub op: WriteOp,
    pub chain: &'a str,
    pub usd_value: f64,
    pub usd_value_known: bool,
    pub daily_used_usd: f64,
    pub slippage_bps: Option<u32>,
    pub to_address: Option<&'a str>,
    pub contract: Option<&'a str>,
    pub leverage: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    AutoApprove,
    RequiresUserConfirm,
}

pub fn evaluate(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<Approval, ToolError> {
    // Internal transfers are "inside the security boundary" and cannot exfiltrate to an external
    // recipient. By default they are policy-exempt (still subject to hard blocks enforced by the
    // tool layer: scam blocklist + OFAC when enabled).
    if ctx.op == WriteOp::InternalTransfer && policy.internal_transfers_exempt.get() {
        return Ok(Approval::AutoApprove);
    }

    let force_confirm_for_unknown_usd = check_unknown_usd(policy, ctx)?;
    check_op_specific(policy, ctx)?;
    check_global_usd_limits(policy, ctx, force_confirm_for_unknown_usd)?;

    if force_confirm_for_unknown_usd {
        return Ok(Approval::RequiresUserConfirm);
    }

    if ctx.usd_value <= policy.auto_approve_usd {
        Ok(Approval::AutoApprove)
    } else {
        Ok(Approval::RequiresUserConfirm)
    }
}

/// Determine whether the USD value is unknown and whether we must force user confirmation.
fn check_unknown_usd(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<bool, ToolError> {
    if ctx.usd_value_known {
        return Ok(false);
    }
    match ctx.op {
        WriteOp::TransferNft | WriteOp::SellNft => Ok(true),
        WriteOp::Send
        | WriteOp::Swap
        | WriteOp::OpenPerpPosition
        | WriteOp::ClosePerpPosition
        | WriteOp::ModifyPerpOrder
        | WriteOp::PlaceLimitOrder
        | WriteOp::BuyNft
        | WriteOp::BidNft
        | WriteOp::PumpfunBuy
        | WriteOp::PumpfunSell
        | WriteOp::Bridge
        | WriteOp::Lend
        | WriteOp::WithdrawLending
        | WriteOp::Borrow
        | WriteOp::RepayBorrow
        | WriteOp::Stake
        | WriteOp::Unstake
        | WriteOp::ProvideLiquidity
        | WriteOp::RemoveLiquidity
        | WriteOp::PlacePrediction
        | WriteOp::ClosePrediction
        | WriteOp::InternalTransfer => {
            if policy.deny_unknown_usd_value.get() {
                return Err(ToolError::new(
                    "policy_usd_value_unknown",
                    "unable to compute USD value (pricing unavailable); refusing to sign",
                ));
            }
            Ok(true)
        }
    }
}

/// Dispatch per-operation policy checks.
fn check_op_specific(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    match ctx.op {
        WriteOp::Send => check_send(policy, ctx),
        WriteOp::Swap => check_swap(policy, ctx),
        WriteOp::OpenPerpPosition
        | WriteOp::ClosePerpPosition
        | WriteOp::ModifyPerpOrder
        | WriteOp::PlaceLimitOrder => check_perps(policy, ctx),
        WriteOp::BuyNft | WriteOp::SellNft | WriteOp::TransferNft | WriteOp::BidNft => {
            check_nft(policy, ctx)
        }
        WriteOp::PumpfunBuy | WriteOp::PumpfunSell => check_pumpfun(policy),
        WriteOp::Bridge => check_bridge(policy, ctx),
        WriteOp::Lend | WriteOp::WithdrawLending | WriteOp::Borrow | WriteOp::RepayBorrow => {
            check_lending(policy, ctx)
        }
        WriteOp::Stake | WriteOp::Unstake => check_staking(policy, ctx),
        WriteOp::ProvideLiquidity | WriteOp::RemoveLiquidity => check_liquidity(policy, ctx),
        WriteOp::PlacePrediction | WriteOp::ClosePrediction => check_prediction(policy, ctx),
        WriteOp::InternalTransfer => Ok(()),
    }
}

/// Enforce global per-tx, hard-block, and daily USD limits.
fn check_global_usd_limits(
    policy: &Policy,
    ctx: &PolicyContext<'_>,
    force_confirm: bool,
) -> Result<(), ToolError> {
    if force_confirm {
        return Ok(());
    }
    if ctx.usd_value.is_nan() || ctx.usd_value.is_infinite() || ctx.usd_value < 0.0_f64 {
        return Err(ToolError::new(
            "invalid_usd_value",
            "invalid computed USD value",
        ));
    }
    if ctx.usd_value > policy.max_usd_per_tx {
        return Err(ToolError::new(
            "policy_max_usd_per_tx",
            format!(
                "usd_value {:.2} exceeds max_usd_per_tx {:.2}",
                ctx.usd_value, policy.max_usd_per_tx
            ),
        ));
    }
    if ctx.usd_value > policy.hard_block_over_usd {
        return Err(ToolError::new(
            "policy_hard_block",
            format!(
                "usd_value {:.2} exceeds hard_block_over_usd {:.2}",
                ctx.usd_value, policy.hard_block_over_usd
            ),
        ));
    }
    let daily_total = crate::financial_math::daily_total_usd(ctx.daily_used_usd, ctx.usd_value);
    if daily_total > policy.max_usd_per_day {
        return Err(ToolError::new(
            "policy_daily_limit",
            format!(
                "daily limit exceeded: used {:.2} + this {:.2} > {:.2}",
                ctx.daily_used_usd, ctx.usd_value, policy.max_usd_per_day
            ),
        ));
    }
    Ok(())
}

fn check_send(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_send.get() {
        return Err(ToolError::new(
            "policy_send_disabled",
            "send is disabled by policy",
        ));
    }
    if !policy.send_allow_any.get() {
        let to = ctx
            .to_address
            .ok_or_else(|| ToolError::new("invalid_request", "missing to address"))?;
        let ok = policy.send_allowlist.iter().any(|a| {
            normalize_addr(ctx.chain, a)
                .is_some_and(|na| normalize_addr(ctx.chain, to).is_some_and(|nt| na == nt))
        });
        if !ok {
            return Err(ToolError::new(
                "policy_recipient_not_allowlisted",
                "recipient is not allowlisted by policy",
            ));
        }
    }
    Ok(())
}

fn check_swap(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_swap.get() {
        return Err(ToolError::new(
            "policy_swap_disabled",
            "swap is disabled by policy",
        ));
    }
    if let Some(slip) = ctx.slippage_bps {
        if slip > policy.max_slippage_bps {
            return Err(ToolError::new(
                "policy_slippage_too_high",
                format!(
                    "slippage_bps {slip} exceeds max_slippage_bps {}",
                    policy.max_slippage_bps
                ),
            ));
        }
    }
    if let Some(contract) = ctx.contract {
        ensure_contract_allowlisted(policy, ctx.chain, contract)?;
    }
    Ok(())
}

fn check_perps(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_perps.get() {
        return Err(ToolError::new(
            "policy_perps_disabled",
            "perpetuals are disabled by policy",
        ));
    }
    if matches!(
        ctx.op,
        WriteOp::OpenPerpPosition | WriteOp::PlaceLimitOrder | WriteOp::ModifyPerpOrder
    ) {
        if ctx.usd_value > policy.max_usd_per_position {
            return Err(ToolError::new(
                "policy_max_usd_per_position",
                format!(
                    "usd_value {:.2} exceeds max_usd_per_position {:.2}",
                    ctx.usd_value, policy.max_usd_per_position
                ),
            ));
        }
        if let Some(lv) = ctx.leverage {
            if lv > policy.max_leverage {
                return Err(ToolError::new(
                    "policy_leverage_too_high",
                    format!("leverage {lv} exceeds max_leverage {}", policy.max_leverage),
                ));
            }
        }
    }
    Ok(())
}

fn check_nft(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_nft.get() {
        return Err(ToolError::new(
            "policy_nft_disabled",
            "nft operations are disabled by policy",
        ));
    }
    if matches!(ctx.op, WriteOp::BuyNft | WriteOp::BidNft)
        && ctx.usd_value > policy.max_usd_per_nft_tx
    {
        return Err(ToolError::new(
            "policy_max_usd_per_nft_tx",
            format!(
                "usd_value {:.2} exceeds max_usd_per_nft_tx {:.2}",
                ctx.usd_value, policy.max_usd_per_nft_tx
            ),
        ));
    }
    if ctx.chain != "solana"
        && matches!(ctx.op, WriteOp::BuyNft | WriteOp::SellNft | WriteOp::BidNft)
    {
        if let Some(contract) = ctx.contract {
            ensure_contract_allowlisted(policy, ctx.chain, contract)?;
        } else if !policy.contract_allow_any.get() {
            return Err(ToolError::new("invalid_request", "missing contract"));
        }
    }
    Ok(())
}

fn check_pumpfun(policy: &Policy) -> Result<(), ToolError> {
    if !policy.enable_pumpfun.get() {
        return Err(ToolError::new(
            "policy_pumpfun_disabled",
            "pump.fun operations are disabled by policy",
        ));
    }
    Ok(())
}

fn check_bridge(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_bridge.get() {
        return Err(ToolError::new(
            "policy_bridge_disabled",
            "bridging is disabled by policy",
        ));
    }
    ensure_evm_contract(policy, ctx)?;
    if ctx.usd_value > policy.max_usd_per_bridge_tx {
        return Err(ToolError::new(
            "policy_max_usd_per_bridge_tx",
            format!(
                "usd_value {:.2} exceeds max_usd_per_bridge_tx {:.2}",
                ctx.usd_value, policy.max_usd_per_bridge_tx
            ),
        ));
    }
    Ok(())
}

fn check_lending(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_lending.get() {
        return Err(ToolError::new(
            "policy_lending_disabled",
            "lending/borrowing is disabled by policy",
        ));
    }
    ensure_evm_contract(policy, ctx)?;
    if matches!(ctx.op, WriteOp::Lend | WriteOp::Borrow)
        && ctx.usd_value > policy.max_usd_per_lending_tx
    {
        return Err(ToolError::new(
            "policy_max_usd_per_lending_tx",
            format!(
                "usd_value {:.2} exceeds max_usd_per_lending_tx {:.2}",
                ctx.usd_value, policy.max_usd_per_lending_tx
            ),
        ));
    }
    Ok(())
}

fn check_staking(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_staking.get() {
        return Err(ToolError::new(
            "policy_staking_disabled",
            "staking is disabled by policy",
        ));
    }
    ensure_evm_contract(policy, ctx)?;
    if matches!(ctx.op, WriteOp::Stake) && ctx.usd_value > policy.max_usd_per_stake_tx {
        return Err(ToolError::new(
            "policy_max_usd_per_stake_tx",
            format!(
                "usd_value {:.2} exceeds max_usd_per_stake_tx {:.2}",
                ctx.usd_value, policy.max_usd_per_stake_tx
            ),
        ));
    }
    Ok(())
}

fn check_liquidity(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_liquidity.get() {
        return Err(ToolError::new(
            "policy_liquidity_disabled",
            "liquidity provision is disabled by policy",
        ));
    }
    ensure_evm_contract(policy, ctx)?;
    if matches!(ctx.op, WriteOp::ProvideLiquidity)
        && ctx.usd_value > policy.max_usd_per_liquidity_tx
    {
        return Err(ToolError::new(
            "policy_max_usd_per_liquidity_tx",
            format!(
                "usd_value {:.2} exceeds max_usd_per_liquidity_tx {:.2}",
                ctx.usd_value, policy.max_usd_per_liquidity_tx
            ),
        ));
    }
    Ok(())
}

fn check_prediction(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if !policy.enable_prediction.get() {
        return Err(ToolError::new(
            "policy_prediction_disabled",
            "prediction markets are disabled by policy",
        ));
    }
    ensure_evm_contract(policy, ctx)?;
    if matches!(ctx.op, WriteOp::PlacePrediction)
        && ctx.usd_value > policy.max_usd_per_prediction_tx
    {
        return Err(ToolError::new(
            "policy_max_usd_per_prediction_tx",
            format!(
                "usd_value {:.2} exceeds max_usd_per_prediction_tx {:.2}",
                ctx.usd_value, policy.max_usd_per_prediction_tx
            ),
        ));
    }
    Ok(())
}

/// Contract allowlisting for non-Solana, non-Bitcoin EVM chains.
fn ensure_evm_contract(policy: &Policy, ctx: &PolicyContext<'_>) -> Result<(), ToolError> {
    if ctx.chain != "solana" && ctx.chain != "bitcoin" {
        if let Some(contract) = ctx.contract {
            ensure_contract_allowlisted(policy, ctx.chain, contract)?;
        }
    }
    Ok(())
}

fn ensure_contract_allowlisted(
    policy: &Policy,
    chain: &str,
    contract: &str,
) -> Result<(), ToolError> {
    if policy.contract_allow_any.get() {
        return Ok(());
    }
    if policy.contract_allowlist.is_empty() {
        // Empty list means "built-in allowlist only".
        let ok = built_in_allowed_contract(chain, contract);
        if !ok {
            return Err(ToolError::new(
                "policy_contract_not_allowlisted",
                "contract is not allowlisted (built-in allowlist)",
            ));
        }
        return Ok(());
    }
    let ok = normalize_addr(chain, contract).is_some_and(|contract_norm| {
        policy
            .contract_allowlist
            .iter()
            .filter_map(|a| normalize_addr(chain, a))
            .any(|na| na == contract_norm)
    });
    if !ok {
        return Err(ToolError::new(
            "policy_contract_not_allowlisted",
            "contract is not allowlisted by policy",
        ));
    }
    Ok(())
}

fn built_in_allowed_contract(chain: &str, contract: &str) -> bool {
    // Best-effort allowlist. EVM swaps use Uniswap routers; Solana swaps
    // are authenticated via HTTPS and validated at the provider level.
    if chain == "solana" {
        return contract.trim().eq_ignore_ascii_case("jupiter");
    }

    let Ok(c) = normalize_evm_address(contract) else {
        return false;
    };

    // LayerZero's EndpointV2 is shared across many EVM networks. Allow it regardless
    // of chain name (writes will still fail if the chain itself is unsupported).
    if c == LAYERZERO_ENDPOINT_V2 {
        return true;
    }

    // 1inch uses the same router address across EVM networks; allow it even if the
    // chain isn't explicitly enumerated below (swaps will still fail if unsupported).
    if c == ONEINCH_ROUTER {
        return true;
    }
    match chain {
        "ethereum" => {
            c == UNISWAP_ROUTER
                || c == AAVE_V3_POOL_ETHEREUM
                || c == COMPOUND_COMET_ETHEREUM
                || c == WORMHOLE_TOKEN_BRIDGE_ETHEREUM
                || c == LIDO_STETH
                || c == LIDO_WITHDRAWAL_QUEUE
        }
        "base" => {
            c == UNISWAP_BASE_ROUTER
                || c == AAVE_V3_POOL_BASE
                || c == COMPOUND_COMET_BASE
                || c == WORMHOLE_TOKEN_BRIDGE_BASE
        }
        "arbitrum" => {
            c == UNISWAP_ROUTER
                || c == AAVE_V3_POOL_ARBITRUM_OPTIMISM_POLYGON
                || c == COMPOUND_COMET_ARBITRUM
                || c == WORMHOLE_TOKEN_BRIDGE_ARBITRUM
        }
        "optimism" => {
            c == UNISWAP_ROUTER
                || c == AAVE_V3_POOL_ARBITRUM_OPTIMISM_POLYGON
                || c == COMPOUND_COMET_OPTIMISM
                || c == WORMHOLE_TOKEN_BRIDGE_OPTIMISM
        }
        "polygon" => {
            c == UNISWAP_ROUTER
                || c == AAVE_V3_POOL_ARBITRUM_OPTIMISM_POLYGON
                || c == COMPOUND_COMET_POLYGON
                || c == WORMHOLE_TOKEN_BRIDGE_POLYGON
        }

        // Wormhole bridge support on additional chains/testnets.
        "bnb" => c == WORMHOLE_TOKEN_BRIDGE_BNB,
        "avalanche" => c == WORMHOLE_TOKEN_BRIDGE_AVALANCHE,
        "sepolia" => c == UNISWAP_ROUTER || c == WORMHOLE_TOKEN_BRIDGE_SEPOLIA,
        "arbitrum-sepolia" => c == WORMHOLE_TOKEN_BRIDGE_ARBITRUM_SEPOLIA,
        "optimism-sepolia" => c == WORMHOLE_TOKEN_BRIDGE_OPTIMISM_SEPOLIA,
        "base-sepolia" => c == UNISWAP_BASE_ROUTER || c == WORMHOLE_TOKEN_BRIDGE_BASE_SEPOLIA,
        "polygon-amoy" => c == WORMHOLE_TOKEN_BRIDGE_POLYGON_AMOY,
        "bnb-testnet" => c == WORMHOLE_TOKEN_BRIDGE_BNB_TESTNET,
        "avalanche-fuji" => c == WORMHOLE_TOKEN_BRIDGE_AVALANCHE_FUJI,
        _ => false,
    }
}

fn normalize_evm_address(s: &str) -> eyre::Result<String> {
    let a = alloy::primitives::Address::from_str(s.trim()).context("parse evm address")?;
    Ok(format!("{a:#x}"))
}

fn normalize_addr(chain: &str, s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if chain == "solana" {
        // Solana pubkeys are base58 and case-sensitive; compare the parsed canonical form.
        let pk = solana_sdk::pubkey::Pubkey::from_str(t).ok()?;
        return Some(pk.to_string());
    }
    if chain == "bitcoin" {
        // Accept common address formats; canonicalize to the normalized string.
        let addr = bitcoin::Address::<bitcoin::address::NetworkUnchecked>::from_str(t)
            .ok()?
            .assume_checked();
        // Normalize bech32 hrp+data to lowercase for comparisons.
        let addr_str = addr.to_string();
        if addr_str.to_ascii_lowercase().starts_with("bc1")
            || addr_str.to_ascii_lowercase().starts_with("tb1")
        {
            return Some(addr_str.to_ascii_lowercase());
        }
        return Some(addr_str);
    }
    // Default to EVM-style address parsing/canonicalization.
    normalize_evm_address(t).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_blocks_when_allowlist_empty_and_send_allow_any_false() -> eyre::Result<()> {
        let policy = Policy::default();
        let ctx = PolicyContext {
            op: WriteOp::Send,
            chain: "ethereum",
            usd_value: 1.0,
            usd_value_known: true,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: Some("0x000000000000000000000000000000000000dEaD"),
            contract: None,
            leverage: None,
        };
        let Err(err) = evaluate(&policy, &ctx) else {
            eyre::bail!("expected policy error");
        };
        assert_eq!(err.code, "policy_recipient_not_allowlisted");
        Ok(())
    }

    #[test]
    fn send_auto_approves_under_threshold_when_allow_any() -> eyre::Result<()> {
        let policy = Policy {
            send_allow_any: true.into(),
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::Send,
            chain: "ethereum",
            usd_value: 5.0,
            usd_value_known: true,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: Some("0x000000000000000000000000000000000000dEaD"),
            contract: None,
            leverage: None,
        };
        match evaluate(&policy, &ctx) {
            Ok(Approval::AutoApprove) => {}
            Ok(other) => eyre::bail!("unexpected approval: {other:?}"),
            Err(e) => eyre::bail!("unexpected error: {}", e.code),
        }
        Ok(())
    }

    #[test]
    fn swap_blocks_when_slippage_exceeds_policy() -> eyre::Result<()> {
        let policy = Policy::default();
        let ctx = PolicyContext {
            op: WriteOp::Swap,
            chain: "ethereum",
            usd_value: 1.0,
            usd_value_known: true,
            daily_used_usd: 0.0,
            slippage_bps: Some(policy.max_slippage_bps + 1),
            to_address: None,
            contract: Some("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"),
            leverage: None,
        };
        let Err(err) = evaluate(&policy, &ctx) else {
            eyre::bail!("expected policy error");
        };
        assert_eq!(err.code, "policy_slippage_too_high");
        Ok(())
    }

    #[test]
    fn blocks_when_daily_limit_exceeded() -> eyre::Result<()> {
        let policy = Policy {
            send_allow_any: true.into(),
            max_usd_per_day: 10.0,
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::Send,
            chain: "ethereum",
            usd_value: 6.0,
            usd_value_known: true,
            daily_used_usd: 6.0,
            slippage_bps: None,
            to_address: Some("0x000000000000000000000000000000000000dEaD"),
            contract: None,
            leverage: None,
        };
        let Err(err) = evaluate(&policy, &ctx) else {
            eyre::bail!("expected policy error");
        };
        assert_eq!(err.code, "policy_daily_limit");
        Ok(())
    }

    #[test]
    fn transfer_nft_unknown_usd_requires_confirm_even_when_deny_unknown_is_true() -> eyre::Result<()>
    {
        let policy = Policy {
            enable_nft: true.into(),
            deny_unknown_usd_value: true.into(),
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::TransferNft,
            chain: "ethereum",
            usd_value: 0.0,
            usd_value_known: false,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: Some("0x000000000000000000000000000000000000dEaD"),
            contract: Some("0x000000000000000000000000000000000000bEEF"),
            leverage: None,
        };
        match evaluate(&policy, &ctx) {
            Ok(Approval::RequiresUserConfirm) => {}
            Ok(other) => eyre::bail!("unexpected approval: {other:?}"),
            Err(e) => eyre::bail!("unexpected error: {}", e.code),
        }
        Ok(())
    }

    #[test]
    fn buy_nft_unknown_usd_is_blocked_when_deny_unknown_is_true() -> eyre::Result<()> {
        let policy = Policy {
            enable_nft: true.into(),
            deny_unknown_usd_value: true.into(),
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::BuyNft,
            chain: "ethereum",
            usd_value: 0.0,
            usd_value_known: false,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: None,
            contract: Some("0x000000000000000000000000000000000000bEEF"),
            leverage: None,
        };
        let Err(err) = evaluate(&policy, &ctx) else {
            eyre::bail!("expected policy error");
        };
        assert_eq!(err.code, "policy_usd_value_unknown");
        Ok(())
    }

    #[test]
    fn buy_nft_blocks_when_contract_not_allowlisted_and_allow_any_false() -> eyre::Result<()> {
        let policy = Policy {
            enable_nft: true.into(),
            deny_unknown_usd_value: false.into(),
            contract_allow_any: false.into(),
            contract_allowlist: vec![],
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::BuyNft,
            chain: "ethereum",
            usd_value: 1.0,
            usd_value_known: true,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: None,
            contract: Some("0x000000000000000000000000000000000000dEaD"),
            leverage: None,
        };
        let Err(err) = evaluate(&policy, &ctx) else {
            eyre::bail!("expected policy error");
        };
        assert_eq!(err.code, "policy_contract_not_allowlisted");
        Ok(())
    }

    #[test]
    fn buy_nft_allows_when_contract_allow_any_true() {
        let policy = Policy {
            enable_nft: true.into(),
            deny_unknown_usd_value: false.into(),
            contract_allow_any: true.into(),
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::BuyNft,
            chain: "ethereum",
            usd_value: 1.0,
            usd_value_known: true,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: None,
            contract: Some("0x000000000000000000000000000000000000dEaD"),
            leverage: None,
        };
        assert!(evaluate(&policy, &ctx).is_ok());
    }

    #[test]
    fn modify_perp_order_blocks_when_usd_exceeds_max_usd_per_position() -> eyre::Result<()> {
        let policy = Policy {
            enable_perps: true.into(),
            max_usd_per_position: 100.0,
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::ModifyPerpOrder,
            chain: "hyperliquid",
            usd_value: 101.0,
            usd_value_known: true,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: None,
            contract: Some("hyperliquid"),
            leverage: Some(1),
        };
        let Err(err) = evaluate(&policy, &ctx) else {
            eyre::bail!("expected policy error");
        };
        assert_eq!(err.code, "policy_max_usd_per_position");
        Ok(())
    }

    #[test]
    fn modify_perp_order_blocks_when_leverage_exceeds_max_leverage() -> eyre::Result<()> {
        let policy = Policy {
            enable_perps: true.into(),
            max_leverage: 3,
            ..Default::default()
        };
        let ctx = PolicyContext {
            op: WriteOp::ModifyPerpOrder,
            chain: "hyperliquid",
            usd_value: 50.0,
            usd_value_known: true,
            daily_used_usd: 0.0,
            slippage_bps: None,
            to_address: None,
            contract: Some("hyperliquid"),
            leverage: Some(4),
        };
        let Err(err) = evaluate(&policy, &ctx) else {
            eyre::bail!("expected policy error");
        };
        assert_eq!(err.code, "policy_leverage_too_high");
        Ok(())
    }

    #[test]
    fn built_in_allowlist_includes_compound_v3_comet_markets() {
        assert!(built_in_allowed_contract(
            "ethereum",
            COMPOUND_COMET_ETHEREUM
        ));
        assert!(built_in_allowed_contract("base", COMPOUND_COMET_BASE));
        assert!(built_in_allowed_contract(
            "arbitrum",
            COMPOUND_COMET_ARBITRUM
        ));
        assert!(built_in_allowed_contract(
            "optimism",
            COMPOUND_COMET_OPTIMISM
        ));
        assert!(built_in_allowed_contract("polygon", COMPOUND_COMET_POLYGON));
    }
}
