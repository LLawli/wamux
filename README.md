# wamux

**WhatsApp multiplexer core daemon: many accounts, one Unix socket, gRPC.**

`wamux` is the core daemon of an unofficial WhatsApp system (in the style of
Evolution API), built directly on the Rust crate
[`whatsapp-rust`](https://crates.io/crates/whatsapp-rust) — no Baileys, no JS
runtime. It multiplexes **many WhatsApp accounts in one process** and exposes a
gRPC API over a **Unix domain socket only** (no TCP, no TLS). It is fully async
on tokio.

> ⚠️ **Unofficial.** This project is not affiliated with, endorsed by, or
> connected to WhatsApp or Meta. It talks to WhatsApp through the unofficial
> `whatsapp-rust` library. Using it may violate WhatsApp's Terms of Service and
> can get an account banned. Use it only with accounts and in ways you are
> authorized to, and at your own risk.

## What it is (and is not)

`wamux` is a **pure relay**. It exposes raw WhatsApp *mechanisms* and nothing
else, and it has **no auth of its own**: anyone who can open the socket sees and
controls everything. A socket client picks which account(s)' events it wants
(one / all / none = send-only).

Every stateful or opinionated behavior (auth, permissions, per-user filtering,
retries, timeouts, fallbacks, webhooks, reconnection policy, an HTTP API) lives
in a **separate edge project, out of scope here**. The core hands the edge the
primitives it needs (`SendResult.message_id`, `Receipt` events, connection-state
events) and lets the edge compose the policy.

**Relay-pure persistence:** the core persists *only* `whatsapp-rust`'s
Signal/session/device state (in Postgres), never business message history.

### Security model

- **Transport is the filesystem's job.** The socket is local, so security is
  enforced by file permissions: the socket is created and then `chmod`'d to
  `0660` (owned by the daemon's group), not protected on the wire. There is no
  token, no TLS, no TCP listener in this binary.
- **No in-process auth.** Authentication and authorization are the edge's
  responsibility. If you need to gate access locally, the daemon can read peer
  credentials (`SO_PEERCRED`) from the Unix stream.
- Network exposure, if ever needed, is a separate front (a reverse proxy or a
  second binary), never a flag flipped here.

## Requirements

- **Rust nightly** — pinned in `rust-toolchain.toml`. `whatsapp-rust` enables
  the `simd` feature (`core::simd` / `portable_simd`) and edition 2024, both
  nightly-only. `rustup` will pick up the pinned toolchain automatically.
- **Postgres or SQLite** for the Signal/device store. The `database_url`
  scheme picks the engine: `postgres://` for the multi-account deployment,
  `sqlite://` for a single file with no server process.
- `protoc` is **not** required on the host — it is vendored and run by
  `build.rs`, which regenerates the Rust gRPC code from `proto/` on every build.

## Install

### Docker (fastest)

```sh
WAMUX_UID=$(id -u) WAMUX_GID=$(id -g) docker compose up -d --build
# socket at ./run/wamux.sock
```

Brings up the daemon and Postgres. The `WAMUX_UID`/`WAMUX_GID` are not
optional decoration: the socket is `0660`, so the container must run as the
user that will open it or every connection fails with permission denied.

### Native, no database server

```sh
cargo build --release --bin wamux
cp target/release/wamux ~/.local/bin/
cp contrib/wamux.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now wamux
# socket at ~/.local/state/wamux/wamux.sock
```

The shipped unit uses the SQLite engine, so there is nothing else to install.
Point `WAMUX_DATABASE_URL` at a `postgres://` DSN when you outgrow it.

[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) covers reaching the socket from
another container, the UID/GID rules, and the failure modes.

## Quick start (from source)

```sh
# 1. Postgres (Docker example; the test/dev default is port 5433)
docker run -d --name wamux-pg \
  -e POSTGRES_USER=wamux -e POSTGRES_PASSWORD=wamux -e POSTGRES_DB=wamux \
  -p 5433:5432 postgres:16

# 2. Configure
cp wamux.toml.example wamux.toml
#   edit database_url / socket_path as needed; every key can also be overridden
#   by an env var (WAMUX_<UPPERCASE_KEY>), e.g. WAMUX_DATABASE_URL=…
#   for a serverless setup, skip step 1 and use:
#     WAMUX_DATABASE_URL=sqlite:///var/lib/wamux/wamux.db

# 3. Build (also regenerates protobuf) and run
cargo run
```

The server binds the Unix socket from `socket_path` (default `/run/wamux.sock`),
`chmod`s it to `socket_mode` (default `0660`), then serves the gRPC API. It
shuts down gracefully on SIGTERM/SIGINT and unlinks the socket on the way out.

Poke the socket by hand (needs `grpcurl`; gRPC reflection is on in dev):

```sh
grpcurl -unix -plaintext /run/wamux.sock list
```

## Configuration

Copy `wamux.toml.example` to `wamux.toml` (or point `WAMUX_CONFIG` at a path).
Keys include `socket_path`, `socket_mode`, `database_url`, `db_max_connections`,
`event_ring_capacity`, `broadcast_capacity`, and `replay_max_event_bytes`. Each
key can be overridden by a `WAMUX_<UPPERCASE_KEY>` environment variable. See the
example file for the full annotated list.

## Development

```sh
cargo build                                   # build + regenerate proto
cargo fmt                                      # format (law)
cargo clippy --all-targets -- -D warnings      # lint, zero tolerance
DATABASE_URL=postgres://wamux:wamux@localhost:5433/wamux cargo test
WAMUX_TEST_ENGINE=sqlite cargo test            # same suite, SQLite engine
scripts/ci.sh                                  # every gate in one run (--full adds scale tests)
```

`scripts/ci.sh` is the pipeline — there is no hosted CI. Run it before declaring
work done.

The `proto/` files are the **source of truth** for the API contract: change the
proto, rebuild, then fix the Rust — never the other way around. Generated code is
never edited by hand.

### Layout

```
proto/        # .proto contracts (source of truth)
build.rs      # tonic-build: regenerates Rust from proto/
src/
  main.rs       # load config, bind socket, run server, handle signals
  server.rs     # assemble the gRPC Server: services + tower layers
  services/     # one gRPC service impl per file (thin handlers)
  domain/       # business logic, transport-agnostic, unit-tested
  state/        # shared state, pools, registry, event bridge
  transport/    # UDS listener, socket lifecycle, peer-cred checks
  storage/      # StorageEngine trait + postgres/ and sqlite/ backends
  config.rs     # Config struct + load-at-startup
  error.rs      # typed errors + mapping to tonic::Status
```

The binaries under `src/bin/` (pairing, e2e, validation, live probes) are
development/diagnostic tools, not part of the daemon.

## Documentation

- `docs/PRD.md` — product requirements and the relay-pure rationale.
- `docs/SPEC.md` — the technical spec.
- `docs/crate-notes/` — a verbatim map of the `whatsapp-rust` API surface used.
- `CLAUDE.md` / `AGENTS.md` — contributor conventions (also consumed by coding agents).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

> Note: this license covers wamux's own source. Third-party dependencies
> (including `whatsapp-rust`) carry their own licenses; review them before
> redistribution.
