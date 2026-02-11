use eyre::Context as _;

pub fn parse_amount_base_u128(s: &str) -> eyre::Result<u128> {
    let s = s.trim();
    if s.is_empty() {
        eyre::bail!("empty amount");
    }
    let v: u128 = s.parse().context("parse base amount")?;
    Ok(v)
}

pub fn parse_amount_ui_to_base_u128(s: &str, decimals: u32) -> eyre::Result<u128> {
    let s = s.trim();
    if s.is_empty() {
        eyre::bail!("empty amount");
    }

    let (whole, frac) = match s.split_once('.') {
        Some((a, b)) => (a, b),
        None => (s, ""),
    };

    if whole.starts_with('-') {
        eyre::bail!("amount must be non-negative");
    }

    let whole_v: u128 = if whole.is_empty() {
        0
    } else {
        whole.parse().context("parse whole")?
    };

    if frac.len() > decimals as usize {
        eyre::bail!("too many decimal places for token (decimals={decimals})");
    }

    let mut frac_s = frac.to_owned();
    while frac_s.len() < decimals as usize {
        frac_s.push('0');
    }
    let frac_v: u128 = if frac_s.is_empty() {
        0
    } else {
        frac_s.parse().context("parse fractional")?
    };

    let scale = 10_u128
        .checked_pow(decimals)
        .ok_or_else(|| eyre::eyre!("decimals too large"))?;

    let base = whole_v
        .checked_mul(scale)
        .and_then(|x| x.checked_add(frac_v))
        .ok_or_else(|| eyre::eyre!("amount overflow"))?;

    Ok(base)
}

/// Format a base-unit integer amount into a UI decimal string without using floats.
///
/// Examples:
/// - base=1500000, decimals=6 => "1.5"
/// - base=1, decimals=6 => "0.000001"
pub fn format_amount_base_to_ui_string(base: u128, decimals: u32) -> eyre::Result<String> {
    if decimals == 0 {
        return Ok(base.to_string());
    }
    let scale = 10_u128
        .checked_pow(decimals)
        .ok_or_else(|| eyre::eyre!("decimals too large"))?;
    let whole = base / scale;
    let frac = base % scale;
    if frac == 0 {
        return Ok(whole.to_string());
    }
    let mut frac_s = format!("{frac:0width$}", width = decimals as usize);
    while frac_s.ends_with('0') {
        frac_s.pop();
    }
    Ok(format!("{whole}.{frac_s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_base_amount() {
        let v0 = parse_amount_base_u128("0");
        assert!(v0.is_ok(), "parse failed: {v0:?}");
        assert_eq!(v0.ok(), Some(0));

        let v42 = parse_amount_base_u128("42");
        assert!(v42.is_ok(), "parse failed: {v42:?}");
        assert_eq!(v42.ok(), Some(42));
    }

    #[test]
    fn parse_ui_amount_basic() {
        let v1 = parse_amount_ui_to_base_u128("1", 6);
        assert!(v1.is_ok(), "parse failed: {v1:?}");
        assert_eq!(v1.ok(), Some(1_000_000));

        let v15 = parse_amount_ui_to_base_u128("1.5", 6);
        assert!(v15.is_ok(), "parse failed: {v15:?}");
        assert_eq!(v15.ok(), Some(1_500_000));

        let vsmall = parse_amount_ui_to_base_u128("0.000001", 6);
        assert!(vsmall.is_ok(), "parse failed: {vsmall:?}");
        assert_eq!(vsmall.ok(), Some(1));

        let v0 = parse_amount_ui_to_base_u128("0", 18);
        assert!(v0.is_ok(), "parse failed: {v0:?}");
        assert_eq!(v0.ok(), Some(0));
    }

    #[test]
    fn parse_ui_rejects_too_many_decimals() {
        let r = parse_amount_ui_to_base_u128("1.0000001", 6);
        assert!(r.is_err(), "expected error, got ok");
        if let Err(err) = r {
            assert!(err.to_string().contains("too many decimal places"));
        }
    }

    #[test]
    fn format_base_to_ui() -> eyre::Result<()> {
        let s1 = format_amount_base_to_ui_string(1_500_000, 6)?;
        assert_eq!(s1, "1.5");
        let s2 = format_amount_base_to_ui_string(1, 6)?;
        assert_eq!(s2, "0.000001");
        let s3 = format_amount_base_to_ui_string(10_000_000, 6)?;
        assert_eq!(s3, "10");
        Ok(())
    }
}
