use alloy::primitives::Address;
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
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
pub struct ScamBlocklistPayload {
    #[serde(default)]
    pub evm: Vec<String>,
    #[serde(default)]
    pub solana: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScamBlocklistEnvelope {
    pub version: u32,
    /// Base64-encoded JSON payload bytes.
    pub payload_b64: String,
    /// Base64-encoded Ed25519 signature over the payload bytes.
    pub signature_b64: String,
    /// Base64-encoded Ed25519 verifying key (32 bytes).
    pub pubkey_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScamBlocklistCacheFile {
    pub fetched_at_ms: i64,
    pub envelope: ScamBlocklistEnvelope,
    pub payload: ScamBlocklistPayload,
}

#[derive(Debug, Clone)]
pub struct ScamBlocklist {
    pub fetched_at_ms: i64,
    evm: BTreeSet<Address>,
    solana: BTreeSet<Pubkey>,
}

impl ScamBlocklist {
    pub fn contains_evm(&self, a: Address) -> bool {
        self.evm.contains(&a)
    }

    pub fn contains_solana(&self, p: Pubkey) -> bool {
        self.solana.contains(&p)
    }
}

pub fn normalize_and_verify(
    fetched_at_ms: i64,
    envelope: ScamBlocklistEnvelope,
    expected_pubkey_b64: Option<&str>,
) -> eyre::Result<(ScamBlocklist, ScamBlocklistCacheFile)> {
    if envelope.version != 1 {
        eyre::bail!("unsupported blocklist version: {}", envelope.version);
    }

    let pubkey_b64 = expected_pubkey_b64.unwrap_or(envelope.pubkey_b64.as_str());
    let pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(pubkey_b64)
        .context("decode blocklist pubkey_b64")?;
    if pubkey_bytes.len() != 32 {
        eyre::bail!("blocklist pubkey_b64 must decode to 32 bytes");
    }
    let pubkey_arr: [u8; 32] = pubkey_bytes
        .as_slice()
        .try_into()
        .map_err(|e| eyre::eyre!("blocklist pubkey_b64 must decode to 32 bytes: {e}"))?;
    let vk = VerifyingKey::from_bytes(&pubkey_arr).context("parse ed25519 verifying key")?;

    let payload_bytes = base64::engine::general_purpose::STANDARD
        .decode(&envelope.payload_b64)
        .context("decode blocklist payload_b64")?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&envelope.signature_b64)
        .context("decode blocklist signature_b64")?;
    if sig_bytes.len() != 64 {
        eyre::bail!("blocklist signature_b64 must decode to 64 bytes");
    }
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|e| eyre::eyre!("blocklist signature_b64 must decode to 64 bytes: {e}"))?;
    let sig = Signature::from_bytes(&sig_arr);

    vk.verify(&payload_bytes, &sig)
        .context("verify blocklist signature")?;

    let payload: ScamBlocklistPayload =
        serde_json::from_slice(&payload_bytes).context("parse blocklist payload json")?;

    let mut evm = BTreeSet::new();
    for s in &payload.evm {
        let addr =
            parse_evm_address(s).with_context(|| format!("invalid blocklist evm address: {s}"))?;
        evm.insert(addr);
    }
    let mut solana = BTreeSet::new();
    for s in &payload.solana {
        let pk = parse_solana_pubkey(s)
            .with_context(|| format!("invalid blocklist solana pubkey: {s}"))?;
        solana.insert(pk);
    }

    let bl = ScamBlocklist {
        fetched_at_ms,
        evm,
        solana,
    };
    let cache = ScamBlocklistCacheFile {
        fetched_at_ms,
        envelope,
        payload,
    };
    Ok((bl, cache))
}

pub async fn fetch_envelope(url: &str) -> eyre::Result<ScamBlocklistEnvelope> {
    ensure_https_or_loopback(url, "scam_blocklist_url")?;
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .await
        .context("fetch blocklist")?;
    if !resp.status().is_success() {
        eyre::bail!("blocklist fetch failed: http {}", resp.status());
    }
    let env: ScamBlocklistEnvelope = resp.json().await.context("decode blocklist json")?;
    Ok(env)
}

pub fn parse_evm_address(s: &str) -> eyre::Result<Address> {
    let t = s.trim();
    let t = t.strip_prefix("0x").unwrap_or(t);
    if t.len() != 40 {
        eyre::bail!("expected 20-byte hex address");
    }
    let bytes = hex::decode(t).context("decode hex")?;
    Ok(Address::from_slice(&bytes))
}

pub fn parse_solana_pubkey(s: &str) -> eyre::Result<Pubkey> {
    let t = s.trim();
    Pubkey::from_str(t).context("parse pubkey")
}
