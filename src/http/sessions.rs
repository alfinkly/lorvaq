//! A small in-memory cache of admin sessions the database has confirmed, so a
//! slow or down database does not lock the admin out of the scenes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A confirmed session is trusted without asking the database for this long.
pub(super) const FRESH: Duration = Duration::from_secs(10);
/// Every session check against the database gives up after this long.
pub(super) const CHECK_TIMEOUT: Duration = Duration::from_secs(2);
/// The database-down fallback only trusts sessions confirmed this recently.
pub(super) const KNOWN_FOR: Duration = Duration::from_secs(3600);
const PRUNE_ABOVE: usize = 256;

struct Entry {
    expires_ms: i64,
    checked: Instant,
}

#[derive(Default)]
pub struct SessionCache {
    entries: HashMap<String, Entry>,
    /// Bumped by every eviction, so a check that began before one cannot cache its stale answer.
    generation: u64,
}

impl SessionCache {
    /// Confirmed less than `FRESH` ago and not expired.
    pub(super) fn fresh(&self, hash: &str, now_ms: i64, now: Instant) -> bool {
        self.entries.get(hash).is_some_and(|e| {
            e.expires_ms > now_ms && now.saturating_duration_since(e.checked) < FRESH
        })
    }

    /// Confirmed within `KNOWN_FOR` and still within its expiry: good enough when the database cannot answer.
    pub(super) fn known(&self, hash: &str, now_ms: i64, now: Instant) -> bool {
        self.entries.get(hash).is_some_and(|e| {
            e.expires_ms > now_ms && now.saturating_duration_since(e.checked) < KNOWN_FOR
        })
    }

    /// Read before a database check starts and pass to `confirm`.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Caches a database answer, unless a session was evicted since `generation` was read:
    /// the answer may be about a session that is gone by now. Returns whether it was cached.
    pub(super) fn confirm(
        &mut self,
        hash: &str,
        expires_ms: i64,
        now_ms: i64,
        now: Instant,
        generation: u64,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
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
        true
    }

    pub(super) fn evict(&mut self, hash: &str) {
        self.generation += 1;
        self.entries.remove(hash);
    }

    pub(super) fn evict_all_except(&mut self, keep: &str) {
        self.generation += 1;
        self.entries.retain(|h, _| h == keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confirm(c: &mut SessionCache, h: &str, expires_ms: i64, now: Instant) {
        let g = c.generation();
        assert!(c.confirm(h, expires_ms, 0, now, g));
    }

    #[test]
    fn a_confirmed_session_is_fresh_for_ten_seconds_and_known_until_it_expires() {
        let mut c = SessionCache::default();
        let t0 = Instant::now();
        confirm(&mut c, "a", 1_000, t0);
        assert!(c.fresh("a", 500, t0 + Duration::from_secs(9)));
        assert!(!c.fresh("a", 500, t0 + Duration::from_secs(11)), "stale");
        assert!(c.known("a", 500, t0), "still within its expiry");
        assert!(!c.known("a", 1_000, t0), "expired");
        assert!(!c.fresh("a", 1_000, t0), "expired is never fresh");
        assert!(!c.known("b", 0, t0), "unknown hashes are not trusted");
    }

    #[test]
    fn the_fallback_trusts_only_sessions_confirmed_within_an_hour() {
        let mut c = SessionCache::default();
        let t0 = Instant::now();
        confirm(&mut c, "a", i64::MAX, t0);
        assert!(c.known("a", 0, t0 + KNOWN_FOR - Duration::from_secs(1)));
        assert!(
            !c.known("a", 0, t0 + KNOWN_FOR),
            "not expired, but not confirmed lately"
        );
    }

    #[test]
    fn evicting_forgets_sessions() {
        let mut c = SessionCache::default();
        let t0 = Instant::now();
        for h in ["a", "b", "c"] {
            confirm(&mut c, h, 1_000, t0);
        }
        c.evict("a");
        assert!(!c.known("a", 0, t0));
        c.evict_all_except("b");
        assert!(c.known("b", 0, t0));
        assert!(!c.known("c", 0, t0));
    }

    #[test]
    fn a_check_that_began_before_an_eviction_cannot_cache_its_answer() {
        let t0 = Instant::now();
        for evict in [
            (|c: &mut SessionCache| c.evict("a")) as fn(&mut SessionCache),
            |c| c.evict_all_except("other"),
        ] {
            let mut c = SessionCache::default();
            let began = c.generation();
            evict(&mut c);
            assert!(!c.confirm("a", 1_000, 0, t0, began), "stale generation");
            assert!(!c.known("a", 0, t0));
            assert!(!c.fresh("a", 0, t0));
            let again = c.generation();
            assert!(c.confirm("a", 1_000, 0, t0, again), "a new check may cache");
            assert!(c.known("a", 0, t0));
        }
    }
}
