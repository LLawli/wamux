# wamux — Technical Spec

## Layout
```
proto/                 # gRPC contracts (package wamux.v1); source of truth
build.rs               # tonic-build + vendored protoc; emits FILE_DESCRIPTOR_SET
migrations/            # sqlx migrations (0001_initial.sql)
docs/crate-notes/      # verbatim whatsapp-rust/wacore API extraction (reference)
src/
  main.rs              # load config, init tracing, migrate, build registry,
                       #   reconnect always-on, bind socket, serve, graceful shutdown
  lib.rs               # module roots
  server.rs            # assemble tonic Server: register 7 services + reflection
  proto.rs             # include_proto!("wamux.v1") + FILE_DESCRIPTOR_SET
  config.rs            # Config (figment: TOML + WAMUX_ env)
  error.rs             # WamuxError (thiserror) -> tonic::Status
  transport/           # uds_listener (bind/chmod/unlink), shutdown (SIGTERM/SIGINT)
  state/               # account_registry, account_handle, event_ring, event_bridge
  domain/              # bot_factory, event_mapping, jid_parse, messaging,
                       #   media_transfer, groups, contacts
  services/            # one gRPC impl per file (thin): account/event/messaging/
                       #   media/group/contact/admin
  storage/postgres/    # PgBackend + the 4 wacore store traits + accounts + error_map
```

## gRPC services (package `wamux.v1`)
- **AccountService**: CreateAccount, ListAccounts, GetAccountStatus,
  PairWithQr (stream), PairWithCode (stream), ConnectAccount, DisconnectAccount,
  Logout, DeleteAccount. (`Connect`/`Disconnect` are renamed to avoid colliding
  with tonic's generated client `connect`.)
- **EventService**: SubscribeEvents (server stream). Selector = account | all | none;
  `replay_from_ring` for ring replay. `EventEnvelope` oneof covers message, receipt,
  undecryptable, connection, pairing, presence, group, push_name, contact, `raw`.
- **MessagingService**: SendText, SendMedia (client stream), SendReaction,
  EditMessage, DeleteMessage, SendPresence, MarkRead.
- **MediaService**: DownloadMedia (server stream: meta frame + byte chunks).
- **GroupService**, **ContactService**, **AdminService** (GetMetrics).
- **Identity (LID<->PN)**: `InboundMessage.sender_alt`/`recipient_alt` relay the
  stanza's other-namespace jids verbatim; `ContactService.ResolveLidPn` (batch,
  live client, cache-aside) and `ContactService.ListLidMappings` (storage-side,
  works disconnected) read the mapping the library keeps. The core only reports
  pairs the client already learned — no lookup policy, no guessing (issue #1).
  `GetPushName` takes an `AccountRef`: it is the account's OWN name, the getter
  that pairs with `SetPushName`.

## Multi-account runtime (`state/`)
- `AccountRegistry` is the single `Arc<T>` injected into every service: owns the
  `PgPool`, `DashMap<Uuid, Arc<AccountHandle>>`, and an `external_ref` index.
- `AccountHandle` holds: `device_id`, the live `Arc<Client>`, the running `Bot`
  (owned by a **supervisor** task), a `broadcast` event channel (capacity from
  `Config::broadcast_capacity`), the `EventRing`, and a `watch<ConnectionState>`.
- `bot_factory::build_bot` wires `PgBackend` + tokio transport/runtime + ureq HTTP +
  the `event_bridge` `on_event` closure; `skip_history_sync` (relay-pure).
- `event_bridge::dispatch` updates state, maps the event (`domain::event_mapping`),
  stamps seq+timestamp, broadcasts, and pushes to the ring **only if `replayable`**
  (history-sync blobs and events over `replay_max_event_bytes` are excluded — bulk,
  not live replay, so they don't pin memory per account).
- `connect` is idempotent and **edge-driven** (no always-on). It honours a soft
  connection budget (`max_connected_accounts`; `ResourceExhausted` past it) and
  spawns a supervisor that awaits the run loop's **terminal** exit. The library
  owns transient reconnection (Fibonacci backoff); the supervisor only fires when
  it has given up, then marks the account down truthfully (so `is_running` never
  lies) and releases the budget slot. Any reconnect-after-terminal is edge policy.
- `stop`/`disconnect` are **graceful**: `Client::disconnect` (flush + close) then
  await the supervisor within `graceful_stop_timeout_ms`, detaching on timeout.
- `EventService` single-account → one forwarder; all → one per handle into a shared
  mpsc. Delivery is **lossy under lag**: a slow subscriber gets a `raw` "gap" marker
  (carrying the current state) and resyncs via `GetAccountStatus` (authoritative
  watch). Per-account forwarders are independent — no cross-account head-of-line
  blocking.

## Postgres storage (`storage/postgres/`)
- Implements the four wacore traits (`SignalStore`, `AppSyncStore`, `ProtocolStore`,
  `DeviceStore`); `Backend` is their blanket impl. `sqlx` runtime queries (no
  compile-time DB needed).
- One DB, one pool; every store row scoped by integer `device_id`. `accounts`
  (UUID/external_ref ↔ `device_id`, IDENTITY) is the parent; all store tables FK to
  it `ON DELETE CASCADE`, so `DeleteAccount` is one delete.
- **Wire format matches the sqlite reference exactly** (see
  `docs/crate-notes/sqlite-reference.md`): raw bytes for keys/sessions/records,
  bincode-standard for app-state keys/versions, serde_json for `device_registry`,
  and the whole `Device` as one bincode blob (runtime-only `device_props` restored
  from `DEVICE_PROPS` on load).

## Media
- Send: `MediaService` not used for send; `MessagingService.SendMedia` client-streams
  a header then inline chunks, or fetches a URL (blocking ureq on `spawn_blocking`);
  uploads via `Client::upload` and builds the matching `wa::Message`.
- Receive: inbound events carry a `MediaDescriptor`; the edge later calls
  `DownloadMedia`, which `download_from_params` decrypts and streams back.

## Notable constraints / gotchas
- waproto uses **prost 0.14**; tonic codegen uses **prost 0.13**. We depend on both
  (`prost` 0.13 for generated code, `prost014` alias for `wa::Message::encode_to_vec`).
- `Client::contacts().is_on_whatsapp` / `get_user_info` return **non-Send** futures
  (HRTB); they're driven on a dedicated current-thread runtime via `spawn_blocking`
  (`domain::contacts::run_isolated`) so they don't poison the `#[async_trait]` future.
- Socket group ownership comes from the daemon's process group (no chown); run wamux
  as the intended group for 0660 sharing.
- Prometheus is exposed via `AdminService.GetMetrics` (no second/TCP listener);
  `AdminService.Check` reports daemon liveness + readiness (Postgres `SELECT 1`).
- **Reconnection is never the core's job.** The library reconnects transient drops
  with backoff; the edge owns any reconnect after a terminal exit (401/409/logout).
  The core only emits truthful connection state + a clean terminal signal.

## Verification
- `cargo clippy --all-targets -- -D warnings` (clean), `cargo fmt`.
- `cargo test` with `DATABASE_URL` set:
  - `tests/postgres_backend.rs`: `device_id` isolation + round-trip + `Backend` proof.
  - `tests/grpc_server.rs`: account lifecycle over a real UDS gRPC connection.
  - `domain::event_mapping` unit tests (pairing mapping).
- Manual (needs a phone): `grpcurl -unix /run/wamux.sock list`; CreateAccount →
  PairWithQr/PairWithCode → send/receive text+media → groups/contacts; restart and
  confirm always-on reconnect.
