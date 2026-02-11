use crate::{
    blocklist,
    config::{NetworkMode, SeashailConfig},
    db::Db,
    keystore::Keystore,
    ofac,
};
use std::time::Duration;
use tracing::warn;
use zeroize::{Zeroize as _, Zeroizing};

pub const fn network_mode_str(m: NetworkMode) -> &'static str {
    match m {
        NetworkMode::Mainnet => "mainnet",
        NetworkMode::Testnet => "testnet",
    }
}

pub fn effective_network_mode(shared: &SharedState, conn: &ConnState) -> NetworkMode {
    conn.network_override
        .unwrap_or_else(|| shared.cfg.effective_network_mode())
}

pub fn parse_network_mode(s: &str) -> Option<NetworkMode> {
    match s.trim().to_lowercase().as_str() {
        "mainnet" | "main" | "prod" | "production" => Some(NetworkMode::Mainnet),
        "testnet" | "test" | "dev" | "devnet" => Some(NetworkMode::Testnet),
        _ => None,
    }
}

#[derive(Debug)]
pub struct PassphraseSession {
    key: Option<Zeroizing<[u8; 32]>>,
    expires_at: Option<std::time::Instant>,
}

impl PassphraseSession {
    pub const fn new() -> Self {
        Self {
            key: None,
            expires_at: None,
        }
    }

    fn clear(&mut self) {
        if let Some(mut k) = self.key.take() {
            k.zeroize();
        }
        self.expires_at = None;
    }

    pub fn get(&mut self) -> Option<&[u8; 32]> {
        if let Some(exp) = self.expires_at {
            if std::time::Instant::now() >= exp {
                self.clear();
                return None;
            }
        }
        self.key.as_deref()
    }

    pub fn set(&mut self, key: [u8; 32], ttl: Duration) {
        self.key = Some(Zeroizing::new(key));
        self.expires_at = Some(std::time::Instant::now() + ttl);
    }
}

#[derive(Debug)]
pub struct SharedState {
    pub ks: Keystore,
    pub cfg: SeashailConfig,
    pub session: PassphraseSession,
    db_default_shared: bool,
    db: Option<Db>,
    db_init_attempted: bool,

    scam_blocklist: Option<blocklist::ScamBlocklist>,
    ofac_sdn: Option<ofac::OfacSdnList>,
}

impl SharedState {
    pub fn new(ks: Keystore, db_default_shared: bool) -> eyre::Result<Self> {
        let cfg = ks.load_config()?;
        Ok(Self {
            ks,
            cfg,
            session: PassphraseSession::new(),
            db_default_shared,
            db: None,
            db_init_attempted: false,
            scam_blocklist: None,
            ofac_sdn: None,
        })
    }

    pub const fn db(&self) -> Option<&Db> {
        self.db.as_ref()
    }

    pub async fn ensure_db(&mut self) {
        if self.db.is_some() || self.db_init_attempted {
            return;
        }

        self.db_init_attempted = true;
        let paths = self.ks.paths().clone();

        // Cache is best-effort; never block tool calls or multi-process startup on it.
        match tokio::time::timeout(
            Duration::from_millis(500),
            Db::open(&paths, self.db_default_shared),
        )
        .await
        {
            Ok(Ok(db)) => {
                self.db = Some(db);
            }
            Ok(Err(e)) => {
                warn!(error = %e, "db init failed; price cache disabled for this process");
            }
            Err(_) => {
                warn!("db init timed out; price cache disabled for this process");
            }
        }
    }

    pub async fn refresh_scam_blocklist_if_needed(&mut self) {
        let Some(url) = self.cfg.http.scam_blocklist_url.as_deref() else {
            return;
        };

        let refresh_seconds = self.cfg.http.scam_blocklist_refresh_seconds.max(60);
        let refresh_interval_ms =
            i64::try_from(refresh_seconds.saturating_mul(1000)).unwrap_or(i64::MAX);
        let now_ms = match Db::now_ms() {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "blocklist: clock read failed; skipping refresh");
                return;
            }
        };

        let stale = self.scam_blocklist.as_ref().map_or(true, |b| {
            now_ms.saturating_sub(b.fetched_at_ms) >= refresh_interval_ms
        });

        if stale && self.scam_blocklist.is_none() {
            // Best-effort: load verified cache file first.
            match self.ks.load_scam_blocklist_cache() {
                Ok(Some(cache)) => {
                    let expected_pk = self.cfg.http.scam_blocklist_pubkey_b64.as_deref();
                    let fetched_at_ms = cache.fetched_at_ms;
                    let envelope = cache.envelope;
                    match blocklist::normalize_and_verify(fetched_at_ms, envelope, expected_pk) {
                        Ok((bl, _)) => self.scam_blocklist = Some(bl),
                        Err(e) => {
                            warn!(error = %e, "blocklist: cached blocklist failed verification; ignoring");
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(error = %e, "blocklist: failed to load cache file");
                }
            }
        }

        let still_stale = self.scam_blocklist.as_ref().map_or(true, |b| {
            now_ms.saturating_sub(b.fetched_at_ms) >= refresh_interval_ms
        });
        if !still_stale {
            return;
        }

        // Best-effort: refresh from network with a short timeout. Keep operating if it fails.
        let expected_pk = self.cfg.http.scam_blocklist_pubkey_b64.as_deref();
        match tokio::time::timeout(Duration::from_millis(1200), blocklist::fetch_envelope(url))
            .await
        {
            Ok(Ok(env)) => match blocklist::normalize_and_verify(now_ms, env, expected_pk) {
                Ok((bl, cache)) => {
                    if let Err(e) = self.ks.save_scam_blocklist_cache(&cache) {
                        warn!(error = %e, "blocklist: failed to save cache file");
                    }
                    self.scam_blocklist = Some(bl);
                }
                Err(e) => {
                    warn!(error = %e, "blocklist: fetched envelope failed verification");
                }
            },
            Ok(Err(e)) => {
                warn!(error = %e, "blocklist: fetch failed");
            }
            Err(_) => {
                warn!("blocklist: fetch timed out");
            }
        }
    }

    pub async fn scam_blocklist_contains_evm(&mut self, a: alloy::primitives::Address) -> bool {
        self.refresh_scam_blocklist_if_needed().await;
        self.scam_blocklist
            .as_ref()
            .is_some_and(|b| b.contains_evm(a))
    }

    pub async fn scam_blocklist_contains_solana(&mut self, p: solana_sdk::pubkey::Pubkey) -> bool {
        self.refresh_scam_blocklist_if_needed().await;
        self.scam_blocklist
            .as_ref()
            .is_some_and(|b| b.contains_solana(p))
    }

    pub async fn refresh_ofac_sdn_if_needed(&mut self) {
        let Some(url) = self.cfg.http.ofac_sdn_url.as_deref() else {
            return;
        };

        let refresh_seconds = self.cfg.http.ofac_sdn_refresh_seconds.max(60);
        let refresh_interval_ms =
            i64::try_from(refresh_seconds.saturating_mul(1000)).unwrap_or(i64::MAX);
        let now_ms = match Db::now_ms() {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "ofac: clock read failed; skipping refresh");
                return;
            }
        };

        let stale = self.ofac_sdn.as_ref().map_or(true, |b| {
            now_ms.saturating_sub(b.fetched_at_ms) >= refresh_interval_ms
        });

        if stale && self.ofac_sdn.is_none() {
            // Best-effort: load cache file first.
            match self.ks.load_ofac_sdn_cache() {
                Ok(Some(cache)) => match ofac::normalize(cache.fetched_at_ms, &cache.payload) {
                    Ok(list) => self.ofac_sdn = Some(list),
                    Err(e) => {
                        warn!(error = %e, "ofac: cached list failed to parse; ignoring");
                    }
                },
                Ok(None) => {}
                Err(e) => {
                    warn!(error = %e, "ofac: failed to load cache file");
                }
            }
        }

        let still_stale = self.ofac_sdn.as_ref().map_or(true, |b| {
            now_ms.saturating_sub(b.fetched_at_ms) >= refresh_interval_ms
        });
        if !still_stale {
            return;
        }

        match tokio::time::timeout(Duration::from_millis(1200), ofac::fetch_payload(url)).await {
            Ok(Ok(payload)) => match ofac::normalize(now_ms, &payload) {
                Ok(list) => {
                    let cache = ofac::OfacSdnCacheFile {
                        fetched_at_ms: now_ms,
                        payload,
                    };
                    if let Err(e) = self.ks.save_ofac_sdn_cache(&cache) {
                        warn!(error = %e, "ofac: failed to save cache file");
                    }
                    self.ofac_sdn = Some(list);
                }
                Err(e) => {
                    warn!(error = %e, "ofac: fetched list failed to normalize");
                }
            },
            Ok(Err(e)) => {
                warn!(error = %e, "ofac: fetch failed");
            }
            Err(_) => {
                warn!("ofac: fetch timed out");
            }
        }
    }

    pub async fn ofac_sdn_contains_evm(&mut self, a: alloy::primitives::Address) -> bool {
        self.refresh_ofac_sdn_if_needed().await;
        self.ofac_sdn.as_ref().is_some_and(|b| b.contains_evm(a))
    }

    pub async fn ofac_sdn_contains_solana(&mut self, p: solana_sdk::pubkey::Pubkey) -> bool {
        self.refresh_ofac_sdn_if_needed().await;
        self.ofac_sdn.as_ref().is_some_and(|b| b.contains_solana(p))
    }

    pub async fn ofac_sdn_contains_bitcoin(&mut self, addr: &str) -> bool {
        self.refresh_ofac_sdn_if_needed().await;
        self.ofac_sdn
            .as_ref()
            .is_some_and(|b| b.contains_bitcoin(addr))
    }
}

#[derive(Debug)]
pub struct ConnState {
    pub next_id: i64,
    pub network_override: Option<NetworkMode>,
}

impl ConnState {
    pub const fn new() -> Self {
        Self {
            next_id: 1_000_000,
            network_override: None,
        }
    }

    pub fn next_server_id(&mut self) -> i64 {
        let v = self.next_id;
        self.next_id += 1;
        v
    }
}
