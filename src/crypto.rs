//! Secrets at rest (LORVAQ_SECRET_KEY), admin passwords, session tokens, setup codes.

use anyhow::{anyhow, bail, Context};
use argon2::password_hash::rand_core::OsRng as PasswordRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};

const NONCE_LEN: usize = 24;
/// No 0/O, 1/I/L: easy to read off a terminal and type.
pub const SETUP_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// Encrypts secrets kept in the database (GitLab tokens and the like).
#[derive(Clone)]
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

impl SecretBox {
    pub fn from_base64(key: &str) -> anyhow::Result<Self> {
        let bytes = STANDARD
            .decode(key.trim())
            .context("LORVAQ_SECRET_KEY is not valid base64")?;
        if bytes.len() != 32 {
            bail!("LORVAQ_SECRET_KEY must be 32 bytes, got {}", bytes.len());
        }
        Ok(Self {
            cipher: XChaCha20Poly1305::new(Key::from_slice(&bytes)),
        })
    }

    pub fn generate_key() -> String {
        STANDARD.encode(XChaCha20Poly1305::generate_key(&mut OsRng))
    }

    /// base64(nonce || ciphertext).
    pub fn seal(&self, plain: &[u8]) -> String {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let mut out = nonce.to_vec();
        out.extend(
            self.cipher
                .encrypt(&nonce, plain)
                .expect("encryption with a valid key cannot fail"),
        );
        STANDARD.encode(out)
    }

    pub fn open(&self, sealed: &str) -> anyhow::Result<Vec<u8>> {
        let bytes = STANDARD
            .decode(sealed)
            .context("the sealed value is not base64")?;
        if bytes.len() < NONCE_LEN {
            bail!("the sealed value is too short");
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        self.cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .map_err(|_| anyhow!("cannot decrypt: another LORVAQ_SECRET_KEY or a damaged value"))
    }
}

pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut PasswordRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 with default parameters cannot fail")
        .to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| {
        Argon2::default()
            .verify_password(password.as_bytes(), &h)
            .is_ok()
    })
}

/// 32 random bytes, base64url without padding (43 characters).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// What the database keeps instead of a token: hex SHA-256.
pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `XXXX-XXXX-XXXX-XXXX`.
pub fn setup_code() -> String {
    let mut rng = rand::rng();
    let chars: Vec<char> = (0..16)
        .map(|_| SETUP_ALPHABET[rng.random_range(0..SETUP_ALPHABET.len())] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_values_open_with_the_same_key_only() {
        let key = SecretBox::generate_key();
        let a = SecretBox::from_base64(&key).unwrap();
        let sealed = a.seal(b"token");
        assert_ne!(sealed, a.seal(b"token"), "a fresh nonce every time");
        assert_eq!(a.open(&sealed).unwrap(), b"token");
        let b = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
        assert!(b.open(&sealed).is_err());
    }

    #[test]
    fn damaged_values_and_bad_keys_are_errors() {
        let a = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
        let mut sealed = a.seal(b"token").into_bytes();
        let last = sealed.len() - 2;
        sealed[last] = if sealed[last] == b'A' { b'B' } else { b'A' };
        assert!(a.open(&String::from_utf8(sealed).unwrap()).is_err());
        assert!(a.open("c2hvcnQ=").is_err(), "shorter than a nonce");
        assert!(a.open("not base64!").is_err());
        assert!(SecretBox::from_base64("c2hvcnQ=").is_err(), "not 32 bytes");
        assert!(SecretBox::from_base64("%%%").is_err());
    }

    #[test]
    fn passwords_verify_only_themselves() {
        let hash = hash_password("correct horse battery");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("correct horse batterY", &hash));
        assert!(!verify_password("anything", "not a hash"));
    }

    #[test]
    fn tokens_are_random_and_hash_stably() {
        let (a, b) = (random_token(), random_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_eq!(token_hash("x"), token_hash("x"));
        assert_eq!(token_hash("x").len(), 64);
        assert_ne!(token_hash("x"), token_hash("y"));
    }

    #[test]
    fn setup_codes_are_typable() {
        let code = setup_code();
        assert_eq!(code.len(), 19);
        let groups: Vec<&str> = code.split('-').collect();
        assert_eq!(groups.len(), 4);
        assert!(groups
            .iter()
            .all(|g| g.len() == 4 && g.chars().all(|c| SETUP_ALPHABET.contains(&(c as u8)))));
        assert_ne!(code, setup_code());
    }
}
