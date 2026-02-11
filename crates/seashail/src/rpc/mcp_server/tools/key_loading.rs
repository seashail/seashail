use crate::wallet::{ImportedKind, WalletKind};
use alloy::signers::local::PrivateKeySigner;
use eyre::Context as _;
use zeroize::Zeroize as _;

use super::super::elicitation::ensure_unlocked;
use super::super::{ConnState, SharedState};

pub async fn load_evm_signer<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<tokio::io::BufReader<R>>,
    stdout: &mut W,
    w: &crate::wallet::WalletRecord,
    account_index: u32,
) -> eyre::Result<PrivateKeySigner>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match w.kind {
        WalletKind::Generated => {
            let mut entropy = if shared.ks.generated_wallet_needs_passphrase(&w.id) {
                let key = ensure_unlocked(shared, conn, stdin, stdout).await?;
                shared.ks.decrypt_generated_entropy(&w.id, &key)?
            } else {
                shared.ks.decrypt_generated_entropy_no_passphrase(&w.id)?
            };
            let mnemonic = bip39::Mnemonic::from_entropy_in(bip39::Language::English, &entropy)
                .context("mnemonic from entropy")?;
            entropy.zeroize();
            let phrase = mnemonic.to_string();
            let wallet = alloy::signers::local::MnemonicBuilder::<
                alloy::signers::local::coins_bip39::English,
            >::default()
            .phrase(phrase.as_str())
            .index(account_index)
            .context("evm index")?
            .build()
            .context("build evm wallet")?;
            Ok(wallet)
        }
        WalletKind::Imported => {
            let key = ensure_unlocked(shared, conn, stdin, stdout).await?;
            let kind = w
                .imported_kind
                .ok_or_else(|| eyre::eyre!("missing imported_kind"))?;
            let mut secret = shared.ks.decrypt_imported_secret(&w.id, &key)?;
            let out = match kind {
                ImportedKind::PrivateKey => {
                    PrivateKeySigner::from_slice(&secret).context("parse evm private key")?
                }
                ImportedKind::Mnemonic => {
                    let phrase =
                        std::str::from_utf8(&secret).context("imported mnemonic must be utf-8")?;
                    alloy::signers::local::MnemonicBuilder::<
                        alloy::signers::local::coins_bip39::English,
                    >::default()
                    .phrase(phrase)
                    .index(account_index)
                    .context("evm index")?
                    .build()
                    .context("build evm wallet")?
                }
            };
            secret.zeroize();
            Ok(out)
        }
    }
}

pub async fn load_solana_keypair<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<tokio::io::BufReader<R>>,
    stdout: &mut W,
    w: &crate::wallet::WalletRecord,
    account_index: u32,
) -> eyre::Result<solana_sdk::signature::Keypair>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match w.kind {
        WalletKind::Generated => {
            let mut entropy = if shared.ks.generated_wallet_needs_passphrase(&w.id) {
                let key = ensure_unlocked(shared, conn, stdin, stdout).await?;
                shared.ks.decrypt_generated_entropy(&w.id, &key)?
            } else {
                shared.ks.decrypt_generated_entropy_no_passphrase(&w.id)?
            };
            let mnemonic = bip39::Mnemonic::from_entropy_in(bip39::Language::English, &entropy)
                .context("mnemonic from entropy")?;
            entropy.zeroize();
            let phrase = mnemonic.to_string();
            let seed =
                solana_seed_phrase::generate_seed_from_seed_phrase_and_passphrase(&phrase, "");
            let path =
                solana_derivation_path::DerivationPath::new_bip44(Some(account_index), Some(0));
            let kp = solana_keypair::seed_derivable::keypair_from_seed_and_derivation_path(
                &seed,
                Some(path),
            )
            .map_err(|e| eyre::eyre!("derive solana keypair: {e}"))?;
            Ok(kp)
        }
        WalletKind::Imported => {
            let key = ensure_unlocked(shared, conn, stdin, stdout).await?;
            let kind = w
                .imported_kind
                .ok_or_else(|| eyre::eyre!("missing imported_kind"))?;
            let mut secret = shared.ks.decrypt_imported_secret(&w.id, &key)?;
            let out = match kind {
                ImportedKind::PrivateKey => {
                    let kp = solana_keypair::Keypair::try_from(secret.as_slice())
                        .context("parse solana keypair bytes")?;
                    kp
                }
                ImportedKind::Mnemonic => {
                    let phrase =
                        std::str::from_utf8(&secret).context("imported mnemonic must be utf-8")?;
                    let seed = solana_seed_phrase::generate_seed_from_seed_phrase_and_passphrase(
                        phrase, "",
                    );
                    let path = solana_derivation_path::DerivationPath::new_bip44(
                        Some(account_index),
                        Some(0),
                    );
                    let kp = solana_keypair::seed_derivable::keypair_from_seed_and_derivation_path(
                        &seed,
                        Some(path),
                    )
                    .map_err(|e| eyre::eyre!("derive solana keypair: {e}"))?;
                    kp
                }
            };
            secret.zeroize();
            Ok(out)
        }
    }
}

pub async fn load_bitcoin_privkey<R, W>(
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<tokio::io::BufReader<R>>,
    stdout: &mut W,
    w: &crate::wallet::WalletRecord,
    account_index: u32,
) -> eyre::Result<bitcoin::PrivateKey>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let secp = bitcoin::secp256k1::Secp256k1::new();

    let derive = |mnemonic: &bip39::Mnemonic| -> eyre::Result<bitcoin::PrivateKey> {
        let seed = mnemonic.to_seed_normalized("");
        let xpriv = bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Bitcoin, &seed)
            .context("btc master xpriv")?;
        let path_s = format!("m/84'/0'/0'/0/{account_index}");
        let path: bitcoin::bip32::DerivationPath =
            path_s.parse().context("parse btc derivation path")?;
        let child = xpriv
            .derive_priv(&secp, &path)
            .context("derive btc child")?;
        Ok(bitcoin::PrivateKey::new(
            child.private_key,
            bitcoin::Network::Bitcoin,
        ))
    };

    match w.kind {
        WalletKind::Generated => {
            let mut entropy = if shared.ks.generated_wallet_needs_passphrase(&w.id) {
                let key = ensure_unlocked(shared, conn, stdin, stdout).await?;
                shared.ks.decrypt_generated_entropy(&w.id, &key)?
            } else {
                shared.ks.decrypt_generated_entropy_no_passphrase(&w.id)?
            };
            let mnemonic = bip39::Mnemonic::from_entropy_in(bip39::Language::English, &entropy)
                .context("mnemonic from entropy")?;
            entropy.zeroize();
            derive(&mnemonic)
        }
        WalletKind::Imported => {
            let key = ensure_unlocked(shared, conn, stdin, stdout).await?;
            let kind = w
                .imported_kind
                .ok_or_else(|| eyre::eyre!("missing imported_kind"))?;
            let mut secret = shared.ks.decrypt_imported_secret(&w.id, &key)?;
            let out = match kind {
                ImportedKind::Mnemonic => {
                    let phrase =
                        std::str::from_utf8(&secret).context("imported mnemonic must be utf-8")?;
                    let mnemonic =
                        bip39::Mnemonic::parse_in_normalized(bip39::Language::English, phrase)
                            .context("parse imported mnemonic")?;
                    derive(&mnemonic)?
                }
                ImportedKind::PrivateKey => {
                    eyre::bail!("bitcoin private key import not supported")
                }
            };
            secret.zeroize();
            Ok(out)
        }
    }
}
