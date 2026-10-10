// Mirror of the Rust world types (src/world/mod.rs). Change both sides together.

export type Id = string;
export type Attrs = Record<string, unknown>;

export interface Place {
  id: Id;
  kind: string;
  parent: Id | null;
  label: string;
  capacity: number | null;
  attrs: Attrs;
}

export interface Entity {
  id: Id;
  kind: string;
  place: Id;
  state: string;
  attrs: Attrs;
  since_ms: number;
}

export interface Level {
  owner: Id;
  metric: string;
  value: number;
  max: number | null;
}

export type Change =
  | { op: "upsert_place"; place: Place }
  | { op: "remove_place"; id: Id }
  | { op: "appear"; entity: Entity }
  | { op: "move"; id: Id; to: Id }
  | { op: "set_state"; id: Id; state: string }
  | { op: "set_level"; level: Level }
  | { op: "clear_level"; owner: Id; metric: string }
  | { op: "flash"; at: Id; kind: string }
  | { op: "gone"; id: Id }
  | { op: "stale"; source: string; since_ms: number }
  | { op: "fresh"; source: string };

export interface WorldEvent {
  seq: number;
  ts_ms: number;
  change: Change;
}

export interface Snapshot {
  seq: number;
  places: Place[];
  entities: Entity[];
  levels: Level[];
  stale: Record<string, number>;
}

export interface SceneInfo {
  id: string;
  title: string;
}
