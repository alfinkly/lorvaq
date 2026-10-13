import { ClientWorld, type Effect } from "./world";
import type { SceneInfo, Snapshot, WorldEvent } from "./types";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** 401: the viewer or admin has to sign in. */
export class AuthError extends Error {
  constructor() {
    super("sign in required");
  }
}

export class ApiError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
  }
}

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 401) throw new AuthError();
  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const message = (data as { error?: string } | null)?.error ?? `${method} ${url}: ${res.status}`;
    throw new ApiError(res.status, message);
  }
  return data as T;
}

const getJson = <T>(url: string) => request<T>("GET", url);

export interface Me {
  demo: boolean;
  setup_needed: boolean;
  admin: boolean;
  viewer: boolean;
}

export type Access = { mode: "open" } | { mode: "admin" } | { mode: "proxy"; header: string; trusted: string[] };

export interface HistoryInfo {
  retention_days: number;
  scenes: { id: string; queued: number; dropped: number; written: number; last_error: string | null }[];
}

export interface ExitRule {
  label: string;
  stage?: string | null;
  name?: string | null;
}

/** The fields the settings page edits; the rest is sent back untouched. */
export interface CiSettings {
  exits: ExitRule[];
  max_projects: number;
  default_capacity: number;
  capacities: Record<string, number>;
  [key: string]: unknown;
}

export const fetchScenes = () => getJson<SceneInfo[]>("/api/v1/scenes");
export const fetchSnapshot = (id: string) => getJson<Snapshot>(`/api/v1/scenes/${encodeURIComponent(id)}/snapshot`);
export const fetchMe = () => getJson<Me>("/api/v1/me");
export const setup = (code: string, password: string) => request<{ ok: true }>("POST", "/api/v1/setup", { code, password });
export const login = (password: string) => request<{ ok: true }>("POST", "/api/v1/login", { password });
export const logout = () => request<{ ok: true }>("POST", "/api/v1/logout");
export const changePassword = (current: string, next: string) => request<{ ok: true }>("POST", "/api/v1/admin/password", { current, new: next });
export const getAccess = () => getJson<Access>("/api/v1/admin/access");
export const putAccess = (access: Access) => request<Access>("PUT", "/api/v1/admin/access", access);
export const getHistory = () => getJson<HistoryInfo>("/api/v1/admin/history");
export const putHistory = (retention_days: number) => request<{ retention_days: number }>("PUT", "/api/v1/admin/history", { retention_days });
export const getSceneSettings = <T>(id: string) => getJson<T>(`/api/v1/admin/scenes/${encodeURIComponent(id)}/settings`);
export const putSceneSettings = <T>(id: string, settings: T) => request<T>("PUT", `/api/v1/admin/scenes/${encodeURIComponent(id)}/settings`, settings);

export interface GitlabStatus {
  connected: boolean;
  url: string | null;
  user: string | null;
  kind: "oauth" | "token" | null;
  state: "disconnected" | "ok" | "failing" | "reauthorize";
  last_ok_ms: number | null;
  last_error: string | null;
  redirect_uri: string | null;
}

export const getGitlab = () => getJson<GitlabStatus>("/api/v1/admin/sources/gitlab");
export const connectGitlabToken = (url: string, token: string) => request<{ connected: true; user: string; url: string }>("POST", "/api/v1/admin/sources/gitlab/token", { url, token });
export const startGitlabOauth = (url: string, client_id: string, client_secret: string | null) =>
  request<{ authorize_url: string }>("POST", "/api/v1/admin/sources/gitlab/oauth", { url, client_id, client_secret });
export const disconnectGitlab = () => request<{ ok: true }>("DELETE", "/api/v1/admin/sources/gitlab");

export interface Subscription {
  close(): void;
}

export type FollowStatus = "live" | "reconnecting" | "locked";

/**
 * Keeps a world in sync: snapshot, then events after its seq. Any break —
 * network, server restart, 410 for a gap — means a fresh snapshot. A 401 means
 * the viewer must sign in: report "locked" and stop.
 */
export function follow(sceneId: string, onWorld: (world: ClientWorld, effects: Effect[]) => void, onStatus: (status: FollowStatus) => void): Subscription {
  let closed = false;
  let source: EventSource | null = null;
  let streamDone: (() => void) | null = null;

  const stream = (world: ClientWorld) =>
    new Promise<void>((done) => {
      if (closed) {
        done();
        return;
      }
      streamDone = done;
      source = new EventSource(`/api/v1/scenes/${encodeURIComponent(sceneId)}/events?since=${world.seq}`);
      source.onopen = () => onStatus("live");
      source.onmessage = (m: MessageEvent<string>) => onWorld(world, [world.apply(JSON.parse(m.data) as WorldEvent)]);
      source.onerror = () => {
        source?.close();
        done();
      };
    });

  void (async () => {
    while (!closed) {
      try {
        const snapshot = await fetchSnapshot(sceneId);
        if (closed) break;
        const world = ClientWorld.fromSnapshot(snapshot);
        onWorld(world, []);
        await stream(world);
      } catch (e) {
        if (e instanceof AuthError) {
          onStatus("locked");
          return;
        }
        // Server unreachable: retry below.
      }
      if (!closed) {
        onStatus("reconnecting");
        await sleep(1500);
      }
    }
  })();

  return {
    close() {
      closed = true;
      source?.close();
      streamDone?.();
    },
  };
}
