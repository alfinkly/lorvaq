//! Each scene's events go to Postgres in batches. The live scene never waits for
//! the database: a down database only grows a bounded queue.

use crate::scene::{now_ms, SceneHandle, LOCK};
use crate::store::Store;
use crate::world::Event;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

pub const QUEUE_CAP: usize = 10_000;
pub const BATCH: usize = 500;
pub const HISTORY_KEY: &str = "history";
const DAY_MS: i64 = 24 * 3600 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistorySettings {
    pub retention_days: i64,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { retention_days: 14 }
    }
}

impl HistorySettings {
    pub fn validate(&self) -> Result<(), String> {
        if (1..=365).contains(&self.retention_days) {
            Ok(())
        } else {
            Err("keep history from 1 to 365 days".into())
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HistoryStatus {
    pub queued: usize,
    pub dropped: u64,
    pub written: u64,
    pub last_error: Option<String>,
}

pub trait EventSink: Send + Sync + 'static {
    fn write(
        &self,
        scene: &str,
        events: &[Event],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl EventSink for Store {
    async fn write(&self, scene: &str, events: &[Event]) -> anyhow::Result<()> {
        self.insert_events(scene, events).await
    }
}

pub struct Writer<S> {
    sink: S,
    scene: &'static str,
    queue: VecDeque<Event>,
    status: Arc<Mutex<HistoryStatus>>,
}

impl<S: EventSink> Writer<S> {
    pub fn new(sink: S, scene: &'static str, status: Arc<Mutex<HistoryStatus>>) -> Self {
        Self {
            sink,
            scene,
            queue: VecDeque::new(),
            status,
        }
    }

    pub fn push(&mut self, ev: Event) {
        let mut st = self.status.lock().expect(LOCK);
        if self.queue.len() == QUEUE_CAP {
            self.queue.pop_front();
            st.dropped += 1;
        }
        self.queue.push_back(ev);
        st.queued = self.queue.len();
    }

    /// Events the bus dropped before we could read them.
    pub fn lost(&mut self, n: u64) {
        self.status.lock().expect(LOCK).dropped += n;
    }

    /// Writes the queue in batches. False when the sink failed; the rest stays queued.
    pub async fn flush(&mut self) -> bool {
        while !self.queue.is_empty() {
            let n = self.queue.len().min(BATCH);
            let batch: Vec<Event> = self.queue.iter().take(n).cloned().collect();
            let result = self.sink.write(self.scene, &batch).await;
            let mut st = self.status.lock().expect(LOCK);
            match result {
                Ok(()) => {
                    self.queue.drain(..n);
                    st.written += n as u64;
                    st.queued = self.queue.len();
                }
                Err(e) => {
                    st.last_error = Some(format!("{e:#}"));
                    return false;
                }
            }
        }
        self.status.lock().expect(LOCK).last_error = None;
        true
    }
}

/// Follows a scene's bus and writes its events, once a second.
pub fn spawn_writer(store: Store, scene: SceneHandle, status: Arc<Mutex<HistoryStatus>>) {
    tokio::spawn(async move {
        // Subscribe first, then take what the bus already holds: the scene may
        // have published before this task started.
        let mut rx = scene.bus.subscribe();
        let mut writer = Writer::new(store, scene.id, status);
        let mut written_up_to = 0;
        for ev in scene.bus.since(0).unwrap_or_default() {
            written_up_to = ev.seq;
            writer.push(ev);
        }
        let mut every = tokio::time::interval(Duration::from_secs(1));
        let mut healthy = true;
        loop {
            tokio::select! {
                received = rx.recv() => match received {
                    Ok(ev) if ev.seq > written_up_to => writer.push(ev),
                    Ok(_) => {}
                    Err(RecvError::Lagged(n)) => writer.lost(n),
                    Err(RecvError::Closed) => break,
                },
                _ = every.tick() => {
                    let ok = writer.flush().await;
                    if ok != healthy {
                        healthy = ok;
                        if ok {
                            tracing::info!(scene = scene.id, "history is written again");
                        } else {
                            tracing::warn!(scene = scene.id, "cannot write history; events wait in memory");
                        }
                    }
                }
            }
        }
    });
}

/// Hourly: drop history older than the retention and expired sessions.
pub fn spawn_pruner(store: Store) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(3600));
        loop {
            every.tick().await;
            let settings = store
                .setting::<HistorySettings>(HISTORY_KEY)
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            let now = now_ms();
            match store
                .prune_events(now - settings.retention_days * DAY_MS)
                .await
            {
                Ok(n) if n > 0 => tracing::info!(removed = n, "old history removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!("cannot prune history: {e:#}"),
            }
            if let Err(e) = store.prune_sessions(now).await {
                tracing::warn!("cannot prune sessions: {e:#}");
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Change;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct FakeSink {
        down: AtomicBool,
        seen: Mutex<Vec<Vec<u64>>>,
    }

    impl EventSink for Arc<FakeSink> {
        async fn write(&self, scene: &str, events: &[Event]) -> anyhow::Result<()> {
            assert_eq!(scene, "ci");
            if self.down.load(Ordering::SeqCst) {
                anyhow::bail!("connection refused");
            }
            self.seen
                .lock()
                .unwrap()
                .push(events.iter().map(|e| e.seq).collect());
            Ok(())
        }
    }

    fn ev(seq: u64) -> Event {
        Event {
            seq,
            ts_ms: 0,
            change: Change::Gone { id: "j".into() },
        }
    }

    fn writer() -> (
        Writer<Arc<FakeSink>>,
        Arc<FakeSink>,
        Arc<Mutex<HistoryStatus>>,
    ) {
        let sink = Arc::new(FakeSink::default());
        let status = Arc::new(Mutex::new(HistoryStatus::default()));
        (
            Writer::new(sink.clone(), "ci", status.clone()),
            sink,
            status,
        )
    }

    #[tokio::test]
    async fn writes_in_batches_and_empties_the_queue() {
        let (mut w, sink, status) = writer();
        for seq in 1..=1_200 {
            w.push(ev(seq));
        }
        assert!(w.flush().await);
        let sizes: Vec<usize> = sink.seen.lock().unwrap().iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![BATCH, BATCH, 200]);
        let st = status.lock().unwrap().clone();
        assert_eq!((st.queued, st.written, st.last_error), (0, 1_200, None));
    }

    #[tokio::test]
    async fn a_failing_database_keeps_events_in_order_until_it_is_back() {
        let (mut w, sink, status) = writer();
        sink.down.store(true, Ordering::SeqCst);
        w.push(ev(1));
        w.push(ev(2));
        assert!(!w.flush().await);
        assert!(status
            .lock()
            .unwrap()
            .last_error
            .as_deref()
            .unwrap()
            .contains("connection refused"));
        assert_eq!(status.lock().unwrap().queued, 2);
        sink.down.store(false, Ordering::SeqCst);
        w.push(ev(3));
        assert!(w.flush().await);
        assert_eq!(sink.seen.lock().unwrap().concat(), vec![1, 2, 3]);
        assert_eq!(status.lock().unwrap().last_error, None);
    }

    #[tokio::test]
    async fn overflow_drops_the_oldest_and_counts_them() {
        let (mut w, sink, status) = writer();
        for seq in 1..=(QUEUE_CAP as u64 + 3) {
            w.push(ev(seq));
        }
        w.lost(5);
        assert_eq!(status.lock().unwrap().dropped, 8);
        assert!(w.flush().await);
        let all = sink.seen.lock().unwrap().concat();
        assert_eq!((all.len(), all[0]), (QUEUE_CAP, 4));
    }

    #[test]
    fn retention_defaults_and_bounds() {
        let s: HistorySettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.retention_days, 14);
        assert!(s.validate().is_ok());
        assert!(HistorySettings { retention_days: 0 }.validate().is_err());
        assert!(HistorySettings {
            retention_days: 366
        }
        .validate()
        .is_err());
    }
}
