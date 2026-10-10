//! Numbers events, keeps the last N for clients catching up, and fans them out live.

use super::{Change, Event};
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::broadcast;

/// Events kept for clients that reconnect (spec §7).
pub const BUFFER: usize = 10_000;

/// The asked-for events are no longer buffered (or never existed): take a fresh snapshot.
#[derive(Debug, PartialEq, Eq)]
pub struct Gap;

pub struct EventBus {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Event>,
}

struct Inner {
    last_seq: u64,
    buffer: VecDeque<Event>,
    cap: usize,
}

impl EventBus {
    pub fn new(cap: usize) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            inner: Mutex::new(Inner {
                last_seq: 0,
                buffer: VecDeque::new(),
                cap,
            }),
            tx,
        }
    }

    pub fn publish(&self, ts_ms: i64, change: Change) -> Event {
        let mut inner = self
            .inner
            .lock()
            .expect("bus lock is never poisoned: no panics inside");
        inner.last_seq += 1;
        let ev = Event {
            seq: inner.last_seq,
            ts_ms,
            change,
        };
        if inner.buffer.len() == inner.cap {
            inner.buffer.pop_front();
        }
        inner.buffer.push_back(ev.clone());
        // Sent under the lock so subscribers see events in seq order; no subscribers is fine.
        let _ = self.tx.send(ev.clone());
        ev
    }

    pub fn last_seq(&self) -> u64 {
        self.inner
            .lock()
            .expect("bus lock is never poisoned: no panics inside")
            .last_seq
    }

    /// Events with seq > `after`.
    pub fn since(&self, after: u64) -> Result<Vec<Event>, Gap> {
        let inner = self
            .inner
            .lock()
            .expect("bus lock is never poisoned: no panics inside");
        if after > inner.last_seq {
            return Err(Gap);
        }
        let oldest = inner.buffer.front().map_or(inner.last_seq + 1, |e| e.seq);
        if after + 1 < oldest {
            return Err(Gap);
        }
        Ok(inner
            .buffer
            .iter()
            .filter(|e| e.seq > after)
            .cloned()
            .collect())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gone(id: &str) -> Change {
        Change::Gone { id: id.into() }
    }

    #[test]
    fn seq_grows_from_one() {
        let bus = EventBus::new(10);
        assert_eq!(bus.last_seq(), 0);
        assert_eq!(bus.publish(5, gone("a")).seq, 1);
        assert_eq!(bus.publish(6, gone("b")).seq, 2);
        assert_eq!(bus.last_seq(), 2);
    }

    #[test]
    fn since_returns_only_newer_events() {
        let bus = EventBus::new(10);
        for id in ["a", "b", "c"] {
            bus.publish(0, gone(id));
        }
        let seqs: Vec<u64> = bus.since(1).unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![2, 3]);
        assert!(bus.since(3).unwrap().is_empty());
        assert_eq!(bus.since(0).unwrap().len(), 3);
    }

    #[test]
    fn dropped_events_are_a_gap() {
        let bus = EventBus::new(3);
        for id in ["a", "b", "c", "d", "e"] {
            bus.publish(0, gone(id));
        }
        assert_eq!(bus.since(1), Err(Gap), "event 2 is no longer buffered");
        let seqs: Vec<u64> = bus.since(2).unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![3, 4, 5]);
    }

    #[test]
    fn a_client_from_the_future_is_a_gap() {
        let bus = EventBus::new(10);
        bus.publish(0, gone("a"));
        assert_eq!(
            bus.since(5),
            Err(Gap),
            "server restarted and lost its numbering"
        );
    }

    #[test]
    fn empty_bus_has_no_gap_at_zero() {
        assert_eq!(EventBus::new(10).since(0), Ok(vec![]));
    }

    #[tokio::test]
    async fn subscribers_receive_published_events() {
        let bus = EventBus::new(10);
        let mut rx = bus.subscribe();
        bus.publish(7, gone("a"));
        let ev = rx.recv().await.unwrap();
        assert_eq!((ev.seq, ev.ts_ms), (1, 7));
    }
}
