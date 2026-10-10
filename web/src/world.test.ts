import { describe, expect, test } from "vitest";
import { ClientWorld } from "./world";
import type { Change, Entity, Place, Snapshot } from "./types";

const place = (id: string, extra: Partial<Place> = {}): Place => ({ id, kind: "zone", parent: null, label: id, capacity: null, attrs: {}, ...extra });
const job = (id: string, at: string): Entity => ({ id, kind: "job", place: at, state: "pending", attrs: {}, since_ms: 0 });
const snap = (): Snapshot => ({ seq: 3, places: [place("q"), place("slot")], entities: [job("j1", "q")], levels: [], stale: {} });
let seq = 3;
const ev = (change: Change) => ({ seq: ++seq, ts_ms: 100, change });

describe("ClientWorld", () => {
  test("starts from a snapshot", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.seq).toBe(3);
    expect(w.entities.get("j1")?.place).toBe("q");
  });

  test("move reports where from and where to", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.apply(ev({ op: "move", id: "j1", to: "slot" }))).toEqual({ type: "move", id: "j1", from: "q", to: "slot" });
    expect(w.entities.get("j1")?.place).toBe("slot");
    expect(w.entities.get("j1")?.since_ms).toBe(100);
    expect(w.seq).toBe(seq);
  });

  test("appear and gone", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.apply(ev({ op: "appear", entity: job("j2", "q") }))).toEqual({ type: "appear", id: "j2", at: "q" });
    w.apply(ev({ op: "set_level", level: { owner: "j2", metric: "mem", value: 1, max: 2 } }));
    expect(w.level("j2", "mem")?.value).toBe(1);
    expect(w.apply(ev({ op: "gone", id: "j2" }))).toEqual({ type: "gone", id: "j2", from: "q" });
    expect(w.entities.has("j2")).toBe(false);
    expect(w.level("j2", "mem")).toBeUndefined();
  });

  test("places, states, levels, flashes, staleness", () => {
    const w = ClientWorld.fromSnapshot(snap());
    w.apply(ev({ op: "upsert_place", place: place("x", { kind: "slot" }) }));
    expect(w.places.get("x")?.kind).toBe("slot");
    w.apply(ev({ op: "set_level", level: { owner: "x", metric: "disk", value: 5, max: 10 } }));
    w.apply(ev({ op: "remove_place", id: "x" }));
    expect(w.places.has("x")).toBe(false);
    expect(w.level("x", "disk")).toBeUndefined();
    w.apply(ev({ op: "set_state", id: "j1", state: "running" }));
    expect(w.entities.get("j1")?.state).toBe("running");
    expect(w.apply(ev({ op: "flash", at: "q", kind: "oom" }))).toEqual({ type: "flash", at: "q", kind: "oom" });
    w.apply(ev({ op: "stale", source: "gitlab", since_ms: 7 }));
    expect(w.stale.get("gitlab")).toBe(7);
    w.apply(ev({ op: "fresh", source: "gitlab" }));
    expect(w.stale.size).toBe(0);
  });

  test("a move of an unknown entity is harmless", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.apply(ev({ op: "move", id: "ghost", to: "q" }))).toEqual({ type: "none" });
  });
});
