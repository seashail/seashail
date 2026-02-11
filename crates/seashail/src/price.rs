use crate::{
    chains::{evm::EvmChain, solana::SolanaChain},
    config::SeashailConfig,
};
use eyre::Context as _;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::warn;

fn usdc_base_str_to_usd_f64(base_amount: &str) -> eyre::Result<f64> {
    // Convert a USDC base-unit integer (6 decimals) string into a decimal string, then parse.
    // This avoids float arithmetic in policy/lint-restricted codepaths.
    let s = base_amount.trim();
    if s.is_empty() {
        eyre::bail!("empty usdc base amount");
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        eyre::bail!("invalid usdc base amount");
    }
    if s == "0" {
        return Ok(0.0_f64);
    }

    let dec = if s.len() <= 6 {
        let mut frac = String::with_capacity(6);
        for _ in 0..(6 - s.len()) {
            frac.push('0');
        }
        frac.push_str(s);
        format!("0.{frac}")
    } else {
        let split = s.len() - 6;
        let (whole, frac) = s.split_at(split);
        let frac_trimmed = frac.trim_end_matches('0');
        if frac_trimmed.is_empty() {
            whole.to_owned()
        } else {
            format!("{whole}.{frac_trimmed}")
        }
    };

    dec.parse::<f64>().context("parse usdc amount")
}

fn allow_insecure_http() -> bool {
    std::env::var("SEASHAIL_ALLOW_INSECURE_HTTP")
        .ok()
        .is_some_and(|v| {
            matches!(
                v.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
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

#[derive(Debug, Clone)]
pub enum PriceSource {
    Binance,
    Jupiter,
    Uniswap,
}

impl PriceSource {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Binance => "Binance",
            Self::Jupiter => "Jupiter",
            Self::Uniswap => "Uniswap",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim() {
            "Binance" | "binance" => Some(Self::Binance),
            "Jupiter" | "jupiter" => Some(Self::Jupiter),
            "Uniswap" | "uniswap" => Some(Self::Uniswap),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TokenPriceUsd {
    pub usd: f64,
    pub source: PriceSource,
}

#[derive(Debug, Deserialize)]
struct BinanceTickerPrice {
    price: String,
}

pub async fn binance_price_usd(cfg: &SeashailConfig, symbol: &str) -> eyre::Result<f64> {
    if symbol.eq_ignore_ascii_case("USD")
        || symbol.eq_ignore_ascii_case("USDT")
        || symbol.eq_ignore_ascii_case("USDC")
    {
        return Ok(1.0_f64);
    }

    let base = cfg.http.binance_base_url.trim();
    if !base.starts_with("https://") && !is_loopback_http(base) && !allow_insecure_http() {
        eyre::bail!(
            "binance_base_url must use https (or loopback); set SEASHAIL_ALLOW_INSECURE_HTTP=1 to override"
        );
    }

    let pair = format!("{}USDT", symbol.to_uppercase());
    let url = format!(
        "{}/api/v3/ticker/price?symbol={}",
        cfg.http.binance_base_url, pair
    );
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build http client")?;
    let v: BinanceTickerPrice = client
        .get(url)
        .send()
        .await
        .context("binance request")?
        .error_for_status()
        .context("binance status")?
        .json()
        .await
        .context("binance json")?;
    let p: f64 = v.price.parse().context("parse binance price")?;
    Ok(p)
}

async fn binance_price_usd_any(cfg: &SeashailConfig, symbols: &[&str]) -> eyre::Result<f64> {
    let mut last_err: Option<eyre::Report> = None;
    for s in symbols {
        match binance_price_usd(cfg, s).await {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| eyre::eyre!("no symbols provided")))
}

pub async fn native_token_price_usd(
    chain: &str,
    cfg: &SeashailConfig,
) -> eyre::Result<TokenPriceUsd> {
    let symbol = match chain {
        "solana" => "SOL",
        "bitcoin" => "BTC",
        "ethereum" | "base" | "arbitrum" | "optimism" | "sepolia" | "base-sepolia"
        | "arbitrum-sepolia" | "optimism-sepolia" => "ETH",
        // Polygon migrated to POL, but MATIC pricing is still commonly available; try both.
        "polygon" | "polygon-amoy" => {
            let usd = binance_price_usd_any(cfg, &["POL", "MATIC"]).await?;
            return Ok(TokenPriceUsd {
                usd,
                source: PriceSource::Binance,
            });
        }
        "bnb" | "bnb-testnet" => "BNB",
        "avalanche" | "avalanche-fuji" => "AVAX",
        "monad" | "monad-testnet" => "MON",
        other => eyre::bail!("unknown native token for chain: {other}"),
    };
    let usd = binance_price_usd(cfg, symbol).await?;
    Ok(TokenPriceUsd {
        usd,
        source: PriceSource::Binance,
    })
}

pub async fn native_token_price_usd_cached(
    chain: &str,
    cfg: &SeashailConfig,
    db: Option<&crate::db::Db>,
) -> eyre::Result<TokenPriceUsd> {
    let Some(db) = db else {
        return native_token_price_usd(chain, cfg).await;
    };
    let now = crate::db::Db::now_ms()?;
    let key = format!("price:native:{chain}");

    match db.get_price_if_fresh(&key, now).await {
        Ok(Some(row)) => {
            if let Some(src) = PriceSource::from_str(&row.source) {
                return Ok(TokenPriceUsd {
                    usd: row.usd,
                    source: src,
                });
            }
        }
        Ok(None) => {}
        Err(e) => warn!(error = %e, "price cache read failed; falling back to live fetch"),
    }

    let p = native_token_price_usd(chain, cfg).await?;

    let ttl_ms =
        i64::try_from(cfg.price_cache_ttl_seconds_native.saturating_mul(1000)).unwrap_or(i64::MAX);
    let stale_at = now.saturating_add(ttl_ms.max(0));
    if let Err(e) = db
        .upsert_price(&key, p.usd, p.source.as_str(), now, stale_at)
        .await
    {
        warn!(error = %e, "price cache write failed");
    }

    Ok(p)
}

pub async fn solana_token_price_usd(
    sol: &SolanaChain,
    mint: &str,
    usdc_mint: &str,
    amount_in_base: u64,
    slippage_bps: u32,
) -> eyre::Result<TokenPriceUsd> {
    let quote = sol
        .jupiter_quote(mint, usdc_mint, amount_in_base, slippage_bps)
        .await?;
    let out_amount = quote
        .get("outAmount")
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre::eyre!("missing outAmount"))?
        .trim();

    let usd = usdc_base_str_to_usd_f64(out_amount)?;
    Ok(TokenPriceUsd {
        usd,
        source: PriceSource::Jupiter,
    })
}

pub async fn solana_token_price_usd_cached(
    sol: &SolanaChain,
    cfg: &SeashailConfig,
    mint: &str,
    usdc_mint: &str,
    amount_in_base: u64,
    slippage_bps: u32,
    db: Option<&crate::db::Db>,
) -> eyre::Result<TokenPriceUsd> {
    let Some(db) = db else {
        return solana_token_price_usd(sol, mint, usdc_mint, amount_in_base, slippage_bps).await;
    };
    let now = crate::db::Db::now_ms()?;
    let key = format!("price:solana:{mint}:{usdc_mint}:{amount_in_base}:{slippage_bps}");

    match db.get_price_if_fresh(&key, now).await {
        Ok(Some(row)) => {
            if let Some(src) = PriceSource::from_str(&row.source) {
                return Ok(TokenPriceUsd {
                    usd: row.usd,
                    source: src,
                });
            }
        }
        Ok(None) => {}
        Err(e) => warn!(error = %e, "price cache read failed; falling back to live fetch"),
    }

    let p = solana_token_price_usd(sol, mint, usdc_mint, amount_in_base, slippage_bps).await?;

    let ttl_ms =
        i64::try_from(cfg.price_cache_ttl_seconds_quote.saturating_mul(1000)).unwrap_or(i64::MAX);
    let stale_at = now.saturating_add(ttl_ms.max(0));
    if let Err(e) = db
        .upsert_price(&key, p.usd, p.source.as_str(), now, stale_at)
        .await
    {
        warn!(error = %e, "price cache write failed");
    }

    Ok(p)
}

pub async fn evm_token_price_usd(
    evm: &EvmChain,
    token: alloy::primitives::Address,
    amount_in_base: alloy::primitives::U256,
    _slippage_bps: u32,
) -> eyre::Result<TokenPriceUsd> {
    let Some(u) = &evm.uniswap else {
        eyre::bail!("uniswap not configured for chain {}", evm.name);
    };

    // Best-effort across common fee tiers. We ignore slippage here; this is price discovery.
    let fees = [500_u32, 3000_u32, 10_000_u32];
    let mut best: Option<alloy::primitives::U256> = None;
    for fee in fees {
        if let Ok(out) = evm
            .quote_uniswap_exact_in(token, u.usdc, amount_in_base, fee)
            .await
        {
            if best.map_or(true, |b| out > b) {
                best = Some(out);
            }
        }
    }

    let out = best.ok_or_else(|| eyre::eyre!("no uniswap quote available"))?;
    let usd = usdc_base_str_to_usd_f64(&out.to_string())?;

    Ok(TokenPriceUsd {
        usd,
        source: PriceSource::Uniswap,
    })
}

pub async fn evm_token_price_usd_cached(
    evm: &EvmChain,
    cfg: &SeashailConfig,
    token: alloy::primitives::Address,
    amount_in_base: alloy::primitives::U256,
    slippage_bps: u32,
    db: Option<&crate::db::Db>,
) -> eyre::Result<TokenPriceUsd> {
    let Some(db) = db else {
        return evm_token_price_usd(evm, token, amount_in_base, slippage_bps).await;
    };
    let now = crate::db::Db::now_ms()?;
    let key = format!(
        "price:evm:{}:{}:{}:{}",
        evm.name, token, amount_in_base, slippage_bps
    );

    match db.get_price_if_fresh(&key, now).await {
        Ok(Some(row)) => {
            if let Some(src) = PriceSource::from_str(&row.source) {
                return Ok(TokenPriceUsd {
                    usd: row.usd,
                    source: src,
                });
            }
        }
        Ok(None) => {}
        Err(e) => warn!(error = %e, "price cache read failed; falling back to live fetch"),
    }

    let p = evm_token_price_usd(evm, token, amount_in_base, slippage_bps).await?;

    let ttl_ms =
        i64::try_from(cfg.price_cache_ttl_seconds_quote.saturating_mul(1000)).unwrap_or(i64::MAX);
    let stale_at = now.saturating_add(ttl_ms.max(0));
    if let Err(e) = db
        .upsert_price(&key, p.usd, p.source.as_str(), now, stale_at)
        .await
    {
        warn!(error = %e, "price cache write failed");
    }

    Ok(p)
}
