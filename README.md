# lorvaq

Live scenes for real-time monitoring. Where Grafana shows numbers over time,
lorvaq shows **things in places that move**: a CI job travels from the queue
into a runner slot, its memory fills the slot, an OOM flashes red, a build flies
off to the registry. You see not only *how much* but *why and where*.

Status: early. This release runs the CI scene on a built-in simulator; GitLab,
the runner agent, history and settings come next.

## Try it

    docker compose -f compose.demo.yml up --build

Open http://localhost:8080.

## Develop

    cargo run -- server --demo          # API and SSE on :8080
    cd web && npm ci && npm run dev     # UI on :5173, proxied to :8080

Tests: `cargo test` and `cd web && npm test`.

## How it works

Every scene is described by four things: **places** (queue, hosts, slots),
**entities** in them (jobs), **moves** between places and **levels** against
limits (memory, disk). Sources feed a scene reducer, the reducer emits changes,
valid changes become numbered events; the browser takes a snapshot and follows
the event stream (SSE).

Design: `docs/design/`.

## License

MIT
