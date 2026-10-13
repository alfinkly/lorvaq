# lorvaq

Live scenes for real-time monitoring. Where Grafana shows numbers over time,
lorvaq shows **things in places that move**: a CI job travels from the queue
into a runner slot, its memory fills the slot, an OOM flashes red, a build flies
off to the registry. You see not only *how much* but *why and where*.

Status: early. The CI scene runs on real GitLab jobs and runners (with an agent
on each runner host for slots, memory, disk and OOM kills), or on a built-in
simulator.

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

## Connect GitLab

lorvaq reads jobs and runners through GitLab's GraphQL API with the `read_api`
scope. It needs a GitLab **administrator** to see the jobs of every runner; a
dedicated service user is best, so the connection does not depend on a person.

In Settings → GitLab, either:

- **Authorize** — in GitLab create an application (Admin → Applications, or
  User settings → Applications) with the redirect URI lorvaq shows and the
  `read_api` scope, paste its Application ID and press Authorize. This needs
  `LORVAQ_PUBLIC_URL`. Tokens are refreshed automatically.
- **Paste a token** — the link opens GitLab's token page with the name and scope
  filled in.

Tokens are stored encrypted with `LORVAQ_SECRET_KEY`. lorvaq polls jobs every 3
seconds and runners every minute. If GitLab stops answering, the scene shows a
"data is old" banner after 10 seconds.

GitLab with a certificate from a private CA is not supported yet.

## Add runner hosts

GitLab knows which jobs run; only the runner host knows how much memory they
take. In Settings → Hosts name a host and press "Add host": lorvaq shows a
`docker run … lorvaq agent` command with a one-time join token. Run it on the
runner host, in the same docker daemon as gitlab-runner (for rootless docker,
as the runner's user). The token is shown once and stored only as a hash;
removing the host disconnects its agent at once.

The agent only reads the docker daemon (list, stats, inspect, events) and opens
no port: it connects out to `LORVAQ_SERVER` over WebSocket and reports every 2
seconds. It needs to reach that address, so allow it in egress rules of the
runner host. It finds job containers by gitlab-runner's labels
(`com.gitlab.gitlab-runner.job.id`, `…type`, `…runner.id`) and gives each
running job a slot of the process.

The slot count comes from the scene settings (`capacities`, `default_capacity`)
unless the agent can read gitlab-runner's metrics: set `listen_address` in the
runner's `config.toml`, run the agent with `--network host` and add
`-e LORVAQ_RUNNER_METRICS=http://127.0.0.1:9252/metrics`. Then lorvaq also
knows each registration's `limit` and shows jobs waiting for it.

Mounting the docker socket gives the agent full access to the daemon, read-only
mount or not; lorvaq's agent never uses more than reading, but treat its image
like any container you give the socket to.

## Behind a reverse proxy

Put https in front of lorvaq and set `LORVAQ_PUBLIC_URL` to its https address.

To let your proxy decide who sees the scenes, choose "People your reverse proxy
signed in" in Settings: name the header your proxy sets with the user name and
list the proxy's addresses. lorvaq then trusts that header — and the client
address in `X-Forwarded-For` for sign-in limits — only from those addresses.
Anything that can reach lorvaq from a listed address can set them, so list only
the proxy itself, not a whole network other services share.

In the other modes, all sign-in attempts through a proxy share the proxy's
address: a few wrong passwords lock sign-in for everyone for 15 minutes.

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
