//! The server side of agents: who is connected now, one connection per agent, and the CI
//! scene told when an agent goes away.

use crate::scene::LOCK;
use crate::scenes::ci::input::CiInput;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::{mpsc, watch};

#[derive(Clone, Debug, PartialEq)]
pub struct Online {
    pub host: String,
    pub version: String,
    pub since_ms: i64,
}

struct Conn {
    id: u64,
    online: Online,
    /// Set to true to close the connection.
    stop: watch::Sender<bool>,
}

pub struct Hub {
    inputs: mpsc::Sender<CiInput>,
    conns: Mutex<BTreeMap<String, Conn>>,
    next: AtomicU64,
}

/// A registered connection: its id and the signal that closes it.
pub struct Registration {
    pub id: u64,
    pub stop: watch::Receiver<bool>,
}

impl Hub {
    pub fn new(inputs: mpsc::Sender<CiInput>) -> Self {
        Self {
            inputs,
            conns: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
        }
    }

    pub fn inputs(&self) -> &mpsc::Sender<CiInput> {
        &self.inputs
    }

    /// A newer connection of the same agent closes the older one.
    pub fn register(&self, name: &str, online: Online) -> Registration {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (stop, rx) = watch::channel(false);
        let old = self
            .conns
            .lock()
            .expect(LOCK)
            .insert(name.to_string(), Conn { id, online, stop });
        if let Some(old) = old {
            let _ = old.stop.send(true);
        }
        Registration { id, stop: rx }
    }

    /// The connection ended. Only the agent's current connection makes it go away.
    pub async fn unregister(&self, name: &str, id: u64) {
        let current = {
            let mut conns = self.conns.lock().expect(LOCK);
            match conns.get(name) {
                Some(c) if c.id == id => conns.remove(name).is_some(),
                _ => false,
            }
        };
        if current {
            let _ = self
                .inputs
                .send(CiInput::AgentGone {
                    agent: name.to_string(),
                })
                .await;
        }
    }

    /// Closes the agent's connection (its token was removed).
    pub fn revoke(&self, name: &str) {
        if let Some(c) = self.conns.lock().expect(LOCK).get(name) {
            let _ = c.stop.send(true);
        }
    }

    pub fn online(&self, name: &str) -> Option<Online> {
        self.conns
            .lock()
            .expect(LOCK)
            .get(name)
            .map(|c| c.online.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn online() -> Online {
        Online {
            host: "h".into(),
            version: "v".into(),
            since_ms: 0,
        }
    }

    #[tokio::test]
    async fn a_newer_connection_replaces_the_older_without_a_gone() {
        let (tx, mut rx) = mpsc::channel(8);
        let hub = Hub::new(tx);
        let first = hub.register("a", online());
        let second = hub.register("a", online());
        assert!(
            *first.stop.borrow(),
            "the older connection is told to close"
        );
        hub.unregister("a", first.id).await;
        assert!(rx.try_recv().is_err(), "the agent is still connected");
        assert!(hub.online("a").is_some());
        hub.revoke("a");
        assert!(*second.stop.borrow());
        hub.unregister("a", second.id).await;
        assert_eq!(
            rx.try_recv().unwrap(),
            CiInput::AgentGone { agent: "a".into() }
        );
        assert!(hub.online("a").is_none());
    }
}
