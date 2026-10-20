# Deferred decisions (deliberately NOT implemented)

A record of what was evaluated in code review and deferred on purpose, with
the reason and the trigger to revisit. It is not a bug backlog: nothing here
is broken today.

## `created_tx`: hardcoded capacity of 64 (crates/wamux/src/state/account_registry.rs)

**What it is.** The broadcast channel that notifies all-accounts streams about
newly created accounts has a fixed capacity of 64, while the sibling capacity
of the per-account event broadcast is injected via config (`RegistryTuning.
broadcast_capacity`, promoted from hardcode to config in Sprint 3).

**Why it was deferred (code review of 2026-06-11).**
- The hardcode is a choice documented on the field itself ("account creation is
  a rare, edge-driven action, never a hot path"), with a defined recovery
  contract (`subscription_gap` marker + reconcile via `ListAccounts`).
- The lag requires a burst of >64 creates WHILE the follower task is starved;
  the follower's per-item work (HashSet lookup + `tokio::spawn`) is orders of
  magnitude faster than the store INSERT that each create does (Postgres,
  SQLite or Turso), so a bulk import alone does not trigger the gap.
- CLAUDE.md's DI rule covers "socket paths, modes, endpoints, feature
  flags", not channel capacities; other channels in the repo also have a
  fixed capacity (mpsc 256 for the event stream, mpsc 16 for account_service's
  pairing stream, mpsc 2 for media_service's download stream). "Every
  capacity becomes config" is not the project's norm; only the hot-path
  broadcast was promoted.

**When to revisit.** When either of these is seen, not "when someone is
nearby":
- an all-accounts stream receives a `RawEvent` of kind `subscription_gap` with
  an empty `account_uuid` (the one `created_gap_marker` in
  `crates/wamux/src/services/event_service.rs` emits). The core does not log
  this lag itself, so the signal lives in the edge's logs; or
- an edge provisions accounts in bursts (tens of `CreateAccount` calls in a
  row) while an all-accounts stream is open.

Then promote the 64 to a `RegistryTuning` field (`created_channel_capacity`,
default 64), wired where `broadcast_capacity` is, and close this entry.

**Status (2026-10-20, #192).** Kept deferred on purpose. The facts above still
hold in the code: the follower's per-item work is a `HashSet` lookup and a
spawned forwarder, and an overflow still surfaces as `subscription_gap` with
the `ListAccounts` recovery. The earlier trigger ("on the next real change to
`RegistryTuning` or to the `account_registry` module") passed twice without
anyone acting on it (#35 on 2026-09-24, #80 on 2026-10-13), because nothing
observable told the person in the file that the change was due. The trigger
above replaces it with signals someone can actually see.
