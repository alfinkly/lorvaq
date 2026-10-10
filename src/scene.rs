//! A scene turns source inputs into world changes. The runtime owns the scene's
//! world, applies the changes, publishes the valid ones and takes new settings.

use crate::world::bus::{EventBus, BUFFER};
use crate::world::{Change, Snapshot, World};
use anyhow::{anyhow, Context};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};

/// Our locks are never held across code that can panic.
pub const LOCK: &str = "lock is never poisoned: no panics while held";

pub trait Scene: Send + 'static {
    type Input: Send + 'static;
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// Pure: reads the world, returns the changes. Must not block.
    fn reduce(&mut self, world: &World, input: Self::Input, now_ms: i64) -> Vec<Change>;
    /// Time-based changes: timeouts, cleanup. Called about once a second.
    fn tick(&mut self, world: &World, now_ms: i64) -> Vec<Change>;
    /// Current settings, as the settings page shows them.
    fn settings(&self) -> Value;
    /// Replaces the settings and keeps the world. On error the old settings stay.
    fn configure(&mut self, settings: Value) -> anyhow::Result<()>;
}

#[derive(Default)]
pub struct SceneState {
    pub world: World,
    pub seq: u64,
}

/// Asks the runtime to apply new settings; the reply carries what was applied.
pub struct Control {
    pub settings: Value,
    pub reply: oneshot::Sender<Result<Value, String>>,
}

#[derive(Clone)]
pub struct SceneHandle {
    pub id: &'static str,
    pub title: &'static str,
    pub state: Arc<RwLock<SceneState>>,
    pub bus: Arc<EventBus>,
    settings: Arc<RwLock<Value>>,
    control: Option<mpsc::Sender<Control>>,
}

impl SceneHandle {
    /// A handle without a runtime: it can be committed to but not configured.
    pub fn new(id: &'static str, title: &'static str, cap: usize) -> Self {
        Self {
            id,
            title,
            state: Arc::new(RwLock::new(SceneState::default())),
            bus: Arc::new(EventBus::new(cap)),
            settings: Arc::new(RwLock::new(Value::Null)),
            control: None,
        }
    }

    /// World and seq read under one lock, so the snapshot matches its seq exactly.
    pub fn snapshot(&self) -> Snapshot {
        let st = self.state.read().expect(LOCK);
        st.world.snapshot(st.seq)
    }

    /// Applies changes in order; the valid ones become events. Returns how many.
    pub fn commit(&self, changes: Vec<Change>, now_ms: i64) -> usize {
        let mut st = self.state.write().expect(LOCK);
        let mut applied = 0;
        for change in changes {
            if st.world.apply(&change, now_ms) {
                st.seq = self.bus.publish(now_ms, change).seq;
                applied += 1;
            }
        }
        applied
    }

    pub fn settings(&self) -> Value {
        self.settings.read().expect(LOCK).clone()
    }

    /// Applies new settings in the running scene; returns them as applied.
    pub async fn configure(&self, settings: Value) -> anyhow::Result<Value> {
        let control = self
            .control
            .as_ref()
            .context("this scene has no runtime to configure")?;
        let (reply, answer) = oneshot::channel();
        control
            .send(Control { settings, reply })
            .await
            .map_err(|_| anyhow!("the scene has stopped"))?;
        answer
            .await
            .map_err(|_| anyhow!("the scene has stopped"))?
            .map_err(|e| anyhow!(e))
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
    let (control_tx, mut control_rx) = mpsc::channel::<Control>(8);
    let mut handle = SceneHandle::new(scene.id(), scene.title(), BUFFER);
    handle.control = Some(control_tx);
    *handle.settings.write().expect(LOCK) = scene.settings();
    let (tx, mut rx) = mpsc::channel::<S::Input>(256);
    let h = handle.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tick_every);
        let mut inputs_open = true;
        loop {
            tokio::select! {
                input = rx.recv(), if inputs_open => match input {
                    Some(input) => {
                        let now = now_ms();
                        let changes = {
                            let st = h.state.read().expect(LOCK);
                            scene.reduce(&st.world, input, now)
                        };
                        h.commit(changes, now);
                    }
                    // No source feeds this scene (yet): keep ticking and taking settings.
                    None => inputs_open = false,
                },
                control = control_rx.recv() => {
                    // The handle keeps a sender, so this only ends with the process.
                    let Some(Control { settings, reply }) = control else { break };
                    let result = scene.configure(settings).map(|()| scene.settings()).map_err(|e| format!("{e:#}"));
                    if let Ok(applied) = &result {
                        *h.settings.write().expect(LOCK) = applied.clone();
                    }
                    let _ = reply.send(result);
                }
                _ = tick.tick() => {
                    let now = now_ms();
                    let changes = {
                        let st = h.state.read().expect(LOCK);
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
    use serde_json::json;

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

    /// Input: a place id to create. Tick: creates "ticked" once. Settings: {"name": String}.
    struct Toy {
        name: String,
    }

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
        fn settings(&self) -> Value {
            json!({ "name": self.name })
        }
        fn configure(&mut self, settings: Value) -> anyhow::Result<()> {
            self.name = settings
                .get("name")
                .and_then(Value::as_str)
                .context("name must be a string")?
                .to_string();
            Ok(())
        }
    }

    fn toy() -> Toy {
        Toy { name: "a".into() }
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
        let (h, tx) = spawn(toy(), Duration::from_millis(20));
        assert_eq!((h.id, h.title), ("toy", "Toy"));
        tx.send("queue".into()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        let snap = h.snapshot();
        let ids: Vec<&str> = snap.places.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"queue") && ids.contains(&"ticked"), "{ids:?}");
    }

    #[tokio::test]
    async fn runtime_applies_settings_and_keeps_them_on_error() {
        let (h, _tx) = spawn(toy(), Duration::from_millis(20));
        assert_eq!(h.settings(), json!({ "name": "a" }));
        assert_eq!(
            h.configure(json!({ "name": "b" })).await.unwrap(),
            json!({ "name": "b" })
        );
        assert_eq!(h.settings(), json!({ "name": "b" }));
        let err = h.configure(json!(42)).await.unwrap_err();
        assert!(err.to_string().contains("name must be a string"), "{err}");
        assert_eq!(h.settings(), json!({ "name": "b" }), "old settings stay");
    }

    #[tokio::test]
    async fn a_scene_without_sources_keeps_ticking_and_configuring() {
        let (h, tx) = spawn(toy(), Duration::from_millis(20));
        drop(tx);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(h.snapshot().places.iter().any(|p| p.id == "ticked"));
        assert!(h.configure(json!({ "name": "c" })).await.is_ok());
    }

    #[tokio::test]
    async fn a_handle_without_a_runtime_cannot_be_configured() {
        let h = SceneHandle::new("t", "T", 10);
        assert!(h.configure(json!({})).await.is_err());
        assert_eq!(h.settings(), Value::Null);
    }
}
