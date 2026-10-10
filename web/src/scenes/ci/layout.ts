import type { ClientWorld } from "../../world";
import type { Entity, Id, Place } from "../../types";

export interface Rect { x: number; y: number; w: number; h: number }
export interface PlaceBox extends Rect { id: Id; kind: string; label: string }
export interface Spot { x: number; y: number }
export interface Wall { x1: number; y1: number; x2: number; y2: number }
export interface Layout {
  width: number;
  height: number;
  boxes: Map<Id, PlaceBox>;
  tokens: Map<Id, Spot>;
  overflow: Map<Id, number>;
  walls: Wall[];
  titles: { x: number; y: number; text: string }[];
}

export const WIDTH = 1200;
export const MAX_TOKENS_IN_ZONE = 40;

const TOP = 48;
const COL = { projects: { x: 20, w: 200 }, queue: { x: 270, w: 220 }, hosts: { x: 540, w: 420 }, exits: { x: 1010, w: 170 } };
const CELL = 26, PER_ROW = 7, ZONE_HEAD = 28, ZONE_PAD = 12;
const SLOT_H = 34, SLOT_GAP = 6, PROC_HEAD = 24, HOST_HEAD = 30, DISK_H = 26;
const MAX_IN_ROW = 10;

const byLabel = (a: Place, b: Place) => a.label.localeCompare(b.label, undefined, { numeric: true }) || a.id.localeCompare(b.id);
const agentOf = (p: Place) => String(p.attrs.agent ?? "");

function children(w: ClientWorld, parent: Id, kind: string): Place[] {
  return [...w.places.values()].filter((p) => p.parent === parent && p.kind === kind).sort(byLabel);
}

function residents(w: ClientWorld, place: Id): Entity[] {
  return [...w.entities.values()].filter((e) => e.place === place).sort((a, b) => a.since_ms - b.since_ms || a.id.localeCompare(b.id));
}

function box(out: Layout, p: Place | { id: Id; kind: string; label: string }, r: Rect): void {
  out.boxes.set(p.id, { id: p.id, kind: p.kind, label: p.label, ...r });
}

/** A grid zone (queue, stuck): tokens in rows, at most MAX_TOKENS_IN_ZONE. Returns the bottom. */
function gridZone(w: ClientWorld, out: Layout, place: Place | undefined, x: number, y: number, width: number, minH: number): number {
  if (!place) return y;
  const people = residents(w, place.id);
  const shown = people.slice(0, MAX_TOKENS_IN_ZONE);
  const rows = Math.max(1, Math.ceil(shown.length / PER_ROW));
  const h = Math.max(minH, ZONE_HEAD + rows * CELL + ZONE_PAD);
  box(out, place, { x, y, w: width, h });
  shown.forEach((e, i) =>
    out.tokens.set(e.id, { x: x + ZONE_PAD + CELL / 2 + (i % PER_ROW) * CELL, y: y + ZONE_HEAD + CELL / 2 + Math.floor(i / PER_ROW) * CELL }),
  );
  if (people.length > shown.length) out.overflow.set(place.id, people.length - shown.length);
  return y + h;
}

/** Tokens in a row starting at x, at most MAX_IN_ROW. */
function row(w: ClientWorld, out: Layout, place: Id, x: number, y: number, step: number): void {
  residents(w, place).slice(0, MAX_IN_ROW).forEach((e, i) => out.tokens.set(e.id, { x: x + i * step, y }));
}

export function layoutCi(w: ClientWorld): Layout {
  const out: Layout = { width: WIDTH, height: 0, boxes: new Map(), tokens: new Map(), overflow: new Map(), walls: [], titles: [] };
  const bottoms: number[] = [];
  const title = (id: Id, x: number) => out.titles.push({ x, y: TOP - 14, text: w.places.get(id)?.label ?? "" });

  // Projects: where jobs are born.
  title("projects", COL.projects.x);
  let y = TOP;
  for (const p of children(w, "projects", "project")) {
    box(out, p, { x: COL.projects.x, y, w: COL.projects.w, h: 36 });
    row(w, out, p.id, COL.projects.x + COL.projects.w - 16, y + 18, -12);
    y += 46;
  }
  bottoms.push(y);

  // Queue and the stuck zone under it.
  let qy = gridZone(w, out, w.places.get("queue"), COL.queue.x, TOP, COL.queue.w, 110);
  qy = gridZone(w, out, w.places.get("stuck"), COL.queue.x, qy + 20, COL.queue.w, 80);
  bottoms.push(qy);

  // Hosts → processes → slots; a wall between processes of different agents.
  title("hosts", COL.hosts.x);
  let hy = TOP;
  for (const host of children(w, "hosts", "host")) {
    const top = hy;
    let cy = hy + HOST_HEAD;
    const procs = children(w, host.id, "process").sort((a, b) => agentOf(a).localeCompare(agentOf(b)) || byLabel(a, b));
    procs.forEach((proc, i) => {
      if (i > 0 && agentOf(procs[i - 1]) !== agentOf(proc)) {
        out.walls.push({ x1: COL.hosts.x + 10, y1: cy + 2, x2: COL.hosts.x + COL.hosts.w - 10, y2: cy + 2 });
        cy += 12;
      }
      const slots = children(w, proc.id, "slot");
      const ph = PROC_HEAD + slots.length * (SLOT_H + SLOT_GAP) + 6;
      box(out, proc, { x: COL.hosts.x + 10, y: cy, w: COL.hosts.w - 20, h: ph });
      row(w, out, proc.id, COL.hosts.x + 170, cy + 12, 16);
      slots.forEach((s, si) => {
        const sy = cy + PROC_HEAD + si * (SLOT_H + SLOT_GAP);
        box(out, s, { x: COL.hosts.x + 20, y: sy, w: COL.hosts.w - 40, h: SLOT_H });
        row(w, out, s.id, COL.hosts.x + 38, sy + SLOT_H / 2, 14);
      });
      cy += ph + 8;
    });
    if (w.level(host.id, "disk")) cy += DISK_H;
    box(out, host, { x: COL.hosts.x, y: top, w: COL.hosts.w, h: cy - top + 6 });
    hy = cy + 26;
  }

  // Runners we see in GitLab but have no agent for.
  const loose = children(w, "nohost", "process");
  if (loose.length > 0) {
    const top = hy;
    let ly = hy + HOST_HEAD;
    for (const r of loose) {
      box(out, r, { x: COL.hosts.x + 10, y: ly, w: COL.hosts.w - 20, h: 30 });
      row(w, out, r.id, COL.hosts.x + 170, ly + 15, 16);
      ly += 38;
    }
    box(out, { id: "nohost", kind: "zone", label: w.places.get("nohost")?.label ?? "" }, { x: COL.hosts.x, y: top, w: COL.hosts.w, h: ly - top });
    hy = ly + 20;
  }
  bottoms.push(hy);

  // Exits.
  title("exits", COL.exits.x);
  let ey = TOP;
  for (const x of children(w, "exits", "exit")) {
    box(out, x, { x: COL.exits.x, y: ey, w: COL.exits.w, h: 44 });
    row(w, out, x.id, COL.exits.x + 20, ey + 22, 16);
    ey += 56;
  }
  bottoms.push(ey);

  out.height = Math.max(...bottoms) + 24;
  return out;
}
