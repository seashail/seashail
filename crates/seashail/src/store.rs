use crate::{
    config::{NetworkMode, SeashailConfig, SOLANA_DEVNET_RPC_URL, SOLANA_MAINNET_RPC_URL},
    paths::SeashailPaths,
};
use eyre::Context as _;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

fn parse_network_mode_env(s: &str) -> Option<NetworkMode> {
    let v = s.trim().to_lowercase();
    match v.as_str() {
        "mainnet" | "main" | "prod" | "production" => Some(NetworkMode::Mainnet),
        "testnet" | "test" | "dev" | "devnet" => Some(NetworkMode::Testnet),
        _ => None,
    }
}

fn is_truthy_env(v: &str) -> bool {
    matches!(v, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
}

/// Apply environment variable overrides to the config (HTTP endpoints, etc.).
fn apply_env_overrides(cfg: &mut SeashailConfig) {
    /// Helper: if an env var is set and non-empty, apply `setter` with the trimmed value.
    fn apply_env(var: &str, setter: impl FnOnce(&str)) {
        if let Ok(u) = std::env::var(var) {
            let t = u.trim();
            if !t.is_empty() {
                setter(t);
            }
        }
    }

    apply_env("SEASHAIL_PUMPFUN_ADAPTER_BASE_URL", |v| {
        cfg.http.pumpfun_adapter_base_url = Some(v.to_owned());
    });
    apply_env("SEASHAIL_KAMINO_API_BASE_URL", |v| {
        v.clone_into(&mut cfg.http.kamino_api_base_url);
    });
    apply_env("SEASHAIL_KAMINO_DEFAULT_LEND_MARKET", |v| {
        v.clone_into(&mut cfg.http.kamino_default_lend_market);
    });
    apply_env("SEASHAIL_MARGINFI_DEFAULT_GROUP", |v| {
        v.clone_into(&mut cfg.http.marginfi_default_group);
    });
    apply_env("SEASHAIL_OFAC_SDN_URL", |v| {
        cfg.http.ofac_sdn_url = Some(v.to_owned());
    });
    apply_env("SEASHAIL_DEFI_ADAPTER_BASE_URL", |v| {
        cfg.http.defi_adapter_base_url = Some(v.to_owned());
    });
    apply_env("SEASHAIL_BITCOIN_API_BASE_URL_MAINNET", |v| {
        v.clone_into(&mut cfg.http.bitcoin_api_base_url_mainnet);
    });
    apply_env("SEASHAIL_BITCOIN_API_BASE_URL_TESTNET", |v| {
        v.clone_into(&mut cfg.http.bitcoin_api_base_url_testnet);
    });
    apply_env("SEASHAIL_POLYMARKET_CLOB_BASE_URL", |v| {
        v.clone_into(&mut cfg.http.polymarket_clob_base_url);
    });
    apply_env("SEASHAIL_POLYMARKET_DATA_BASE_URL", |v| {
        v.clone_into(&mut cfg.http.polymarket_data_base_url);
    });
    apply_env("SEASHAIL_POLYMARKET_GAMMA_BASE_URL", |v| {
        v.clone_into(&mut cfg.http.polymarket_gamma_base_url);
    });
    apply_env("SEASHAIL_POLYMARKET_GEOBLOCK_BASE_URL", |v| {
        v.clone_into(&mut cfg.http.polymarket_geoblock_base_url);
    });
    if let Ok(v) = std::env::var("SEASHAIL_OFAC_SDN_REFRESH_SECONDS") {
        if let Ok(n) = v.trim().parse::<u64>() {
            if n > 0 {
                cfg.http.ofac_sdn_refresh_seconds = n;
            }
        }
    }
}

/// Apply zero-config network-mode env vars on first run.
fn apply_first_run_network_mode(cfg: &mut SeashailConfig) {
    let env_mode = std::env::var("SEASHAIL_NETWORK_MODE")
        .ok()
        .and_then(|v| parse_network_mode_env(&v))
        .or_else(|| {
            std::env::var("SEASHAIL_NETWORK")
                .ok()
                .and_then(|v| parse_network_mode_env(&v))
        });

    if let Some(m) = env_mode {
        cfg.network_mode = Some(m);
        cfg.testnet_mode = m == NetworkMode::Testnet;
        cfg.rpc.solana_rpc_url = match m {
            NetworkMode::Mainnet => SOLANA_MAINNET_RPC_URL.into(),
            NetworkMode::Testnet => SOLANA_DEVNET_RPC_URL.into(),
        };
    } else if std::env::var("SEASHAIL_TESTNET_MODE")
        .ok()
        .is_some_and(|v| is_truthy_env(&v))
    {
        cfg.network_mode = Some(NetworkMode::Testnet);
        cfg.testnet_mode = true;
        cfg.rpc.solana_rpc_url = SOLANA_DEVNET_RPC_URL.into();
    }
}

impl ConfigStore {
    pub fn new(paths: &SeashailPaths) -> Self {
        Self {
            path: paths.config_dir.join("config.toml"),
        }
    }

    pub fn load_or_init_default(&self) -> eyre::Result<SeashailConfig> {
        if !self.path.exists() {
            let mut cfg = SeashailConfig::default();
            apply_first_run_network_mode(&mut cfg);
            apply_env_overrides(&mut cfg);
            self.save(&cfg)?;
            return Ok(cfg);
        }

        let s = fs::read_to_string(&self.path).context("read config.toml")?;
        let mut cfg: SeashailConfig = toml::from_str(&s).context("parse config.toml")?;
        apply_env_overrides(&mut cfg);
        Ok(cfg)
    }

    pub fn save(&self, cfg: &SeashailConfig) -> eyre::Result<()> {
        if let Some(parent) = self.path.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }
        let s = toml::to_string_pretty(cfg).context("serialize config.toml")?;
        crate::fsutil::write_string_atomic_restrictive(
            &self.path,
            &s,
            crate::fsutil::MODE_FILE_PRIVATE,
        )
        .context("write config.toml")?;
        Ok(())
    }
}
