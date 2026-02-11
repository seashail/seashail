// Port of the `shamir-secret-sharing` TypeScript implementation (0.0.4), kept in parity with
// a known reference. We treat this as a verified algorithm implementation (not an audited Rust crate).
// The field polynomial is x^8 + x^4 + x^3 + x + 1.
//
// Shares are byte arrays of length secret_len + 1, where the last byte is the x coordinate (1..=255).

const LOG_TABLE: [u8; 256] = [
    0x00, 0xff, 0xc8, 0x08, 0x91, 0x10, 0xd0, 0x36, 0x5a, 0x3e, 0xd8, 0x43, 0x99, 0x77, 0xfe, 0x18,
    0x23, 0x20, 0x07, 0x70, 0xa1, 0x6c, 0x0c, 0x7f, 0x62, 0x8b, 0x40, 0x46, 0xc7, 0x4b, 0xe0, 0x0e,
    0xeb, 0x16, 0xe8, 0xad, 0xcf, 0xcd, 0x39, 0x53, 0x6a, 0x27, 0x35, 0x93, 0xd4, 0x4e, 0x48, 0xc3,
    0x2b, 0x79, 0x54, 0x28, 0x09, 0x78, 0x0f, 0x21, 0x90, 0x87, 0x14, 0x2a, 0xa9, 0x9c, 0xd6, 0x74,
    0xb4, 0x7c, 0xde, 0xed, 0xb1, 0x86, 0x76, 0xa4, 0x98, 0xe2, 0x96, 0x8f, 0x02, 0x32, 0x1c, 0xc1,
    0x33, 0xee, 0xef, 0x81, 0xfd, 0x30, 0x5c, 0x13, 0x9d, 0x29, 0x17, 0xc4, 0x11, 0x44, 0x8c, 0x80,
    0xf3, 0x73, 0x42, 0x1e, 0x1d, 0xb5, 0xf0, 0x12, 0xd1, 0x5b, 0x41, 0xa2, 0xd7, 0x2c, 0xe9, 0xd5,
    0x59, 0xcb, 0x50, 0xa8, 0xdc, 0xfc, 0xf2, 0x56, 0x72, 0xa6, 0x65, 0x2f, 0x9f, 0x9b, 0x3d, 0xba,
    0x7d, 0xc2, 0x45, 0x82, 0xa7, 0x57, 0xb6, 0xa3, 0x7a, 0x75, 0x4f, 0xae, 0x3f, 0x37, 0x6d, 0x47,
    0x61, 0xbe, 0xab, 0xd3, 0x5f, 0xb0, 0x58, 0xaf, 0xca, 0x5e, 0xfa, 0x85, 0xe4, 0x4d, 0x8a, 0x05,
    0xfb, 0x60, 0xb7, 0x7b, 0xb8, 0x26, 0x4a, 0x67, 0xc6, 0x1a, 0xf8, 0x69, 0x25, 0xb3, 0xdb, 0xbd,
    0x66, 0xdd, 0xf1, 0xd2, 0xdf, 0x03, 0x8d, 0x34, 0xd9, 0x92, 0x0d, 0x63, 0x55, 0xaa, 0x49, 0xec,
    0xbc, 0x95, 0x3c, 0x84, 0x0b, 0xf5, 0xe6, 0xe7, 0xe5, 0xac, 0x7e, 0x6e, 0xb9, 0xf9, 0xda, 0x8e,
    0x9a, 0xc9, 0x24, 0xe1, 0x0a, 0x15, 0x6b, 0x3a, 0xa0, 0x51, 0xf4, 0xea, 0xb2, 0x97, 0x9e, 0x5d,
    0x22, 0x88, 0x94, 0xce, 0x19, 0x01, 0x71, 0x4c, 0xa5, 0xe3, 0xc5, 0x31, 0xbb, 0xcc, 0x1f, 0x2d,
    0x3b, 0x52, 0x6f, 0xf6, 0x2e, 0x89, 0xf7, 0xc0, 0x68, 0x1b, 0x64, 0x04, 0x06, 0xbf, 0x83, 0x38,
];

const EXP_TABLE: [u8; 256] = [
    0x01, 0xe5, 0x4c, 0xb5, 0xfb, 0x9f, 0xfc, 0x12, 0x03, 0x34, 0xd4, 0xc4, 0x16, 0xba, 0x1f, 0x36,
    0x05, 0x5c, 0x67, 0x57, 0x3a, 0xd5, 0x21, 0x5a, 0x0f, 0xe4, 0xa9, 0xf9, 0x4e, 0x64, 0x63, 0xee,
    0x11, 0x37, 0xe0, 0x10, 0xd2, 0xac, 0xa5, 0x29, 0x33, 0x59, 0x3b, 0x30, 0x6d, 0xef, 0xf4, 0x7b,
    0x55, 0xeb, 0x4d, 0x50, 0xb7, 0x2a, 0x07, 0x8d, 0xff, 0x26, 0xd7, 0xf0, 0xc2, 0x7e, 0x09, 0x8c,
    0x1a, 0x6a, 0x62, 0x0b, 0x5d, 0x82, 0x1b, 0x8f, 0x2e, 0xbe, 0xa6, 0x1d, 0xe7, 0x9d, 0x2d, 0x8a,
    0x72, 0xd9, 0xf1, 0x27, 0x32, 0xbc, 0x77, 0x85, 0x96, 0x70, 0x08, 0x69, 0x56, 0xdf, 0x99, 0x94,
    0xa1, 0x90, 0x18, 0xbb, 0xfa, 0x7a, 0xb0, 0xa7, 0xf8, 0xab, 0x28, 0xd6, 0x15, 0x8e, 0xcb, 0xf2,
    0x13, 0xe6, 0x78, 0x61, 0x3f, 0x89, 0x46, 0x0d, 0x35, 0x31, 0x88, 0xa3, 0x41, 0x80, 0xca, 0x17,
    0x5f, 0x53, 0x83, 0xfe, 0xc3, 0x9b, 0x45, 0x39, 0xe1, 0xf5, 0x9e, 0x19, 0x5e, 0xb6, 0xcf, 0x4b,
    0x38, 0x04, 0xb9, 0x2b, 0xe2, 0xc1, 0x4a, 0xdd, 0x48, 0x0c, 0xd0, 0x7d, 0x3d, 0x58, 0xde, 0x7c,
    0xd8, 0x14, 0x6b, 0x87, 0x47, 0xe8, 0x79, 0x84, 0x73, 0x3c, 0xbd, 0x92, 0xc9, 0x23, 0x8b, 0x97,
    0x95, 0x44, 0xdc, 0xad, 0x40, 0x65, 0x86, 0xa2, 0xa4, 0xcc, 0x7f, 0xec, 0xc0, 0xaf, 0x91, 0xfd,
    0xf7, 0x4f, 0x81, 0x2f, 0x5b, 0xea, 0xa8, 0x1c, 0x02, 0xd1, 0x98, 0x71, 0xed, 0x25, 0xe3, 0x24,
    0x06, 0x68, 0xb3, 0x93, 0x2c, 0x6f, 0x3e, 0x6c, 0x0a, 0xb8, 0xce, 0xae, 0x74, 0xb1, 0x42, 0xb4,
    0x1e, 0xd3, 0x49, 0xe9, 0x9c, 0xc8, 0xc6, 0xc7, 0x22, 0x6e, 0xdb, 0x20, 0xbf, 0x43, 0x51, 0x52,
    0x66, 0xb2, 0x76, 0x60, 0xda, 0xc5, 0xf3, 0xf6, 0xaa, 0xcd, 0x9a, 0xa0, 0x75, 0x54, 0x0e, 0x01,
];

const fn add(a: u8, b: u8) -> u8 {
    a ^ b
}

fn table_u8(table: &[u8; 256], idx: u8) -> u8 {
    // `idx` is always within bounds for `u8`; use `get` to avoid `indexing_slicing`.
    table.get(usize::from(idx)).copied().unwrap_or(0)
}

fn table_usize(table: &[u8; 256], idx: usize) -> u8 {
    table.get(idx).copied().unwrap_or(0)
}

fn div(a: u8, b: u8) -> eyre::Result<u8> {
    if b == 0 {
        eyre::bail!("cannot divide by zero");
    }
    if a == 0 {
        return Ok(0);
    }
    let log_a = i16::from(table_u8(&LOG_TABLE, a));
    let log_b = i16::from(table_u8(&LOG_TABLE, b));
    let diff_i16 = (log_a - log_b + 255) % 255;
    let diff_usize = usize::try_from(diff_i16).map_err(|e| eyre::eyre!("{e}"))?;
    Ok(table_usize(&EXP_TABLE, diff_usize))
}

fn mult(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let log_a = u16::from(table_u8(&LOG_TABLE, a));
    let log_b = u16::from(table_u8(&LOG_TABLE, b));
    let sum = (log_a + log_b) % 255;
    table_usize(&EXP_TABLE, usize::from(sum))
}

fn interpolate_polynomial(x_samples: &[u8], y_samples: &[u8], x: u8) -> eyre::Result<u8> {
    if x_samples.len() != y_samples.len() {
        eyre::bail!("sample length mismatch");
    }

    let mut result = 0_u8;

    for (i, &xi) in x_samples.iter().enumerate() {
        let mut basis = 1_u8;
        for (j, &xj) in x_samples.iter().enumerate() {
            if i == j {
                continue;
            }
            let num = add(x, xj);
            let denom = add(xi, xj);
            let term = div(num, denom)?;
            basis = mult(basis, term);
        }
        let Some(&yi) = y_samples.get(i) else {
            eyre::bail!("sample length mismatch");
        };
        result = add(result, mult(yi, basis));
    }

    Ok(result)
}

fn evaluate(coefficients: &[u8], x: u8, degree: usize) -> eyre::Result<u8> {
    if x == 0 {
        eyre::bail!("cannot evaluate secret polynomial at zero");
    }
    if coefficients.len() <= degree {
        eyre::bail!("coefficients length too short for degree");
    }

    let mut it = coefficients.iter().take(degree + 1).rev();
    let Some(&first) = it.next() else {
        eyre::bail!("missing coefficients");
    };
    let mut result = first;
    for &c in it {
        result = add(mult(result, x), c);
    }
    Ok(result)
}

fn new_coordinates() -> Vec<u8> {
    let mut coords: Vec<u8> = (1_u8..=255_u8).collect();

    let mut random_indices = vec![0_u8; 255];
    crate::keystore::crypto::fill_random(&mut random_indices);

    for (i, b) in random_indices.iter().enumerate().take(255) {
        let j = usize::from(*b % 255);
        coords.swap(i, j);
    }

    coords
}

pub fn split(secret: &[u8], shares: usize, threshold: usize) -> eyre::Result<Vec<Vec<u8>>> {
    if secret.is_empty() {
        eyre::bail!("secret cannot be empty");
    }
    if !(2..256).contains(&shares) {
        eyre::bail!("shares must be between 2 and 255");
    }
    if !(2..256).contains(&threshold) {
        eyre::bail!("threshold must be between 2 and 255");
    }
    if shares < threshold {
        eyre::bail!("shares cannot be less than threshold");
    }

    let secret_len = secret.len();
    let x_coords = new_coordinates();
    let xs: Vec<u8> = x_coords.into_iter().take(shares).collect();

    let mut result = Vec::with_capacity(shares);
    for &x in &xs {
        let mut share = vec![0_u8; secret_len + 1];
        let Some(last) = share.last_mut() else {
            eyre::bail!("share must be at least 1 byte");
        };
        *last = x;
        result.push(share);
    }

    let degree = threshold - 1;
    let mut coeffs = vec![0_u8; degree + 1];
    let mut random_coeffs = vec![0_u8; degree];

    for (i, &b) in secret.iter().enumerate() {
        let Some(first) = coeffs.first_mut() else {
            eyre::bail!("coefficients must be non-empty");
        };
        *first = b;
        crate::keystore::crypto::fill_random(&mut random_coeffs);
        for (dst, src) in coeffs.iter_mut().skip(1).zip(random_coeffs.iter()) {
            *dst = *src;
        }

        for (share, &x) in result.iter_mut().zip(xs.iter()) {
            let y = evaluate(&coeffs, x, degree)?;
            let Some(cell) = share.get_mut(i) else {
                eyre::bail!("share length mismatch");
            };
            *cell = y;
        }
    }

    Ok(result)
}

pub fn combine(shares: &[Vec<u8>], threshold: usize) -> eyre::Result<Vec<u8>> {
    if shares.len() < threshold {
        eyre::bail!("not enough shares");
    }
    if !(2..256).contains(&shares.len()) {
        eyre::bail!("shares must have at least 2 and at most 255 elements");
    }
    let Some(first) = shares.first() else {
        eyre::bail!("not enough shares");
    };
    let share_len = first.len();
    if share_len < 2 {
        eyre::bail!("each share must be at least 2 bytes");
    }
    for s in shares {
        if s.len() != share_len {
            eyre::bail!("all shares must have the same byte length");
        }
    }

    let secret_len = share_len - 1;
    let mut secret = vec![0_u8; secret_len];

    let mut seen = std::collections::HashSet::new();
    let mut x_samples = Vec::with_capacity(shares.len());
    for share in shares {
        let Some(&sample) = share.last() else {
            eyre::bail!("each share must be at least 2 bytes");
        };
        if !seen.insert(sample) {
            eyre::bail!("duplicate share x coordinate");
        }
        x_samples.push(sample);
    }

    let mut y_samples = vec![0_u8; shares.len()];
    for (i, out) in secret.iter_mut().enumerate() {
        for (y, share) in y_samples.iter_mut().zip(shares.iter()) {
            let Some(&b) = share.get(i) else {
                eyre::bail!("share length mismatch");
            };
            *y = b;
        }
        *out = interpolate_polynomial(&x_samples, &y_samples, 0)?;
    }

    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use eyre::{Context as _, ContextCompat as _};

    #[test]
    fn split_combine_any_2_of_3_recovers_secret() -> eyre::Result<()> {
        let secret = b"seashail-test-secret-32-bytes-1234".to_vec();
        let shares = split(&secret, 3, 2).context("split")?;
        assert_eq!(shares.len(), 3);

        let s0 = shares.first().context("shares[0]")?.clone();
        let s1 = shares.get(1).context("shares[1]")?.clone();
        let s2 = shares.get(2).context("shares[2]")?.clone();

        let s01 = combine(&[s0.clone(), s1.clone()], 2).context("combine 0+1")?;
        assert_eq!(s01, secret);
        let s02 = combine(&[s0, s2.clone()], 2).context("combine 0+2")?;
        assert_eq!(s02, secret);
        let s12 = combine(&[s1, s2], 2).context("combine 1+2")?;
        assert_eq!(s12, secret);
        Ok(())
    }

    #[test]
    fn combine_requires_threshold_shares() -> eyre::Result<()> {
        let secret = b"hello".to_vec();
        let shares = split(&secret, 3, 2).context("split")?;
        let s0 = shares.first().context("shares[0]")?.clone();
        let err = combine(&[s0], 2)
            .err()
            .context("expected combine to fail")?;
        assert!(err.to_string().contains("not enough shares"));
        Ok(())
    }

    #[test]
    fn combine_rejects_duplicate_x_coordinates() -> eyre::Result<()> {
        let secret = b"hello".to_vec();
        let shares = split(&secret, 3, 2).context("split")?;
        let mut s0 = shares.first().context("shares[0]")?.clone();
        let s1 = shares.get(1).context("shares[1]")?.clone();
        // Force duplicate x coordinate.
        let Some(last) = s1.last().copied() else {
            eyre::bail!("share must have at least 1 byte");
        };
        let Some(dst_last) = s0.last_mut() else {
            eyre::bail!("share must have at least 1 byte");
        };
        *dst_last = last;
        let err = combine(&[s0, s1], 2)
            .err()
            .context("expected combine to fail")?;
        assert!(err.to_string().contains("duplicate share x coordinate"));
        Ok(())
    }

    #[test]
    fn combine_matches_privy_shamir_secret_sharing_vectors() -> eyre::Result<()> {
        // Vectors generated with `shamir-secret-sharing` (Privy audited implementation) 0.0.4:
        // secret = bytes 0..=31, shares = 3, threshold = 2
        let secret = (0_u8..32_u8).collect::<Vec<_>>();
        let s1 = base64::engine::general_purpose::STANDARD
            .decode("HASw3IFdRU8lfXvophswvJKkYlL6KkZ/64xgb8ZcwB2C")
            .context("decode s1")?;
        let s3 = base64::engine::general_purpose::STANDARD
            .decode("iIBFBtB33hrtQsBYnpz7lOZ0BGh8PN7UJKIVUJhmx7z2")
            .context("decode s3")?;
        let got = combine(&[s1, s3], 2).context("combine")?;
        assert_eq!(got, secret);
        Ok(())
    }
}
