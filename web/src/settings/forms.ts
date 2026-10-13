import type { Access, ExitRule } from "../api";
import { t } from "../i18n";

export const MIN_PASSWORD = 12;
export const MAX_PASSWORD = 1024;
const HEADER = /^[A-Za-z0-9-]+$/;
const NETWORK = /^[0-9A-Fa-f:.]+(\/\d{1,3})?$/;

export function passwordProblem(password: string, confirm: string): string | null {
  if ([...password].length < MIN_PASSWORD) return t.passwordTooShort(MIN_PASSWORD);
  if ([...password].length > MAX_PASSWORD) return t.passwordTooLong(MAX_PASSWORD);
  if (password !== confirm) return t.passwordMismatch;
  return null;
}

export function parseList(text: string): string[] {
  return text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);
}

/** A first check in the browser; the server has the final word. */
export function accessProblem(access: Access): string | null {
  if (access.mode !== "proxy") return null;
  if (!HEADER.test(access.header)) return t.badHeader;
  if (access.trusted.length === 0) return t.noTrusted;
  const bad = access.trusted.find((x) => !NETWORK.test(x));
  return bad === undefined ? null : t.badNetwork(bad);
}

export function exitsProblem(exits: ExitRule[]): string | null {
  const seen = new Set<string>();
  for (const exit of exits) {
    const label = exit.label.trim();
    if (!label) return t.exitNoLabel;
    if (seen.has(label)) return t.exitTwice(label);
    seen.add(label);
    if (!exit.stage && !exit.name) return t.exitNoRule(label);
    for (const pattern of [exit.stage, exit.name]) {
      if (!pattern) continue;
      try {
        new RegExp(`^(?:${pattern})$`);
      } catch {
        return t.badPattern(pattern);
      }
    }
  }
  return null;
}

export function parseCapacities(text: string): Record<string, number> | string {
  const out: Record<string, number> = {};
  for (const line of text.split("\n").map((l) => l.trim()).filter(Boolean)) {
    const [name, slots, ...rest] = line.split("=").map((s) => s.trim());
    const n = Number(slots);
    if (!name || rest.length > 0 || !Number.isInteger(n) || n < 1) return t.badCapacity(line);
    out[name] = n;
  }
  return out;
}

export function formatCapacities(capacities: Record<string, number>): string {
  return Object.keys(capacities).sort().map((k) => `${k}=${capacities[k]}`).join("\n");
}

export function retentionProblem(days: number): string | null {
  return Number.isInteger(days) && days >= 1 && days <= 365 ? null : t.badRetention;
}

export function normalizeGitlabUrl(url: string): string {
  return url.trim().replace(/\/+$/, "");
}

export function gitlabUrlProblem(url: string): string | null {
  try {
    const u = new URL(url.trim());
    if (!["http:", "https:"].includes(u.protocol) || u.search || u.hash || u.username || u.password) return t.gitlabBadUrl;
    return null;
  } catch {
    return t.gitlabBadUrl;
  }
}

export function tokenPageUrl(base: string): string {
  return `${normalizeGitlabUrl(base)}/-/user_settings/personal_access_tokens?name=lorvaq&scopes=read_api`;
}

/** The result of an OAuth round trip, from `#/settings?gitlab=<outcome>`. */
export function oauthOutcome(hash: string): string | null {
  const query = hash.split("?")[1];
  if (!query) return null;
  const outcome = new URLSearchParams(query).get("gitlab");
  if (!outcome) return null;
  return t.gitlabOutcome[outcome] ?? t.gitlabOutcomeOther;
}

export function hostNameProblem(name: string): string | null {
  return /^[a-z0-9][a-z0-9._-]{0,39}$/.test(name.trim()) ? null : t.badHostName;
}
