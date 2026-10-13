//! How lorvaq holds a GitLab connection: a pasted token, or OAuth (authorization
//! code + PKCE) with refresh. Every secret is sealed before it reaches the database.

use super::client::{decode, network, GitlabError};
use crate::crypto::{self, SecretBox};
use crate::store::Store;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CONNECTION_KEY: &str = "gitlab";
pub const PENDING_KEY: &str = "gitlab_oauth";
pub const SCOPE: &str = "read_api";
/// An OAuth round trip that took longer is abandoned.
pub const PENDING_TTL_MS: i64 = 10 * 60 * 1000;
/// An access token with less left than this is refreshed first.
pub const REFRESH_BEFORE_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub url: String,
    pub user: String,
    pub auth: StoredAuth,
    pub connected_ms: i64,
}

/// Every string here except `client_id` and `redirect_uri` is sealed with `SecretBox`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredAuth {
    Token {
        token: String,
    },
    Oauth {
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
        verifier: String,
        access: String,
        refresh: String,
        expires_ms: i64,
    },
}

impl StoredAuth {
    pub fn kind(&self) -> &'static str {
        match self {
            StoredAuth::Token { .. } => "token",
            StoredAuth::Oauth { .. } => "oauth",
        }
    }
}

/// An OAuth round trip in progress: what the callback needs to finish it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub state_hash: String,
    pub url: String,
    pub client_id: String,
    /// Sealed.
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    /// Sealed.
    pub verifier: String,
    pub created_ms: i64,
}

pub fn pkce_verifier() -> String {
    crypto::random_token()
}

/// RFC 7636 S256: base64url(sha256(verifier)) without padding.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn authorize_url(
    base: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> String {
    let mut url = reqwest::Url::parse(&format!("{base}/oauth/authorize"))
        .expect("the base address was validated");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    url.into()
}

/// GitLab's "new personal access token" page with the name and scope filled in.
pub fn token_page_url(base: &str) -> String {
    format!("{base}/-/user_settings/personal_access_tokens?name=lorvaq&scopes={SCOPE}")
}

pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_ms: i64,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_ms", &self.expires_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

pub struct OauthClient<'a> {
    pub http: &'a reqwest::Client,
    pub base: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub redirect_uri: &'a str,
}

impl OauthClient<'_> {
    pub async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        now_ms: i64,
    ) -> Result<Tokens, GitlabError> {
        self.token(
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", verifier),
            ],
            now_ms,
        )
        .await
    }

    pub async fn refresh(
        &self,
        refresh_token: &str,
        verifier: &str,
        now_ms: i64,
    ) -> Result<Tokens, GitlabError> {
        self.token(
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("code_verifier", verifier),
            ],
            now_ms,
        )
        .await
    }

    async fn token(&self, grant: &[(&str, &str)], now_ms: i64) -> Result<Tokens, GitlabError> {
        let mut form: Vec<(&str, &str)> = vec![
            ("client_id", self.client_id),
            ("redirect_uri", self.redirect_uri),
        ];
        form.extend_from_slice(grant);
        if let Some(secret) = self.client_secret {
            form.push(("client_secret", secret));
        }
        let res = self
            .http
            .post(format!("{}/oauth/token", self.base))
            .form(&form)
            .send()
            .await
            .map_err(network)?;
        match res.status().as_u16() {
            400 | 401 => return Err(GitlabError::Unauthorized),
            code if !(200..300).contains(&code) => return Err(GitlabError::Status(code)),
            _ => {}
        }
        let t: TokenResponse = res.json().await.map_err(decode)?;
        Ok(Tokens {
            access: t.access_token,
            refresh: t.refresh_token.unwrap_or_default(),
            expires_ms: now_ms + t.expires_in.unwrap_or(7200) * 1000,
        })
    }
}

fn open(secrets: &SecretBox, sealed: &str) -> Result<String, GitlabError> {
    let bytes = secrets
        .open(sealed)
        .map_err(|_| GitlabError::Local("the stored GitLab credentials cannot be decrypted; was LORVAQ_SECRET_KEY changed? Connect GitLab again".into()))?;
    String::from_utf8(bytes).map_err(|_| {
        GitlabError::Local("the stored GitLab credentials are damaged; connect GitLab again".into())
    })
}

/// The token to call GitLab with. An OAuth token about to expire is refreshed and
/// the new pair stored before it is used (GitLab rotates refresh tokens).
pub async fn bearer(
    conn: &mut Connection,
    secrets: &SecretBox,
    http: &reqwest::Client,
    store: &Store,
    now_ms: i64,
) -> Result<String, GitlabError> {
    let url = conn.url.clone();
    match &mut conn.auth {
        StoredAuth::Token { token } => open(secrets, token),
        StoredAuth::Oauth {
            client_id,
            client_secret,
            redirect_uri,
            verifier,
            access,
            refresh,
            expires_ms,
        } => {
            if *expires_ms - now_ms > REFRESH_BEFORE_MS {
                return open(secrets, access);
            }
            let secret = client_secret
                .as_deref()
                .map(|s| open(secrets, s))
                .transpose()?;
            let client = OauthClient {
                http,
                base: &url,
                client_id,
                client_secret: secret.as_deref(),
                redirect_uri,
            };
            let tokens = client
                .refresh(&open(secrets, refresh)?, &open(secrets, verifier)?, now_ms)
                .await?;
            *access = secrets.seal(tokens.access.as_bytes());
            *refresh = secrets.seal(tokens.refresh.as_bytes());
            *expires_ms = tokens.expires_ms;
            store
                .put_setting(CONNECTION_KEY, &*conn)
                .await
                .map_err(|e| {
                    GitlabError::Local(format!("cannot save the refreshed GitLab token: {e:#}"))
                })?;
            Ok(tokens.access)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::gitlab::client::http_client;
    use crate::sources::gitlab::fake::FakeGitlab;

    #[test]
    fn pkce_matches_the_rfc_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let v = pkce_verifier();
        assert!((43..=128).contains(&v.len()));
        assert_ne!(v, pkce_verifier());
    }

    #[test]
    fn the_authorize_url_carries_everything() {
        let url = authorize_url(
            "https://gitlab.example.com",
            "app id",
            "https://lorvaq.example.com/api/v1/oauth/gitlab/callback",
            "st",
            "ch",
        );
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.path(), "/oauth/authorize");
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "app id");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["scope"], "read_api");
        assert_eq!(
            (
                q["state"].as_str(),
                q["code_challenge"].as_str(),
                q["code_challenge_method"].as_str()
            ),
            ("st", "ch", "S256")
        );
        assert_eq!(
            q["redirect_uri"],
            "https://lorvaq.example.com/api/v1/oauth/gitlab/callback"
        );
        assert_eq!(token_page_url("https://gitlab.example.com"), "https://gitlab.example.com/-/user_settings/personal_access_tokens?name=lorvaq&scopes=read_api");
    }

    #[tokio::test]
    async fn code_exchange_needs_the_right_verifier_and_refresh_rotates() {
        let fake = FakeGitlab::start().await;
        let verifier = pkce_verifier();
        fake.state.lock().unwrap().challenge = Some(pkce_challenge(&verifier));
        let http = http_client();
        let client = OauthClient {
            http: &http,
            base: &fake.url,
            client_id: "app-id",
            client_secret: None,
            redirect_uri: "http://l/cb",
        };
        assert_eq!(
            client
                .exchange("the-code", "wrong verifier", 0)
                .await
                .unwrap_err(),
            GitlabError::Unauthorized
        );
        let first = client.exchange("the-code", &verifier, 1_000).await.unwrap();
        assert_eq!(
            (
                first.access.as_str(),
                first.refresh.as_str(),
                first.expires_ms
            ),
            ("access-1", "refresh-1", 1_000 + 7_200_000)
        );
        let second = client
            .refresh(&first.refresh, &verifier, 2_000)
            .await
            .unwrap();
        assert_eq!(second.access, "access-2");
        assert_eq!(
            client
                .refresh(&first.refresh, &verifier, 3_000)
                .await
                .unwrap_err(),
            GitlabError::Unauthorized,
            "the old refresh token is spent"
        );
    }
}
