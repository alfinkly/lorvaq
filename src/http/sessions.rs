//! A small in-memory cache of admin sessions the database has confirmed, so a
//! slow or down database does not lock the admin out of the scenes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A confirmed session is trusted without asking the database for this long.
pub(super) const FRESH: Duration = Duration::from_secs(10);
/// Every session check against the database gives up after this long.
pub(super) const CHECK_TIMEOUT: Duration = Duration::from_secs(2);
const PRUNE_ABOVE: usize = 256;

struct Entry {
    expires_ms: i64,
    checked: Instant,
}

#[derive(Default)]
pub struct SessionCache {
    entries: HashMap<String, Entry>,
}

impl SessionCache {
    /// Confirmed less than `FRESH` ago and not expired.
    pub(super) fn fresh(&self, hash: &str, now_ms: i64, now: Instant) -> bool {
        self.entries.get(hash).is_some_and(|e| {
            e.expires_ms > now_ms && now.saturating_duration_since(e.checked) < FRESH
        })
    }

    /// Confirmed at some point and still within its expiry: good enough when the database cannot answer.
    pub(super) fn known(&self, hash: &str, now_ms: i64) -> bool {
        self.entries
            .get(hash)
            .is_some_and(|e| e.expires_ms > now_ms)
    }

    pub(super) fn confirm(&mut self, hash: &str, expires_ms: i64, now_ms: i64, now: Instant) {
        if self.entries.len() >= PRUNE_ABOVE {
            self.entries.retain(|_, e| e.expires_ms > now_ms);
        }
        self.entries.insert(
            hash.to_string(),
            Entry {
                expires_ms,
                checked: now,
            },
        );
    }

    pub(super) fn evict(&mut self, hash: &str) {
        self.entries.remove(hash);
    }

    pub(super) fn evict_except(&mut self, keep: &str) {
        self.entries.retain(|h, _| h == keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_confirmed_session_is_fresh_for_ten_seconds_and_known_until_it_expires() {
        let mut c = SessionCache::default();
        let t0 = Instant::now();
        c.confirm("a", 1_000, 0, t0);
        assert!(c.fresh("a", 500, t0 + Duration::from_secs(9)));
        assert!(!c.fresh("a", 500, t0 + Duration::from_secs(11)), "stale");
        assert!(c.known("a", 500), "still within its expiry");
        assert!(!c.known("a", 1_000), "expired");
        assert!(!c.fresh("a", 1_000, t0), "expired is never fresh");
        assert!(!c.known("b", 0), "unknown hashes are not trusted");
    }

    #[test]
    fn evicting_forgets_sessions() {
        let mut c = SessionCache::default();
        let t0 = Instant::now();
        for h in ["a", "b", "c"] {
            c.confirm(h, 1_000, 0, t0);
        }
        c.evict("a");
        assert!(!c.known("a", 0));
        c.evict_except("b");
        assert!(c.known("b", 0));
        assert!(!c.known("c", 0));
    }
}
