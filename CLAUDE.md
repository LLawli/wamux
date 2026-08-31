# CLAUDE.md: wamux — WhatsApp multiplexer core (gRPC over Unix socket)

`wamux` is the **core daemon** of an unofficial WhatsApp system (Evolution-API-style)
built on the Rust crate `whatsapp-rust` (no Baileys, no JS runtime). It exposes a gRPC
API over a **Unix domain socket only** (no TCP, no TLS): the socket is local, so
transport security is the filesystem's job (0660 + owning group), not the wire's.
Fully async on tokio. Agents are the primary reader: optimize for grep, small files,
explicit types.

**What the core is.** A *pure, functional* relay that multiplexes **many WhatsApp
accounts** in one process. It has **no auth of its own**: anyone who can open the
socket sees everything. Auth, permissions, per-user filtering, and any HTTP API live
in a **separate edge project (out of scope here)**. A socket client picks which
account(s)' events to receive (one / all / none = send-only).

**Relay-pure.** The core persists ONLY whatsapp-rust's Signal/session/device state
(Postgres), never business message history. See `docs/PRD.md` and `docs/SPEC.md` for
the product/spec, and `docs/crate-notes/` for the verbatim whatsapp-rust API map.

**Toolchain.** Pinned **nightly** (`rust-toolchain.toml`): `whatsapp-rust` enables the
`simd` feature (`core::simd`/`portable_simd`) and edition 2024, both nightly-only.

> Convention note: `AGENTS.md` is the canonical cross-tool file; keep this as
> `CLAUDE.md` and symlink `AGENTS.md -> CLAUDE.md` if you add other agents.

## Core purity (the prime directive)
The core is a **pure relay**: it exposes raw WhatsApp *mechanisms* and nothing else.
Every stateful, opinionated, or "smart" behavior belongs in the **edge** (the separate
auth/HTTP project), not here. When in doubt, **the core does LESS**. This rule overrides
feature convenience — prefer cutting a feature from the core over baking policy into it.

**Belongs in the edge, never the core:** retries, timeouts, fallbacks, recipient/JID
rewriting or normalization ("guessing" identity), per-user filtering, auth/permissions,
webhooks, and any "if X didn't happen in N seconds, do Y" logic. The core hands back the
primitives the edge needs (e.g. `SendResult.message_id`, `Receipt` events) and lets the
edge compose the policy.

**Worked example (Sprint 1, 2026-06-09): DM routing was removed entirely.** The core does
**not** rewrite a recipient JID, ever. The edge sends the recipient it wants and the core
relays to it verbatim. There is no `dm_routing` knob anywhere, and that is still the rule.

**What this example is NOT (updated 2026-08-31, issue #4).** It used to record a specific
routing recipe — send `<number>@c.us` to dodge the library's PN→LID upgrade — and called
it safe because `resolve_encryption_jid` leaves a legacy JID untouched. Both halves aged
badly, and the second one was the trap:

- The upgrade *was* undeliverable on companion accounts, on whatsapp-rust **0.6.0**. That
  was upstream [#941](https://github.com/oxidezap/whatsapp-rust/issues/941), fixed
  2026-07-02 and first released in **0.7.0**. The reason to dodge it stopped existing the
  day we ported.
- "Passes through untouched" was never a safety property. It is the defect. `@c.us` parses
  as `Server::Legacy`, so `is_pn()` is false and the send path stops treating the recipient
  as a phone user at all: it skips the self-chat check and the LID resolution, and it
  matches the pre-key IQ response by whole-JID equality including the server field, so a
  bundle the server *did* return reads as absent. No session is built, `send::encrypt`
  skips the device, and the stanza ships carrying nothing for the recipient's handset.
  Fixed upstream in [#1362](https://github.com/oxidezap/whatsapp-rust/pull/1362).

Measured on 2026-08-31 against the two production accounts on 0.7.0: the modern spelling
delivers (`delivered` receipts, no library warnings), and it is the legacy spelling that
does not. **Do not reintroduce a `@c.us` recipe here or recommend one to the edge.**

The durable lesson is the one the example was written for, and it survived: keeping
recipient choice out of the core is what let the edge fix this on its own timetable
without a core release. What did not survive is writing down a *specific* routing recipe,
which is edge policy pinned to a library version, in the file that teaches the rule.
Record the constraint, not the workaround.

**Judge a send by `delivered`, never by the ack.** `SendResult` means the library accepted
the message; `ServerAckEvent` means the server accepted the stanza. Neither means anyone
received it — a stanza that encrypted for nobody who matters gets both. There is no
`delivered` receipt for a note to self, which is why that case needs a human or, once
[#1362](https://github.com/oxidezap/whatsapp-rust/pull/1362) is released,
`SendResult.recipient_fanout` (`encrypted`, `skipped_primary`, `is_partial()`).

**Worked example (Sprint 3, 2026-06-09): reconnection is not the core's job.** The
`whatsapp-rust` run loop already reconnects transient drops with Fibonacci backoff;
the core must **not** add its own backoff (it would fight the lib). The per-account
**supervisor** only awaits the run loop's *terminal* exit (the lib gave up: 401/409/
logout) and then makes state truthful (clears `running`, marks `Disconnected`, frees
the connection-budget slot) and lets the event flow through. Any reconnect *after* a
terminal exit is **edge** policy, composed from the connection-state events + status.
The core also exposes a soft `max_connected_accounts` budget (fd self-protection,
`ResourceExhausted` past it) — a resource guard, not connection policy.

**Litmus test before adding anything to the core:** "Could the edge do this with the
primitives the core already exposes?" If yes, it does not belong in the core.

## Stack
- `tonic` for the gRPC server and client. This is the only RPC layer; never hand-roll
  framing over the socket.
- `prost` for protobuf messages (generated). `prost-types` only if you actually need
  well-known types like `Timestamp`.
- `tonic-build` runs in `build.rs` and regenerates Rust from `proto/` on every build.
  The `.proto` files are the source of truth; never edit generated code.
- `tokio` (multi-thread runtime) for async.
- `tokio-stream` for `UnixListenerStream`, which feeds accepted connections into
  `serve_with_incoming`.
- `tower` for middleware (layers/interceptors): auth, logging, timeouts.
- `tonic-reflection` (dev only) so `grpcurl` can list and describe services.
- `thiserror` for typed domain errors; `anyhow` only at the top of `main`.
- `tracing` plus `tracing-subscriber` for structured logs.

## Commands (the agent must be able to run these unattended)
- Build (also regenerates proto):  `cargo build` (proto via vendored `protoc`, no host install)
- Run the server:                  `cargo run` (reads `wamux.toml` + `WAMUX_*` env)
- Format:                          `cargo fmt`
- Lint (zero tolerance):           `cargo clippy --all-targets -- -D warnings`
- Tests:                           `DATABASE_URL=postgres://wamux:wamux@localhost:5433/wamux cargo test`
- Postgres for tests/dev (docker): `docker run -d --name wamux-pg -e POSTGRES_USER=wamux -e POSTGRES_PASSWORD=wamux -e POSTGRES_DB=wamux -p 5433:5432 postgres:16`
- Poke the socket by hand:         `grpcurl -unix -plaintext /run/wamux.sock list`
  (needs `grpcurl` installed; reflection is on by default in dev)
- CI (all gates, one command):     `scripts/ci.sh` (add `--full` for the #[ignore] scale tests).
  There is no hosted CI; this script is the pipeline — run it before declaring work done.

## Code style
- Functions: 4-20 lines. Split if longer.
- Files: under 500 lines. One service or one cohesive unit per file.
- Names: specific and unique. Prefer names that return under 5 grep hits. This is the
  agent's navigation API, so `handle`, `process`, `data`, `util`, `manager` are banned.
- Types: explicit at every boundary. Annotate public function signatures and the
  request/response types crossing a module edge. Don't lean on inference for anything
  another module touches.
- No code duplication. Early returns over nested ifs. Max 2 indent levels.
- No `.unwrap()` / `.expect()` on anything reachable from an incoming request. Map the
  error to a `tonic::Status` and return it. `unwrap` is allowed only on invariants that
  hold at startup (a parsed const, a path you just created), and only with a comment.
- Error values carry context: the offending value and the expected shape. Use
  `thiserror` for typed errors; `anyhow` only in `main` where you're about to exit.

## gRPC services (tonic)
- The `.proto` files in `proto/` define the contract. Change the proto, rebuild, then
  fix the Rust. Never the other way around.
- One service `impl` per file in `services/`. File name = snake_case of the service
  (`order_service.rs` -> `OrderService`).
- RPC method bodies stay thin: validate the request, call a `domain/` function, map the
  result to a response or a `Status`. Business logic does not live in the trait impl.
- Map errors explicitly. A `domain` error becomes a specific `Status` code
  (`NotFound`, `InvalidArgument`, `FailedPrecondition`, and so on). Never `unwrap` into
  a panic and never leak an internal error string to the client.
- Streaming RPCs are typed and backpressure-aware: yield from a `Stream`, don't buffer
  the whole result into a `Vec` first.

## Unix socket transport
- Bind with `tokio::net::UnixListener`, wrap in `UnixListenerStream`, serve via
  `Server::builder().add_service(...).serve_with_incoming(stream)`.
- On startup: remove a stale socket file if one exists, bind, then `chmod` the socket to
  the intended mode (for example `0660`) before announcing readiness. Document the
  chosen mode and the owning group.
- Authn/authz is local: read peer credentials (`SO_PEERCRED`: uid/gid/pid) from the
  `UnixStream` if you need to gate access. There is no token on the wire by default.
- No TCP listener and no TLS in this binary. If you ever need network exposure, that is
  a separate front (a reverse proxy or a second binary), not a flag flipped here.
- Graceful shutdown: drive `serve_with_incoming_shutdown` with a `tokio::signal` future
  (SIGTERM/SIGINT). Unlink the socket on the way out.

## State and concurrency
- Shared state is injected as an `Arc<T>` into the service struct at construction. No
  globals, no `lazy_static`/`OnceCell` singletons for things that vary by deployment.
- Prefer message passing (`tokio::sync::mpsc`) when an owner-task pattern reads clearer
  than a shared lock. Reach for `Mutex`/`RwLock` only for genuinely shared mutable state.
- Never hold a `std::sync` lock across an `.await` (clippy will catch it). If a lock must
  span async work, it's a `tokio::sync` lock, and the critical section is as short as
  you can make it.
- One pool (DB, cache, whatever), created once, cloned into each service that needs it.
  Pools are cheap to clone and own their own synchronization.
- Keep state granularity tight: one type per independently-changing concern, not one
  god-struct behind a single lock that serializes unrelated work.

## Dependency injection
- Constructor injection. A service struct takes its dependencies (pool, clients, config)
  as fields; `main`/`server.rs` wires them together. This is the whole DI mechanism.
- Wrap shared dependencies in named newtypes (`struct Db(PgPool)`,
  `struct UpstreamClient(...)`) so a field's purpose is obvious and greppable, never a
  bare `String` or bare `Arc<RwLock<...>>` passed around.
- No hardcoded socket paths, modes, endpoints, or feature flags inside services. Load
  config once at startup into a `Config` struct and inject it.
- Cross-cutting concerns (auth check, request logging, timeout) are `tower` layers added
  in `server.rs`, not copy-pasted into every RPC method.

## Comments
- Keep your own comments. Don't strip them on refactor: they carry provenance.
- Write WHY, not WHAT. Reference issue numbers / commit SHAs for any line driven by a
  bug, a protocol quirk, or a tokio/tonic constraint.
- The subtle stuff is what earns a one-line note: why a lock is `tokio::sync` not `std`,
  why an error maps to *this* `Status` code, why the socket mode is what it is, why a
  stream yields instead of collecting.

## Tests
- Pure logic (no socket, no async I/O): plain `cargo test` unit tests next to the code
  in `domain/`.
- Service behavior: bind the server on a throwaway socket inside a `tempdir`, connect a
  real tonic client over that socket, and assert over the wire. This exercises the same
  path production uses and needs no human setup.
- Every non-trivial function gets a test. Every fixed bug gets a regression test that
  fails before the fix.
- F.I.R.S.T: fast, independent, repeatable, self-validating, timely.

## Formatting & lint
- `cargo fmt` is law. Don't discuss style.
- `cargo clippy --all-targets -- -D warnings` must pass clean. Fix the lint, don't
  `#[allow(...)]` it unless the allow has a comment and a reason.

## Directory structure
```
proto/               # .proto contracts (the source of truth)
build.rs             # tonic-build: regenerates Rust from proto/
src/
  main.rs            # load config, bind socket, run server, handle signals
  server.rs          # assemble Server: register services + tower layers
  services/          # one gRPC service impl per file (thin RPC handlers)
  domain/            # business logic, transport-agnostic, unit-tested
  state/             # shared state, pools, context newtypes
  transport/         # UDS listener, socket lifecycle, peer-cred checks
  config.rs          # Config struct + load-at-startup
  error.rs           # typed errors (thiserror) + mapping to tonic::Status
  proto.rs           # tonic::include_proto!(...) module(s)
Cargo.toml
``` 
- Mirror this layout so the agent can predict paths without listing directories.

## Observability
- `tracing` everywhere; init a `tracing-subscriber` in `main`. Structured fields, not
  string interpolation.
- A `tower` layer opens a span per request with method name, peer uid, latency, and the
  final `Status` code. One log line per request, at the edge.
- On error: log the internal cause at the boundary (with context), then return a clean
  `Status` to the client. The client never sees an internal error string or a stack.
