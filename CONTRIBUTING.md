# Contributing

This file holds the project's rules. `CLAUDE.md` is a thin layer on top of it
for coding agents (how to navigate, what to run unattended, how to diagnose a
library bug), and it imports this file, so humans and agents follow the same
rules.

## Before anything else

This is an **unofficial** WhatsApp client. It talks to WhatsApp through the
`whatsapp-rust` library, which reimplements a protocol Meta does not publish.
Using it may violate WhatsApp's Terms of Service and can get an account
banned. Contribute and test only with accounts you are authorized to use.

Never test against a real account you cannot afford to lose. Pair a spare
number: the repo has helpers
(`crates/wamux-tools/src/bin/pair_socket.rs`) for exactly that.

## Getting the gates green

`scripts/ci.sh` **is** the pipeline. CI runs that same script, so if it passes
locally it passes on GitHub. Run it before declaring work done.

```sh
scripts/ci.sh                # everything (needs Postgres)
scripts/ci.sh --no-postgres  # the database-free subset
scripts/ci.sh --full         # also the #[ignore] scale tests
```

Hosted CI (`.github/workflows/ci.yml`) runs on every pull request and every
push to `main`: `scripts/ci.sh (postgres)` (without `--full`, so the scale
tests are still yours to run locally), `fmt + clippy + tests (sqlite)`,
`scripts/ci.sh (macos, sqlite)`, the macOS package, the Android/Termux build
and package, and `cargo audit`. Merge only after those checks pass.

The dev profile uses `debug = "line-tables-only"`: backtraces and panics keep
file:line, and `target/debug` is about a third of the size. If you need a
debugger with variables, build with `CARGO_PROFILE_DEV_DEBUG=2`.

Postgres for the full run (the tests default to this DSN,
`postgres://wamux:wamux@localhost:5433/wamux`):

```sh
docker run -d --name wamux-pg -e POSTGRES_USER=wamux -e POSTGRES_PASSWORD=wamux \
  -e POSTGRES_DB=wamux -p 5433:5432 postgres:16
```

The toolchain is a pinned stable release (`rust-toolchain.toml`), the one
upstream `whatsapp-rust` builds on; `rustup` picks it up on its own. The lib
itself is pinned to a git revision on main, not a crates.io release (#30).
Re-align both on every bump, and move every crate of the `whatsapp-rust`
family to the same revision together.

## The rule that overrides the others

**The core is a pure relay.** It exposes raw WhatsApp mechanisms and nothing
else. Every stateful, opinionated or "smart" behavior belongs to the edge (the
separate auth/HTTP project), not here. When in doubt, the core does less: this
rule overrides feature convenience, so prefer cutting a feature from the core
over baking policy into it.

Belongs to the edge, never the core: retries, timeouts, fallbacks, recipient
(JID) rewriting or normalization ("guessing" identity), per-user filtering,
auth and permissions, webhooks, and any "if X hasn't happened in N seconds, do
Y" logic. The core hands back the primitives the edge needs (for example
`SendResult.message_id` and `Receipt` events) and lets the edge compose the
policy.

Before adding anything, apply the litmus test: **could the edge do this with
the primitives the core already exposes?** If yes, it does not belong here.

What the rule means in practice:

- **The core never rewrites a recipient JID.** The edge sends the recipient it
  wants and the core relays to it verbatim; there is no routing knob. The
  library's own parse is the only normalization (it reads `@c.us` as a phone
  user, so `wamux_types::Jid` holds `@s.whatsapp.net`; code that tries to
  refuse or special-case `@c.us` after parsing cannot see it). Do not add a
  routing recipe for a JID spelling to the core, and do not recommend one to
  the edge (#4, upstream
  [whatsapp-rust#1362](https://github.com/oxidezap/whatsapp-rust/pull/1362)).
  Keeping recipient choice out of the core is what lets the edge fix routing
  on its own timetable, without a core release. Write down the constraint, not
  a workaround pinned to a library version.
- **Reconnection is not the core's job.** The `whatsapp-rust` run loop already
  reconnects transient drops with its own backoff, and a second backoff in the
  core would fight it. The per-account supervisor only awaits the run loop's
  terminal exit (the lib gave up: 401, 409, logout), then makes state truthful
  (clears `running`, marks `Disconnected`, frees the connection-budget slot)
  and lets the event through. A reconnect after a terminal exit is edge
  policy. The soft `max_connected_accounts` budget (`ResourceExhausted` past
  it) is a resource guard for file descriptors, not connection policy.
- **Judge a send by `delivered`, never by the ack.** `SendResult` means the
  library accepted the message; `ServerAckEvent` means the server accepted the
  stanza. Neither means anyone received it: a stanza that encrypted for nobody
  who matters gets both. For a DM to someone else,
  `SendResult.recipient_fanout` (#47) says at send time how many of the
  recipient's devices got an `<enc>` and whether the phone was among the
  misses. A note to self has no `delivered` receipt and its fan-out reports all
  zeros (a self-chat has no recipient half), so that case still needs a human
  to look.

## Changing the API

The `.proto` files in `crates/wamux-proto/proto/` are the **source of truth**.
Change the proto, rebuild (`crates/wamux-proto/build.rs` runs
`tonic-prost-build` on every build, with a vendored `protoc`), then fix the
Rust. Never the other way around, and never edit generated code.

- One service `impl` per file in `crates/wamux/src/services/`, named after the
  service in snake_case (`messaging_service.rs` holds `MessagingService`).
- RPC bodies stay thin: validate the request, call a `domain/` function, map
  the result to a response. Business logic does not live in the trait impl.
- Streaming RPCs are typed and backpressure-aware: yield from a `Stream`, do
  not buffer the whole result into a `Vec` first.
- A wire-breaking change needs a note in `CHANGELOG.md` under **Changed** with
  the migration path, because a separate edge project consumes this contract.
  When the migration takes more than a paragraph, it gets a guide under
  `docs/upgrading/`, listed in [docs/upgrading/README.md](docs/upgrading/README.md).

## Stack

- `tonic` for the gRPC server and client: the only RPC layer, never hand-rolled
  framing over the socket. `prost` for the generated messages; `prost-types`
  only where a well-known type is needed.
- `tokio` (multi-thread runtime), and `tokio-stream`'s `UnixListenerStream` to
  feed accepted connections into `serve_with_incoming`.
- `tower` for the cross-cutting layers (see [Transport](#transport)).
- `tonic-reflection` is a normal dependency, compiled into every build, and
  `enable_reflection` defaults to `true` so `grpcurl` can list and describe the
  services. Set `enable_reflection = false` to turn it off.
- `thiserror` for typed errors; `anyhow` only at the top of `main`.
- `tracing` plus `tracing-subscriber` for structured logs.

## Code style

Enforced by the gates: `cargo fmt` is law (do not discuss style) and
`cargo clippy --workspace --all-targets -- -D warnings` must pass. Fix a lint
rather than `#[allow(...)]` it; an allow needs a comment with the reason.

The size and naming rules below are enforced by `scripts/ci.sh`
(`check-file-size.py`, clippy `too_many_lines`, `check-banned-names.py`), with
named exceptions: each exception is listed where it is granted, with the reason
it holds.

- **Files under 500 lines.** One service or one cohesive unit per file.
- **Short functions,** 4-20 lines as the target: split a longer one. Clippy's
  `too_many_lines` fails CI past its threshold; a function that reads better
  whole (a large `match` over every event variant) carries an
  `#[allow(clippy::too_many_lines)]` with a comment saying why.
- **Specific names,** unique enough to return under 5 grep hits. Names are the
  navigation API of a codebase read by agents, so `handle`, `process`, `data`,
  `util` and `manager` are banned.
- **Explicit types at every boundary:** annotate public function signatures
  and the request/response types that cross a module edge. Do not lean on
  inference for anything another module touches.
- No duplication. Early returns over nested `if`s, at most 2 indent levels.
- **No `.unwrap()` / `.expect()` on anything reachable from a request.** Map
  the error and return it. `unwrap` is allowed only on invariants that hold at
  startup (a parsed const, a path you just created), with a comment saying why
  it holds.

## Errors

- `WamuxError` (`thiserror`) and its mapping to `tonic::Status`
  (`impl From<WamuxError> for tonic::Status`) live in
  `crates/wamux-types/src/error.rs`. That file is the one place a `Status` is
  constructed (`scripts/check-status-sites.py`): `domain/` and `services/`
  return `WamuxError`, and `?` converts at the service boundary.
- `crates/wamux/src/error.rs` re-exports `WamuxError` and builds one from a
  `whatsapp-rust` error (`client_err`), because that needs the library's
  types and `wamux-types` does not depend on `whatsapp-rust`.
- Storage trait impls return wacore's `StoreError`; that mapping is
  `crates/wamux/src/storage/sqlx_error.rs`.
- Each error maps to a specific code (`NotFound`, `InvalidArgument`,
  `FailedPrecondition`, and so on). Never `unwrap` into a panic, and never leak
  an internal error string to the client.
- Error values carry context: the offending value and the expected shape.

## Transport

- A Unix domain socket only: no TCP listener and no TLS in this binary.
  Transport security is the filesystem's job (`socket_mode`, default `0660`,
  plus `socket_group`). Network exposure, if ever needed, is a separate front
  (a reverse proxy or a second binary), not a flag here.
- The core has no auth. `server.rs` adds exactly two `tower` layers:
  `RequestObserveLayer` (`observe.rs`: one log line and the metrics per
  request) and `PeerAllowlistLayer` (`transport/peer_allowlist.rs`: an optional
  `SO_PEERCRED` uid/gid allowlist, `allowed_uids`/`allowed_gids`, empty by
  default). The allowlist is access control on a local socket, not
  authentication, and there is no token on the wire. Cross-cutting concerns go in a layer, not copied into every
  RPC method.
- Startup binds through `transport/uds_listener.rs`: a file already at the path
  is probed, and only a socket that refuses connections counts as stale; the
  socket gets its group and mode in a private directory before it is renamed
  onto the path.
- Graceful shutdown: SIGTERM/SIGINT drive `serve_with_incoming_shutdown`, the
  drain is bounded, and the socket is unlinked on the way out.

## State, concurrency and dependency injection

- Constructor injection is the whole DI mechanism: a service struct takes its
  dependencies (pool, clients, config) as fields, and `main`/`server.rs` wire
  them together. Shared state is an `Arc<T>` passed in at construction.
- No globals and no `lazy_static`/`OnceCell` singletons for anything that
  varies by deployment. (The Prometheus recorder is process-global by the
  `metrics` facade's design, and says so where it is installed.)
- Wrap shared dependencies in named newtypes (`struct Db(PgPool)`) so a field's
  purpose is greppable, never a bare `String` or bare `Arc<RwLock<...>>`.
- No hardcoded socket paths, modes, endpoints or feature flags inside
  services: load config once at startup into `Config` and inject it.
- Prefer message passing (`tokio::sync::mpsc`) when an owner task reads
  clearer than a shared lock; `Mutex`/`RwLock` only for genuinely shared
  mutable state.
- Never hold a `std::sync` lock across an `.await` (clippy catches it). A lock
  that must span async work is a `tokio::sync` lock, with the shortest
  critical section you can make.
- One pool, created once, cloned into each service that needs it.
- Tight state granularity: one type per independently-changing concern, not
  one god-struct behind a single lock.

## Observability

- `tracing` everywhere, with structured fields, not string interpolation.
  `main` initializes the subscriber.
- The request layer logs one line per request with the method, the peer uid,
  the latency and the final `Status` code.
- On error, log the internal cause at the boundary, with context, and return a
  clean `Status`. The client never sees an internal error string or a stack.

## Comments

- **A comment states an invariant, a constraint or a reason:** why the code is
  this way, what would break if it changed, or what was measured on the wire.
  Write why, not what, and keep that reason when you refactor the code around
  it.
- **An issue reference is a pointer, not the explanation.** "See #56" next to
  the reason is fine; "#56" in place of the reason is not. Reference an issue
  or commit for anything driven by a protocol quirk, a bug, or a tokio/tonic
  constraint.
- **History goes to the commit message and the CHANGELOG, not the code.** What
  the code used to do, which version changed what, which part of which issue
  added it, and who reviewed it. A history comment goes stale silently.
  `scripts/check-comment-provenance.py` fails CI on review provenance
  (`code-review 2026-...`, `edge-review`) in the sources.
- **A comment that describes code that no longer exists is a bug,** and is
  reviewed like one.
- The subtle choices are what earn a one-line note: why a lock is
  `tokio::sync` and not `std`, why an error maps to this `Status` code, why the
  socket mode is what it is, why a stream yields instead of collecting.

## Tests

- Pure logic (no socket, no async I/O): unit tests next to the code in
  `domain/`.
- Service behavior: bind the server on a throwaway socket in a tempdir,
  connect a real tonic client over it, and assert over the wire. That is the
  path production uses, and it needs no human setup.
- Every non-trivial function gets a test. Every fixed bug gets a regression
  test that **fails before the fix**. A test that cannot fail is not a test.
- Storage changes must hold for every engine. `tests/storage/storage_backend.rs`
  runs the shared body against Postgres and SQLite and asserts they persist
  byte-identical blobs.
- F.I.R.S.T: fast, independent, repeatable, self-validating, timely.

## Migrations

Two engines, one numbering.

- `crates/wamux/migrations/` (Postgres) and `crates/wamux/migrations_sqlite/`
  take the **same number and the same name** for the same change, from 0005
  on. A new migration takes the next free number in both directories.
- A change that only one engine needs still gets a file on the other: same
  number, same name, a no-op whose comment says why. That keeps the numbers
  aligned for good.
- Never renumber or edit an applied migration. sqlx stores each one's version
  and checksum, and a changed file breaks every existing store.
- The one exception is history, pinned by name: Postgres 0002
  (`drop_connection_policy`) has no SQLite counterpart, so Postgres 0003/0004
  are SQLite 0002/0003, and SQLite has no 0004.
- `tests/storage/migration_alignment` enforces all of this on every
  `cargo test` (#66).

## Commits and pull requests

- Conventional commits with a scope and the issue number:
  `fix(storage): taken external_ref answers ALREADY_EXISTS on every engine (#269)`.
  Types in use: `feat`, `fix`, `refactor`, `test`, `docs`, `build`, `ci`,
  `chore`. One logical change per commit; the body says **why**, not a restated
  diff. Run the gates before pushing.
- The `CHANGELOG.md` entry (Keep a Changelog, under `[Unreleased]`) goes in its
  own `docs(changelog): ... (#N)` commit in the same pull request.
- Branches are named `<type>/<issue>-<slug>` (`fix/274-list-accounts-jid`).
- A pull request body opens with `Closes #N`, then says what the problem was,
  what changes (and what changes on the wire contract, or "No wire change"),
  what was verified in the code, what was inferred and not verified, and how it
  was validated (the gates, plus any live run). Pull requests merge with a
  merge commit.
- Right after `gh pr create`, `gh pr checks` can print "no checks reported":
  the runs are not registered yet. Wait until they appear, then `--watch`.
- No tool-generated footers or attribution lines in commits, pull requests or
  issues.

Documentation, code comments, commit messages and issues are written in English.
`scripts/check-doc-language.py` fails CI on Portuguese in docs and comments.

## Releasing

`bin/release X.Y.Z` cuts a release from an empty working copy on top of
`main@origin`. It bumps `[workspace.package].version`, turns `[Unreleased]` in
`CHANGELOG.md` into the dated `[X.Y.Z]` section, runs `scripts/ci.sh`, commits
`chore: release vX.Y.Z`, pushes `main`, then tags that commit with a signed tag
and pushes the tag. The tag starts `.github/workflows/release.yml`: the tarball
with its `.sha256` and attestation on the GitHub release, then the GHCR image.

`--no-push` stops after the local commit, to read what it did. In `0.x` a
breaking wire change bumps the minor version.
