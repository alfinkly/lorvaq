# lorvaq

Live scenes for real-time monitoring. Where Grafana shows numbers over time,
lorvaq shows **things in places that move**: a CI job travels from the queue
into a runner slot, its memory fills the slot, an OOM flashes red, a build flies
off to the registry. You see not only *how much* but *why and where*.

Status: early. This release runs the CI scene on a built-in simulator; GitLab,
the runner agent, history and settings come next.

## Try it

On simulated data, no database:

    docker compose -f compose.demo.yml up --build

Open http://localhost:8080.

## Run it

lorvaq keeps its history and settings in Postgres. Create `.env` next to
`compose.yml`:

    POSTGRES_PASSWORD=<output of: openssl rand -hex 24>
    LORVAQ_SECRET_KEY=<output of: openssl rand -base64 32, or lorvaq keygen>
    LORVAQ_PUBLIC_URL=https://lorvaq.example.com   # optional; https turns on Secure cookies

The database password goes into a URL, so keep it URL-safe: hex digits, as
`openssl rand -hex 24` prints, not `-base64` (its `/` and `+` break the URL).

Then:

    docker compose up -d --build
    docker compose logs lorvaq | grep "setup code"

Open lorvaq, enter the one-time setup code and choose the admin password. In
Settings decide who sees the scenes: anyone, people your reverse proxy signed
in, or only the admin. Lost the password? `docker compose exec lorvaq /lorvaq
admin reset-password` prints a new setup code and signs everybody out.

Sign-in attempts are limited to five per address in 15 minutes (IPv6 clients
count by their /64). Behind a reverse proxy that lorvaq does not trust (access
mode other than `proxy`), every client arrives from the proxy's address, so all
sign-in attempts share one allowance: a stranger who guesses wrong five times
blocks the admin's sign-in for 15 minutes. In `proxy` mode lorvaq takes the client
address from `X-Forwarded-For` when the request comes from a trusted proxy.

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

Copyright (C) 2026 Alimzhan Kangerey.

Licensed under the GNU Affero General Public License v3.0 only
(`AGPL-3.0-only`). See [LICENSE](LICENSE).
