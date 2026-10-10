// Every UI string lives here.
export const t = {
  connecting: "connecting…",
  live: "live",
  reconnecting: "reconnecting…",
  offline: "server unreachable",
  stale: (source: string, seconds: number) => `${source} data is ${seconds}s old — showing the last known state`,
  free: "free",
  running: "running",
  disk: "disk",
  agentOffline: "agent offline",
  slots: (n: number) => `${n} slot${n === 1 ? "" : "s"}`,
};
