import type { Entity, Id, Level, Place, Snapshot, WorldEvent } from "./types";

/** What an event means for animation. */
export type Effect =
  | { type: "appear"; id: Id; at: Id }
  | { type: "move"; id: Id; from: Id; to: Id }
  | { type: "gone"; id: Id; from: Id }
  | { type: "flash"; at: Id; kind: string }
  | { type: "none" };

const NONE: Effect = { type: "none" };
const levelKey = (owner: Id, metric: string) => `${owner}\u001f${metric}`;

/** The browser's copy of a scene's world. The server sends only valid events. */
export class ClientWorld {
  seq = 0;
  places = new Map<Id, Place>();
  entities = new Map<Id, Entity>();
  levels = new Map<string, Level>();
  stale = new Map<string, number>();

  static fromSnapshot(s: Snapshot): ClientWorld {
    const w = new ClientWorld();
    w.seq = s.seq;
    for (const p of s.places) w.places.set(p.id, p);
    for (const e of s.entities) w.entities.set(e.id, e);
    for (const l of s.levels) w.levels.set(levelKey(l.owner, l.metric), l);
    for (const [source, since] of Object.entries(s.stale)) w.stale.set(source, since);
    return w;
  }

  level(owner: Id, metric: string): Level | undefined {
    return this.levels.get(levelKey(owner, metric));
  }

  private dropLevelsOf(owner: Id): void {
    for (const [key, l] of this.levels) if (l.owner === owner) this.levels.delete(key);
  }

  apply(ev: WorldEvent): Effect {
    this.seq = ev.seq;
    const c = ev.change;
    switch (c.op) {
      case "upsert_place":
        this.places.set(c.place.id, c.place);
        return NONE;
      case "remove_place":
        this.places.delete(c.id);
        this.dropLevelsOf(c.id);
        return NONE;
      case "appear":
        this.entities.set(c.entity.id, c.entity);
        return { type: "appear", id: c.entity.id, at: c.entity.place };
      case "move": {
        const e = this.entities.get(c.id);
        if (!e) return NONE;
        this.entities.set(c.id, { ...e, place: c.to, since_ms: ev.ts_ms });
        return { type: "move", id: c.id, from: e.place, to: c.to };
      }
      case "set_state": {
        const e = this.entities.get(c.id);
        if (e) this.entities.set(c.id, { ...e, state: c.state });
        return NONE;
      }
      case "set_level":
        this.levels.set(levelKey(c.level.owner, c.level.metric), c.level);
        return NONE;
      case "clear_level":
        this.levels.delete(levelKey(c.owner, c.metric));
        return NONE;
      case "flash":
        return { type: "flash", at: c.at, kind: c.kind };
      case "gone": {
        const e = this.entities.get(c.id);
        if (!e) return NONE;
        this.entities.delete(c.id);
        this.dropLevelsOf(c.id);
        return { type: "gone", id: c.id, from: e.place };
      }
      case "stale":
        this.stale.set(c.source, c.since_ms);
        return NONE;
      case "fresh":
        this.stale.delete(c.source);
        return NONE;
    }
  }
}
