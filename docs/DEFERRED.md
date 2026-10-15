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

**When to revisit.** On the next real change to `RegistryTuning` or to the
`account_registry` module: promoting the 64 to a `created_channel_capacity`
field (default 64) is cheap while you are already there and removes the
asymmetry with `broadcast_capacity`. Also revisit earlier if any deployment
does bulk provisioning with all-accounts streams open and `subscription_gap`
markers show up in the edge's logs.

**Status (rechecked 2026-10-15, #189).** The facts above still hold in the
code. The opportunistic trigger passed twice without the promotion: #35
(2026-09-24) changed the module and #80 (2026-10-13) changed a
`RegistryTuning` default. The decision is open again in #192: do the
promotion, or rewrite this trigger so it can be acted on.
