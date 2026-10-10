import type { ClientWorld, Effect } from "../../world";
import type { Id } from "../../types";
import { ColorBook } from "../../colors";
import { t } from "../../i18n";
import { layoutCi, type Layout, type PlaceBox } from "./layout";

const NS = "http://www.w3.org/2000/svg";
const MOVE_MS = 700;
const FADE_MS = 700;
const FLASH_MS = 1200;
const RADIUS = 7;
const DEPTH: Record<string, number> = { zone: 0, queue: 0, stuck: 0, project: 1, host: 1, exit: 1, process: 2, slot: 3 };

interface Token {
  el: SVGCircleElement;
  x: number; y: number;
  fromX: number; fromY: number;
  toX: number; toY: number;
  start: number;
  fadeStart: number | null;
  hidden: boolean;
}

interface PlaceEls { g: SVGGElement; rect: SVGRectElement; fill: SVGRectElement; label: SVGTextElement; extra: SVGTextElement }

function el<K extends keyof SVGElementTagNameMap>(tag: K, parent: Element, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const node = document.createElementNS(NS, tag);
  set(node, attrs);
  parent.appendChild(node);
  return node;
}

function set(node: Element, attrs: Record<string, string | number>): void {
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
}

const ease = (p: number) => 1 - Math.pow(1 - p, 3);
const gib = (bytes: number) => (bytes / 2 ** 30).toFixed(1);

/** Draws a CI world into an SVG and animates tokens between places. */
export class CiRenderer {
  private placesLayer: SVGGElement;
  private overlay: SVGGElement;
  private tokenLayer: SVGGElement;
  private placeEls = new Map<Id, PlaceEls>();
  private tokens = new Map<Id, Token>();
  private births = new Map<Id, Id>();
  private flashes = new Map<Id, number>();
  private colors = new ColorBook();
  private world: ClientWorld | null = null;
  private dirty = false;
  private raf = 0;

  constructor(private svg: SVGSVGElement) {
    this.placesLayer = el("g", svg);
    this.overlay = el("g", svg);
    this.tokenLayer = el("g", svg);
    this.raf = requestAnimationFrame(this.frame);
  }

  update(world: ClientWorld, effects: Effect[]): void {
    this.world = world;
    const now = performance.now();
    for (const e of effects) {
      if (e.type === "flash") this.flashes.set(e.at, now + FLASH_MS);
      if (e.type === "appear") this.births.set(e.id, e.at);
    }
    this.dirty = true;
  }

  destroy(): void {
    cancelAnimationFrame(this.raf);
    this.svg.replaceChildren();
  }

  private frame = (now: number): void => {
    if (this.dirty && this.world) {
      this.dirty = false;
      const layout = layoutCi(this.world);
      this.svg.setAttribute("viewBox", `0 0 ${layout.width} ${layout.height}`);
      this.drawPlaces(this.world, layout, now);
      this.drawOverlay(layout);
      this.placeTokens(this.world, layout, now);
    }
    this.animate(now);
    this.raf = requestAnimationFrame(this.frame);
  };

  private drawPlaces(w: ClientWorld, layout: Layout, now: number): void {
    for (const [id, p] of this.placeEls) {
      if (!layout.boxes.has(id)) {
        p.g.remove();
        this.placeEls.delete(id);
      }
    }
    const boxes = [...layout.boxes.values()].sort((a, b) => (DEPTH[a.kind] ?? 0) - (DEPTH[b.kind] ?? 0));
    for (const b of boxes) {
      let p = this.placeEls.get(b.id);
      if (!p) {
        const g = el("g", this.placesLayer, { class: `place ${b.kind}` });
        p = { g, rect: el("rect", g, { rx: 6 }), fill: el("rect", g, { rx: 6, class: "fill" }), label: el("text", g, { class: "label" }), extra: el("text", g, { class: "extra" }) };
        this.placeEls.set(b.id, p);
      }
      this.placesLayer.appendChild(p.g); // keeps depth order
      set(p.rect, { x: b.x, y: b.y, width: b.w, height: b.h });
      p.label.textContent = b.kind === "slot" ? "" : b.label;
      set(p.label, { x: b.x + 10, y: b.y + 18 });
      p.g.classList.toggle("flash", (this.flashes.get(b.id) ?? 0) > now);
      p.g.classList.toggle("offline", w.places.get(b.id)?.attrs.online === false);
      this.drawFill(w, b, p);
    }
  }

  private drawFill(w: ClientWorld, b: PlaceBox, p: PlaceEls): void {
    set(p.fill, { x: b.x, y: b.y, width: 0, height: b.h, class: "fill" });
    p.fill.style.fill = "";
    p.extra.textContent = "";
    if (b.kind === "slot") {
      set(p.extra, { x: b.x + b.w - 10, y: b.y + 21 });
      const job = [...w.entities.values()].find((e) => e.place === b.id);
      if (!job) {
        p.extra.textContent = t.free;
        return;
      }
      const mem = w.level(job.id, "mem");
      const frac = mem?.max ? Math.min(1, mem.value / mem.max) : 0;
      set(p.fill, { width: b.w * frac });
      p.fill.style.fill = frac >= 0.9 ? "var(--warn)" : this.colors.get(String(job.attrs.project ?? ""));
      p.fill.classList.toggle("oom", job.state === "oom");
      const usage = mem?.max ? `${gib(mem.value)} / ${gib(mem.max)} GB` : t.running;
      p.extra.textContent = `${String(job.attrs.name ?? "")} · ${usage}`;
    } else if (b.kind === "host") {
      const disk = w.level(b.id, "disk");
      if (!disk?.max) return;
      const frac = Math.min(1, disk.value / disk.max);
      set(p.fill, { x: b.x + 10, y: b.y + b.h - 22, width: (b.w - 20) * frac, height: 8 });
      p.fill.style.fill = frac >= 0.85 ? "var(--bad)" : frac >= 0.75 ? "var(--warn)" : "var(--ok)";
      p.extra.textContent = `${t.disk} ${Math.round(frac * 100)}%`;
      set(p.extra, { x: b.x + b.w - 10, y: b.y + b.h - 28 });
    } else if (b.kind === "process") {
      const place = w.places.get(b.id);
      p.extra.textContent = place?.attrs.online === false ? t.agentOffline : place?.capacity ? t.slots(place.capacity) : "";
      set(p.extra, { x: b.x + b.w - 10, y: b.y + 16 });
    }
  }

  private drawOverlay(layout: Layout): void {
    this.overlay.replaceChildren();
    for (const wl of layout.walls) el("line", this.overlay, { ...wl, class: "wall" });
    for (const ti of layout.titles) el("text", this.overlay, { x: ti.x, y: ti.y, class: "title" }).textContent = ti.text;
    for (const [id, n] of layout.overflow) {
      const b = layout.boxes.get(id);
      if (b) el("text", this.overlay, { x: b.x + b.w - 10, y: b.y + 18, class: "extra" }).textContent = `+${n}`;
    }
  }

  private placeTokens(w: ClientWorld, layout: Layout, now: number): void {
    for (const [id, tk] of this.tokens) if (!w.entities.has(id) && tk.fadeStart === null) tk.fadeStart = now;
    for (const e of w.entities.values()) {
      const spot = layout.tokens.get(e.id);
      let tk = this.tokens.get(e.id);
      if (!tk) {
        if (!spot) continue;
        const born = this.births.get(e.id);
        const from = born ? layout.boxes.get(born) : undefined;
        const sx = from ? from.x + from.w - 16 : spot.x;
        const sy = from ? from.y + 18 : spot.y;
        this.births.delete(e.id);
        tk = { el: el("circle", this.tokenLayer, { r: RADIUS }), x: sx, y: sy, fromX: sx, fromY: sy, toX: spot.x, toY: spot.y, start: now, fadeStart: null, hidden: false };
        this.tokens.set(e.id, tk);
      }
      tk.el.style.fill = this.colors.get(String(e.attrs.project ?? ""));
      tk.el.setAttribute("class", `token ${e.state}`);
      if (e.place === "sink") {
        if (tk.fadeStart === null) tk.fadeStart = now;
        continue;
      }
      tk.hidden = !spot;
      if (spot && (spot.x !== tk.toX || spot.y !== tk.toY)) {
        tk.fromX = tk.x;
        tk.fromY = tk.y;
        tk.toX = spot.x;
        tk.toY = spot.y;
        tk.start = now;
      }
    }
  }

  private animate(now: number): void {
    for (const [id, tk] of this.tokens) {
      const p = Math.min(1, (now - tk.start) / MOVE_MS);
      tk.x = tk.fromX + (tk.toX - tk.fromX) * ease(p);
      tk.y = tk.fromY + (tk.toY - tk.fromY) * ease(p);
      set(tk.el, { cx: tk.x, cy: tk.y });
      let opacity = tk.hidden ? 0 : 1;
      if (tk.fadeStart !== null) {
        opacity = Math.max(0, 1 - (now - tk.fadeStart) / FADE_MS);
        if (opacity === 0) {
          tk.el.remove();
          this.tokens.delete(id);
          continue;
        }
      }
      tk.el.style.opacity = String(opacity);
    }
    for (const [id, until] of this.flashes) {
      if (until <= now) {
        this.flashes.delete(id);
        this.dirty = true;
      }
    }
  }
}
