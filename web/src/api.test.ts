import { afterEach, describe, expect, test, vi } from "vitest";
import { follow } from "./api";
import type { Snapshot } from "./types";

const snapshot: Snapshot = { seq: 1, places: [], entities: [], levels: [], stale: {} };

class FakeEventSource {
  static opened: FakeEventSource[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((m: MessageEvent<string>) => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(readonly url: string) {
    FakeEventSource.opened.push(this);
  }
  close() {
    this.closed = true;
  }
}

afterEach(() => {
  vi.unstubAllGlobals();
  FakeEventSource.opened = [];
});

describe("follow", () => {
  test("close() while the snapshot is loading leaves no EventSource open", async () => {
    let release: (() => void) | undefined;
    const gate = new Promise<void>((resolve) => (release = resolve));
    vi.stubGlobal("fetch", async () => {
      await gate;
      return { ok: true, json: async () => snapshot };
    });
    vi.stubGlobal("EventSource", FakeEventSource);

    const onWorld = vi.fn();
    const sub = follow("ci", onWorld, () => {});
    sub.close();
    release?.();
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(FakeEventSource.opened.filter((s) => !s.closed)).toEqual([]);
    expect(onWorld).not.toHaveBeenCalled();
  });

  test("opens the stream after the snapshot while still subscribed", async () => {
    vi.stubGlobal("fetch", async () => ({ ok: true, json: async () => snapshot }));
    vi.stubGlobal("EventSource", FakeEventSource);

    const sub = follow("ci", () => {}, () => {});
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(FakeEventSource.opened).toHaveLength(1);
    expect(FakeEventSource.opened[0].url).toContain("since=1");
    sub.close();
    expect(FakeEventSource.opened[0].closed).toBe(true);
  });

  test("a 401 on the snapshot reports locked and stops", async () => {
    let calls = 0;
    vi.stubGlobal("fetch", async () => {
      calls += 1;
      return { ok: false, status: 401, json: async () => ({ error: "sign in" }) };
    });
    vi.stubGlobal("EventSource", FakeEventSource);
    const statuses: string[] = [];
    follow("ci", () => {}, (s) => statuses.push(s));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(statuses).toEqual(["locked"]);
    expect(calls).toBe(1);
    expect(FakeEventSource.opened).toHaveLength(0);
  });
});
