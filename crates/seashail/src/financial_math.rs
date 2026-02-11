//! Centralised float-arithmetic helpers for USD value estimation.
//!
//! All `cast_precision_loss` / `float_arithmetic` lint expects live here so that
//! call-sites in individual handlers can be lint-clean.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_arithmetic,
    reason = "dedicated float-math module; casts and arithmetic are intentional"
)]

/// Convert a token amount in base units to a USD value.
///
/// `base`     – raw amount (e.g. lamports, wei, base-unit count).
/// `decimals` – number of decimal places the token uses.
/// `price`    – USD price per **one whole** token.
pub fn token_base_to_usd(base: u128, decimals: u8, price: f64) -> f64 {
    let divisor = 10_f64.powi(i32::from(decimals));
    (base as f64 / divisor) * price
}

/// Shorthand for SOL: lamports (9 decimals) to USD.
pub fn lamports_to_usd(lamports: u64, sol_price_usd: f64) -> f64 {
    token_base_to_usd(u128::from(lamports), 9, sol_price_usd)
}

/// Convert basis-points to a fractional multiplier (e.g. 50 bps → 0.005).
pub fn bps_to_fraction(bps: u32) -> f64 {
    f64::from(bps) / 10_000.0_f64
}

/// Compute a worst-case execution price given slippage.
///
/// * For a **long** (buy): the limit price should be *below* market → `px * (1 - slip)`.
/// * For a **short** (sell): the limit price should be *above* market → `px * (1 + slip)`.
///
/// This matches Jupiter Perps convention where the keeper fills at the
/// slippage-adjusted price.
pub fn worst_case_price(px: f64, is_long: bool, slip: f64) -> f64 {
    if is_long {
        px * (1.0_f64 - slip)
    } else {
        px * (1.0_f64 + slip)
    }
}

/// Returns `true` when two USD caps differ by more than `f64::EPSILON`.
pub fn usd_cap_mismatch(a: f64, b: f64) -> bool {
    (a - b).abs() > f64::EPSILON
}

/// Sum a slice of `f64` values.
pub fn sum_f64(values: &[f64]) -> f64 {
    values.iter().fold(0.0_f64, |a, b| a + b)
}

/// Accumulate a value into a running total (in-place addition).
pub fn accum(total: &mut f64, value: f64) {
    *total += value;
}

/// Multiply two `f64` values (for use in contexts that must avoid inline float arithmetic).
pub fn mul_f64(a: f64, b: f64) -> f64 {
    a * b
}

/// Divide two `f64` values (for use in contexts that must avoid inline float arithmetic).
pub fn div_f64(a: f64, b: f64) -> f64 {
    a / b
}

/// Compute the absolute value of an `f64`.
pub fn abs_f64(v: f64) -> f64 {
    v.abs()
}

/// Subtract two `f64` values (`a - b`).
pub fn sub_f64(a: f64, b: f64) -> f64 {
    a - b
}

/// Compute the projected daily total USD spend after a new transaction.
pub fn daily_total_usd(daily_used_usd: f64, tx_usd_value: f64) -> f64 {
    daily_used_usd + tx_usd_value
}

/// Round a float to `sig` significant figures.
pub fn round_sig(x: f64, sig: i32) -> f64 {
    if x == 0.0_f64 || !x.is_finite() {
        return x;
    }
    let abs = x.abs();
    let log10 = abs.log10().floor();
    let scale = 10_f64.powf(f64::from(sig) - 1.0_f64 - log10);
    (x * scale).round() / scale
}

/// Round a float to `decimals` decimal places.
pub fn round_decimals(x: f64, decimals: i32) -> f64 {
    if !x.is_finite() {
        return x;
    }
    if decimals <= 0_i32 {
        return x.round();
    }
    let scale = 10_f64.powi(decimals);
    (x * scale).round() / scale
}

/// Compute a slippage-adjusted limit price for Hyperliquid order placement.
///
/// Rounds to 5 significant figures, then to `max(0, 6 - sz_decimals)` decimal places.
pub fn slippage_limit_px(mid_px: f64, is_buy: bool, slippage: f64, sz_decimals: u32) -> f64 {
    let mut px = if is_buy {
        mid_px * (1.0_f64 + slippage)
    } else {
        mid_px * (1.0_f64 - slippage)
    };
    px = round_sig(px, 5);
    let sz_decimals_i32 = i32::try_from(sz_decimals).unwrap_or(i32::MAX);
    let decimals = 6_i32 - sz_decimals_i32;
    round_decimals(px, decimals.max(0))
}

/// Format a float as a wire-safe string (up to 8 decimal places, trailing zeros stripped).
///
/// Returns an error if the 8-decimal representation causes measurable rounding.
pub fn float_to_wire(x: f64) -> eyre::Result<String> {
    let s = format!("{x:.8}");
    let y = s.parse::<f64>().unwrap_or(x);
    if (y - x).abs() >= 1e-12_f64 {
        eyre::bail!("float_to_wire causes rounding");
    }
    let t = s.trim_end_matches('0').trim_end_matches('.').to_owned();
    Ok(if t.is_empty() { "0".to_owned() } else { t })
}

/// Clamp a fee rate (as f64) to a safe u64 range.
///
/// Used by Bitcoin fee estimation where the upstream returns f64 sats/vbyte.
/// Safety: the `clamp` ensures the value is in `[1.0, 5000.0]` before truncating,
/// so the cast cannot lose sign or significant magnitude.
pub fn clamp_fee_rate(fee: f64) -> u64 {
    fee.clamp(1.0_f64, 5000.0_f64) as u64
}
