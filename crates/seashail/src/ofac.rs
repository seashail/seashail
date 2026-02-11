use alloy::primitives::Address;
use eyre::Context as _;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeSet;
use std::str::FromStr as _;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OfacSdnPayload {
    /// EVM 0x addresses.
    #[serde(default)]
    pub evm: Vec<String>,
    /// Solana base58 pubkeys.
    #[serde(default)]
    pub solana: Vec<String>,
    /// Bitcoin addresses (bech32/base58).
    #[serde(default)]
    pub bitcoin: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OfacSdnCacheFile {
    pub fetched_at_ms: i64,
    pub payload: OfacSdnPayload,
}

#[derive(Debug, Clone)]
pub struct OfacSdnList {
    pub fetched_at_ms: i64,
    evm: BTreeSet<Address>,
    solana: BTreeSet<Pubkey>,
    bitcoin: BTreeSet<String>,
}

impl OfacSdnList {
    pub fn contains_evm(&self, a: Address) -> bool {
        self.evm.contains(&a)
    }

    pub fn contains_solana(&self, p: Pubkey) -> bool {
        self.solana.contains(&p)
    }

    pub fn contains_bitcoin(&self, addr: &str) -> bool {
        parse_bitcoin_address_norm(addr)
            .ok()
            .is_some_and(|a| self.bitcoin.contains(&a))
    }
}

pub fn normalize(fetched_at_ms: i64, payload: &OfacSdnPayload) -> eyre::Result<OfacSdnList> {
    let mut evm = BTreeSet::new();
    for s in &payload.evm {
        let addr =
            parse_evm_address(s).with_context(|| format!("invalid ofac evm address: {s}"))?;
        evm.insert(addr);
    }
    let mut solana = BTreeSet::new();
    for s in &payload.solana {
        let pk =
            parse_solana_pubkey(s).with_context(|| format!("invalid ofac solana pubkey: {s}"))?;
        solana.insert(pk);
    }
    let mut bitcoin = BTreeSet::new();
    for s in &payload.bitcoin {
        let a = parse_bitcoin_address_norm(s)
            .with_context(|| format!("invalid ofac bitcoin address: {s}"))?;
        bitcoin.insert(a);
    }
    Ok(OfacSdnList {
        fetched_at_ms,
        evm,
        solana,
        bitcoin,
    })
}

pub async fn fetch_payload(url: &str) -> eyre::Result<OfacSdnPayload> {
    ensure_https_or_loopback(url, "ofac_sdn_url")?;
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .await
        .context("fetch ofac sdn list")?;
    if !resp.status().is_success() {
        eyre::bail!("ofac fetch failed: http {}", resp.status());
    }
    resp.json::<OfacSdnPayload>()
        .await
        .context("decode ofac json")
}

fn parse_evm_address(s: &str) -> eyre::Result<Address> {
    let t = s.trim();
    let t = t.strip_prefix("0x").unwrap_or(t);
    if t.len() != 40 {
        eyre::bail!("expected 20-byte hex address");
    }
    let bytes = hex::decode(t).context("decode hex")?;
    Ok(Address::from_slice(&bytes))
}

fn parse_solana_pubkey(s: &str) -> eyre::Result<Pubkey> {
    let t = s.trim();
    Pubkey::from_str(t).context("parse pubkey")
}

fn parse_bitcoin_address_norm(s: &str) -> eyre::Result<String> {
    let t = s.trim();
    let a = bitcoin::Address::<bitcoin::address::NetworkUnchecked>::from_str(t)
        .context("parse bitcoin address")?
        .assume_checked();
    let out = a.to_string();
    if out.to_ascii_lowercase().starts_with("bc1") || out.to_ascii_lowercase().starts_with("tb1") {
        Ok(out.to_ascii_lowercase())
    } else {
        Ok(out)
    }
}
