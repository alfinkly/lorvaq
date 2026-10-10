import { describe, expect, test } from "vitest";
import { ClientWorld } from "../../world";
import type { Entity, Level, Place } from "../../types";
import { layoutCi, MAX_TOKENS_IN_ZONE, type PlaceBox, type Spot } from "./layout";

type P = Partial<Place> & { id: string };
type E = Partial<Entity> & { id: string; place: string };

function world(places: P[], entities: E[] = [], levels: Level[] = []): ClientWorld {
  return ClientWorld.fromSnapshot({
    seq: 1,
    stale: {},
    levels,
    places: places.map((p) => ({ kind: "zone", parent: null, label: p.id, capacity: null, attrs: {}, ...p })),
    entities: entities.map((e, i) => ({ kind: "job", state: "pending", attrs: {}, since_ms: i, ...e })),
  });
}

const zones: P[] = [
  { id: "projects" }, { id: "queue", kind: "queue" }, { id: "stuck", kind: "stuck" },
  { id: "hosts" }, { id: "nohost" }, { id: "exits" }, { id: "sink", kind: "sink" },
];

const host: P[] = [
  { id: "host:box", kind: "host", parent: "hosts" },
  { id: "process:a/p", kind: "process", parent: "host:box", label: "p", capacity: 2, attrs: { agent: "a", online: true } },
  { id: "slot:a/p/0", kind: "slot", parent: "process:a/p", label: "#0" },
  { id: "slot:a/p/1", kind: "slot", parent: "process:a/p", label: "#1" },
  { id: "process:b/q", kind: "process", parent: "host:box", label: "q", capacity: 1, attrs: { agent: "b", online: true } },
  { id: "slot:b/q/0", kind: "slot", parent: "process:b/q", label: "#0" },
];

const inside = (s: Spot, b: PlaceBox) => s.x >= b.x && s.x <= b.x + b.w && s.y >= b.y && s.y <= b.y + b.h;

describe("layoutCi", () => {
  test("every token sits inside its place", () => {
    const w = world(
      [...zones, ...host,
        { id: "project:g/a", kind: "project", parent: "projects", label: "a" },
        { id: "exit:prod", kind: "exit", parent: "exits", label: "prod" },
        { id: "runner:7", kind: "process", parent: "nohost", label: "runner7" }],
      [
        { id: "job:1", place: "project:g/a" }, { id: "job:2", place: "queue" }, { id: "job:3", place: "stuck" },
        { id: "job:4", place: "slot:a/p/1" }, { id: "job:5", place: "process:a/p" }, { id: "job:6", place: "exit:prod" },
        { id: "job:7", place: "runner:7" },
      ],
    );
    const l = layoutCi(w);
    expect(l.tokens.size).toBe(7);
    for (const e of w.entities.values()) {
      const spot = l.tokens.get(e.id)!;
      const box = l.boxes.get(e.place)!;
      expect(inside(spot, box), `${e.id} in ${e.place}`).toBe(true);
    }
  });

  test("a huge queue shows 40 tokens and counts the rest", () => {
    const jobs: E[] = Array.from({ length: 200 }, (_, i) => ({ id: `job:${i}`, place: "queue" }));
    const l = layoutCi(world(zones, jobs));
    const queue = l.boxes.get("queue")!;
    const shown = jobs.filter((j) => l.tokens.has(j.id));
    expect(shown.length).toBe(MAX_TOKENS_IN_ZONE);
    expect(l.overflow.get("queue")).toBe(160);
    for (const j of shown) expect(inside(l.tokens.get(j.id)!, queue)).toBe(true);
    expect(l.tokens.has("job:0")).toBe(true);
    expect(l.tokens.has("job:199")).toBe(false);
  });

  test("processes of different agents are split by a wall", () => {
    const l = layoutCi(world([...zones, ...host]));
    expect(l.walls.length).toBe(1);
    const p = l.boxes.get("process:a/p")!;
    const q = l.boxes.get("process:b/q")!;
    expect(q.y).toBeGreaterThan(p.y + p.h);
  });

  test("the sink has no spots and the layout is deterministic", () => {
    const w = world([...zones, ...host], [{ id: "job:1", place: "sink" }]);
    const a = layoutCi(w);
    expect(a.tokens.has("job:1")).toBe(false);
    expect(JSON.stringify([...a.boxes])).toBe(JSON.stringify([...layoutCi(w).boxes]));
  });

  test("the scene grows with hosts", () => {
    const one = layoutCi(world([...zones, ...host])).height;
    const two = layoutCi(world([...zones, ...host,
      { id: "host:other", kind: "host", parent: "hosts" },
      { id: "process:c/r", kind: "process", parent: "host:other", label: "r", attrs: { agent: "c" } },
      { id: "slot:c/r/0", kind: "slot", parent: "process:c/r", label: "#0" }])).height;
    expect(two).toBeGreaterThan(one);
  });
});
