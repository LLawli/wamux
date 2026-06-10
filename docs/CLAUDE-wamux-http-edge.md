# CLAUDE.md: wamux-http-edge — reference HTTP edge for the wamux core

> **Handoff:** rename this file to `CLAUDE.md` at the root of the new edge repo.
> It is written for an agent building the edge in a fresh folder. Read
> `PRD-wamux-http-edge.md` (carried alongside this file) for the product spec, and
> copy the core's `proto/` directory in verbatim — it is the gRPC contract and the
> source of truth. This file is the engineering doctrine; the PRD is the *what*.

`wamux-http-edge` is **the reference edge** of `wamux` and a **multi-tenant SaaS**.
The `wamux` core is a *relay-pure* WhatsApp daemon: it exposes raw WhatsApp
mechanisms over a **gRPC Unix domain socket with no auth of its own**, and pushes
*every* policy decision outward — to here. This project both (a) proves the
open-source core is usable, by being the canonical example of how to consume the
socket, and (b) is a real product: an own, clean HTTP API for third parties to pair
WhatsApp numbers, send/receive messages, and integrate via webhooks/streams.

Fully async on tokio. Agents are the primary reader: optimize for grep, small files,
explicit types.

## The prime directive: two layers, one of them is documentation

The code splits into **two layers** and the split is sacred:

- **`core-client/`** — the only code that talks to the wamux socket. It owns every
  responsibility the core refused (see "Inherited policy" below). It knows **nothing**
  about tenants, HTTP, auth, or billing. Its types are clean Rust. **A future author
  of a different edge copies this layer wholesale**, so write it to be read: no
  product concepts leak in, no `tenant_id` in a function that sends a WhatsApp message.
- **`product/`** — the SaaS: auth, tenants, REST, webhooks, quotas, dashboard. It
  depends on `core-client`, never the reverse.

When in doubt about where something goes: if it would still make sense in *someone
else's* edge, it belongs in `core-client`. If it only matters because we sell this,
it belongs in `product`. **Litmus test:** could a hobbyist building a personal
WhatsApp bot reuse this function unchanged? If yes, it's `core-client`.

## Inherited policy (what the core made *our* job)

The core is pure on purpose; these are non-negotiable and live in `core-client`:

- **Auth is ours, entirely.** The socket has none. In production this process is the
  *only* thing allowed to open the socket (same host, 0660 + owning group).
- **Connection policy.** The core boots every account **disconnected**. We hold the
  set of instances that should be live and call `ConnectAccount` on boot and after a
  core restart. The library handles *transient* drops itself (Fibonacci backoff); we
  react only to **terminal** exits (401/409/logout) with our own backoff + a
  `connection.update` webhook. Never add reconnect logic that fights the library.
- **JID policy.** Send 1:1 to `<number>@c.us` (legacy) to dodge the lib's PN→LID
  upgrade. Never guess the BR 9th digit — resolve number→canonical JID with
  `CheckOnWhatsApp` and cache it. The core relays the JID verbatim.
- **Delivery/retries/timeouts.** Composed from `SendResult.message_id` + `Receipt`
  events. The core has no "if X didn't happen in N seconds".
- **Gap recovery.** Core event delivery is lossy under lag: a `RawEvent{kind:"gap"}`
  arrives carrying current state. On gap, resync via `GetAccountStatus` and accept
  the loss. On the "all" subscription, a created-channel gap means new accounts may
  be missing → reconcile via `ListAccounts`.
- **Outbound HTTP is OURS — and ONLY ours.** The core never makes an outbound
  request (no SSRF surface there). The edge *does*: webhook delivery, fetching a
  media URL the tenant gave us, fetching a link-preview target. **This is the inverse
  of the core's doctrine** — every outbound request lives behind an SSRF guard
  (block private ranges/localhost by default; operator allow-list).
- **Media.** Send by URL = we download and stream bytes inline to `SendMedia`
  (client-streaming: header + chunks). Receive = the event carries a
  `MediaDescriptor`; we call `DownloadMedia` (server-streaming) on demand.
- **PTT.** WhatsApp renders voice notes only for **OGG/Opus**. Transcode (ffmpeg via
  `std::process` on `spawn_blocking`) and set `ptt`/`seconds`/`waveform`.
- **Link preview / ephemeral.** We supply the preview fields verbatim; we pass
  `ephemeral_seconds`. The core relays both.
- **History.** The core stores none. Backfill only with `backfill_history=true`;
  on-demand `FetchMessageHistory` needs the primary phone online.

## Stack
- `axum` for the HTTP server (routers, extractors, SSE, WebSocket). The only public
  surface; serve **plain HTTP** — TLS terminates at an external reverse proxy.
- `tonic` for the gRPC **client** to the core over UDS; `prost` for the generated
  messages. `tonic-build` in `build.rs` regenerates from the copied `proto/`. The
  `.proto` files are vendored from the core, version-pinned; never edit generated code.
- `tokio` (multi-thread) for async; `tower`/`tower-http` for middleware (auth,
  request logging, timeout, rate limit).
- `sqlx` (Postgres, runtime queries — no compile-time DB needed) for the edge's own
  database; one pool, cloned into services. Migrations via `sqlx::migrate!`.
- `askama` (compiled templates) + `htmx` for the server-rendered dashboard. One
  binary, no JS toolchain. Bilingual (PT + EN).
- `reqwest` for outbound HTTP (webhooks, URL/preview fetch) — always behind the SSRF
  guard.
- `argon2` (password hashing), `jsonwebtoken` (JWT), HMAC-SHA256 (webhook signing).
- `serde`/`serde_json`, `ulid` (public ids + event cursors).
- `thiserror` for typed domain errors; `anyhow` only at the top of `main`.
- `tracing` + `tracing-subscriber` for structured logs; a Prometheus exporter for
  `/metrics` (mirror the core: text over the existing HTTP server, no second listener).

## Commands (the agent must run these unattended)
- Build (regenerates proto):  `cargo build`
- Run the edge:               `cargo run` (reads config TOML + `WAMUX_EDGE_*` env)
- Format:                     `cargo fmt`
- Lint (zero tolerance):      `cargo clippy --all-targets -- -D warnings`
- Tests:                      `EDGE_DATABASE_URL=… cargo test` (needs Postgres + a running wamux core on a socket for integration tests)
- CI (all gates, one command): `scripts/ci.sh` — fmt + clippy + tests; this script
  IS the pipeline (no hosted CI by default). Run it before declaring work done.
- Postgres for tests/dev:     `docker run -d --name edge-pg -e POSTGRES_USER=edge -e POSTGRES_PASSWORD=edge -e POSTGRES_DB=edge -p 5434:5432 postgres:16`
- A wamux core for integration tests: build/run the core on a temp socket (the
  integration harness should spawn it, mirroring the core's own socket tests).

## Code style (shared DNA with the core)
- Functions: 4-20 lines. Split if longer.
- Files: under 500 lines. One cohesive unit per file.
- Names: specific and unique, under ~5 grep hits. `handle`, `process`, `data`,
  `util`, `manager`, `service` (bare) are banned — name the thing.
- Types: explicit at every boundary. Annotate public signatures and any
  request/response type crossing a module edge.
- No code duplication. Early returns over nested ifs. Max 2 indent levels.
- No `.unwrap()`/`.expect()` on anything reachable from an HTTP request or a core
  event. Map the error to a typed error → HTTP status (problem+json) or log + drop.
  `unwrap` only on startup invariants, with a comment.
- Errors carry context: the offending value and the expected shape.

## HTTP layer (`product/http/`)
- One router module per resource (`instances`, `messages`, `events`, `webhooks`,
  `auth`, `admin`). Handlers stay **thin**: extract + validate the request, call a
  `core-client` function or a `product` service, map the result to a response or a
  problem+json error. No business logic in handlers.
- Auth via extractors: a `Bearer` JWT extractor (dashboard) and an `X-Api-Key`
  extractor (m2m) that resolve to a `TenantCtx`. A handler that needs a tenant takes
  `TenantCtx`; one that needs an instance takes a resolved `Instance` (ownership
  checked in the extractor, never in the handler).
- Send is **synchronous** and honors `Idempotency-Key`: dedupe on
  (tenant, key) → return the stored `message_id` on replay.
- Errors map explicitly to stable codes (`number_already_paired` → 409,
  `quota_exceeded` → 403, rate limit → 429 + Retry-After). Never leak an internal
  error string or a core `Status` detail to the tenant.
- Streaming endpoints (SSE/WS) are backpressure-aware and resumable via
  `Last-Event-ID`/cursor against the retention buffer.

## Consuming the core (`core-client/`)
- One long-lived `tonic` channel over the UDS, reconnecting if the core restarts.
  Wrap it so callers get typed methods, not raw gRPC.
- The "all-accounts" event subscription is the spine: one task drains it, fans out
  per-instance into broadcast channels the product layer subscribes to. Translate
  `EventEnvelope` → the edge's flat event type once, here.
- Keep this layer free of `sqlx`, `axum`, tenants. It takes WhatsApp-shaped inputs
  and returns WhatsApp-shaped outputs. If you're importing a product type here, stop.

## State and concurrency
- Shared state is an `Arc<AppState>` (the core client, the pool, config, the event
  fan-out, the webhook dispatcher) injected at construction. No globals, no
  singletons that vary by deployment.
- Prefer message passing (`tokio::sync::mpsc`) for owner-task patterns (webhook
  delivery queue per instance/webhook; the event fan-out). Reach for `Mutex`/`RwLock`
  only for genuinely shared mutable state.
- Never hold a `std::sync` lock across `.await` (clippy catches it). One pool,
  created once, cloned into services.
- Webhook delivery: a queue per webhook, at-least-once, idempotent by `event_id`,
  backoff schedule per the PRD, circuit-breaker on repeated failure.

## Dependency injection
- Constructor injection. A service struct takes its dependencies (pool, core client,
  config) as fields; `main` wires them. Wrap shared deps in named newtypes
  (`struct EdgeDb(PgPool)`, `struct CoreClient(...)`) — never a bare `Arc<…>` passed
  around.
- No hardcoded socket paths, DB URLs, ports, retention windows, or rate limits inside
  services. Load config once at startup into a `Config` and inject it.
- Cross-cutting concerns (auth, request logging, rate limit, timeout) are `tower`
  layers, not copy-pasted into handlers.

## Security (this layer exists because the core has none)
- Passwords: argon2id. API keys: random with a public prefix (`we_live_…`), stored
  hashed, shown once. JWT short-lived (15 min) + refresh with logout revocation.
- Open signup requires email verification; rate-limit signup per IP; the free plan's
  quota is the abuse ceiling.
- Every outbound HTTP request (webhook, media URL, link preview) goes through the
  SSRF guard. The inbound billing webhook verifies the provider's signature.
- Rate limit per API key/tenant (token bucket); quota enforcement (hard caps → 403).
- The retention buffer holds third parties' message content: document the window,
  auto-expire it, hard-delete on instance/tenant deletion.

## Comments
- Keep your own comments; don't strip them on refactor — they carry provenance.
- Write WHY, not WHAT. Reference the PRD section or a WhatsApp/core quirk for any line
  driven by one (the `@c.us` JID choice, the OGG/Opus PTT requirement, the gap
  contract, why a webhook retry schedule is what it is).

## Tests
- Pure logic (JID policy, idempotency keys, signature/HMAC, quota math): plain unit
  tests next to the code.
- HTTP behavior: drive the axum app in-process (tower `oneshot`) with a test DB;
  assert status + body. Auth/quota/rate-limit paths get tests.
- Core integration: spin up a real wamux core on a throwaway socket in a tempdir,
  connect the real `core-client`, exercise the path end to end (mirrors how the core
  tests its own socket). Mark slow/scale ones `#[ignore]` and run them in `ci.sh --full`.
- Every fixed bug gets a regression test that fails before the fix. F.I.R.S.T.

## Formatting & lint
- `cargo fmt` is law. `cargo clippy --all-targets -- -D warnings` must pass clean.
  Fix the lint; `#[allow(...)]` only with a comment and a reason.

## Directory structure
```
proto/                 # copied from the core, version-pinned (gRPC contract; never edit generated)
build.rs               # tonic-build: regenerate client stubs from proto/
migrations/            # sqlx migrations for the edge DB
templates/             # askama templates for the dashboard (PT + EN)
scripts/ci.sh          # the pipeline (fmt + clippy + tests)
src/
  main.rs              # load config, init tracing, migrate, build AppState, connect core, serve
  config.rs            # Config (TOML + WAMUX_EDGE_ env)
  error.rs             # typed errors -> problem+json
  core_client/         # LAYER 1: the reference consumer of wamux (no tenants/HTTP)
    channel.rs         #   UDS tonic channel + reconnect
    connection.rs      #   account connect/reconnect policy (terminal-exit handling)
    jid.rs             #   @c.us policy + CheckOnWhatsApp cache
    events.rs          #   subscribe-all drain + fan-out + gap recovery
    send.rs            #   text/media/PTT/preview/ephemeral senders
    media.rs           #   DownloadMedia on-demand
  product/             # LAYER 2: the SaaS
    auth/              #   signup/login/JWT/API keys
    http/              #   axum routers (thin handlers), SSE, WebSocket
    webhooks/          #   delivery queue, retry, HMAC signing, SSRF guard
    events/            #   retention buffer + cursor/polling
    billing/           #   usage metering, quotas, admin API, billing webhooks
    dashboard/         #   askama + htmx handlers
  state/               # AppState + newtypes wired in main
```
Mirror this so the agent can predict paths without listing directories.

## Observability
- `tracing` everywhere; init in `main`. One log line per HTTP request at the edge
  (route, tenant, latency, status) via a `tower` layer.
- `/metrics` (Prometheus text) and `/healthz` — and `/healthz` reports unhealthy if
  the core's `AdminService.Check` over the socket fails (the edge is useless without
  the core).
- On error: log the internal cause at the boundary (with context), return a clean
  problem+json. The tenant never sees an internal string or a core `Status`.

## Relationship to the core (operational truths)
- `proto/` is a **pinned copy**; re-sync and rebuild when the core's contract changes,
  and pin the core version/commit your CI tests against.
- Edge and core are **co-located** (UDS, same host/pod). The reference deploy is a
  compose with `wamux` + `wamux-http-edge` + Postgres, socket shared by volume, edge
  in the socket's owning group.
- The **core DB is the crown jewel**: it holds the Signal keys. Losing it means every
  instance must re-pair. Back it up (PITR), test the restore. The edge DB is mostly
  reconstructible except users/tenants/instances/webhooks. (PRD §13.)
