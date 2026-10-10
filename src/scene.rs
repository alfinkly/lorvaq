//! A scene turns source inputs into world changes. The runtime owns the scene's
//! world, applies the changes and publishes the valid ones as events.

use crate::world::bus::{EventBus, BUFFER};
use crate::world::{Change, Snapshot, World};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

pub trait Scene: Send + 'static {
    type Input: Send + 'static;
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// Pure: reads the world, returns the changes. Must not block.
    fn reduce(&mut self, world: &World, input: Self::Input, now_ms: i64) -> Vec<Change>;
    /// Time-based changes: timeouts, cleanup. Called about once a second.
    fn tick(&mut self, world: &World, now_ms: i64) -> Vec<Change>;
}

#[derive(Default)]
pub struct SceneState {
    pub world: World,
    pub seq: u64,
}

#[derive(Clone)]
pub struct SceneHandle {
    pub id: &'static str,
    pub title: &'static str,
    pub state: Arc<RwLock<SceneState>>,
    pub bus: Arc<EventBus>,
}

impl SceneHandle {
    pub fn new(id: &'static str, title: &'static str, cap: usize) -> Self {
        Self {
            id,
            title,
            state: Arc::new(RwLock::new(SceneState::default())),
            bus: Arc::new(EventBus::new(cap)),
        }
    }

    /// World and seq read under one lock, so the snapshot matches its seq exactly.
    pub fn snapshot(&self) -> Snapshot {
        let st = self
            .state
            .read()
            .expect("scene lock is never poisoned: no panics inside");
        st.world.snapshot(st.seq)
    }

    /// Applies changes in order; the valid ones become events. Returns how many.
    pub fn commit(&self, changes: Vec<Change>, now_ms: i64) -> usize {
        let mut st = self
            .state
            .write()
            .expect("scene lock is never poisoned: no panics inside");
        let mut applied = 0;
        for change in changes {
            if st.world.apply(&change, now_ms) {
                st.seq = self.bus.publish(now_ms, change).seq;
                applied += 1;
            }
        }
        applied
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

pub fn spawn<S: Scene>(
    mut scene: S,
    tick_every: Duration,
) -> (SceneHandle, mpsc::Sender<S::Input>) {
    let handle = SceneHandle::new(scene.id(), scene.title(), BUFFER);
    let (tx, mut rx) = mpsc::channel::<S::Input>(256);
    let h = handle.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tick_every);
        loop {
            tokio::select! {
                input = rx.recv() => {
                    let Some(input) = input else { break };
                    let now = now_ms();
                    let changes = {
                        let st = h.state.read().expect("scene lock is never poisoned: no panics inside");
                        scene.reduce(&st.world, input, now)
                    };
                    h.commit(changes, now);
                }
                _ = tick.tick() => {
                    let now = now_ms();
                    let changes = {
                        let st = h.state.read().expect("scene lock is never poisoned: no panics inside");
                        scene.tick(&st.world, now)
                    };
                    h.commit(changes, now);
                }
            }
        }
    });
    (handle, tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Attrs, Entity, Place};

    fn zone(id: &str) -> Change {
        Change::UpsertPlace {
            place: Place {
                id: id.into(),
                kind: "zone".into(),
                parent: None,
                label: id.into(),
                capacity: None,
                attrs: Attrs::new(),
            },
        }
    }

    /// Input: a place id to create. Tick: creates "ticked" once.
    struct Toy;

    impl Scene for Toy {
        type Input = String;
        fn id(&self) -> &'static str {
            "toy"
        }
        fn title(&self) -> &'static str {
            "Toy"
        }
        fn reduce(&mut self, _world: &World, input: String, _now_ms: i64) -> Vec<Change> {
            vec![zone(&input)]
        }
        fn tick(&mut self, world: &World, _now_ms: i64) -> Vec<Change> {
            if world.place("ticked").is_none() {
                vec![zone("ticked")]
            } else {
                vec![]
            }
        }
    }

    #[test]
    fn commit_publishes_only_valid_changes() {
        let h = SceneHandle::new("t", "T", 100);
        let moved = Change::Move {
            id: "nobody".into(),
            to: "a".into(),
        };
        assert_eq!(h.commit(vec![zone("a"), zone("a"), moved], 1), 1);
        assert_eq!(h.bus.last_seq(), 1);
        assert_eq!(h.snapshot().seq, 1);
        assert_eq!(h.snapshot().places.len(), 1);
    }

    #[test]
    fn snapshot_plus_events_rebuild_the_world() {
        let h = SceneHandle::new("t", "T", 100);
        h.commit(vec![zone("a"), zone("b")], 1);
        let snap = h.snapshot();
        let job = Entity {
            id: "j".into(),
            kind: "job".into(),
            place: "a".into(),
            state: "pending".into(),
            attrs: Attrs::new(),
            since_ms: 2,
        };
        h.commit(vec![zone("c"), Change::Appear { entity: job }], 2);
        h.commit(
            vec![Change::Move {
                id: "j".into(),
                to: "c".into(),
            }],
            3,
        );
        let mut rebuilt = World::from_snapshot(&snap);
        for ev in h.bus.since(snap.seq).unwrap() {
            assert!(
                rebuilt.apply(&ev.change, ev.ts_ms),
                "event {} replays",
                ev.seq
            );
        }
        assert_eq!(rebuilt.snapshot(h.bus.last_seq()), h.snapshot());
    }

    #[tokio::test]
    async fn runtime_feeds_inputs_and_ticks() {
        let (h, tx) = spawn(Toy, Duration::from_millis(20));
        assert_eq!((h.id, h.title), ("toy", "Toy"));
        tx.send("queue".into()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        let snap = h.snapshot();
        let ids: Vec<&str> = snap.places.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"queue") && ids.contains(&"ticked"), "{ids:?}");
    }
}
