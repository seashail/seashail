use eyre::Context as _;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

use bitcoin::address::KnownHrp;
use bitcoin::consensus::encode::serialize;
use bitcoin::hashes::Hash as _;
use bitcoin::secp256k1::{All, Message, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{
    Address, Amount, CompressedPublicKey, Network, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Witness,
};

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

#[derive(Debug, Clone)]
pub struct BitcoinChain {
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AddrStats {
    funded_txo_sum: u64,
    spent_txo_sum: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct AddrResp {
    chain_stats: AddrStats,
    mempool_stats: AddrStats,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
}

impl BitcoinChain {
    pub fn new(base_url: &str) -> eyre::Result<Self> {
        ensure_https_or_loopback(base_url, "bitcoin_api_base_url")?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    fn client() -> eyre::Result<Client> {
        Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("build http client")
    }

    pub async fn get_address_balance_sats(&self, address: &str) -> eyre::Result<(u64, u64)> {
        let client = Self::client()?;
        let url = format!("{}/address/{}", self.base_url, address.trim());
        let resp = client.get(url).send().await.context("fetch btc address")?;
        if !resp.status().is_success() {
            eyre::bail!("bitcoin upstream returned http {}", resp.status());
        }
        let v: AddrResp = resp.json().await.context("decode btc address json")?;
        let confirmed = v
            .chain_stats
            .funded_txo_sum
            .saturating_sub(v.chain_stats.spent_txo_sum);
        let unconfirmed = v
            .mempool_stats
            .funded_txo_sum
            .saturating_sub(v.mempool_stats.spent_txo_sum);
        Ok((confirmed, unconfirmed))
    }

    pub async fn list_utxos(&self, address: &str) -> eyre::Result<Vec<Utxo>> {
        let client = Self::client()?;
        let url = format!("{}/address/{}/utxo", self.base_url, address.trim());
        let resp = client.get(url).send().await.context("fetch btc utxos")?;
        if !resp.status().is_success() {
            eyre::bail!("bitcoin upstream returned http {}", resp.status());
        }
        let v: Vec<Utxo> = resp.json().await.context("decode btc utxos json")?;
        Ok(v)
    }

    pub async fn fee_rate_sats_per_vb(&self) -> eyre::Result<u64> {
        // blockstream: GET /fee-estimates returns a map of confirmation target -> sats/vbyte
        let client = Self::client()?;
        let url = format!("{}/fee-estimates", self.base_url);
        let resp = client
            .get(url)
            .send()
            .await
            .context("fetch btc fee estimates")?;
        if !resp.status().is_success() {
            // Best-effort fallback; don't hard-fail reads.
            return Ok(5);
        }
        let v: serde_json::Value = resp.json().await.context("decode btc fee json")?;
        let fee = v
            .get("5")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(5.0_f64)
            .ceil();
        let rate = crate::financial_math::clamp_fee_rate(fee);
        Ok(rate)
    }

    pub async fn broadcast_tx_hex(&self, tx_hex: &str) -> eyre::Result<String> {
        // blockstream: POST /tx returns txid as text
        let client = Self::client()?;
        let url = format!("{}/tx", self.base_url);
        let resp = client
            .post(url)
            .header("content-type", "text/plain")
            .body(tx_hex.trim().to_owned())
            .send()
            .await
            .context("broadcast btc tx")?;
        if !resp.status().is_success() {
            eyre::bail!("bitcoin broadcast returned http {}", resp.status());
        }
        let txid = resp.text().await.context("read txid")?;
        Ok(txid.trim().to_owned())
    }
}

#[derive(Debug, Clone)]
pub struct SignedSend {
    pub tx_hex: String,
    pub fee_sats: u64,
}

const fn dust_threshold_sats() -> u64 {
    546
}

const fn estimate_vbytes_p2wpkh(inputs: usize, outputs: usize) -> u64 {
    // Conservative estimate for P2WPKH:
    // - overhead: ~10 vB
    // - input: ~68 vB
    // - output: ~31 vB
    10 + 68 * (inputs as u64) + 31 * (outputs as u64)
}

pub fn build_and_sign_p2wpkh_send(
    secp: &Secp256k1<All>,
    network: Network,
    from_key: &bitcoin::PrivateKey,
    to_address: &Address,
    amount_sats: u64,
    fee_rate_sats_per_vb: u64,
    mut utxos: Vec<Utxo>,
) -> eyre::Result<SignedSend> {
    if amount_sats == 0 {
        eyre::bail!("amount must be > 0");
    }

    // Only segwit v0 P2WPKH is supported.
    let pubkey = from_key.public_key(secp);
    let cpk = CompressedPublicKey::try_from(pubkey).context("btc compressed pubkey")?;
    let hrp = match network {
        Network::Bitcoin => KnownHrp::Mainnet,
        Network::Regtest => KnownHrp::Regtest,
        Network::Testnet | Network::Testnet4 | Network::Signet => KnownHrp::Testnets,
    };
    let change_addr = Address::p2wpkh(&cpk, hrp);
    let prev_spk = change_addr.script_pubkey();

    // Deterministic selection: largest first to reduce inputs.
    utxos.sort_by_key(|u| std::cmp::Reverse(u.value));

    let mut selected: Vec<Utxo> = vec![];
    let mut total_in = 0_u64;
    let mut fee_sats = 0_u64;
    let mut change_sats = 0_u64;

    for u in utxos {
        selected.push(u.clone());
        total_in = total_in.saturating_add(u.value);

        // Assume we will include a change output; adjust below if change is dust.
        let vbytes = estimate_vbytes_p2wpkh(selected.len(), 2);
        fee_sats = fee_rate_sats_per_vb.saturating_mul(vbytes);
        if total_in >= amount_sats.saturating_add(fee_sats) {
            change_sats = total_in - amount_sats - fee_sats;
            break;
        }
    }

    if total_in < amount_sats.saturating_add(fee_sats) {
        eyre::bail!("insufficient funds");
    }

    let mut outputs: Vec<TxOut> = vec![TxOut {
        value: Amount::from_sat(amount_sats),
        script_pubkey: to_address.script_pubkey(),
    }];

    if change_sats >= dust_threshold_sats() {
        outputs.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: change_addr.script_pubkey(),
        });
    } else {
        // If change is dust, convert it to fee and omit change output.
        fee_sats = fee_sats.saturating_add(change_sats);
    }

    let mut inputs: Vec<TxIn> = vec![];
    for u in &selected {
        let txid: bitcoin::Txid = u.txid.parse().context("parse utxo txid")?;
        inputs.push(TxIn {
            previous_output: OutPoint { txid, vout: u.vout },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        });
    }

    let mut tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    // Sign each input (BIP143).
    let mut cache = SighashCache::new(&mut tx);

    for (i, u) in selected.iter().enumerate() {
        let sighash = cache
            .p2wpkh_signature_hash(
                i,
                &prev_spk,
                Amount::from_sat(u.value),
                EcdsaSighashType::All,
            )
            .context("compute sighash")?;
        let digest = sighash.to_byte_array();
        let msg = Message::from_digest_slice(&digest).context("sighash to secp message")?;
        let sig = secp.sign_ecdsa(&msg, &from_key.inner);
        let btc_sig = bitcoin::ecdsa::Signature::sighash_all(sig);
        let w = cache
            .witness_mut(i)
            .ok_or_else(|| eyre::eyre!("witness index out of bounds"))?;
        *w = Witness::p2wpkh(&btc_sig, &pubkey.inner);
    }

    let tx_hex = hex::encode(serialize(&tx));

    Ok(SignedSend { tx_hex, fee_sats })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::consensus::encode::deserialize;
    use bitcoin::secp256k1::SecretKey;

    fn txid_hex(n: u64) -> String {
        // 32-byte hex string.
        format!("{n:064x}")
    }

    #[test]
    fn selects_largest_utxo_first_and_includes_change_when_not_dust() -> eyre::Result<()> {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[1_u8; 32]).context("secret key")?;
        let from_key = bitcoin::PrivateKey::new(sk, Network::Bitcoin);

        let to_sk = SecretKey::from_slice(&[2_u8; 32]).context("to secret key")?;
        let to_key = bitcoin::PrivateKey::new(to_sk, Network::Bitcoin);
        let to_pub = to_key.public_key(&secp);
        let to_cpk = CompressedPublicKey::try_from(to_pub).context("to compressed pubkey")?;
        let to = Address::p2wpkh(&to_cpk, KnownHrp::Mainnet);

        let utxos = vec![
            Utxo {
                txid: txid_hex(1),
                vout: 0,
                value: 5_000,
            },
            Utxo {
                txid: txid_hex(2),
                vout: 1,
                value: 20_000,
            },
        ];

        let signed =
            build_and_sign_p2wpkh_send(&secp, Network::Bitcoin, &from_key, &to, 10_000, 1, utxos)?;

        let tx: Transaction = deserialize(&hex::decode(&signed.tx_hex)?).context("decode tx")?;
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 2);
        // Ensure we used the largest UTXO (txid=2) first.
        let first = tx
            .input
            .first()
            .ok_or_else(|| eyre::eyre!("missing tx input"))?;
        assert_eq!(first.previous_output.txid.to_string(), txid_hex(2));
        Ok(())
    }

    #[test]
    fn omits_dust_change_and_adds_it_to_fee() -> eyre::Result<()> {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[1_u8; 32]).context("secret key")?;
        let from_key = bitcoin::PrivateKey::new(sk, Network::Bitcoin);

        let to_sk = SecretKey::from_slice(&[3_u8; 32]).context("to secret key")?;
        let to_key = bitcoin::PrivateKey::new(to_sk, Network::Bitcoin);
        let to_pub = to_key.public_key(&secp);
        let to_cpk = CompressedPublicKey::try_from(to_pub).context("to compressed pubkey")?;
        let to = Address::p2wpkh(&to_cpk, KnownHrp::Mainnet);

        let utxos = vec![Utxo {
            txid: txid_hex(10),
            vout: 0,
            value: 12_000,
        }];

        // With fee_rate=1 and 1 input + 2 outputs estimate, fee=140. Change=60 (dust) so it is
        // added to fee and change output is omitted.
        let signed =
            build_and_sign_p2wpkh_send(&secp, Network::Bitcoin, &from_key, &to, 11_800, 1, utxos)?;
        assert_eq!(signed.fee_sats, 200);

        let tx: Transaction = deserialize(&hex::decode(&signed.tx_hex)?).context("decode tx")?;
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 1);
        Ok(())
    }
}
