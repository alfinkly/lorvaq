import { ClientWorld, type Effect } from "./world";
import type { SceneInfo, Snapshot, WorldEvent } from "./types";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: ${res.status}`);
  return (await res.json()) as T;
}

export const fetchScenes = () => getJson<SceneInfo[]>("/api/v1/scenes");
export const fetchSnapshot = (id: string) => getJson<Snapshot>(`/api/v1/scenes/${encodeURIComponent(id)}/snapshot`);

export interface Subscription {
  close(): void;
}

/**
 * Keeps a world in sync: snapshot, then events after its seq. Any break —
 * network, server restart, 410 for a gap — means a fresh snapshot.
 */
export function follow(
  sceneId: string,
  onWorld: (world: ClientWorld, effects: Effect[]) => void,
  onStatus: (status: "live" | "reconnecting") => void,
): Subscription {
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
      } catch {
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
