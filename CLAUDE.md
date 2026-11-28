# CLAUDE.md: wamux, the WhatsApp multiplexer core (gRPC over a Unix socket)

This is the agent layer. The project's rules are in `CONTRIBUTING.md`, imported
below so Claude Code loads them with this file. `AGENTS.md` is a symlink to this
file; an agent that does not follow `@` imports must read `CONTRIBUTING.md`
itself before changing anything.

@CONTRIBUTING.md

## What wamux is

The **core daemon** of an unofficial WhatsApp system (Evolution-API-style), built
on the Rust crate `whatsapp-rust` (no Baileys, no JS runtime), fully async on
tokio. It multiplexes many WhatsApp accounts in one process and exposes them
over gRPC on a **Unix domain socket only** (no TCP, no TLS). A socket client picks
which accounts' events it receives (one, all, or none for send-only).

- **No auth of its own:** anyone who can open the socket sees everything. Auth,
  permissions, per-user filtering and any HTTP API live in a separate edge
  project, out of scope here.
- **Relay-pure:** the core persists only whatsapp-rust's Signal/session/device
  state (Postgres, SQLite, or Turso behind the `turso` cargo feature), never
  business message history.
- Product and spec: `docs/PRD.md`, `docs/SPEC.md`. `docs/archive/crate-notes/`
  is a historical snapshot of the whatsapp-rust 0.6.0 API: check the pinned
  revision's source before relying on it.

## The prime directive, for an agent

The core is a pure relay (`CONTRIBUTING.md`, "The rule that overrides the
others"). In practice, when you work here:

- Before adding anything, run the litmus test: could the edge do this with the
  primitives the core already exposes? If yes, stop and propose it for the edge.
  A task that asks for a retry, a timeout, a fallback, a JID rewrite or any "if X
  did not happen in N seconds" logic is edge work, even when it would be easy here.
- Never rewrite, normalize or special-case a recipient JID, and never write a
  JID-spelling routing recipe into a doc, a comment or advice to the edge.
- When you verify a send (a live run, a tool, a test), judge it by `delivered`
  and `recipient_fanout`, never by `SendResult` or the server ack. A note to self
  cannot be verified that way: say so and leave it to a human.
- When in doubt, the core does less. Cutting a feature beats baking policy in.

## Navigating the repo

Names are the navigation API: grep for the specific name first, then read the
file. `docs/SPEC.md` has the long map; this one is enough to predict a path.

```
Cargo.toml              # virtual workspace: members, shared deps, lints, the whatsapp-rust rev
crates/
  wamux-proto/          # generated gRPC types; depends on no wamux crate
    proto/              # .proto contracts (source of truth); proto/store/ = stored blob formats
    build.rs            # tonic-prost-build: regenerates Rust from proto/ on every build
  wamux-types/          # domain types (Jid, MediaKind, closed enums, messaging/) and
                        #   WamuxError with its tonic::Status mapping (error.rs)
  wamux/                # the daemon: lib + `wamux` bin
    migrations/         # Postgres schema   } same number and name per change
    migrations_sqlite/  # SQLite schema     } (CONTRIBUTING.md, Migrations)
    src/
      main.rs           # entry point: config, umask, store, registry, bind, serve
      cli.rs            # `wamux` (serve), `wamux store decrypt|rotate-key`
      daemon_runtime.rs # the hand-built tokio runtime, with a bounded exit
      config.rs         # Config: wamux.toml + WAMUX_* env, loaded once
      server.rs         # build_router: services, the two tower layers, reflection
      observe.rs        # tower layer: one log line + Prometheus metrics per request
      error.rs          # re-exports WamuxError; client_err() from whatsapp-rust errors
      services/         # one thin gRPC service impl per file
      domain/           # transport-agnostic logic (event mapping, sends, groups...)
      state/            # account registry, per-account handles, event bridge and ring
      storage/          # StorageEngine: sql (Postgres/SQLite via sqlx), turso, encryption
      spool/            # media spool: unlinked temp files under a global byte budget
      transport/        # UDS bind/mode/group, SO_PEERCRED allowlist layer, shutdown
      stress/           # mock WhatsApp server for scale tests (`stress` feature only)
    tests/              # over-the-wire suites: services/, storage/, daemon/, ...
  wamux-tools/          # development binaries (src/bin/) and their shared lib; not shipped
scripts/                # ci.sh and the check-* gates it runs; their tests in scripts/tests/
docs/                   # PRD, SPEC, DEPLOYMENT, TERMUX, upgrading/ (breaking-change guides)
```

Unit tests sit next to their file as `<name>_tests.rs`. New code goes where this
map says it goes, so the next reader can predict its path; a change that adds a
top-level module updates the map in the same commit.

## Commands (run these unattended)

- Build (also regenerates proto): `cargo build` (vendored `protoc`, no host install)
- Run the server: `cargo run` (reads `wamux.toml` + `WAMUX_*` env)
- Format: `cargo fmt`
- Lint (zero tolerance): `cargo clippy --workspace --all-targets -- -D warnings`
- Tests: `DATABASE_URL=postgres://wamux:wamux@localhost:5433/wamux cargo test -p wamux`
- Postgres for tests and dev: the `docker run ... wamux-pg` line in
  `CONTRIBUTING.md`, "Getting the gates green".
- Poke the socket by hand: `grpcurl -unix -plaintext /run/wamux.sock list`
  (needs `grpcurl`; reflection is on by default, `enable_reflection`).
- All gates, one command: `scripts/ci.sh` (`--full` adds the `#[ignore]` scale
  tests). Run it before declaring work done.

Notes that only apply to one machine (build caps, local wrappers) live in
`CLAUDE.local.md`, which is gitignored; follow it when it is present.

## Diagnosing a library bug (the first fit is a hypothesis, not a cause)

An explanation that accounts for the log is where the investigation starts, not
where it ends. Two of this project's upstream reports came back corrected, one in
its cause and one in its proposed remedy, by the same mistake: reading until the
symptom was explained, then stopping.

**Two searches before you write the report, the workaround, or the mitigation:**
1. **Does the abstraction already exist?** Grep the crate for the concept before
   concluding the fix would be invasive. The answer is often in the file the
   evidence came from.
2. **Read the path you are accusing, end to end.** A mechanism that *could*
   produce the symptom is not the mechanism that *did*. Confirm the failure mode
   in the code rather than inferring it backwards from the log.

Then **say which half is which**: what you verified in the code, and what you
inferred from behaviour. A report that marks its inferred half survives being
wrong; one that states both as fact has to be retracted.

**Worked example (upstream
[#1376](https://github.com/oxidezap/whatsapp-rust/issues/1376)).** The report said
the keepalive watchdog measured in-process elapsed time on the wall clock
(correct) and argued a local fix might be invasive. It was not: `wacore::time`
already shipped `MonotonicProvider`, `Instant` and `set_monotonic_provider`, in
the same file the `now_millis` evidence was quoted from, with a module doc that
described this exact failure mode. It was also three bugs, not one, and the two
unreported ones were worse for this relay.

**Worked example (upstream
[#1377](https://github.com/oxidezap/whatsapp-rust/issues/1377), fixed in
[#1380](https://github.com/oxidezap/whatsapp-rust/pull/1380)).** The report called
an offline resume *abandoned*, needing a re-arm. Re-arming already worked:
`OfflineBatchCoordinator::reset()` ran in both connection-state resets and
`is_armed_for` is generation-gated. The real gap was that the old resume never
announced its end. The assumed risk never existed either (an interrupted drain
acks nothing, so the server redelivers all of it), yet a mitigation had been
recommended to the edge on the strength of that wrong model.

**The tell in both:** the fix was proposed before the accused code had been read
whole. Ten more minutes of grep would have caught each one.
