//! The world every scene is described in: places, entities in places, levels
//! against limits, and the changes between them. Knows nothing about sources.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Id = String;
pub type Attrs = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub id: Id,
    pub kind: String,
    pub parent: Option<Id>,
    pub label: String,
    pub capacity: Option<u32>,
    #[serde(default)]
    pub attrs: Attrs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: Id,
    pub kind: String,
    pub place: Id,
    pub state: String,
    #[serde(default)]
    pub attrs: Attrs,
    pub since_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub owner: Id,
    pub metric: String,
    pub value: f64,
    pub max: Option<f64>,
}

/// A change a scene asks for. The world checks it; only valid changes become events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    UpsertPlace { place: Place },
    RemovePlace { id: Id },
    Appear { entity: Entity },
    Move { id: Id, to: Id },
    SetState { id: Id, state: String },
    SetLevel { level: Level },
    ClearLevel { owner: Id, metric: String },
    Flash { at: Id, kind: String },
    Gone { id: Id },
    Stale { source: String, since_ms: i64 },
    Fresh { source: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub ts_ms: i64,
    pub change: Change,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub seq: u64,
    pub places: Vec<Place>,
    pub entities: Vec<Entity>,
    pub levels: Vec<Level>,
    pub stale: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    places: BTreeMap<Id, Place>,
    entities: BTreeMap<Id, Entity>,
    levels: BTreeMap<(Id, String), Level>,
    stale: BTreeMap<String, i64>,
}

impl World {
    pub fn place(&self, id: &str) -> Option<&Place> {
        self.places.get(id)
    }

    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn places(&self) -> impl Iterator<Item = &Place> {
        self.places.values()
    }

    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn level(&self, owner: &str, metric: &str) -> Option<&Level> {
        self.levels.get(&(owner.to_string(), metric.to_string()))
    }

    /// Applies a change. Returns false when it is invalid or changes nothing:
    /// such changes are dropped and never become events.
    pub fn apply(&mut self, change: &Change, now_ms: i64) -> bool {
        match change {
            Change::UpsertPlace { place } => {
                if self.places.get(&place.id) == Some(place) {
                    return false;
                }
                self.places.insert(place.id.clone(), place.clone());
                true
            }
            Change::RemovePlace { id } => {
                let occupied = self.entities.values().any(|e| &e.place == id);
                let has_children = self.places.values().any(|p| p.parent.as_ref() == Some(id));
                if occupied || has_children || self.places.remove(id).is_none() {
                    return false;
                }
                self.levels.retain(|(owner, _), _| owner != id);
                true
            }
            Change::Appear { entity } => {
                if self.entities.contains_key(&entity.id)
                    || !self.places.contains_key(&entity.place)
                {
                    return false;
                }
                self.entities.insert(entity.id.clone(), entity.clone());
                true
            }
            Change::Move { id, to } => {
                if !self.places.contains_key(to) {
                    return false;
                }
                match self.entities.get_mut(id) {
                    Some(e) if &e.place != to => {
                        e.place = to.clone();
                        e.since_ms = now_ms;
                        true
                    }
                    _ => false,
                }
            }
            Change::SetState { id, state } => match self.entities.get_mut(id) {
                Some(e) if &e.state != state => {
                    e.state = state.clone();
                    true
                }
                _ => false,
            },
            Change::SetLevel { level } => {
                let known = self.places.contains_key(&level.owner)
                    || self.entities.contains_key(&level.owner);
                let key = (level.owner.clone(), level.metric.clone());
                if !known || self.levels.get(&key) == Some(level) {
                    return false;
                }
                self.levels.insert(key, level.clone());
                true
            }
            Change::ClearLevel { owner, metric } => self
                .levels
                .remove(&(owner.clone(), metric.clone()))
                .is_some(),
            Change::Flash { at, .. } => self.places.contains_key(at),
            Change::Gone { id } => {
                if self.entities.remove(id).is_none() {
                    return false;
                }
                self.levels.retain(|(owner, _), _| owner != id);
                true
            }
            Change::Stale { source, since_ms } => {
                self.stale.insert(source.clone(), *since_ms) != Some(*since_ms)
            }
            Change::Fresh { source } => self.stale.remove(source).is_some(),
        }
    }

    pub fn snapshot(&self, seq: u64) -> Snapshot {
        Snapshot {
            seq,
            places: self.places.values().cloned().collect(),
            entities: self.entities.values().cloned().collect(),
            levels: self.levels.values().cloned().collect(),
            stale: self.stale.clone(),
        }
    }

    pub fn from_snapshot(s: &Snapshot) -> World {
        World {
            places: s.places.iter().map(|p| (p.id.clone(), p.clone())).collect(),
            entities: s
                .entities
                .iter()
                .map(|e| (e.id.clone(), e.clone()))
                .collect(),
            levels: s
                .levels
                .iter()
                .map(|l| ((l.owner.clone(), l.metric.clone()), l.clone()))
                .collect(),
            stale: s.stale.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn zone(id: &str) -> Place {
        Place {
            id: id.into(),
            kind: "zone".into(),
            parent: None,
            label: id.into(),
            capacity: None,
            attrs: Attrs::new(),
        }
    }
    fn child(id: &str, parent: &str) -> Place {
        Place {
            parent: Some(parent.into()),
            ..zone(id)
        }
    }
    fn job(id: &str, place: &str) -> Entity {
        Entity {
            id: id.into(),
            kind: "job".into(),
            place: place.into(),
            state: "pending".into(),
            attrs: Attrs::new(),
            since_ms: 0,
        }
    }
    fn up(p: Place) -> Change {
        Change::UpsertPlace { place: p }
    }
    fn level(owner: &str, value: f64) -> Level {
        Level {
            owner: owner.into(),
            metric: "mem".into(),
            value,
            max: Some(10.0),
        }
    }

    #[test]
    fn appear_needs_an_existing_place_and_a_new_id() {
        let mut w = World::default();
        assert!(!w.apply(
            &Change::Appear {
                entity: job("j1", "queue")
            },
            1
        ));
        assert!(w.apply(&up(zone("queue")), 1));
        assert!(w.apply(
            &Change::Appear {
                entity: job("j1", "queue")
            },
            1
        ));
        assert!(
            !w.apply(
                &Change::Appear {
                    entity: job("j1", "queue")
                },
                1
            ),
            "same id twice"
        );
    }

    #[test]
    fn move_changes_place_and_time_only_once() {
        let mut w = World::default();
        w.apply(&up(zone("queue")), 0);
        w.apply(&up(zone("slot")), 0);
        w.apply(
            &Change::Appear {
                entity: job("j1", "queue"),
            },
            0,
        );
        assert!(w.apply(
            &Change::Move {
                id: "j1".into(),
                to: "slot".into()
            },
            5
        ));
        assert_eq!(w.entity("j1").unwrap().place, "slot");
        assert_eq!(w.entity("j1").unwrap().since_ms, 5);
        assert!(
            !w.apply(
                &Change::Move {
                    id: "j1".into(),
                    to: "slot".into()
                },
                6
            ),
            "already there"
        );
        assert!(
            !w.apply(
                &Change::Move {
                    id: "j1".into(),
                    to: "nowhere".into()
                },
                6
            ),
            "missing place"
        );
        assert!(
            !w.apply(
                &Change::Move {
                    id: "ghost".into(),
                    to: "queue".into()
                },
                6
            ),
            "missing entity"
        );
    }

    #[test]
    fn upsert_place_reports_only_real_changes() {
        let mut w = World::default();
        assert!(w.apply(&up(zone("q")), 0));
        assert!(!w.apply(&up(zone("q")), 0));
        let renamed = Place {
            label: "queue".into(),
            ..zone("q")
        };
        assert!(w.apply(&up(renamed), 0));
        assert_eq!(w.place("q").unwrap().label, "queue");
    }

    #[test]
    fn occupied_or_parent_places_are_not_removed() {
        let mut w = World::default();
        w.apply(&up(zone("host")), 0);
        w.apply(&up(child("slot", "host")), 0);
        w.apply(
            &Change::Appear {
                entity: job("j1", "slot"),
            },
            0,
        );
        assert!(
            !w.apply(&Change::RemovePlace { id: "host".into() }, 0),
            "has a child"
        );
        assert!(
            !w.apply(&Change::RemovePlace { id: "slot".into() }, 0),
            "has an entity"
        );
        w.apply(&Change::Gone { id: "j1".into() }, 0);
        assert!(w.apply(&Change::RemovePlace { id: "slot".into() }, 0));
        assert!(w.apply(&Change::RemovePlace { id: "host".into() }, 0));
        assert!(
            !w.apply(&Change::RemovePlace { id: "host".into() }, 0),
            "already gone"
        );
    }

    #[test]
    fn levels_need_an_owner_and_leave_with_it() {
        let mut w = World::default();
        assert!(
            !w.apply(
                &Change::SetLevel {
                    level: level("j1", 1.0)
                },
                0
            ),
            "no owner yet"
        );
        w.apply(&up(zone("q")), 0);
        w.apply(
            &Change::Appear {
                entity: job("j1", "q"),
            },
            0,
        );
        assert!(w.apply(
            &Change::SetLevel {
                level: level("j1", 1.0)
            },
            0
        ));
        assert!(
            !w.apply(
                &Change::SetLevel {
                    level: level("j1", 1.0)
                },
                0
            ),
            "same value"
        );
        assert!(w.apply(
            &Change::SetLevel {
                level: level("j1", 2.0)
            },
            0
        ));
        assert_eq!(w.level("j1", "mem").unwrap().value, 2.0);
        w.apply(&Change::Gone { id: "j1".into() }, 0);
        assert!(w.level("j1", "mem").is_none());
    }

    #[test]
    fn clear_level_and_set_state() {
        let mut w = World::default();
        w.apply(&up(zone("q")), 0);
        w.apply(
            &Change::Appear {
                entity: job("j1", "q"),
            },
            0,
        );
        assert!(
            !w.apply(
                &Change::SetState {
                    id: "j1".into(),
                    state: "pending".into()
                },
                0
            ),
            "same state"
        );
        assert!(w.apply(
            &Change::SetState {
                id: "j1".into(),
                state: "running".into()
            },
            0
        ));
        w.apply(
            &Change::SetLevel {
                level: level("j1", 1.0),
            },
            0,
        );
        assert!(w.apply(
            &Change::ClearLevel {
                owner: "j1".into(),
                metric: "mem".into()
            },
            0
        ));
        assert!(!w.apply(
            &Change::ClearLevel {
                owner: "j1".into(),
                metric: "mem".into()
            },
            0
        ));
    }

    #[test]
    fn flash_needs_a_place_and_changes_nothing() {
        let mut w = World::default();
        assert!(!w.apply(
            &Change::Flash {
                at: "slot".into(),
                kind: "oom".into()
            },
            0
        ));
        w.apply(&up(zone("slot")), 0);
        let before = w.clone();
        assert!(w.apply(
            &Change::Flash {
                at: "slot".into(),
                kind: "oom".into()
            },
            0
        ));
        assert_eq!(w, before);
    }

    #[test]
    fn stale_and_fresh() {
        let mut w = World::default();
        assert!(w.apply(
            &Change::Stale {
                source: "gitlab".into(),
                since_ms: 10
            },
            0
        ));
        assert!(!w.apply(
            &Change::Stale {
                source: "gitlab".into(),
                since_ms: 10
            },
            0
        ));
        assert_eq!(w.snapshot(0).stale.get("gitlab"), Some(&10));
        assert!(w.apply(
            &Change::Fresh {
                source: "gitlab".into()
            },
            0
        ));
        assert!(!w.apply(
            &Change::Fresh {
                source: "gitlab".into()
            },
            0
        ));
    }

    #[test]
    fn snapshot_round_trips() {
        let mut w = World::default();
        w.apply(&up(zone("q")), 0);
        w.apply(
            &Change::Appear {
                entity: job("j1", "q"),
            },
            0,
        );
        w.apply(
            &Change::SetLevel {
                level: level("q", 3.0),
            },
            0,
        );
        w.apply(
            &Change::Stale {
                source: "gitlab".into(),
                since_ms: 1,
            },
            0,
        );
        let snap = w.snapshot(7);
        assert_eq!(snap.seq, 7);
        assert_eq!(World::from_snapshot(&snap), w);
    }

    #[test]
    fn wire_format_is_stable() {
        let mv = Change::Move {
            id: "j".into(),
            to: "q".into(),
        };
        assert_eq!(
            serde_json::to_value(&mv).unwrap(),
            json!({"op": "move", "id": "j", "to": "q"})
        );
        let ev = Event {
            seq: 1,
            ts_ms: 2,
            change: Change::Gone { id: "j".into() },
        };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({"seq": 1, "ts_ms": 2, "change": {"op": "gone", "id": "j"}})
        );
        let p = serde_json::to_value(up(zone("q"))).unwrap();
        assert_eq!(p["op"], "upsert_place");
        assert_eq!(p["place"]["parent"], serde_json::Value::Null);
    }
}
