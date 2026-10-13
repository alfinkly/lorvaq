//! OAuth (authorization code + PKCE) and stored GitLab credentials. Grows in the next task.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// RFC 7636 S256: base64url(sha256(verifier)) without padding.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
