use crate::{errors::SeashailError, paths::SeashailPaths};
use alloy::signers::local::{coins_bip39::English, MnemonicBuilder, PrivateKeySigner};
use bip39::{Language, Mnemonic};
use bitcoin::bip32::{DerivationPath as BtcDerivationPath, Xpriv as BtcXpriv};
use bitcoin::secp256k1::Secp256k1 as BtcSecp256k1;
use bitcoin::{
    address::KnownHrp as BtcKnownHrp, Address as BtcAddress,
    CompressedPublicKey as BtcCompressedPublicKey, Network as BtcNetwork,
    PrivateKey as BtcPrivateKey,
};
use eyre::Context as _;
use serde::{Deserialize, Serialize};
use solana_derivation_path::DerivationPath as SolanaDerivationPath;
use solana_keypair::seed_derivable::keypair_from_seed_and_derivation_path;
use solana_seed_phrase::generate_seed_from_seed_phrase_and_passphrase;
use solana_signer::Signer as _;
use std::{fs, path::PathBuf};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WalletKind {
    Generated,
    Imported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportedKind {
    PrivateKey,
    Mnemonic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportedPrivateKeyChain {
    Evm,
    Solana,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRecord {
    pub id: String,
    pub name: String,
    pub kind: WalletKind,
    pub accounts: u32,
    /// Last active account index for this wallet (purely UX metadata).
    pub last_active_account: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_kind: Option<ImportedKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_private_key_chain: Option<ImportedPrivateKeyChain>,

    /// Cached public addresses so read-only tools don't require unlocking.
    #[serde(default)]
    pub evm_addresses: Vec<String>,
    #[serde(default)]
    pub solana_addresses: Vec<String>,
    #[serde(default)]
    pub bitcoin_addresses_mainnet: Vec<String>,
    #[serde(default)]
    pub bitcoin_addresses_testnet: Vec<String>,
}

pub struct WalletAddressSets {
    pub evm: Vec<String>,
    pub solana: Vec<String>,
    pub bitcoin_mainnet: Vec<String>,
    pub bitcoin_testnet: Vec<String>,
}

impl WalletAddressSets {
    fn account_count(&self) -> u32 {
        u32::try_from(
            self.evm
                .len()
                .max(self.solana.len())
                .max(self.bitcoin_mainnet.len())
                .max(self.bitcoin_testnet.len()),
        )
        .unwrap_or(u32::MAX)
    }
}

impl WalletRecord {
    pub fn new_generated(id: String, name: String, addrs: WalletAddressSets) -> Self {
        let accounts = addrs.account_count();
        Self {
            id,
            name,
            kind: WalletKind::Generated,
            accounts,
            last_active_account: 0,
            imported_kind: None,
            imported_private_key_chain: None,
            evm_addresses: addrs.evm,
            solana_addresses: addrs.solana,
            bitcoin_addresses_mainnet: addrs.bitcoin_mainnet,
            bitcoin_addresses_testnet: addrs.bitcoin_testnet,
        }
    }

    pub fn new_imported(
        id: String,
        name: String,
        imported_kind: ImportedKind,
        imported_private_key_chain: Option<ImportedPrivateKeyChain>,
        addrs: WalletAddressSets,
    ) -> Self {
        let accounts = addrs.account_count();
        Self {
            id,
            name,
            kind: WalletKind::Imported,
            accounts,
            last_active_account: 0,
            imported_kind: Some(imported_kind),
            imported_private_key_chain,
            evm_addresses: addrs.evm,
            solana_addresses: addrs.solana,
            bitcoin_addresses_mainnet: addrs.bitcoin_mainnet,
            bitcoin_addresses_testnet: addrs.bitcoin_testnet,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WalletIndex {
    pub wallets: Vec<WalletRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_wallet_name: Option<String>,
    #[serde(default)]
    pub active_account: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletInfo {
    pub name: String,
    pub kind: WalletKind,
    pub accounts: u32,
    pub active_account: u32,
    pub addresses: WalletAddresses,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletAddresses {
    pub evm: Vec<String>,
    pub solana: Vec<String>,
    pub bitcoin_mainnet: Vec<String>,
    pub bitcoin_testnet: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WalletStore {
    index_path: PathBuf,
}

impl WalletStore {
    pub fn new(paths: &SeashailPaths) -> Self {
        Self {
            index_path: paths.config_dir.join("wallets").join("index.json"),
        }
    }

    fn load_index(&self) -> eyre::Result<WalletIndex> {
        if !self.index_path.exists() {
            return Ok(WalletIndex::default());
        }
        let s = fs::read_to_string(&self.index_path).context("read wallet index")?;
        let v: WalletIndex = serde_json::from_str(&s).context("parse wallet index")?;
        Ok(v)
    }

    fn save_index(&self, idx: &WalletIndex) -> eyre::Result<()> {
        if let Some(parent) = self.index_path.parent() {
            crate::fsutil::ensure_private_dir(parent)?;
        }
        let s = serde_json::to_string_pretty(idx).context("serialize wallet index")?;
        crate::fsutil::write_string_atomic_restrictive(
            &self.index_path,
            &s,
            crate::fsutil::MODE_FILE_PRIVATE,
        )
        .context("write wallet index")?;
        Ok(())
    }

    pub fn list(&self) -> eyre::Result<Vec<WalletRecord>> {
        Ok(self.load_index()?.wallets)
    }

    pub fn get_by_name(&self, name: &str) -> eyre::Result<Option<WalletRecord>> {
        let idx = self.load_index()?;
        Ok(idx.wallets.into_iter().find(|w| w.name == name))
    }

    pub fn add(&self, wallet: WalletRecord) -> eyre::Result<()> {
        let mut idx = self.load_index()?;
        if idx.wallets.iter().any(|w| w.name == wallet.name) {
            eyre::bail!("wallet name already exists");
        }
        let wallet_name = wallet.name.clone();
        idx.wallets.push(wallet);
        if idx.active_wallet_name.is_none() {
            idx.active_wallet_name = Some(wallet_name);
            idx.active_account = 0;
        }
        self.save_index(&idx)?;
        Ok(())
    }

    pub fn update(&self, wallet: &WalletRecord) -> eyre::Result<()> {
        let mut idx = self.load_index()?;
        let pos = idx
            .wallets
            .iter()
            .position(|w| w.id == wallet.id)
            .ok_or_else(|| SeashailError::WalletNotFound(wallet.name.clone()))?;
        let Some(dst) = idx.wallets.get_mut(pos) else {
            // Should be unreachable since `position` came from the same vec.
            return Err(SeashailError::WalletNotFound(wallet.name.clone()).into());
        };
        *dst = wallet.clone();
        self.save_index(&idx)?;
        Ok(())
    }

    pub fn set_active(&self, name: &str, account_index: u32) -> eyre::Result<()> {
        let mut idx = self.load_index()?;
        let w = idx
            .wallets
            .iter()
            .find(|w| w.name == name)
            .ok_or_else(|| SeashailError::WalletNotFound(name.to_owned()))?;
        if account_index >= w.accounts {
            return Err(SeashailError::AccountIndexOutOfRange.into());
        }
        idx.active_wallet_name = Some(name.to_owned());
        idx.active_account = account_index;
        self.save_index(&idx)?;
        Ok(())
    }

    pub fn get_active(&self) -> eyre::Result<Option<(WalletRecord, u32)>> {
        let idx = self.load_index()?;
        let Some(name) = idx.active_wallet_name else {
            return Ok(None);
        };
        let w = idx.wallets.into_iter().find(|w| w.name == name);
        Ok(w.map(|w| (w, idx.active_account)))
    }

    pub fn wallet_info(w: &WalletRecord, active_account: u32) -> WalletInfo {
        WalletInfo {
            name: w.name.clone(),
            kind: w.kind,
            accounts: w.accounts,
            active_account,
            addresses: WalletAddresses {
                evm: w.evm_addresses.clone(),
                solana: w.solana_addresses.clone(),
                bitcoin_mainnet: w.bitcoin_addresses_mainnet.clone(),
                bitcoin_testnet: w.bitcoin_addresses_testnet.clone(),
            },
        }
    }
}

pub fn generate_mnemonic_entropy() -> eyre::Result<(Vec<u8>, u8)> {
    // 24 words -> 32 bytes entropy.
    let mnemonic = Mnemonic::generate_in(Language::English, 24).context("generate mnemonic")?;
    let entropy = mnemonic.to_entropy();
    Ok((entropy, 24))
}

pub fn addresses_from_entropy(
    entropy: &[u8],
    account_indices: &[u32],
) -> eyre::Result<(Vec<String>, Vec<String>)> {
    let mnemonic =
        Mnemonic::from_entropy_in(Language::English, entropy).context("mnemonic from entropy")?;
    addresses_from_mnemonic(&mnemonic, account_indices)
}

pub fn bitcoin_addresses_from_entropy(
    entropy: &[u8],
    account_indices: &[u32],
) -> eyre::Result<(Vec<String>, Vec<String>)> {
    let mnemonic =
        Mnemonic::from_entropy_in(Language::English, entropy).context("mnemonic from entropy")?;
    bitcoin_addresses_from_mnemonic(&mnemonic, account_indices)
}

pub fn addresses_from_mnemonic(
    mnemonic: &Mnemonic,
    account_indices: &[u32],
) -> eyre::Result<(Vec<String>, Vec<String>)> {
    let mut evm = vec![];
    let mut sol = vec![];
    let phrase = Zeroizing::new(mnemonic.to_string());
    let seed = Zeroizing::new(generate_seed_from_seed_phrase_and_passphrase(
        phrase.as_str(),
        "",
    ));
    for &i in account_indices {
        let wallet = MnemonicBuilder::<English>::default()
            .phrase(phrase.as_str())
            .index(i)
            .context("evm index")?
            .build()
            .context("build evm wallet")?;
        evm.push(wallet.address().to_checksum(None));

        let path = SolanaDerivationPath::new_bip44(Some(i), Some(0));
        let kp = keypair_from_seed_and_derivation_path(&seed, Some(path))
            .map_err(|e| eyre::eyre!("derive solana keypair: {e}"))?;
        sol.push(kp.pubkey().to_string());
    }
    Ok((evm, sol))
}

pub fn bitcoin_addresses_from_mnemonic(
    mnemonic: &Mnemonic,
    account_indices: &[u32],
) -> eyre::Result<(Vec<String>, Vec<String>)> {
    let seed = mnemonic.to_seed_normalized("");
    let secp = BtcSecp256k1::new();
    let xpriv = BtcXpriv::new_master(BtcNetwork::Bitcoin, &seed).context("btc master xpriv")?;

    let mut mainnet = vec![];
    let mut testnet = vec![];
    for &i in account_indices {
        // BIP84 (native segwit): m/84'/0'/0'/0/i
        let path_s = format!("m/84'/0'/0'/0/{i}");
        let path: BtcDerivationPath = path_s.parse().context("parse btc derivation path")?;
        let child = xpriv
            .derive_priv(&secp, &path)
            .context("derive btc child")?;
        let sk = BtcPrivateKey::new(child.private_key, BtcNetwork::Bitcoin);
        let pk = sk.public_key(&secp);
        let cpk = BtcCompressedPublicKey::try_from(pk).context("btc compressed pubkey")?;
        let addr_main = BtcAddress::p2wpkh(&cpk, BtcKnownHrp::Mainnet);
        let addr_test = BtcAddress::p2wpkh(&cpk, BtcKnownHrp::Testnets);
        mainnet.push(addr_main.to_string());
        testnet.push(addr_test.to_string());
    }
    Ok((mainnet, testnet))
}

pub fn addresses_from_import(
    kind: ImportedKind,
    secret_bytes: &[u8],
) -> eyre::Result<(Vec<String>, Vec<String>, Option<ImportedPrivateKeyChain>)> {
    match kind {
        ImportedKind::Mnemonic => {
            let phrase = std::str::from_utf8(secret_bytes).context("mnemonic must be utf-8")?;
            let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase)
                .context("parse mnemonic")?;
            let (evm, sol) = addresses_from_mnemonic(&mnemonic, &[0])?;
            Ok((evm, sol, None))
        }
        ImportedKind::PrivateKey => {
            // Heuristic: 32 bytes => EVM secp256k1 secret key, 64 bytes => Solana keypair bytes.
            match secret_bytes.len() {
                32 => {
                    let wallet = PrivateKeySigner::from_slice(secret_bytes)
                        .context("parse evm private key")?;
                    Ok((
                        vec![wallet.address().to_checksum(None)],
                        vec![],
                        Some(ImportedPrivateKeyChain::Evm),
                    ))
                }
                64 => {
                    let kp = solana_keypair::Keypair::try_from(secret_bytes)
                        .context("parse solana keypair bytes")?;
                    Ok((
                        vec![],
                        vec![kp.pubkey().to_string()],
                        Some(ImportedPrivateKeyChain::Solana),
                    ))
                }
                _ => eyre::bail!("unsupported private key length"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitcoin_bip84_derivation_vectors_abandon_about() -> eyre::Result<()> {
        // This anchors our BIP84 derivation path and address formatting:
        // m/84'/0'/0'/0/i (P2WPKH, bech32) for mainnet + testnet HRPs.
        let mnemonic = Mnemonic::parse_in_normalized(
            Language::English,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .context("parse mnemonic")?;

        let (mainnet, testnet) = bitcoin_addresses_from_mnemonic(&mnemonic, &[0, 1])?;

        assert_eq!(
            mainnet
                .first()
                .ok_or_else(|| eyre::eyre!("missing mainnet[0]"))?,
            "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
        );
        assert_eq!(
            mainnet
                .get(1)
                .ok_or_else(|| eyre::eyre!("missing mainnet[1]"))?,
            "bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g"
        );
        assert_eq!(
            testnet
                .first()
                .ok_or_else(|| eyre::eyre!("missing testnet[0]"))?,
            "tb1qcr8te4kr609gcawutmrza0j4xv80jy8zmfp6l0"
        );
        assert_eq!(
            testnet
                .get(1)
                .ok_or_else(|| eyre::eyre!("missing testnet[1]"))?,
            "tb1qnjg0jd8228aq7egyzacy8cys3knf9xvrn9d67m"
        );
        Ok(())
    }
}
