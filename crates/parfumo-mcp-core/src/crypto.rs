//! Encryption at rest (AES-256-GCM), token generation and hashing.
//!
//! Stored blob layout: `nonce (12 bytes) || ciphertext+tag`. The associated data binds a
//! blob to its owner and purpose, so a blob copied onto another user's row fails to decrypt.

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("master key must be 32 bytes encoded in base64")]
    BadKey,
    #[error("encrypted data is corrupt, or was written with another master key or for another user")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
}

#[derive(Clone)]
pub struct MasterKey([u8; 32]);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::fill(&mut b[..]);
    b
}

impl MasterKey {
    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let bytes = STANDARD.decode(s.trim()).or_else(|_| URL_SAFE_NO_PAD.decode(s.trim())).map_err(|_| CryptoError::BadKey)?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| CryptoError::BadKey)?;
        Ok(Self(arr))
    }

    pub fn generate_base64() -> String {
        STANDARD.encode(random_bytes::<32>())
    }

    /// A throw-away key, for runs that never open a stored secret (tests, public canary).
    pub fn random() -> Self {
        Self(random_bytes::<32>())
    }

    pub fn encrypt(&self, plaintext: &[u8], aad: &str) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|_| CryptoError::BadKey)?;
        let nonce = random_bytes::<12>();
        let ct = cipher.encrypt((&nonce).into(), Payload { msg: plaintext, aad: aad.as_bytes() }).map_err(|_| CryptoError::Encrypt)?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn decrypt(&self, blob: &[u8], aad: &str) -> Result<Vec<u8>, CryptoError> {
        if blob.len() < 12 + 16 {
            return Err(CryptoError::Decrypt);
        }
        let (nonce, ct) = blob.split_at(12);
        let nonce: [u8; 12] = nonce.try_into().map_err(|_| CryptoError::Decrypt)?;
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|_| CryptoError::BadKey)?;
        cipher.decrypt((&nonce).into(), Payload { msg: ct, aad: aad.as_bytes() }).map_err(|_| CryptoError::Decrypt)
    }
}

/// Random secret with a recognisable prefix (256 bits of entropy).
pub fn new_secret(prefix: &str) -> String {
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(random_bytes::<32>()))
}

/// High-entropy tokens are stored as SHA-256 (no salt needed: they cannot be guessed).
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// PKCE S256: `BASE64URL(SHA256(verifier))`.
pub fn pkce_s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_fresh_nonce() {
        let k = MasterKey::random();
        let a = k.encrypt(b"hunter2", "user:1:credentials").unwrap();
        let b = k.encrypt(b"hunter2", "user:1:credentials").unwrap();
        assert_ne!(a, b, "nonce must be random");
        assert_eq!(k.decrypt(&a, "user:1:credentials").unwrap(), b"hunter2");
        assert!(!a.windows(7).any(|w| w == b"hunter2"), "plaintext must not appear in the blob");
    }

    #[test]
    fn wrong_key_wrong_user_or_tampering_fails() {
        let k = MasterKey::random();
        let blob = k.encrypt(b"secret", "user:1:credentials").unwrap();
        assert_eq!(MasterKey::random().decrypt(&blob, "user:1:credentials"), Err(CryptoError::Decrypt));
        assert_eq!(k.decrypt(&blob, "user:2:credentials"), Err(CryptoError::Decrypt));
        assert_eq!(k.decrypt(&blob, "user:1:cookies"), Err(CryptoError::Decrypt));
        let mut t = blob.clone();
        *t.last_mut().unwrap() ^= 1;
        assert_eq!(k.decrypt(&t, "user:1:credentials"), Err(CryptoError::Decrypt));
        assert_eq!(k.decrypt(&blob[..10], "user:1:credentials"), Err(CryptoError::Decrypt));
    }

    #[test]
    fn master_key_parsing() {
        assert!(MasterKey::from_base64(&MasterKey::generate_base64()).is_ok());
        assert_eq!(MasterKey::from_base64("dG9vIHNob3J0").unwrap_err(), CryptoError::BadKey);
        assert_eq!(MasterKey::from_base64("not base64 !!").unwrap_err(), CryptoError::BadKey);
        assert_eq!(format!("{:?}", MasterKey::random()), "MasterKey(<redacted>)");
    }

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(pkce_s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn access_keys_are_long_and_random() {
        let t = new_secret("pmcp_");
        assert!(t.starts_with("pmcp_") && t.len() > 40);
        assert_ne!(t, new_secret("pmcp_"));
        assert_eq!(token_hash("a").len(), 64);
    }
}
