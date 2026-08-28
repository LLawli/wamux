# Contributing

## Before anything else

This is an **unofficial** WhatsApp client. It talks to WhatsApp through the
`whatsapp-rust` library, which reimplements a protocol Meta does not publish.
Using it may violate WhatsApp's Terms of Service and can get an account
banned. Contribute and test only with accounts you are authorized to use.

Never test against a real account you cannot afford to lose. Pair a spare
number: the repo has helpers (`src/bin/pair_socket.rs`) for exactly that.

## Getting the gates green

`scripts/ci.sh` **is** the pipeline. CI runs that same script, so if it passes
locally it passes on GitHub.

```sh
scripts/ci.sh                # everything (needs Postgres)
scripts/ci.sh --no-postgres  # the database-free subset
scripts/ci.sh --full         # also the #[ignore] scale tests
```

Postgres for the full run:

```sh
docker run -d --name wamux-pg -e POSTGRES_USER=wamux -e POSTGRES_PASSWORD=wamux \
  -e POSTGRES_DB=wamux -p 5433:5432 postgres:16
```

The toolchain is a pinned nightly (`rust-toolchain.toml`) because
`whatsapp-rust` uses `core::simd` and edition 2024. `rustup` picks it up on its
own.

## The rule that overrides the others

**The core is a pure relay.** It exposes raw WhatsApp mechanisms and nothing
else. Every stateful, opinionated or "smart" behavior belongs to the edge (the
separate auth/HTTP project), not here.

Belongs to the edge, never the core: retries, timeouts, fallbacks, recipient
rewriting or normalization, per-user filtering, auth, webhooks, and any "if X
hasn't happened in N seconds, do Y" logic.

Before adding anything, apply the litmus test: **could the edge do this with
the primitives the core already exposes?** If yes, it does not belong here.
Prefer cutting a feature from the core over baking policy into it.

`CLAUDE.md` carries the full engineering doctrine, including worked examples of
features that were removed for violating this rule. Read it before a first
contribution; it is also the file agents load.

## Changing the API

The `.proto` files in `proto/` are the **source of truth**. Change the proto,
rebuild (`build.rs` regenerates on every build), then fix the Rust. Never the
other way around, and never edit generated code.

A wire-breaking change needs a note in `CHANGELOG.md` under **Changed** with
the migration path, because a separate edge project consumes this contract.

## Code style

Enforced by the gates: `cargo fmt` is law and `cargo clippy -D warnings` must
pass. Beyond that, from `CLAUDE.md`:

- Functions 4-20 lines; files under 500 lines.
- Names specific enough to return under 5 grep hits. `handle`, `process`,
  `data`, `util`, `manager` are banned - this codebase is read by agents, and
  names are the navigation API.
- Explicit types at every module boundary.
- No `.unwrap()` on anything reachable from a request. Map the error to a
  `tonic::Status`. `unwrap` is allowed only on startup invariants, with a
  comment saying why it holds.
- Comments say **why**, not what. Keep existing ones on refactor: they carry
  provenance. Reference an issue or commit for anything driven by a protocol
  quirk or a bug.

## Tests

- Pure logic: unit tests next to the code in `domain/`.
- Service behavior: bind the server on a throwaway socket in a tempdir,
  connect a real tonic client over it, and assert over the wire.
- Every fixed bug gets a regression test that **fails before the fix**. A test
  that cannot fail is not a test.
- Storage changes must hold for both engines. `tests/storage_backend.rs` runs
  the shared body against Postgres and SQLite and asserts they persist
  byte-identical blobs.

## Commits

Conventional commits (`feat:`, `fix:`, `refactor:`, `docs:`, `ci:`, `chore:`),
scoped to one logical change. The body should say **why**, not restate the
diff. Run the gates before pushing.
