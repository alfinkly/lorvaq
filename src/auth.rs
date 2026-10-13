//! Who may see the scenes, how hard one may guess the password, and the
//! one-time code that creates the admin on a fresh install.

use crate::crypto;
use crate::store::Store;
use axum::http::{HeaderMap, HeaderName};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

pub const ACCESS_KEY: &str = "access";
pub const SETUP_CODE_KEY: &str = "setup_code";
const SETUP_TTL_MS: i64 = 24 * 3600 * 1000;
const WINDOW_MS: i64 = 15 * 60 * 1000;
const MAX_FAILURES: usize = 5;
pub(crate) const MAX_TRACKED: usize = 20_000;

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

/// Header a reverse proxy appends the client address to.
const FORWARDED_FOR: &str = "x-forwarded-for";

fn is_trusted(trusted: &[String], ip: IpAddr) -> bool {
    trusted
        .iter()
        .filter_map(|t| parse_net(t))
        .any(|n| n.contains(&ip))
}

/// IPv6 clients usually own a whole /64, so they count as one address.
fn fold_v6(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        v4 => v4,
    }
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
                let from_proxy = peer.is_some_and(|ip| is_trusted(trusted, ip));
                from_proxy
                    && headers
                        .get(header.as_str())
                        .is_some_and(|v| !v.as_bytes().is_empty())
            }
        }
    }
}

/// One `X-Forwarded-For` hop: an address, optionally with a port (`1.2.3.4:80`, `[::1]:80`).
fn parse_hop(hop: &[u8]) -> Option<IpAddr> {
    let text = std::str::from_utf8(hop).ok()?.trim();
    text.parse::<SocketAddr>()
        .map(|a| a.ip())
        .or_else(|_| text.parse::<IpAddr>())
        .ok()
}

impl Access {
    /// Whom a sign-in attempt counts against. Behind a trusted proxy that is the
    /// rightmost `X-Forwarded-For` hop that is not itself a trusted proxy; otherwise
    /// the TCP peer. IPv6 addresses count by their /64.
    pub fn limiter_key(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> IpAddr {
        let peer_key = fold_v6(peer.unwrap_or(IpAddr::from([0, 0, 0, 0])));
        let (Access::Proxy { trusted, .. }, Some(peer)) = (self, peer) else {
            return peer_key;
        };
        if !is_trusted(trusted, peer) {
            return peer_key;
        }
        for value in headers.get_all(FORWARDED_FOR).iter().rev() {
            // By bytes: a forged non-ASCII hop on the left must not hide the hops the proxy appended.
            for hop in value.as_bytes().rsplit(|&b| b == b',') {
                let Some(ip) = parse_hop(hop) else {
                    return peer_key;
                };
                let ip = ip.to_canonical();
                if !is_trusted(trusted, ip) {
                    return fold_v6(ip);
                }
            }
        }
        peer_key
    }
}

/// Five attempts per address in 15 minutes. An attempt is counted when it starts,
/// before the expensive password check, and forgotten once it succeeds.
#[derive(Default)]
pub struct LoginLimiter {
    attempts: HashMap<IpAddr, Vec<i64>>,
}

impl LoginLimiter {
    /// Counts an attempt; false (and nothing counted) when the address is used up.
    pub fn try_begin(&mut self, key: IpAddr, now_ms: i64) -> bool {
        if self.attempts.len() >= MAX_TRACKED {
            self.attempts
                .retain(|_, times| times.last().is_some_and(|t| now_ms - t < WINDOW_MS));
        }
        if self.attempts.len() >= MAX_TRACKED && !self.attempts.contains_key(&key) {
            // Still full of recent addresses: forget the one that tried longest ago.
            if let Some(oldest) = self
                .attempts
                .iter()
                .min_by_key(|(_, times)| times.last().copied().unwrap_or(i64::MIN))
                .map(|(k, _)| *k)
            {
                self.attempts.remove(&oldest);
            }
        }
        let times = self.attempts.entry(key).or_default();
        times.retain(|t| now_ms - t < WINDOW_MS);
        if times.len() >= MAX_FAILURES {
            return false;
        }
        times.push(now_ms);
        true
    }

    /// Takes back the attempt counted at `at_ms`: it was not a password guess.
    pub fn release(&mut self, key: IpAddr, at_ms: i64) {
        if let Some(times) = self.attempts.get_mut(&key) {
            if let Some(i) = times.iter().rposition(|t| *t == at_ms) {
                times.remove(i);
            }
            if times.is_empty() {
                self.attempts.remove(&key);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.attempts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attempts.is_empty()
    }

    pub fn marks(&self, key: IpAddr) -> Vec<i64> {
        self.attempts.get(&key).cloned().unwrap_or_default()
    }

    pub fn reset(&mut self, key: IpAddr) {
        self.attempts.remove(&key);
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

/// On a fresh install there is no admin: print a one-time code to create one. A code
/// that is still valid (from a first start or `reset-password`) is kept, not replaced.
pub async fn announce_setup_code(store: &Store, now_ms: i64) -> anyhow::Result<()> {
    if store.admin_hash().await?.is_some() {
        return Ok(());
    }
    if store
        .setting::<SetupCode>(SETUP_CODE_KEY)
        .await?
        .is_some_and(|c| c.expires_ms > now_ms)
    {
        tracing::warn!("no admin yet: use the setup code printed earlier, or run `lorvaq admin reset-password` for a new one");
        return Ok(());
    }
    let code = issue_setup_code(store, now_ms).await?;
    tracing::warn!(
        "no admin yet: open lorvaq and enter the one-time setup code {code} (valid for 24 hours)"
    );
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
    fn five_attempts_in_fifteen_minutes_block_the_address() {
        let mut l = LoginLimiter::default();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        for t in 0..5 {
            assert!(l.try_begin(a, t), "attempt {t}");
        }
        assert!(!l.try_begin(a, 10), "the sixth is refused");
        assert!(l.try_begin(b, 10), "other addresses are fine");
        assert!(l.try_begin(a, 15 * 60 * 1000 + 5), "the window slides");
        l.reset(a);
        assert!(l.try_begin(a, 1));
    }

    #[test]
    fn attempts_are_counted_before_they_finish() {
        // Five attempts still in flight (none has failed or succeeded yet) use the address up.
        let mut l = LoginLimiter::default();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let started = (0..6).filter(|_| l.try_begin(a, 1)).count();
        assert_eq!(started, 5, "the sixth concurrent attempt is blocked");
        l.release(a, 1);
        assert!(l.try_begin(a, 2), "a released attempt frees a place");
        assert!(!l.try_begin(a, 2));
        l.reset(a);
        assert!(l.try_begin(a, 3), "success clears the address");
    }

    fn forwarded(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn limiter_key_without_a_proxy_is_the_peer() {
        let h = forwarded("203.0.113.7");
        let peer = ip("10.0.0.1");
        assert_eq!(Access::Admin.limiter_key(peer, &h), peer.unwrap());
        assert_eq!(Access::Open.limiter_key(peer, &h), peer.unwrap());
        assert_eq!(
            Access::Admin.limiter_key(None, &h),
            IpAddr::from([0, 0, 0, 0])
        );
    }

    #[test]
    fn limiter_key_behind_a_trusted_proxy_is_the_rightmost_untrusted_hop() {
        let p = proxy(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.1");
        let key = |h: &HeaderMap| p.limiter_key(peer, h);
        assert_eq!(
            key(&forwarded("198.51.100.1, 203.0.113.7")),
            ip("203.0.113.7").unwrap()
        );
        assert_eq!(
            key(&forwarded("198.51.100.1, 203.0.113.7, 10.0.0.2")),
            ip("203.0.113.7").unwrap(),
            "trusted hops are skipped"
        );
        assert_eq!(
            key(&forwarded("evil, 203.0.113.7")),
            ip("203.0.113.7").unwrap(),
            "what the client forges to the left does not matter"
        );
        let mut two = forwarded("198.51.100.1");
        two.append("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        assert_eq!(
            key(&two),
            ip("203.0.113.7").unwrap(),
            "several header lines"
        );
        assert_eq!(key(&HeaderMap::new()), peer.unwrap(), "no header");
        assert_eq!(
            key(&forwarded("10.0.0.5, 10.0.0.6")),
            peer.unwrap(),
            "all trusted"
        );
        assert_eq!(
            key(&forwarded("203.0.113.7, junk")),
            peer.unwrap(),
            "unreadable hop"
        );
        assert_eq!(
            key(&forwarded("::ffff:203.0.113.7")),
            ip("203.0.113.7").unwrap(),
            "mapped addresses count as IPv4"
        );
    }

    #[test]
    fn limiter_key_survives_non_ascii_garbage_to_the_left() {
        let p = proxy(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.1");
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_bytes(b"\xff, 203.0.113.7").unwrap(),
        );
        assert_eq!(p.limiter_key(peer, &h), ip("203.0.113.7").unwrap());
        let mut line = HeaderMap::new();
        line.append(
            "x-forwarded-for",
            HeaderValue::from_bytes(b"\xff\xfe").unwrap(),
        );
        line.append("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        assert_eq!(p.limiter_key(peer, &line), ip("203.0.113.7").unwrap());
    }

    #[test]
    fn limiter_key_reads_hops_with_ports() {
        let p = proxy(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.1");
        assert_eq!(
            p.limiter_key(peer, &forwarded("198.51.100.1, 203.0.113.7:1234")),
            ip("203.0.113.7").unwrap()
        );
        assert_eq!(
            p.limiter_key(peer, &forwarded("[2001:db8:1:2:ffff::9]:443")),
            ip("2001:db8:1:2::").unwrap()
        );
        assert_eq!(
            p.limiter_key(peer, &forwarded("203.0.113.7:1234, 10.0.0.2:80")),
            ip("203.0.113.7").unwrap(),
            "a trusted hop with a port is still skipped"
        );
    }

    #[test]
    fn limiter_key_ignores_the_header_from_a_stranger() {
        let p = proxy(&["10.0.0.0/8"]);
        let stranger = ip("192.0.2.9");
        assert_eq!(
            p.limiter_key(stranger, &forwarded("203.0.113.7")),
            stranger.unwrap()
        );
        let mut raw = HeaderMap::new();
        raw.insert(
            "x-forwarded-for",
            HeaderValue::from_bytes(b"\xff, 203.0.113.7").unwrap(),
        );
        assert_eq!(p.limiter_key(stranger, &raw), stranger.unwrap());
    }

    #[test]
    fn limiter_key_folds_ipv6_to_the_64() {
        let a = ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd");
        let b = ip("2001:db8:1:2:1::1");
        let other = ip("2001:db8:1:3::1");
        let key = |p| Access::Open.limiter_key(p, &HeaderMap::new());
        assert_eq!(key(a), key(b));
        assert_ne!(key(a), key(other));
        assert_eq!(key(a), ip("2001:db8:1:2::").unwrap());
        // Behind a proxy the forwarded client is folded too.
        let p = proxy(&["10.0.0.0/8"]);
        assert_eq!(
            p.limiter_key(ip("10.0.0.1"), &forwarded("2001:db8:1:2:ffff::9")),
            ip("2001:db8:1:2::").unwrap()
        );
    }

    #[test]
    fn setup_codes_ignore_case_and_dashes() {
        assert_eq!(normalize(" abcd-efgh-2345 6789 "), "ABCDEFGH23456789");
    }

    #[test]
    fn the_limiter_stays_bounded_and_releases_its_own_mark() {
        let mut l = LoginLimiter::default();
        for i in 0..(MAX_TRACKED as u32 + 10) {
            assert!(l.try_begin(IpAddr::from(std::net::Ipv4Addr::from(i)), i64::from(i)));
        }
        assert!(l.len() <= MAX_TRACKED, "{}", l.len());
        let key: IpAddr = "10.9.9.9".parse().unwrap();
        assert!(l.try_begin(key, 100));
        assert!(l.try_begin(key, 200));
        l.release(key, 100);
        assert_eq!(l.marks(key), vec![200]);
    }
}
