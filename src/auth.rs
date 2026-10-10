//! Who may see the scenes, how hard one may guess the password, and the
//! one-time code that creates the admin on a fresh install.

use crate::crypto;
use crate::store::Store;
use axum::http::{HeaderMap, HeaderName};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

pub const ACCESS_KEY: &str = "access";
pub const SETUP_CODE_KEY: &str = "setup_code";
const SETUP_TTL_MS: i64 = 24 * 3600 * 1000;
const WINDOW_MS: i64 = 15 * 60 * 1000;
const MAX_FAILURES: usize = 5;
const MAX_TRACKED: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Access {
    /// Anyone who can reach lorvaq sees the scenes.
    Open,
    /// A reverse proxy in front signs viewers in and passes `header`; only
    /// requests coming from `trusted` addresses count.
    Proxy {
        header: String,
        trusted: Vec<String>,
    },
    /// Only the signed-in admin.
    #[default]
    Admin,
}

fn parse_net(s: &str) -> Option<IpNet> {
    let s = s.trim();
    s.parse::<IpNet>()
        .ok()
        .or_else(|| s.parse::<IpAddr>().ok().map(IpNet::from))
}

impl Access {
    pub fn validate(&self) -> Result<(), String> {
        if let Access::Proxy { header, trusted } = self {
            if header.is_empty() || HeaderName::from_bytes(header.as_bytes()).is_err() {
                return Err(format!("{header:?} is not a valid header name"));
            }
            if trusted.is_empty() {
                return Err("list at least one trusted proxy address".into());
            }
            if let Some(bad) = trusted.iter().find(|t| parse_net(t).is_none()) {
                return Err(format!("{bad:?} is neither an address nor a network"));
            }
        }
        Ok(())
    }

    pub fn viewer_allowed(&self, peer: Option<IpAddr>, headers: &HeaderMap, admin: bool) -> bool {
        if admin {
            return true;
        }
        match self {
            Access::Open => true,
            Access::Admin => false,
            Access::Proxy { header, trusted } => {
                let from_proxy = peer.is_some_and(|ip| {
                    trusted
                        .iter()
                        .filter_map(|t| parse_net(t))
                        .any(|n| n.contains(&ip))
                });
                from_proxy
                    && headers
                        .get(header.as_str())
                        .is_some_and(|v| !v.as_bytes().is_empty())
            }
        }
    }
}

#[derive(Default)]
pub struct LoginLimiter {
    failures: HashMap<IpAddr, Vec<i64>>,
}

impl LoginLimiter {
    pub fn blocked(&mut self, ip: IpAddr, now_ms: i64) -> bool {
        match self.failures.get_mut(&ip) {
            Some(times) => {
                times.retain(|t| now_ms - t < WINDOW_MS);
                times.len() >= MAX_FAILURES
            }
            None => false,
        }
    }

    pub fn fail(&mut self, ip: IpAddr, now_ms: i64) {
        if self.failures.len() >= MAX_TRACKED {
            self.failures
                .retain(|_, times| times.last().is_some_and(|t| now_ms - t < WINDOW_MS));
        }
        self.failures.entry(ip).or_default().push(now_ms);
    }

    pub fn reset(&mut self, ip: IpAddr) {
        self.failures.remove(&ip);
    }
}

#[derive(Serialize, Deserialize)]
struct SetupCode {
    hash: String,
    expires_ms: i64,
}

fn normalize(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase()
}

/// A new code replaces any older one.
pub async fn issue_setup_code(store: &Store, now_ms: i64) -> anyhow::Result<String> {
    let code = crypto::setup_code();
    let saved = SetupCode {
        hash: crypto::token_hash(&normalize(&code)),
        expires_ms: now_ms + SETUP_TTL_MS,
    };
    store.put_setting(SETUP_CODE_KEY, &saved).await?;
    Ok(code)
}

pub async fn check_setup_code(store: &Store, code: &str, now_ms: i64) -> anyhow::Result<bool> {
    Ok(match store.setting::<SetupCode>(SETUP_CODE_KEY).await? {
        Some(saved) => {
            saved.expires_ms > now_ms && saved.hash == crypto::token_hash(&normalize(code))
        }
        None => false,
    })
}

/// On a fresh install there is no admin: print a one-time code to create one.
pub async fn announce_setup_code(store: &Store, now_ms: i64) -> anyhow::Result<()> {
    if store.admin_hash().await?.is_none() {
        let code = issue_setup_code(store, now_ms).await?;
        tracing::warn!("no admin yet: open lorvaq and enter the one-time setup code {code} (valid for 24 hours)");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn proxy(trusted: &[&str]) -> Access {
        Access::Proxy {
            header: "x-forwarded-user".into(),
            trusted: trusted.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn with_user() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-user", HeaderValue::from_static("dana"));
        h
    }

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn json_shape_is_the_api_contract() {
        assert_eq!(
            serde_json::to_value(Access::Admin).unwrap(),
            serde_json::json!({ "mode": "admin" })
        );
        assert_eq!(Access::default(), Access::Admin);
        let p: Access = serde_json::from_value(
            serde_json::json!({ "mode": "proxy", "header": "x-u", "trusted": ["10.0.0.1"] }),
        )
        .unwrap();
        assert_eq!(
            p,
            Access::Proxy {
                header: "x-u".into(),
                trusted: vec!["10.0.0.1".into()]
            }
        );
    }

    #[test]
    fn validation() {
        assert!(Access::Open.validate().is_ok());
        assert!(proxy(&["10.0.0.0/8", "192.168.1.5", "::1"])
            .validate()
            .is_ok());
        assert!(proxy(&[]).validate().is_err(), "no trusted proxy");
        assert!(proxy(&["10.0.0.0/33"]).validate().is_err());
        assert!(
            proxy(&["proxy.local"]).validate().is_err(),
            "names are not addresses"
        );
        let bad_header = Access::Proxy {
            header: "bad header".into(),
            trusted: vec!["10.0.0.1".into()],
        };
        assert!(bad_header.validate().is_err());
        let empty_header = Access::Proxy {
            header: String::new(),
            trusted: vec!["10.0.0.1".into()],
        };
        assert!(empty_header.validate().is_err());
    }

    #[test]
    fn viewer_matrix() {
        let none = HeaderMap::new();
        assert!(Access::Open.viewer_allowed(None, &none, false));
        assert!(!Access::Admin.viewer_allowed(ip("10.0.0.1"), &with_user(), false));
        assert!(
            Access::Admin.viewer_allowed(None, &none, true),
            "the admin always sees"
        );
        let p = proxy(&["10.0.0.0/8"]);
        assert!(p.viewer_allowed(ip("10.1.2.3"), &with_user(), false));
        assert!(
            !p.viewer_allowed(ip("10.1.2.3"), &none, false),
            "no user header"
        );
        assert!(
            !p.viewer_allowed(ip("203.0.113.9"), &with_user(), false),
            "header from a stranger"
        );
        assert!(!p.viewer_allowed(None, &with_user(), false), "unknown peer");
        let mut empty = HeaderMap::new();
        empty.insert("x-forwarded-user", HeaderValue::from_static(""));
        assert!(!p.viewer_allowed(ip("10.1.2.3"), &empty, false));
    }

    #[test]
    fn five_failures_in_fifteen_minutes_block_the_address() {
        let mut l = LoginLimiter::default();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        for t in 0..5 {
            assert!(!l.blocked(a, t));
            l.fail(a, t);
        }
        assert!(l.blocked(a, 10));
        assert!(!l.blocked(b, 10), "other addresses are fine");
        assert!(!l.blocked(a, 15 * 60 * 1000 + 5), "the window slides");
        l.fail(a, 0);
        l.reset(a);
        assert!(!l.blocked(a, 1));
    }

    #[test]
    fn setup_codes_ignore_case_and_dashes() {
        assert_eq!(normalize(" abcd-efgh-2345 6789 "), "ABCDEFGH23456789");
    }
}
