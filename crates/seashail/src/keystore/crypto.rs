use aes_gcm::{
    aead::{Aead as _, KeyInit as _},
    Aes256Gcm, Nonce,
};
use argon2::{
    password_hash::{PasswordHasher as _, SaltString},
    Algorithm, Argon2, Params, Version,
};
use base64::Engine as _;
use eyre::Context as _;
use hkdf::Hkdf;
use rand::Rng as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoBox {
    pub v: u8,
    pub nonce_b64: String,
    pub ct_b64: String,
}

pub fn fill_random(buf: &mut [u8]) {
    let mut rng = rand::rng();
    rng.fill_bytes(buf);
}

pub fn random_salt16() -> [u8; 16] {
    let mut s = [0_u8; 16];
    fill_random(&mut s);
    s
}

pub fn derive_passphrase_key(
    passphrase: &SecretString,
    salt16: &[u8; 16],
) -> eyre::Result<[u8; 32]> {
    // Freeze Argon2id parameters to avoid accidental changes across dependency updates.
    // These match `argon2::Params::DEFAULT` in argon2 0.5.x.
    let params =
        Params::new(19 * 1024, 2, 1, Some(32)).map_err(|e| eyre::eyre!("argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::encode_b64(salt16).map_err(|e| eyre::eyre!("encode salt: {e}"))?;
    let mut out = [0_u8; 32];

    // We use a PHC hash but only take the raw bytes; this keeps parameters versioned.
    let hash = argon2
        .hash_password(passphrase.expose_secret().as_bytes(), &salt)
        .map_err(|e| eyre::eyre!("argon2 hash: {e}"))?;
    let bytes = hash
        .hash
        .ok_or_else(|| eyre::eyre!("argon2 missing hash"))?;
    let raw = bytes.as_bytes();
    if raw.len() < 32 {
        eyre::bail!("argon2 hash too short");
    }
    let Some(prefix) = raw.get(..32) else {
        eyre::bail!("argon2 hash too short");
    };
    out.copy_from_slice(prefix);
    Ok(out)
}

pub fn derive_subkey_machine(
    machine_secret: &[u8; 32],
    wallet_id: &str,
    purpose: &str,
) -> eyre::Result<[u8; 32]> {
    derive_subkey(machine_secret, wallet_id, purpose)
}

pub fn derive_subkey_passphrase(
    base_key: &[u8; 32],
    wallet_id: &str,
    purpose: &str,
) -> eyre::Result<[u8; 32]> {
    derive_subkey(base_key, wallet_id, purpose)
}

fn derive_subkey(master: &[u8; 32], wallet_id: &str, purpose: &str) -> eyre::Result<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, master);
    let info = format!("seashail:{wallet_id}:{purpose}");
    let mut out = [0_u8; 32];
    hk.expand(info.as_bytes(), &mut out)
        .map_err(|e| eyre::eyre!("hkdf expand: {e}"))?;
    Ok(out)
}

pub fn encrypt_aes_gcm(key32: &[u8; 32], plaintext: &[u8]) -> eyre::Result<CryptoBox> {
    let cipher = Aes256Gcm::new_from_slice(key32).context("aes init")?;
    let mut nonce = [0_u8; 12];
    fill_random(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| eyre::eyre!("aes encrypt: {e}"))?;

    Ok(CryptoBox {
        v: 1,
        nonce_b64: base64::engine::general_purpose::STANDARD.encode(nonce),
        ct_b64: base64::engine::general_purpose::STANDARD.encode(ct),
    })
}

pub fn decrypt_aes_gcm(key32: &[u8; 32], b: &CryptoBox) -> eyre::Result<Vec<u8>> {
    if b.v != 1 {
        eyre::bail!("unsupported CryptoBox version: {}", b.v);
    }
    let cipher = Aes256Gcm::new_from_slice(key32).context("aes init")?;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(&b.nonce_b64)
        .context("decode nonce")?;
    if nonce.len() != 12 {
        eyre::bail!("invalid nonce length");
    }
    let ct = base64::engine::general_purpose::STANDARD
        .decode(&b.ct_b64)
        .context("decode ciphertext")?;

    let pt = cipher
        .decrypt(Nonce::from_slice(&nonce), ct.as_ref())
        .map_err(|e| eyre::eyre!("aes decrypt: {e}"))?;
    Ok(pt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eyre::ContextCompat as _;

    #[test]
    fn aes_gcm_roundtrip() -> eyre::Result<()> {
        let key = [7_u8; 32];
        let pt = b"test plaintext".to_vec();
        let boxv = encrypt_aes_gcm(&key, &pt).context("encrypt")?;
        let out = decrypt_aes_gcm(&key, &boxv).context("decrypt")?;
        assert_eq!(out, pt);
        Ok(())
    }

    #[test]
    fn aes_gcm_wrong_key_fails() -> eyre::Result<()> {
        let key = [7_u8; 32];
        let pt = b"test plaintext".to_vec();
        let boxv = encrypt_aes_gcm(&key, &pt).context("encrypt")?;
        let wrong = [8_u8; 32];
        let err = decrypt_aes_gcm(&wrong, &boxv)
            .err()
            .context("wrong key must fail")?;
        assert!(err.to_string().contains("aes decrypt"));
        Ok(())
    }

    #[test]
    fn derive_passphrase_key_is_deterministic_for_same_inputs() -> eyre::Result<()> {
        let passphrase = SecretString::new("correct horse battery staple".to_owned().into());
        let salt = [1_u8; 16];
        let k1 = derive_passphrase_key(&passphrase, &salt).context("k1")?;
        let k2 = derive_passphrase_key(&passphrase, &salt).context("k2")?;
        assert_eq!(k1, k2);
        Ok(())
    }
}
