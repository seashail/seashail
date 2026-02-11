use eyre::Context as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::config::HttpConfig;

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TxEnvelopeRequest<'a> {
    pub marketplace: &'a str,
    pub op: &'a str,
    pub chain: &'a str,
    pub wallet_address: &'a str,
    pub asset: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvmTxEnvelopeResponse {
    pub to: String,
    pub data: String,
    #[serde(default)]
    pub value_wei: String,
    #[serde(default)]
    pub usd_value: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SolanaTxEnvelopeResponse {
    pub tx_b64: String,
    pub allowed_program_ids: Vec<String>,
    #[serde(default)]
    pub usd_value: Option<f64>,
}

fn base_url_for_marketplace<'a>(cfg: &'a HttpConfig, marketplace: &str) -> Option<&'a str> {
    match marketplace {
        "blur" => cfg.blur_adapter_base_url.as_deref(),
        "magic_eden" => cfg.magic_eden_adapter_base_url.as_deref(),
        "opensea" => cfg.opensea_adapter_base_url.as_deref(),
        "tensor" => cfg.tensor_adapter_base_url.as_deref(),
        "pumpfun" => cfg.pumpfun_adapter_base_url.as_deref(),
        "wormhole" | "layerzero" | "aave" | "compound" | "kamino" | "marginfi" | "lido"
        | "eigenlayer" | "marinade" | "jito" | "uniswap_lp" | "orca_lp" | "polymarket" => {
            cfg.defi_adapter_base_url.as_deref()
        }
        _ => None,
    }
}

pub async fn fetch_evm_tx_envelope(
    cfg: &HttpConfig,
    marketplace: &str,
    op: &str,
    chain: &str,
    from_addr: &str,
    asset: Value,
) -> eyre::Result<EvmTxEnvelopeResponse> {
    let base_url = base_url_for_marketplace(cfg, marketplace)
        .ok_or_else(|| eyre::eyre!("marketplace adapter not configured for {marketplace}"))?;
    ensure_https_or_loopback(base_url, "marketplace adapter base url")?;

    // OpenSea frequently requires an API key (or rate-limits heavily). We treat the key
    // as required whenever the OpenSea adapter is used, so failures are explicit.
    if marketplace == "opensea" {
        let ok = cfg
            .opensea_api_key
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        if !ok {
            eyre::bail!("missing_api_key: opensea_api_key is not configured");
        }
    }

    let url = format!("{}/tx-envelope", base_url.trim_end_matches('/'));
    let req = TxEnvelopeRequest {
        marketplace,
        op,
        chain,
        wallet_address: from_addr,
        asset,
    };

    let mut rb = reqwest::Client::new()
        .post(url)
        .timeout(Duration::from_millis(1600))
        .json(&req);

    if marketplace == "opensea" {
        if let Some(k) = cfg.opensea_api_key.as_deref() {
            rb = rb.header("x-api-key", k);
        }
    }

    let resp = rb.send().await.context("fetch evm tx envelope")?;
    if !resp.status().is_success() {
        eyre::bail!("marketplace adapter http {}", resp.status());
    }
    resp.json::<EvmTxEnvelopeResponse>()
        .await
        .context("decode evm tx envelope json")
}

pub async fn fetch_solana_tx_envelope(
    cfg: &HttpConfig,
    marketplace: &str,
    op: &str,
    from_pubkey: &str,
    asset: Value,
) -> eyre::Result<SolanaTxEnvelopeResponse> {
    let base_url = base_url_for_marketplace(cfg, marketplace)
        .ok_or_else(|| eyre::eyre!("marketplace adapter not configured for {marketplace}"))?;
    ensure_https_or_loopback(base_url, "marketplace adapter base url")?;

    let url = format!("{}/tx-envelope", base_url.trim_end_matches('/'));
    let req = TxEnvelopeRequest {
        marketplace,
        op,
        chain: "solana",
        wallet_address: from_pubkey,
        asset,
    };

    let resp = reqwest::Client::new()
        .post(url)
        .timeout(Duration::from_millis(1600))
        .json(&req)
        .send()
        .await
        .context("fetch solana tx envelope")?;
    if !resp.status().is_success() {
        eyre::bail!("marketplace adapter http {}", resp.status());
    }
    resp.json::<SolanaTxEnvelopeResponse>()
        .await
        .context("decode solana tx envelope json")
}
