pub mod crypto;
pub mod shamir;

use crate::{
    blocklist::ScamBlocklistCacheFile,
    config::SeashailConfig,
    errors::SeashailError,
    ofac::OfacSdnCacheFile,
    paths::SeashailPaths,
    store::ConfigStore,
    wallet::{WalletInfo, WalletKind, WalletRecord, WalletStore},
};
use base64::Engine as _;
use chrono::{Datelike as _, Utc};
use eyre::Context as _;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zeroize::Zeroize as _;

#[derive(Debug, Clone)]
pub struct Keystore {
    paths: SeashailPaths,
    cfg_store: ConfigStore,
    wallets: WalletStore,
}

impl Keystore {
    pub fn open(paths: SeashailPaths) -> eyre::Result<Self> {
        paths.ensure_private_dirs()?;

        let cfg_store = ConfigStore::new(&paths);
        let wallets = WalletStore::new(&paths);

        Ok(Self {
            paths,
            cfg_store,
            wallets,
        })
    }

    pub(crate) const fn paths(&self) -> &SeashailPaths {
        &self.paths
    }

    pub fn load_config(&self) -> eyre::Result<SeashailConfig> {
        self.cfg_store.load_or_init_default()
    }

    pub fn save_config(&self, cfg: &SeashailConfig) -> eyre::Result<()> {
        self.cfg_store.save(cfg)
    }

    pub(crate) fn ensure_passphrase_salt(
        &self,
        cfg: &mut SeashailConfig,
    ) -> eyre::Result<[u8; 16]> {
        if let Some(s) = &cfg.passphrase_salt_b64 {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(s)
                .context("decode passphrase_salt_b64")?;
            if bytes.len() != 16 {
                eyre::bail!("passphrase_salt_b64 must decode to 16 bytes");
            }
            let mut out = [0_u8; 16];
            out.copy_from_slice(&bytes);
            return Ok(out);
        }

        let salt = crypto::random_salt16();
        cfg.passphrase_salt_b64 = Some(base64::engine::general_purpose::STANDARD.encode(salt));
        self.save_config(cfg)?;
        Ok(salt)
    }

    pub fn machine_secret_path(&self) -> PathBuf {
        self.paths.config_dir.join("machine_secret.bin")
    }

    pub fn lock_path(&self) -> PathBuf {
        self.paths.data_dir.join("seashail.lock")
    }

    pub fn tx_history_path(&self) -> PathBuf {
        self.paths.data_dir.join("tx_history.jsonl")
    }

    pub fn audit_log_path(&self) -> PathBuf {
        self.paths.data_dir.join("audit.jsonl")
    }

    pub fn scam_blocklist_cache_path(&self) -> PathBuf {
        self.paths.data_dir.join("scam_blocklist_cache.json")
    }

    pub fn ofac_sdn_cache_path(&self) -> PathBuf {
        self.paths.data_dir.join("ofac_sdn_cache.json")
    }

    pub fn append_audit_log(&self, entry: &serde_json::Value) -> eyre::Result<()> {
        let entry = crate::audit::normalize_entry(entry.clone());
        let p = self.audit_log_path();
        if let Some(parent) = p.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }
        let mut f = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .mode(0o600)
                    .open(&p)
                    .context("open audit log")?
            }
            #[cfg(not(unix))]
            {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&p)
                    .context("open audit log")?
            }
        };
        writeln!(f, "{entry}").context("write audit log")?;
        Ok(())
    }

    pub fn load_scam_blocklist_cache(&self) -> eyre::Result<Option<ScamBlocklistCacheFile>> {
        let p = self.scam_blocklist_cache_path();
        if !p.exists() {
            return Ok(None);
        }
        let s = fs::read_to_string(&p).context("read scam blocklist cache")?;
        let v: ScamBlocklistCacheFile =
            serde_json::from_str(&s).context("parse scam blocklist cache")?;
        Ok(Some(v))
    }

    pub fn save_scam_blocklist_cache(&self, cache: &ScamBlocklistCacheFile) -> eyre::Result<()> {
        let p = self.scam_blocklist_cache_path();
        if let Some(parent) = p.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }
        let s = serde_json::to_string_pretty(cache).context("serialize scam blocklist cache")?;
        crate::fsutil::write_string_atomic_restrictive(&p, &s, crate::fsutil::MODE_FILE_PRIVATE)
            .context("write scam blocklist cache")?;
        Ok(())
    }

    pub fn load_ofac_sdn_cache(&self) -> eyre::Result<Option<OfacSdnCacheFile>> {
        let p = self.ofac_sdn_cache_path();
        if !p.exists() {
            return Ok(None);
        }
        let s = fs::read_to_string(&p).context("read ofac sdn cache")?;
        let v: OfacSdnCacheFile = serde_json::from_str(&s).context("parse ofac sdn cache")?;
        Ok(Some(v))
    }

    pub fn save_ofac_sdn_cache(&self, cache: &OfacSdnCacheFile) -> eyre::Result<()> {
        let p = self.ofac_sdn_cache_path();
        if let Some(parent) = p.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }
        let s = serde_json::to_string_pretty(cache).context("serialize ofac sdn cache")?;
        crate::fsutil::write_string_atomic_restrictive(&p, &s, crate::fsutil::MODE_FILE_PRIVATE)
            .context("write ofac sdn cache")?;
        Ok(())
    }

    pub fn ensure_machine_secret(&self) -> eyre::Result<[u8; 32]> {
        let p = self.machine_secret_path();
        if p.exists() {
            let buf = fs::read(&p).context("read machine secret")?;
            if buf.len() != 32 {
                eyre::bail!("machine secret wrong length");
            }
            let mut out = [0_u8; 32];
            out.copy_from_slice(&buf);
            return Ok(out);
        }

        let mut secret = [0_u8; 32];
        crypto::fill_random(&mut secret);

        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).context("create config dir")?;
        }

        // Best-effort restrictive perms (Unix). Windows ignores.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&p)
                .context("create machine secret")?;
            f.write_all(&secret).context("write machine secret")?;
        }
        #[cfg(not(unix))]
        {
            fs::write(&p, &secret).context("write machine secret")?;
        }

        Ok(secret)
    }

    /// Exclusive lock for write operations across multiple `seashail mcp` processes.
    pub fn acquire_write_lock(&self) -> eyre::Result<File> {
        let p = self.lock_path();
        if let Some(parent) = p.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }
        let f = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .truncate(false)
                    .mode(0o600)
                    .open(&p)
                    .context("open lock file")?
            }
            #[cfg(not(unix))]
            {
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .truncate(false)
                    .open(&p)
                    .context("open lock file")?
            }
        };
        // Avoid indefinite blocking inside an MCP tool call. If another process is actively
        // writing, fail fast and let the client retry.
        match f.try_lock_exclusive() {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(SeashailError::KeystoreBusy.into());
            }
            Err(e) => return Err(eyre::Report::new(e).wrap_err("lock exclusive")),
        }
        Ok(f)
    }

    pub fn release_lock(mut f: File) -> eyre::Result<()> {
        FileExt::unlock(&f).context("unlock")?;
        // Close.
        f.flush().ok();
        Ok(())
    }

    pub fn list_wallets(&self) -> eyre::Result<Vec<WalletRecord>> {
        self.wallets.list()
    }

    pub fn ensure_default_wallet(&self) -> eyre::Result<()> {
        if !self.list_wallets()?.is_empty() {
            return Ok(());
        }

        // Serialize first-run initialization across competing binaries/agents.
        let lock = self.acquire_write_lock()?;
        if !self.list_wallets()?.is_empty() {
            Self::release_lock(lock)?;
            return Ok(());
        }

        let info = self.create_generated_wallet_machine_only("default".to_owned())?;
        // Best-effort: make it active for immediate use.
        let _set_active_wallet = self.set_active_wallet(&info.name, 0);
        Self::release_lock(lock)?;
        Ok(())
    }

    pub fn get_wallet_by_name(&self, name: &str) -> eyre::Result<Option<WalletRecord>> {
        self.wallets.get_by_name(name)
    }

    pub fn get_wallet_info(&self, name: &str) -> eyre::Result<WalletInfo> {
        let w = self
            .wallets
            .get_by_name(name)?
            .ok_or_else(|| SeashailError::WalletNotFound(name.to_owned()))?;
        let active = self
            .wallets
            .get_active()?
            .and_then(|(aw, idx)| (aw.name == w.name).then_some(idx))
            .unwrap_or(0);
        Ok(WalletStore::wallet_info(&w, active))
    }

    pub fn set_active_wallet(&self, name: &str, account_index: u32) -> eyre::Result<()> {
        self.wallets.set_active(name, account_index)
    }

    pub fn get_active_wallet(&self) -> eyre::Result<Option<(WalletRecord, u32)>> {
        self.wallets.get_active()
    }

    pub fn add_account(
        &self,
        wallet_name: &str,
        passphrase_key: &[u8; 32],
    ) -> eyre::Result<(WalletInfo, u32)> {
        self.add_account_auto(wallet_name, Some(passphrase_key))
    }

    pub fn add_account_auto(
        &self,
        wallet_name: &str,
        passphrase_key: Option<&[u8; 32]>,
    ) -> eyre::Result<(WalletInfo, u32)> {
        let mut w = self
            .get_wallet_by_name(wallet_name)?
            .ok_or_else(|| SeashailError::WalletNotFound(wallet_name.to_owned()))?;

        let new_index = w.accounts;

        match w.kind {
            WalletKind::Generated => {
                let mut entropy = self.decrypt_generated_entropy_maybe(&w.id, passphrase_key)?;
                let (evm, sol) = crate::wallet::addresses_from_entropy(&entropy, &[new_index])?;
                let (btc_main, btc_test) =
                    crate::wallet::bitcoin_addresses_from_entropy(&entropy, &[new_index])?;
                entropy.zeroize();
                w.evm_addresses.extend(evm);
                w.solana_addresses.extend(sol);
                w.bitcoin_addresses_mainnet.extend(btc_main);
                w.bitcoin_addresses_testnet.extend(btc_test);
            }
            WalletKind::Imported => match w.imported_kind {
                Some(crate::wallet::ImportedKind::Mnemonic) => {
                    let Some(passphrase_key) = passphrase_key else {
                        return Err(SeashailError::PassphraseRequired.into());
                    };
                    let mut secret = self.decrypt_imported_secret(&w.id, passphrase_key)?;
                    let phrase =
                        std::str::from_utf8(&secret).context("imported mnemonic must be utf-8")?;
                    let mnemonic =
                        bip39::Mnemonic::parse_in_normalized(bip39::Language::English, phrase)
                            .context("parse imported mnemonic")?;
                    let (evm, sol) =
                        crate::wallet::addresses_from_mnemonic(&mnemonic, &[new_index])?;
                    let (btc_main, btc_test) =
                        crate::wallet::bitcoin_addresses_from_mnemonic(&mnemonic, &[new_index])?;
                    secret.zeroize();
                    w.evm_addresses.extend(evm);
                    w.solana_addresses.extend(sol);
                    w.bitcoin_addresses_mainnet.extend(btc_main);
                    w.bitcoin_addresses_testnet.extend(btc_test);
                }
                _ => eyre::bail!("cannot add accounts to imported private key wallets"),
            },
        }

        w.accounts += 1;
        self.wallets.update(&w)?;
        Ok((WalletStore::wallet_info(&w, new_index), new_index))
    }

    pub fn add_account_no_passphrase(&self, wallet_name: &str) -> eyre::Result<(WalletInfo, u32)> {
        self.add_account_auto(wallet_name, None)
    }

    pub fn configure_rpc(
        &self,
        cfg: &mut SeashailConfig,
        chain: &str,
        url: &str,
        fallback_urls: Option<Vec<String>>,
        solana_mode: Option<crate::config::NetworkMode>,
    ) -> eyre::Result<()> {
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

        if chain == "solana" {
            url.clone_into(&mut cfg.rpc.solana_rpc_url);
            match solana_mode.unwrap_or_else(|| cfg.effective_network_mode()) {
                crate::config::NetworkMode::Mainnet => {
                    if let Some(v) = fallback_urls {
                        cfg.rpc.solana_fallback_rpc_urls_mainnet = v;
                    } else if is_loopback_http(url) {
                        // Avoid accidentally falling back to public mainnet RPCs when the user is
                        // pointing at a local validator (different cluster/genesis hash).
                        cfg.rpc.solana_fallback_rpc_urls_mainnet = vec![];
                    }
                }
                crate::config::NetworkMode::Testnet => {
                    if let Some(v) = fallback_urls {
                        cfg.rpc.solana_fallback_rpc_urls_devnet = v;
                    } else if is_loopback_http(url) {
                        cfg.rpc.solana_fallback_rpc_urls_devnet = vec![];
                    }
                }
            }
            self.save_config(cfg)?;
            return Ok(());
        }
        if cfg.rpc.evm_rpc_urls.contains_key(chain) {
            cfg.rpc
                .evm_rpc_urls
                .insert(chain.to_owned(), url.to_owned());
            if let Some(v) = fallback_urls {
                cfg.rpc.evm_fallback_rpc_urls.insert(chain.to_owned(), v);
            }
            self.save_config(cfg)?;
            return Ok(());
        }
        eyre::bail!("unknown chain: {chain}");
    }

    pub fn append_tx_history(&self, entry: &serde_json::Value) -> eyre::Result<()> {
        let p = self.tx_history_path();
        if let Some(parent) = p.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }

        let mut f = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .mode(0o600)
                    .open(&p)
                    .context("open tx history")?
            }
            #[cfg(not(unix))]
            {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&p)
                    .context("open tx history")?
            }
        };
        writeln!(f, "{entry}").context("write tx history")?;
        Ok(())
    }

    pub fn read_tx_history_filtered(
        &self,
        limit: usize,
        wallet: Option<&str>,
        chain: Option<&str>,
        type_filter: Option<&str>,
        since_ts: Option<&str>,
        until_ts: Option<&str>,
    ) -> eyre::Result<Vec<serde_json::Value>> {
        let p = self.tx_history_path();
        if !p.exists() {
            return Ok(vec![]);
        }
        let since = if let Some(s) = since_ts {
            Some(chrono::DateTime::parse_from_rfc3339(s).context("parse since_ts")?)
        } else {
            None
        };
        let until = if let Some(s) = until_ts {
            Some(chrono::DateTime::parse_from_rfc3339(s).context("parse until_ts")?)
        } else {
            None
        };
        let contents = fs::read_to_string(&p).context("read tx history")?;
        let mut out = vec![];
        for line in contents.lines().rev() {
            if out.len() >= limit {
                break;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };

            if let Some(w) = wallet {
                if v.get("wallet").and_then(|x| x.as_str()) != Some(w) {
                    continue;
                }
            }
            if let Some(c) = chain {
                if v.get("chain").and_then(|x| x.as_str()) != Some(c) {
                    continue;
                }
            }
            if let Some(t) = type_filter {
                if v.get("type").and_then(|x| x.as_str()) != Some(t) {
                    continue;
                }
            }

            if since.is_some() || until.is_some() {
                let Some(ts_s) = v.get("ts").and_then(|x| x.as_str()) else {
                    continue;
                };
                let ts = chrono::DateTime::parse_from_rfc3339(ts_s).context("parse entry ts")?;
                if since.as_ref().is_some_and(|since_dt| ts < *since_dt) {
                    continue;
                }
                if until.as_ref().is_some_and(|u| ts > *u) {
                    continue;
                }
            }

            out.push(v);
        }
        out.reverse();
        Ok(out)
    }

    pub fn daily_used_usd_filtered(&self, day: &str, wallet: Option<&str>) -> eyre::Result<f64> {
        let p = self.tx_history_path();
        if !p.exists() {
            return Ok(0.0_f64);
        }
        let s = fs::read_to_string(&p).context("read tx history")?;
        let mut total = 0.0_f64;
        for line in s.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if let Some(w) = wallet {
                if v.get("wallet").and_then(|x| x.as_str()) != Some(w) {
                    continue;
                }
            }
            if v.get("day").and_then(|x| x.as_str()) != Some(day) {
                continue;
            }
            let Some(t) = v.get("type").and_then(|x| x.as_str()) else {
                continue;
            };
            // Any action that can move funds should count against daily spend limits.
            if !matches!(
                t,
                "send"
                    | "swap"
                    | "approve"
                    | "perp_open"
                    | "perp_close"
                    | "perp_modify"
                    | "perp_limit"
                    | "nft_buy"
                    | "nft_sell"
                    | "nft_transfer"
                    | "nft_bid"
                    | "pumpfun_buy"
                    | "pumpfun_sell"
                    | "internal_transfer_strict"
                    | "bridge"
                    | "lend"
                    | "withdraw_lending"
                    | "borrow"
                    | "repay_borrow"
                    | "stake"
                    | "unstake"
                    | "provide_liquidity"
                    | "remove_liquidity"
                    | "prediction_place"
                    | "prediction_close"
            ) {
                continue;
            }
            if let Some(usd) = v.get("usd_value").and_then(serde_json::Value::as_f64) {
                crate::financial_math::accum(&mut total, usd);
            }
        }
        Ok(total)
    }

    pub fn current_utc_day_key() -> String {
        let now = Utc::now();
        format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
    }

    pub fn wallets_dir(&self) -> PathBuf {
        self.paths.config_dir.join("wallets")
    }

    fn wallet_dir(&self, id: &str) -> PathBuf {
        self.wallets_dir().join(id)
    }

    fn write_json_restrictive(path: &Path, v: &impl Serialize) -> eyre::Result<()> {
        if let Some(parent) = path.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }

        let s = serde_json::to_string_pretty(v).context("serialize json")?;
        crate::fsutil::write_string_atomic_restrictive(path, &s, crate::fsutil::MODE_FILE_PRIVATE)
            .with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> eyre::Result<T> {
        let s = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let v = serde_json::from_str(&s).with_context(|| format!("parse {}", path.display()))?;
        Ok(v)
    }

    pub fn create_generated_wallet(
        &self,
        name: String,
        passphrase_key: [u8; 32],
    ) -> eyre::Result<(WalletInfo, String)> {
        self.ensure_machine_secret()?;

        let wallet_id = Uuid::new_v4().to_string();
        let wdir = self.wallet_dir(&wallet_id);
        fs::create_dir_all(&wdir).context("create wallet dir")?;

        // Create mnemonic entropy and Shamir split it.
        let (mut entropy, mnemonic_words) = crate::wallet::generate_mnemonic_entropy()?;
        let (evm_addrs, sol_addrs) = crate::wallet::addresses_from_entropy(&entropy, &[0])?;
        let (btc_main, btc_test) = crate::wallet::bitcoin_addresses_from_entropy(&entropy, &[0])?;

        let wallet = crate::wallet::WalletRecord::new_generated(
            wallet_id.clone(),
            name,
            crate::wallet::WalletAddressSets {
                evm: evm_addrs,
                solana: sol_addrs,
                bitcoin_mainnet: btc_main,
                bitcoin_testnet: btc_test,
            },
        );
        self.wallets.add(wallet.clone())?;
        let shares = shamir::split(&entropy, 3, 2)?;
        let [s1_share, s2_share, s3_share] = shares.as_slice() else {
            eyre::bail!("shamir split returned wrong number of shares");
        };

        let mut cfg = self.load_config()?;
        let pass_salt = self.ensure_passphrase_salt(&mut cfg)?;

        let machine = self.ensure_machine_secret()?;

        let s1_path = wdir.join("share1.machine.json");
        let s2_path = wdir.join("share2.pass.json");
        let meta_path = wdir.join("wallet.json");

        // Encrypt shares.
        let s1_key = crypto::derive_subkey_machine(&machine, &wallet_id, "share1")?;
        let s2_key = crypto::derive_subkey_passphrase(&passphrase_key, &wallet_id, "share2")?;

        let s1_box = crypto::encrypt_aes_gcm(&s1_key, s1_share)?;
        let s2_box = crypto::encrypt_aes_gcm(&s2_key, s2_share)?;

        Self::write_json_restrictive(&s1_path, &s1_box)?;
        Self::write_json_restrictive(&s2_path, &s2_box)?;

        let secret_len = u32::try_from(entropy.len()).context("entropy length overflow")?;
        let meta = GeneratedWalletMeta {
            id: wallet_id,
            kind: "generated".into(),
            passphrase_salt_b64: base64::engine::general_purpose::STANDARD.encode(pass_salt),
            shamir: ShamirMeta {
                shares: 3,
                threshold: 2,
                secret_len,
            },
            // This isn't sensitive, but we keep it local for convenience (wallet restore UI).
            mnemonic_words,
        };
        Self::write_json_restrictive(&meta_path, &meta)?;

        // Clear sensitive material.
        entropy.zeroize();

        let share3_display = base64::engine::general_purpose::STANDARD.encode(s3_share);

        Ok((WalletStore::wallet_info(&wallet, 0), share3_display))
    }

    /// Create a generated wallet that is immediately usable without a user-provided passphrase.
    ///
    /// Security tradeoff: this wallet is machine-bound (Share 1 + Share 2 are both encrypted
    /// with the machine secret). Users can later opt into portable recovery by rotating shares
    /// with a passphrase and exporting Share 3.
    pub fn create_generated_wallet_machine_only(&self, name: String) -> eyre::Result<WalletInfo> {
        self.ensure_machine_secret()?;

        let wallet_id = Uuid::new_v4().to_string();
        let wdir = self.wallet_dir(&wallet_id);
        fs::create_dir_all(&wdir).context("create wallet dir")?;

        let (mut entropy, mnemonic_words) = crate::wallet::generate_mnemonic_entropy()?;
        let (evm_addrs, sol_addrs) = crate::wallet::addresses_from_entropy(&entropy, &[0])?;
        let (btc_main, btc_test) = crate::wallet::bitcoin_addresses_from_entropy(&entropy, &[0])?;

        let wallet = crate::wallet::WalletRecord::new_generated(
            wallet_id.clone(),
            name,
            crate::wallet::WalletAddressSets {
                evm: evm_addrs,
                solana: sol_addrs,
                bitcoin_mainnet: btc_main,
                bitcoin_testnet: btc_test,
            },
        );
        self.wallets.add(wallet.clone())?;

        let shares = shamir::split(&entropy, 3, 2)?;
        let [s1_share, s2_share, _s3_share] = shares.as_slice() else {
            eyre::bail!("shamir split returned wrong number of shares");
        };

        let mut cfg = self.load_config()?;
        let pass_salt = self.ensure_passphrase_salt(&mut cfg)?;

        let machine = self.ensure_machine_secret()?;
        let s1_path = wdir.join("share1.machine.json");
        let s2_path = wdir.join("share2.machine.json");
        let meta_path = wdir.join("wallet.json");

        let s1_key = crypto::derive_subkey_machine(&machine, &wallet_id, "share1")?;
        let s2_key = crypto::derive_subkey_machine(&machine, &wallet_id, "share2")?;

        let s1_box = crypto::encrypt_aes_gcm(&s1_key, s1_share)?;
        let s2_box = crypto::encrypt_aes_gcm(&s2_key, s2_share)?;

        Self::write_json_restrictive(&s1_path, &s1_box)?;
        Self::write_json_restrictive(&s2_path, &s2_box)?;

        let secret_len = u32::try_from(entropy.len()).context("entropy length overflow")?;
        let meta = GeneratedWalletMeta {
            id: wallet_id,
            kind: "generated".into(),
            passphrase_salt_b64: base64::engine::general_purpose::STANDARD.encode(pass_salt),
            shamir: ShamirMeta {
                shares: 3,
                threshold: 2,
                secret_len,
            },
            mnemonic_words,
        };
        Self::write_json_restrictive(&meta_path, &meta)?;

        entropy.zeroize();
        Ok(WalletStore::wallet_info(&wallet, 0))
    }

    pub fn import_wallet(
        &self,
        name: String,
        kind: crate::wallet::ImportedKind,
        mut secret_bytes: Vec<u8>,
        passphrase_key: [u8; 32],
    ) -> eyre::Result<WalletInfo> {
        self.ensure_machine_secret()?;
        let wallet_id = Uuid::new_v4().to_string();
        let wdir = self.wallet_dir(&wallet_id);
        fs::create_dir_all(&wdir).context("create wallet dir")?;

        let (evm_addrs, sol_addrs, pk_chain) =
            crate::wallet::addresses_from_import(kind, &secret_bytes)?;
        let (btc_main, btc_test) = if kind == crate::wallet::ImportedKind::Mnemonic {
            let phrase = std::str::from_utf8(&secret_bytes).context("mnemonic must be utf-8")?;
            let mnemonic = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, phrase)
                .context("parse mnemonic")?;
            crate::wallet::bitcoin_addresses_from_mnemonic(&mnemonic, &[0])?
        } else {
            (vec![], vec![])
        };

        let wallet = crate::wallet::WalletRecord::new_imported(
            wallet_id.clone(),
            name,
            kind,
            pk_chain,
            crate::wallet::WalletAddressSets {
                evm: evm_addrs,
                solana: sol_addrs,
                bitcoin_mainnet: btc_main,
                bitcoin_testnet: btc_test,
            },
        );
        self.wallets.add(wallet.clone())?;

        let mut cfg = self.load_config()?;
        let pass_salt = self.ensure_passphrase_salt(&mut cfg)?;
        let key = crypto::derive_subkey_passphrase(&passphrase_key, &wallet_id, "imported")?;

        let enc = crypto::encrypt_aes_gcm(&key, &secret_bytes)?;
        Self::write_json_restrictive(&wdir.join("imported.secret.json"), &enc)?;
        secret_bytes.zeroize();

        let meta = ImportedWalletMeta {
            id: wallet_id,
            kind: "imported".into(),
            imported_kind: kind,
            passphrase_salt_b64: base64::engine::general_purpose::STANDARD.encode(pass_salt),
        };
        Self::write_json_restrictive(&wdir.join("wallet.json"), &meta)?;

        Ok(WalletStore::wallet_info(&wallet, 0))
    }

    fn load_generated_wallet_meta(&self, wallet_id: &str) -> eyre::Result<GeneratedWalletMeta> {
        let p = self.wallet_dir(wallet_id).join("wallet.json");
        Self::read_json(&p)
    }

    fn decrypt_generated_entropy_maybe(
        &self,
        wallet_id: &str,
        passphrase_key: Option<&[u8; 32]>,
    ) -> eyre::Result<Vec<u8>> {
        let wdir = self.wallet_dir(wallet_id);
        let s1_box: crypto::CryptoBox = Self::read_json(&wdir.join("share1.machine.json"))?;
        let meta = self.load_generated_wallet_meta(wallet_id)?;

        let machine = self.ensure_machine_secret()?;
        let s1_key = crypto::derive_subkey_machine(&machine, wallet_id, "share1")?;
        let s1 = crypto::decrypt_aes_gcm(&s1_key, &s1_box)?;

        let s2 = if wdir.join("share2.machine.json").exists() {
            let s2_box: crypto::CryptoBox = Self::read_json(&wdir.join("share2.machine.json"))?;
            let s2_key = crypto::derive_subkey_machine(&machine, wallet_id, "share2")?;
            crypto::decrypt_aes_gcm(&s2_key, &s2_box)?
        } else {
            let Some(passphrase_key) = passphrase_key else {
                return Err(SeashailError::PassphraseRequired.into());
            };
            let s2_box: crypto::CryptoBox = Self::read_json(&wdir.join("share2.pass.json"))?;
            let s2_key = crypto::derive_subkey_passphrase(passphrase_key, wallet_id, "share2")?;
            crypto::decrypt_aes_gcm(&s2_key, &s2_box)?
        };

        let entropy = shamir::combine(&[s1, s2], meta.shamir.threshold as usize)?;
        Ok(entropy)
    }

    pub fn generated_wallet_needs_passphrase(&self, wallet_id: &str) -> bool {
        let wdir = self.wallet_dir(wallet_id);
        // If Share 2 is passphrase-encrypted, signing requires an unlock.
        wdir.join("share2.pass.json").exists() && !wdir.join("share2.machine.json").exists()
    }

    pub fn decrypt_generated_entropy(
        &self,
        wallet_id: &str,
        passphrase_key: &[u8; 32],
    ) -> eyre::Result<Vec<u8>> {
        self.decrypt_generated_entropy_maybe(wallet_id, Some(passphrase_key))
    }

    pub fn decrypt_generated_entropy_no_passphrase(
        &self,
        wallet_id: &str,
    ) -> eyre::Result<Vec<u8>> {
        self.decrypt_generated_entropy_maybe(wallet_id, None)
    }

    pub fn decrypt_imported_secret(
        &self,
        wallet_id: &str,
        passphrase_key: &[u8; 32],
    ) -> eyre::Result<Vec<u8>> {
        let wdir = self.wallet_dir(wallet_id);
        let boxv: crypto::CryptoBox = Self::read_json(&wdir.join("imported.secret.json"))?;
        let key = crypto::derive_subkey_passphrase(passphrase_key, wallet_id, "imported")?;
        crypto::decrypt_aes_gcm(&key, &boxv)
    }

    pub(crate) fn plan_rotate_shares(
        &self,
        wallet_id: &str,
        passphrase_key: &[u8; 32],
    ) -> eyre::Result<PlannedShareRotation> {
        let wdir = self.wallet_dir(wallet_id);
        let meta = self.load_generated_wallet_meta(wallet_id)?;

        let mut entropy = self.decrypt_generated_entropy_maybe(wallet_id, Some(passphrase_key))?;
        let shares = shamir::split(&entropy, 3, 2)?;
        let [s1_share, s2_share, s3_share] = shares.as_slice() else {
            eyre::bail!("shamir split returned wrong number of shares");
        };

        let machine = self.ensure_machine_secret()?;
        let s1_key = crypto::derive_subkey_machine(&machine, wallet_id, "share1")?;
        let s2_machine_key = crypto::derive_subkey_machine(&machine, wallet_id, "share2")?;
        let s2_key = crypto::derive_subkey_passphrase(passphrase_key, wallet_id, "share2")?;

        let s1_box = crypto::encrypt_aes_gcm(&s1_key, s1_share)?;
        let s2_pass_box = crypto::encrypt_aes_gcm(&s2_key, s2_share)?;
        let s2_machine_box = if wdir.join("share2.machine.json").exists() {
            Some(crypto::encrypt_aes_gcm(&s2_machine_key, s2_share)?)
        } else {
            None
        };

        entropy.zeroize();

        Ok(PlannedShareRotation {
            s1_box,
            s2_pass_box,
            s2_machine_box,
            meta,
            share3_base64: base64::engine::general_purpose::STANDARD.encode(s3_share),
        })
    }

    pub(crate) fn commit_rotate_shares(
        &self,
        wallet_id: &str,
        plan: &PlannedShareRotation,
    ) -> eyre::Result<()> {
        let wdir = self.wallet_dir(wallet_id);
        Self::write_json_restrictive(&wdir.join("share1.machine.json"), &plan.s1_box)?;
        Self::write_json_restrictive(&wdir.join("share2.pass.json"), &plan.s2_pass_box)?;
        if let Some(s2m) = &plan.s2_machine_box {
            Self::write_json_restrictive(&wdir.join("share2.machine.json"), s2m)?;
        }

        // We never persist Share 3; delete any legacy copies.
        let _remove_share3_backup = fs::remove_file(wdir.join("share3.backup.json"));
        let _remove_share3_pass = fs::remove_file(wdir.join("share3.pass.json"));

        Self::write_json_restrictive(&wdir.join("wallet.json"), &plan.meta)?;
        Ok(())
    }
}

pub struct PlannedShareRotation {
    pub share3_base64: String,
    s1_box: crypto::CryptoBox,
    s2_pass_box: crypto::CryptoBox,
    s2_machine_box: Option<crypto::CryptoBox>,
    meta: GeneratedWalletMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShamirMeta {
    shares: u8,
    threshold: u8,
    secret_len: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeneratedWalletMeta {
    id: String,
    kind: String,
    passphrase_salt_b64: String,
    shamir: ShamirMeta,
    mnemonic_words: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportedWalletMeta {
    id: String,
    kind: String,
    imported_kind: crate::wallet::ImportedKind,
    passphrase_salt_b64: String,
}

pub fn utc_now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}
