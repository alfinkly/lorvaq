/** Categorical palette, fixed order. A color follows its key, never its rank. */
export const PALETTE = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948"];
export const NEUTRAL = "#888780";

export class ColorBook {
  private assigned = new Map<string, string>();

  get(key: string): string {
    if (!key || key === "~other") return NEUTRAL;
    const known = this.assigned.get(key);
    if (known) return known;
    if (this.assigned.size >= PALETTE.length) return NEUTRAL;
    const color = PALETTE[this.assigned.size];
    this.assigned.set(key, color);
    return color;
  }
}
