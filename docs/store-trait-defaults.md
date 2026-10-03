# Store trait defaults

The wacore store traits (`wacore/src/store/traits.rs`) give some methods a
default body. A backend that leaves one on its default gets the behaviour
written there, which is not always what the trait doc asks of a real backend.
This table records, for every such method, whether the wamux engine families
(today `sql`, one implementation for Postgres and SQLite, #65) override it, and
why a default that stays is right for a relay.

`scripts/check-store-defaults.py` holds the table to the code, in CI: every
default the pinned wacore declares must have a row, every row must name a
default that still exists, and `override` must mean that every family
implements the method while `default` means that none does. A
`whatsapp-rust` bump that adds a default fails there until it is classified
here.

Decided in #93 (2026-10-02) against wacore at `6f07e3ab`. The throughput
overrides landed in #104 (2026-10-03), after the storage rewrite (#65).

| Method | Trait | Decision | Why |
|---|---|---|---|
| `put_identities_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `delete_identities_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `put_sessions_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `get_sessions_batch` | SignalStore | override | Fixed-size `IN` list of 100 values per query, the last chunk padded with a repeat (#104): one statement text, no round trip per item. |
| `delete_sessions_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `has_session` | SignalStore | default | `get_session(..).is_some()` is the exact answer; a dedicated `EXISTS` only saves reading the record. |
| `has_signal_state_for_user` | SignalStore | default | The default answers `true`, the conservative value: the caller then runs its full per-device PN to LID migration scan instead of skipping it. Slower, never wrong. |
| `store_prekeys_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `load_prekeys_batch` | SignalStore | override | Fixed-size `IN` list of 100 values per query, the last chunk padded with a repeat (#104): one statement text, no round trip per item. |
| `remove_prekeys_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `put_sender_keys_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `delete_sender_keys_batch` | SignalStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `get_mutation_macs` | AppSyncStore | override | Fixed-size `IN` list of 100 values per query, the last chunk padded with a repeat (#104): one statement text, no round trip per item. |
| `commit_patch` | AppSyncStore | override | One transaction for the version, the removed MACs and the added ones (#104): faster, and a crash between them can no longer leave a new version over old MACs. |
| `put_lid_mappings` | ProtocolStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `delete_expired_base_keys` | ProtocolStore | override | The default is `Ok(0)`, and the keepalive sweep calls it every cycle with a one-hour cutoff: `base_keys` was never pruned. |
| `update_device_lists` | ProtocolStore | override | One transaction per batch (#104): one connection and one commit instead of one per row, and all-or-nothing, which the flush already assumes (it clears `dirty` only after the whole batch returns `Ok`). |
| `get_devices_batch` | ProtocolStore | override | Fixed-size `IN` list of 100 values per query, the last chunk padded with a repeat (#104): one statement text, no round trip per item. |
| `get_group_metadata` | ProtocolStore | default | Opt-in persisted cache for the participant-phash re-query skip. The in-memory L1 covers a live session; without persistence a group is queried in full once after a restart. Persisting it needs a new table for a cache. |
| `put_group_metadata` | ProtocolStore | default | Same opt-in cache as `get_group_metadata`: the no-op keeps it off. |
| `delete_group_metadata` | ProtocolStore | default | Same opt-in cache as `get_group_metadata`: nothing is persisted, so there is nothing to delete. |
| `get_tc_tokens` | ProtocolStore | override | Fixed-size `IN` list of 100 values per query, the last chunk padded with a repeat (#104): one statement text, no round trip per item. Answers in the order asked, `None` where no row. |
| `touch_tc_token_sender_timestamp` | ProtocolStore | override | The default is a read-modify-write, and the trait doc requires it to be atomic against `put_tc_token`: concurrent history sync and send-path writers dropped a field. One upsert now. |
| `store_received_tc_token` | ProtocolStore | override | Same atomicity requirement as `touch_tc_token_sender_timestamp`: the newer-wins write is one conditional upsert now. |
| `get_sent_message` | ProtocolStore | override | The default errors with `Unsupported`, so a group repair or resend that missed the in-memory recent-message cache found no payload. |
| `store_pending_inbound` | ProtocolStore | default | The pending inbound buffer backs `InboundDurabilityHook` only (`ClientBuilder::with_inbound_durability_hook`). wamux registers no hook, so the library never calls it; the default fails closed, which is the right answer if one ever did. |
| `get_pending_inbound` | ProtocolStore | default | Hook-only, like `store_pending_inbound`: fails closed. |
| `delete_pending_inbound` | ProtocolStore | default | Hook-only, like `store_pending_inbound`: fails closed. |
| `delete_expired_pending_inbound` | ProtocolStore | default | The keepalive sweep calls it for every backend; with no buffer there is nothing to expire, and the default `Ok(0)` says so. |
| `store_pending_inbound_batch` | ProtocolStore | default | Hook-only: loops over `store_pending_inbound`, which fails closed. |
| `delete_pending_inbound_batch` | ProtocolStore | default | Hook-only: loops over `delete_pending_inbound`, which fails closed. |
| `snapshot_db` | DeviceStore | default | A debugging snapshot of the database to a file, on library request. The relay writes no files of its own; the operator backs the store up. |
| `resource_report` | DeviceStore | default | "Not reported" is accurate: the engines do not introspect their memory. |
| `maintenance` | DeviceStore | override | SQLite: `PRAGMA analysis_limit = 400`, `PRAGMA optimize` and `PRAGMA wal_checkpoint(TRUNCATE)`, a busy checkpoint skipped, not failed (#104). Postgres: nothing, autovacuum does the upkeep. |
| `put_msg_secret` | MsgSecretStore | default | A wrapper over `put_msg_secrets` with no expiry, which both engines implement; nothing to add. |
| `get_msg_secret_with_ts` | MsgSecretStore | override | Both engines store `message_ts`; the default pairs the secret with `0` ("unknown parent time") and loses what the receive path uses for the edit-processing window. |
