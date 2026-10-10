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
/// One batch write gets this long before it counts as failed.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
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

struct Queue {
    events: VecDeque<Event>,
    /// Events pushed out by overflow so far; lets a flush see the front move under it.
    evicted: u64,
}

/// A bounded queue of events and the code that writes it out. Pushing never waits
/// for the sink, so the two sides can run in different tasks.
pub struct Writer<S> {
    sink: S,
    scene: &'static str,
    queue: Mutex<Queue>,
    status: Arc<Mutex<HistoryStatus>>,
    write_timeout: Duration,
}

impl<S: EventSink> Writer<S> {
    pub fn new(sink: S, scene: &'static str, status: Arc<Mutex<HistoryStatus>>) -> Self {
        Self::with_timeout(sink, scene, status, WRITE_TIMEOUT)
    }

    /// Like `new`, with another limit for one batch write.
    pub fn with_timeout(
        sink: S,
        scene: &'static str,
        status: Arc<Mutex<HistoryStatus>>,
        write_timeout: Duration,
    ) -> Self {
        Self {
            sink,
            scene,
            queue: Mutex::new(Queue {
                events: VecDeque::new(),
                evicted: 0,
            }),
            status,
            write_timeout,
        }
    }

    pub fn push(&self, ev: Event) {
        let mut q = self.queue.lock().expect(LOCK);
        let mut st = self.status.lock().expect(LOCK);
        if q.events.len() == QUEUE_CAP {
            q.events.pop_front();
            q.evicted += 1;
            st.dropped += 1;
        }
        q.events.push_back(ev);
        st.queued = q.events.len();
    }

    /// Events the bus dropped before we could read them.
    pub fn lost(&self, n: u64) {
        self.status.lock().expect(LOCK).dropped += n;
    }

    fn fail(&self, message: String) -> bool {
        self.status.lock().expect(LOCK).last_error = Some(message);
        false
    }

    /// Writes the queue in batches, each within the write timeout. False when the sink
    /// failed or timed out (recorded in `last_error`); the rest stays queued.
    pub async fn flush(&self) -> bool {
        loop {
            let (batch, evicted_before) = {
                let q = self.queue.lock().expect(LOCK);
                if q.events.is_empty() {
                    break;
                }
                let n = q.events.len().min(BATCH);
                (
                    q.events.iter().take(n).cloned().collect::<Vec<_>>(),
                    q.evicted,
                )
            };
            let n = batch.len();
            let result =
                tokio::time::timeout(self.write_timeout, self.sink.write(self.scene, &batch)).await;
            match result {
                Ok(Ok(())) => {
                    let mut q = self.queue.lock().expect(LOCK);
                    // Overflow may have removed some of this batch from the front meanwhile.
                    let pushed_out = (q.evicted - evicted_before) as usize;
                    let remove = n.saturating_sub(pushed_out).min(q.events.len());
                    q.events.drain(..remove);
                    let mut st = self.status.lock().expect(LOCK);
                    st.written += n as u64;
                    st.queued = q.events.len();
                }
                Ok(Err(e)) => return self.fail(format!("{e:#}")),
                Err(_) => {
                    return self.fail(format!(
                        "writing history timed out after {} s",
                        self.write_timeout.as_secs_f32()
                    ))
                }
            }
        }
        self.status.lock().expect(LOCK).last_error = None;
        true
    }
}

/// Follows a scene's bus into the queue and, in a second task, writes the queue once
/// a second. The receiving side never waits for the database, so a hanging database
/// cannot make the bus drop events before they reach the bounded queue.
pub fn spawn_writer(store: Store, scene: SceneHandle, status: Arc<Mutex<HistoryStatus>>) {
    // Subscribe first, then take what the bus already holds: the scene may
    // have published before these tasks started.
    let mut rx = scene.bus.subscribe();
    let writer = Arc::new(Writer::new(store, scene.id, status));
    let mut written_up_to = 0;
    for ev in scene.bus.since(0).unwrap_or_default() {
        written_up_to = ev.seq;
        writer.push(ev);
    }
    let (done_tx, mut done_rx) = tokio::sync::watch::channel(false);
    let receiving = writer.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) if ev.seq > written_up_to => receiving.push(ev),
                Ok(_) => {}
                Err(RecvError::Lagged(n)) => receiving.lost(n),
                Err(RecvError::Closed) => break,
            }
        }
        let _ = done_tx.send(true);
    });
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(1));
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut healthy = true;
        loop {
            tokio::select! {
                _ = every.tick() => {}
                _ = done_rx.changed() => {
                    writer.flush().await;
                    break;
                }
            }
            let ok = writer.flush().await;
            if ok != healthy {
                healthy = ok;
                if ok {
                    tracing::info!(scene = scene.id, "history is written again");
                } else {
                    tracing::warn!(
                        scene = scene.id,
                        "cannot write history; events wait in memory"
                    );
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
        let (w, sink, status) = writer();
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
        let (w, sink, status) = writer();
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
        let (w, sink, status) = writer();
        for seq in 1..=(QUEUE_CAP as u64 + 3) {
            w.push(ev(seq));
        }
        w.lost(5);
        assert_eq!(status.lock().unwrap().dropped, 8);
        assert!(w.flush().await);
        let all = sink.seen.lock().unwrap().concat();
        assert_eq!((all.len(), all[0]), (QUEUE_CAP, 4));
    }

    struct HangingSink;

    impl EventSink for HangingSink {
        async fn write(&self, _scene: &str, _events: &[Event]) -> anyhow::Result<()> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn a_hanging_sink_times_out_with_an_error_and_pushing_goes_on() {
        let status = Arc::new(Mutex::new(HistoryStatus::default()));
        let w = Arc::new(Writer::with_timeout(
            HangingSink,
            "ci",
            status.clone(),
            Duration::from_millis(100),
        ));
        w.push(ev(1));
        let flushing = tokio::spawn({
            let w = w.clone();
            async move { w.flush().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        w.push(ev(2)); // the flush is still waiting for the sink
        assert_eq!(status.lock().unwrap().queued, 2);
        let ok = tokio::time::timeout(Duration::from_secs(2), flushing)
            .await
            .expect("the flush gives up within the timeout")
            .unwrap();
        assert!(!ok);
        let st = status.lock().unwrap().clone();
        assert!(
            st.last_error.as_deref().unwrap().contains("timed out"),
            "{st:?}"
        );
        assert_eq!((st.queued, st.written), (2, 0), "nothing is lost");
    }

    #[tokio::test]
    async fn overflow_during_a_write_does_not_drop_the_wrong_events() {
        // The sink fills the queue past its cap while the first batch is being written.
        #[derive(Default)]
        struct PushingSink {
            writer: std::sync::OnceLock<Arc<Writer<Arc<PushingSink>>>>,
            seen: Mutex<Vec<u64>>,
        }
        impl EventSink for Arc<PushingSink> {
            async fn write(&self, _scene: &str, events: &[Event]) -> anyhow::Result<()> {
                if events[0].seq == 1 {
                    let w = self.writer.get().unwrap();
                    for seq in 20_000..20_000 + BATCH as u64 {
                        w.push(ev(seq));
                    }
                }
                self.seen
                    .lock()
                    .unwrap()
                    .extend(events.iter().map(|e| e.seq));
                Ok(())
            }
        }
        let status = Arc::new(Mutex::new(HistoryStatus::default()));
        let sink = Arc::new(PushingSink::default());
        let w = Arc::new(Writer::new(sink.clone(), "ci", status.clone()));
        sink.writer.set(w.clone()).ok();
        for seq in 1..=QUEUE_CAP as u64 {
            w.push(ev(seq));
        }
        assert!(w.flush().await);
        let seen = sink.seen.lock().unwrap().clone();
        // The first batch went out; the 500 oldest queued events were pushed out by the
        // overflow, so the flush continues with 501.. and neither repeats nor skips.
        let expected: Vec<u64> = (1..=QUEUE_CAP as u64)
            .chain(20_000..20_000 + BATCH as u64)
            .collect();
        assert_eq!(seen, expected);
        assert_eq!(status.lock().unwrap().queued, 0);
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
