# Store trait defaults

The wacore store traits (`wacore/src/store/traits.rs`) give some methods a
default body. A backend that leaves one on its default gets the behaviour
written there, which is not always what the trait doc asks of a real backend.
This table records, for every such method, whether both wamux engines
(Postgres and SQLite) override it, and why a default that stays is right for a
relay.

`scripts/check-store-defaults.py` holds the table to the code, in CI: every
default the pinned wacore declares must have a row, every row must name a
default that still exists, and `override` must mean that both engines
implement the method while `default` means that neither does. A
`whatsapp-rust` bump that adds a default fails there until it is classified
here.

Decided in #93 (2026-10-02) against wacore at `6f07e3ab`. The throughput
overrides are #104, after the storage rewrite (#65).

| Method | Trait | Decision | Why |
|---|---|---|---|
| `put_identities_batch` | SignalStore | default | Throughput only: the loop over `put_identity` is correct, and the signal cache flush keeps a failed address dirty for the next flush. Override in #104. |
| `delete_identities_batch` | SignalStore | default | Throughput only: idempotent per-address deletes, retried by the next flush. Override in #104. |
| `put_sessions_batch` | SignalStore | default | Throughput only: idempotent per-address upserts, retried by the next flush. Override in #104. |
| `get_sessions_batch` | SignalStore | default | Throughput only: the loop over `get_session` returns the same rows as one query. Override in #104. |
| `delete_sessions_batch` | SignalStore | default | Throughput only: idempotent per-address deletes, retried by the next flush. Override in #104. |
| `has_session` | SignalStore | default | `get_session(..).is_some()` is the exact answer; a dedicated `EXISTS` only saves reading the record. |
| `has_signal_state_for_user` | SignalStore | default | The default answers `true`, the conservative value: the caller then runs its full per-device PN to LID migration scan instead of skipping it. Slower, never wrong. |
| `store_prekeys_batch` | SignalStore | default | Throughput only: the loop over `store_prekey` runs at prekey upload, a few times per account lifetime. Override in #104. |
| `load_prekeys_batch` | SignalStore | default | Throughput only: the loop over `load_prekey` returns the same keys. Override in #104. |
| `remove_prekeys_batch` | SignalStore | default | Throughput only: idempotent per-id deletes, retried by the next flush. Override in #104. |
| `put_sender_keys_batch` | SignalStore | default | Throughput only: idempotent per-address upserts, retried by the next flush. Override in #104. |
| `delete_sender_keys_batch` | SignalStore | default | Throughput only: idempotent per-address deletes, retried by the next flush. Override in #104. |
| `get_mutation_macs` | AppSyncStore | default | Throughput only: an N+1 over `get_mutation_mac` per app state patch, same result. Override in #104. |
| `commit_patch` | AppSyncStore | default | Three separate writes per applied patch. The trait doc frames the override as speed; whether it also needs one transaction for crash consistency is an open question in #104. |
| `put_lid_mappings` | ProtocolStore | default | Throughput only: the loop over `put_lid_mapping` is the trait's own "default loops for correctness". Override in #104. |
| `delete_expired_base_keys` | ProtocolStore | override | The default is `Ok(0)`, and the keepalive sweep calls it every cycle with a one-hour cutoff: `base_keys` was never pruned. |
| `update_device_lists` | ProtocolStore | default | Throughput only: the loop over `update_device_list` costs on usync of large groups. Override in #104. |
| `get_devices_batch` | ProtocolStore | default | Throughput only: the loop over `get_devices` costs on a cold large group. Override in #104. |
| `get_group_metadata` | ProtocolStore | default | Opt-in persisted cache for the participant-phash re-query skip. The in-memory L1 covers a live session; without persistence a group is queried in full once after a restart. Persisting it needs a new table for a cache. |
| `put_group_metadata` | ProtocolStore | default | Same opt-in cache as `get_group_metadata`: the no-op keeps it off. |
| `delete_group_metadata` | ProtocolStore | default | Same opt-in cache as `get_group_metadata`: nothing is persisted, so there is nothing to delete. |
| `get_tc_tokens` | ProtocolStore | default | Throughput only: the loop over `get_tc_token` on the reconnect presence re-subscribe. Override in #104. |
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
| `maintenance` | DeviceStore | default | No-op. Postgres has autovacuum; SQLite `PRAGMA optimize` and the WAL truncate are in #104. |
| `put_msg_secret` | MsgSecretStore | default | A wrapper over `put_msg_secrets` with no expiry, which both engines implement; nothing to add. |
| `get_msg_secret_with_ts` | MsgSecretStore | override | Both engines store `message_ts`; the default pairs the secret with `0` ("unknown parent time") and loses what the receive path uses for the edit-processing window. |
