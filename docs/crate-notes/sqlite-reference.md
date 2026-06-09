I now have the complete picture. I have read all source files, the schema, the Cargo.toml, and every migration. Let me compile the exhaustive answer.

# whatsapp-rust-sqlite-storage 0.6.0 — Complete Storage Reference

Crate root: `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-sqlite-storage-0.6.0/`
Source files: `src/lib.rs` (only re-exports `SqliteStore`), `src/schema.rs`, `src/sqlite_store.rs` (3091 lines, all stores + the store struct).

---

## 1. The store struct: `SqliteStore`

File: `src/sqlite_store.rs`. The store keeps an r2d2 Diesel pool, a 1-permit semaphore that serializes writes, the on-disk path (for `VACUUM INTO` snapshots), and a single `device_id` (i32) that scopes every query.

```rust
type SqlitePool = Pool<ConnectionManager<SqliteConnection>>;

#[derive(Clone)]
pub struct SqliteStore {
    pub(crate) pool: SqlitePool,
    pub(crate) db_semaphore: Arc<tokio::sync::Semaphore>,
    pub(crate) database_path: String,
    device_id: i32,
}
```

Constructors and `device_id()` verbatim:

```rust
impl SqliteStore {
    pub async fn new(database_url: &str) -> std::result::Result<Self, StoreError> {
        let manager = ConnectionManager::<SqliteConnection>::new(database_url);

        let pool_size = 2;

        let pool = Pool::builder()
            .max_size(pool_size)
            .connection_customizer(Box::new(ConnectionOptions))
            .build(manager)
            .map_err(|e| StoreError::Connection(Box::new(e)))?;

        let pool_clone = pool.clone();
        tokio::task::spawn_blocking(move || -> std::result::Result<(), StoreError> {
            let mut conn = pool_clone
                .get()
                .map_err(|e| StoreError::Connection(Box::new(e)))?;

            diesel::sql_query("PRAGMA journal_mode = WAL;")
                .execute(&mut conn)
                .map_err(|e| StoreError::Database(Box::new(e)))?;

            conn.run_pending_migrations(MIGRATIONS)
                .map_err(StoreError::Migration)?;

            Ok(())
        })
        .await
        .map_err(|e| StoreError::Database(Box::new(e)))??;

        let database_path = parse_database_path(database_url)?;

        Ok(Self {
            pool,
            db_semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
            database_path,
            device_id: 1,
        })
    }

    pub async fn new_for_device(
        database_url: &str,
        device_id: i32,
    ) -> std::result::Result<Self, StoreError> {
        let mut store = Self::new(database_url).await?;
        store.device_id = device_id;
        Ok(store)
    }

    pub fn device_id(&self) -> i32 {
        self.device_id
    }
```

Key behavioral notes:
- `new()` defaults `device_id = 1`. `new_for_device(url, id)` calls `new()` then overrides. Pool size = 2.
- Per-connection PRAGMAs set on acquire via `ConnectionOptions`: `busy_timeout = 30000`, `synchronous = NORMAL`, `cache_size = 512`, `temp_store = memory`, `foreign_keys = ON`. Plus `journal_mode = WAL` set once on first connection in `new()`.
- Writes are serialized through a 1-permit `tokio::sync::Semaphore` plus retry-on-busy. Two retry helpers exist: `with_semaphore` (serialize, no retry) and `with_retry` (serialize + up to 5 retries with exponential backoff on SQLite "locked"/"busy"). Many simple reads bypass both and just `spawn_blocking` on the pool directly.
- `device_id` is the only multi-account scoping field. Every store trait method filters/inserts on `self.device_id`.

---

## 2. Serialization crates and helpers

From `Cargo.toml`:
- `bincode = "2.0.1"` with `features = ["serde"]` — used via `bincode::serde::encode_to_vec(&val, bincode::config::standard())` and `bincode::serde::decode_from_slice(&bytes, bincode::config::standard())`. **Config is always `bincode::config::standard()`** (little-endian, variable-int length encoding). bincode 2.x is NOT wire-compatible with bincode 1.x; to replicate in Postgres you must use bincode 2 standard config, or store the same byte blobs.
- `serde_json = "1.0"` — used only for `device_registry.devices_json` (a TEXT column holding a JSON array of `DeviceInfo`).
- `wacore::store::device::account_serde::{to_bytes, from_bytes}` — the device `account` BLOB encoder (an opaque wacore helper; treat its output as opaque bytes — `to_bytes` is infallible returning `Vec<u8>`, `from_bytes` returns a Result).

Custom byte helpers in the store itself (NOT bincode/json — raw concatenation):

```rust
fn serialize_keypair(&self, key_pair: &KeyPair) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(key_pair.private_key.serialize());
    bytes.extend_from_slice(key_pair.public_key.public_key_bytes());
    Ok(bytes)
}

fn deserialize_keypair(&self, bytes: &[u8]) -> Result<KeyPair> {
    if bytes.len() != 64 {
        return Err(StoreError::Validation(format!("Invalid KeyPair length: {}", bytes.len())));
    }
    let private_key = PrivateKey::deserialize(&bytes[0..32])
        .map_err(|e| StoreError::Serialization(Box::new(e)))?;
    let public_key = PublicKey::from_djb_public_key_bytes(&bytes[32..64])
        .map_err(|e| StoreError::Serialization(Box::new(e)))?;
    Ok(KeyPair::new(public_key, private_key))
}
```

So a KeyPair BLOB = **64 raw bytes**: 32-byte private key `||` 32-byte DJB public key (no length prefix, no framing). This applies to `noise_key`, `identity_key`, `signed_pre_key` in the `device` table.

**Encoding summary by value type:**
| Value | Encoding |
|---|---|
| `KeyPair` (noise/identity/signed_pre_key) | raw 64 bytes: priv[32] ‖ pub[32] |
| `signed_pre_key_signature` | raw `[u8;64]` (stored/read as exactly 64 bytes) |
| `adv_secret_key` | raw `[u8;32]` (stored/read as exactly 32 bytes) |
| `account` | `wacore::store::device::account_serde::to_bytes` (opaque) |
| `edge_routing_info`, `nct_salt` | raw bytes (`Option<Vec<u8>>`, no transform) |
| `server_cert_chain` | **bincode** standard (`Option<CachedServerCertChain>`); decode failure logs + degrades to None |
| identity `key` | raw `[u8;32]` |
| session `record`, sender_key `record`, prekey `key`, signed_prekey `record`, base_key | raw bytes, no transform |
| `app_state_keys.key_data` (AppStateSyncKey) | **bincode** standard |
| `app_state_versions.state_data` (HashState) | **bincode** standard |
| `app_state_mutation_macs.index_mac/value_mac` | raw bytes |
| `device_registry.devices_json` | **serde_json** string of `Vec<DeviceInfo>` |
| `tc_tokens.token` | raw bytes (`Vec<u8>`) |
| `sent_messages.payload` | raw bytes |
| prekey id / signed_prekey id | `i32` in DB, `u32` in app (cast `id as i32` / `as u32`) |
| device_id everywhere | `i32` |

---

## 2b. Per-method serialization + SQL (exhaustive)

`device_id` is applied in EVERY method as a WHERE filter (reads/deletes) and as a column value (inserts), using `self.device_id` (the trait methods delegate to `*_for_device` helpers passing `self.device_id`). Conflict targets always include `device_id`.

### DeviceStore (`impl DeviceStore`)

The trait methods just delegate:
```rust
async fn save(&self, device: &CoreDevice) -> Result<()> {
    SqliteStore::save_device_data_for_device(self, self.device_id, device).await }
async fn load(&self) -> Result<Option<CoreDevice>> {
    SqliteStore::load_device_data_for_device(self, self.device_id).await }
async fn exists(&self) -> Result<bool> { SqliteStore::device_exists(self, self.device_id).await }
async fn create(&self) -> Result<i32> { SqliteStore::create_new_device(self).await }
```

**`save_device_data_for_device`** — table `device`, `INSERT ... ON CONFLICT(device::id) DO UPDATE SET (...)` upsert on the device id (NOT device_id; the `device` table PK is `id`). Serialization happens before the insert:
```rust
let noise_key_data: Arc<[u8]> = self.serialize_keypair(&device_data.noise_key)?.into();
let identity_key_data: Arc<[u8]> = self.serialize_keypair(&device_data.identity_key)?.into();
let signed_pre_key_data: Arc<[u8]> = self.serialize_keypair(&device_data.signed_pre_key)?.into();
let account_data: Option<Arc<[u8]>> = device_data.account.as_ref()
    .map(|a| Arc::from(wacore::store::device::account_serde::to_bytes(a)));
...
let signed_pre_key_signature: Arc<[u8]> = Arc::from(&device_data.signed_pre_key_signature[..]);
let adv_secret_key: Arc<[u8]> = Arc::from(&device_data.adv_secret_key[..]);
let edge_routing_info: Option<Arc<[u8]>> = device_data.edge_routing_info.as_deref().map(Arc::from);
let nct_salt: Option<Arc<[u8]>> = device_data.nct_salt.as_deref().map(Arc::from);
let server_cert_chain: Option<Arc<[u8]>> = device_data.server_cert_chain.as_ref()
    .map(|chain| bincode::serde::encode_to_vec(chain, bincode::config::standard())
        .map(Arc::from).map_err(|e| StoreError::Serialization(Box::new(e))))
    .transpose()?;
let new_lid: Arc<str> = Arc::from(device_data.lid.as_ref().map(|j| j.to_string()).unwrap_or_default().as_str());
let new_pn: Arc<str>  = Arc::from(device_data.pn.as_ref().map(|j| j.to_string()).unwrap_or_default().as_str());
```
Insert uses `device::id.eq(device_id)` (the i32 passed in) and writes every column; numeric app fields cast `as i32` / `as i64`. `lid`/`pn` are JID `.to_string()` or `""` when None.

**`create_new_device`** — plain `INSERT INTO device` (no on_conflict) with `device::id.eq(self.device_id)`, `lid=""`, `pn=""`, all Option columns set `None`, `next_pre_key_id` and `server_has_prekeys` from a fresh `Device::new()`. Returns `device_id`. Note: the table has `AUTOINCREMENT` but this inserts an explicit id.

**`load_device_data_for_device`** — `device::table.filter(device::id.eq(device_id)).first::<DeviceRow>().optional()`. Then deserializes:
- `noise_key/identity_key/signed_pre_key` via `deserialize_keypair` (expects 64 bytes).
- `signed_pre_key_signature: [u8;64] = row...try_into()` (errors if not 64).
- `adv_secret_key: [u8;32] = row...try_into()` (errors if not 32).
- `account` via `account_serde::from_bytes`.
- `server_cert_chain` via `bincode::serde::decode_from_slice(bytes, standard())`; on Err logs a warning and yields `None` (cache, non-fatal).
- `pn`/`lid` parsed only if non-empty (`row.pn.parse().ok()`), else None.
- Numeric fields cast back to u32 (`as u32`, `app_version_tertiary.try_into().unwrap_or(0u32)`).
- Sets `device_props = wacore::store::device::DEVICE_PROPS.clone()`, `client_profile = ClientProfile::web()`, `nct_salt_sync_seen = false` (not persisted columns).

`DeviceRow` is a `#[derive(Queryable, Selectable)]` struct whose field order matches `schema::device` (see §1 source). `device_exists`: `device::table.filter(device::id.eq(device_id)).count() > 0`.

**`snapshot_db`** (DeviceStore): sanitizes name, then `format!("VACUUM INTO '{}'", target_path)` where `target_path = "{db_path}.snapshot-{timestamp}-{name}"`; optionally writes `{target_path}.json` with extra bytes. Rejects in-memory DBs (see `parse_database_path`).

### SignalStore (`impl SignalStore`)

- **`put_identity(address, key:[u8;32])`** → `put_identity_for_device`: table `identities`, `INSERT (address, key=&key_vec[..], device_id) ON CONFLICT(address, device_id) DO UPDATE SET key=...`. Key stored as raw 32 bytes. Inline 5-retry loop.
- **`load_identity(address) -> Option<[u8;32]>`** → `load_identity_for_device` selects `identities::key` filtered by address+device_id (`with_semaphore`), then `v.try_into()` to `[u8;32]` (errors on wrong length).
- **`delete_identity`** → `DELETE FROM identities WHERE address=? AND device_id=?`.
- **`get_session(address) -> Option<Bytes>`** → `get_session_for_device`: `SELECT record FROM sessions WHERE address=? AND device_id=?` (`with_semaphore`), wrapped `Bytes::from`. Record stored/read as raw bytes.
- **`has_session`** → `diesel::select(exists(sessions WHERE address & device_id))` via `with_semaphore`.
- **`put_session(address, session:&[u8])`** → `put_session_for_device`: `INSERT (address, record=&session_vec, device_id) ON CONFLICT(address, device_id) DO UPDATE SET record=...`. Raw bytes. Inline 5-retry loop.
- **`delete_session`** → `DELETE FROM sessions WHERE address & device_id`.
- **`store_prekey(id:u32, record:&[u8], uploaded:bool)`** — table `prekeys`: `INSERT (id as i32, key=&record, uploaded, device_id) ON CONFLICT(id, device_id) DO UPDATE SET key=..., uploaded=...`. Inline 5-retry loop. Raw bytes.
- **`store_prekeys_batch(keys:&[(u32,Bytes)], uploaded)`** — wraps a `conn.transaction` looping the same upsert per `(id, record.as_ref())`. Inline 5-retry loop.
- **`load_prekey(id) -> Option<Bytes>`** — `SELECT key FROM prekeys WHERE id=(id as i32) AND device_id=?`, mapped `Bytes::from`.
- **`load_prekeys_batch(ids) -> Vec<(u32,Bytes)>`** — `SELECT id,key WHERE id = ANY(ids as i32) AND device_id` via `with_semaphore`; maps `(id as u32, Bytes::from(key))`.
- **`remove_prekey(id)`** — `DELETE FROM prekeys WHERE id=(id as i32) AND device_id`. Inline 5-retry loop.
- **`get_max_prekey_id() -> u32`** — `SELECT max(prekeys::id) WHERE device_id`, `unwrap_or(0) as u32` (acquires semaphore once).
- **`store_signed_prekey(id, record)`** — table `signed_prekeys`: `INSERT (id as i32, record=&record, device_id) ON CONFLICT(id, device_id) DO UPDATE SET record=...`. Raw bytes. Inline 5-retry loop.
- **`load_signed_prekey(id) -> Option<Vec<u8>>`** — `SELECT record FROM signed_prekeys WHERE id=(id as i32) AND device_id`.
- **`load_all_signed_prekeys() -> Vec<(u32,Vec<u8>)>`** — `SELECT id,record WHERE device_id`, maps `id as u32`.
- **`remove_signed_prekey(id)`** — `DELETE FROM signed_prekeys WHERE id & device_id`. Inline 5-retry loop.
- **`put_sender_key(address, record)`** → `put_sender_key_for_device`: table `sender_keys`, `INSERT (address, record=&record_vec, device_id) ON CONFLICT(address, device_id) DO UPDATE SET record=...`. Raw bytes.
- **`get_sender_key(address) -> Option<Vec<u8>>`** → `SELECT record FROM sender_keys WHERE address & device_id`.
- **`delete_sender_key(address)`** → `DELETE FROM sender_keys WHERE address & device_id`.

### AppSyncStore (`impl AppSyncStore`)

- **`get_sync_key(key_id) -> Option<AppStateSyncKey>`** → `get_app_state_sync_key_for_device`: `SELECT key_data FROM app_state_keys WHERE key_id=? AND device_id=?`, then:
  ```rust
  let (key, _) = bincode::serde::decode_from_slice(&data, bincode::config::standard())?;
  ```
- **`set_sync_key(key_id, key:AppStateSyncKey)`** → `set_app_state_sync_key_for_device`:
  ```rust
  let data = bincode::serde::encode_to_vec(&key, bincode::config::standard())?;
  // INSERT (key_id, key_data=&data, device_id) ON CONFLICT(key_id, device_id) DO UPDATE SET key_data=...
  ```
  `key_id` stored as raw bytes (BLOB PK).
- **`get_latest_sync_key_id() -> Option<Vec<u8>>`** → `SELECT key_id FROM app_state_keys WHERE device_id ORDER BY key_id DESC LIMIT 1` (raw bytes, **lexicographic** ordering on the BLOB).
- **`get_version(name) -> HashState`** → `get_app_state_version_for_device`: `SELECT state_data FROM app_state_versions WHERE name=? AND device_id=?`; if present `bincode::serde::decode_from_slice(... standard())` else `HashState::default()`.
- **`set_version(name, state:HashState)`** → `set_app_state_version_for_device`:
  ```rust
  let data = bincode::serde::encode_to_vec(&state, bincode::config::standard())?;
  // INSERT (name, state_data=&data, device_id) ON CONFLICT(name, device_id) DO UPDATE SET state_data=...
  ```
  Uses `with_retry`.
- **`put_mutation_macs(name, version:u64, mutations:&[AppStateMutationMAC])`** → `put_app_state_mutation_macs_for_device`: table `app_state_mutation_macs`, batched insert in chunks of 100 rows, each row `(name, version as i64, index_mac=&m.index_mac, value_mac=&m.value_mac, device_id)`, `ON CONFLICT(name, index_mac, device_id) DO UPDATE SET version=excluded.version, value_mac=excluded.value_mac`. `index_mac`/`value_mac` are raw `Vec<u8>`. Uses `with_retry`.
- **`get_mutation_mac(name, index_mac) -> Option<Vec<u8>>`** → `SELECT value_mac WHERE name=? AND index_mac=? AND device_id=?`.
- **`delete_mutation_macs(name, index_macs:&[Vec<u8>])`** → `DELETE WHERE name=? AND index_mac IN (chunk) AND device_id=?`, chunked at 500. Uses `with_retry`.

### ProtocolStore (`impl ProtocolStore`)

- **`get_sender_key_devices(group_jid) -> Vec<(String,bool)>`** — table `sender_key_devices`: `SELECT device_jid, has_key WHERE group_jid=? AND device_id=?`; `has_key` is INTEGER, mapped `has_key != 0`.
- **`set_sender_key_status(group_jid, entries:&[(&str,bool)])`** — chunked (190) upsert: `(group_jid, device_jid, has_key = i32::from(bool), device_id, updated_at = now_secs)`, `ON CONFLICT(group_jid, device_jid, device_id) DO UPDATE SET has_key=excluded.has_key, updated_at=now`. `with_retry`.
- **`clear_sender_key_devices(group_jid)`** — `DELETE WHERE group_jid & device_id`. `with_retry`.
- **`clear_all_sender_key_devices()`** — `DELETE WHERE device_id`. `with_retry`.
- **`delete_sender_key_device_rows(device_jids:&[&str])`** — `DELETE WHERE device_jid IN (chunk) AND device_id`, chunk 190. `with_retry`.
- **`get_lid_mapping(lid) -> Option<LidPnMappingEntry>`** — table `lid_pn_mapping`: `SELECT (lid, phone_number, created_at, learning_source, updated_at) WHERE lid=? AND device_id=?`. Columns are TEXT/TEXT/BIGINT/TEXT/BIGINT; built into `LidPnMappingEntry { lid, phone_number, created_at, updated_at, learning_source }` (note field order in struct differs from select order — they reorder by name).
- **`get_pn_mapping(phone) -> Option<...>`** — same SELECT but `WHERE phone_number=? AND device_id=? ORDER BY updated_at DESC LIMIT 1`.
- **`put_lid_mapping(entry)`** → calls `put_lid_mappings(&[entry])`.
- **`put_lid_mappings(entries)`** — `conn.transaction` looping: `INSERT (lid, phone_number, created_at, learning_source, updated_at, device_id) ON CONFLICT(lid, device_id) DO UPDATE SET phone_number=, learning_source=, updated_at=`. All scalar columns, no blob. `with_retry`.
- **`get_all_lid_mappings() -> Vec<...>`** — `SELECT 5 cols WHERE device_id`.
- **`save_base_key(address, message_id, base_key:&[u8])`** — table `base_keys`: `INSERT (address, message_id, base_key=&base_key, device_id, created_at = now_secs as i32) ON CONFLICT(address, message_id, device_id) DO UPDATE SET base_key=...`. Raw bytes.
- **`has_same_base_key(address, message_id, current:&[u8]) -> bool`** — `SELECT base_key WHERE address & message_id & device_id`; returns `stored == Some(current)`.
- **`delete_base_key(address, message_id)`** — `DELETE WHERE address & message_id & device_id`.
- **`update_device_list(record:DeviceListRecord)`** — table `device_registry`:
  ```rust
  let devices_json = serde_json::to_string(&record.devices)?;   // JSON array of DeviceInfo, stored in TEXT col
  // INSERT (user_id=record.user, devices_json, timestamp = record.timestamp as i32, phash, device_id,
  //         updated_at = now_secs as i32, raw_id = record.raw_id.map(|r| r as i32))
  // ON CONFLICT(user_id, device_id) DO UPDATE SET devices_json=, timestamp=, phash=, updated_at=, raw_id=
  ```
- **`update_device_lists(records)`** — pre-serializes each `devices_json` with serde_json, then a transaction looping the same upsert. `with_retry`.
- **`get_devices(user) -> Option<DeviceListRecord>`** — `SELECT (user_id, devices_json, timestamp, phash, raw_id) WHERE user_id & device_id`; then `serde_json::from_str::<Vec<DeviceInfo>>(&devices_json)`; `timestamp as i64`, `raw_id.map(|r| r as u32)`.
- **`delete_devices(user)`** — `DELETE WHERE user_id & device_id`.
- **`get_tc_token(jid) -> Option<TcTokenEntry>`** — table `tc_tokens`: `SELECT (token, token_timestamp, sender_timestamp) WHERE jid & device_id`. `token` raw `Vec<u8>`, timestamps `i64`/`Option<i64>`. Built `TcTokenEntry { token, token_timestamp, sender_timestamp }`.
- **`put_tc_token(jid, entry)`** — `INSERT (jid, token=&entry.token, token_timestamp, sender_timestamp, device_id, updated_at = now_secs) ON CONFLICT(jid, device_id) DO UPDATE SET token=, token_timestamp=, sender_timestamp=, updated_at=`. `token` raw bytes.
- **`delete_tc_token(jid)`** — `DELETE WHERE jid & device_id`.
- **`get_all_tc_token_jids() -> Vec<String>`** — `SELECT jid WHERE device_id`.
- **`delete_expired_tc_tokens(cutoff:i64) -> u32`** — `DELETE WHERE token_timestamp < cutoff AND device_id`; returns affected rows as u32.
- **`store_sent_message(chat_jid, message_id, payload:&[u8])`** — table `sent_messages`: `REPLACE INTO sent_messages (chat_jid, message_id, payload, device_id)` (uses `diesel::replace_into`, note `created_at` left to DEFAULT). Raw bytes. `with_retry`.
- **`take_sent_message(chat_jid, message_id) -> Option<Vec<u8>>`** — `conn.immediate_transaction`: `SELECT payload WHERE chat_jid & message_id & device_id`; if found, `DELETE` the same row; returns the payload (atomic SELECT+DELETE). `with_retry`.
- **`delete_expired_sent_messages(cutoff:i64) -> u32`** — `DELETE WHERE created_at < cutoff AND device_id`.

---

## 3. Full SQLite schema

### `src/schema.rs` (Diesel `table!` macros, verbatim — the post-all-migrations shape)

```rust
// @generated automatically by Diesel CLI.

diesel::table! {
    app_state_keys (key_id, device_id) {
        key_id -> Binary,
        key_data -> Binary,
        device_id -> Integer,
    }
}

diesel::table! {
    app_state_mutation_macs (name, index_mac, device_id) {
        name -> Text,
        version -> BigInt,
        index_mac -> Binary,
        value_mac -> Binary,
        device_id -> Integer,
    }
}

diesel::table! {
    app_state_versions (name, device_id) {
        name -> Text,
        state_data -> Binary,
        device_id -> Integer,
    }
}

diesel::table! {
    base_keys (address, message_id, device_id) {
        address -> Text,
        message_id -> Text,
        base_key -> Binary,
        device_id -> Integer,
        created_at -> Integer,
    }
}

diesel::table! {
    device_registry (user_id, device_id) {
        user_id -> Text,
        devices_json -> Text,
        timestamp -> Integer,
        phash -> Nullable<Text>,
        device_id -> Integer,
        updated_at -> Integer,
        raw_id -> Nullable<Integer>,
    }
}

diesel::table! {
    device (id) {
        id -> Integer,
        lid -> Text,
        pn -> Text,
        registration_id -> Integer,
        noise_key -> Binary,
        identity_key -> Binary,
        signed_pre_key -> Binary,
        signed_pre_key_id -> Integer,
        signed_pre_key_signature -> Binary,
        adv_secret_key -> Binary,
        account -> Nullable<Binary>,
        push_name -> Text,
        app_version_primary -> Integer,
        app_version_secondary -> Integer,
        app_version_tertiary -> BigInt,
        app_version_last_fetched_ms -> BigInt,
        edge_routing_info -> Nullable<Binary>,
        props_hash -> Nullable<Text>,
        next_pre_key_id -> Integer,
        nct_salt -> Nullable<Binary>,
        server_has_prekeys -> Bool,
        server_cert_chain -> Nullable<Binary>,
    }
}

diesel::table! {
    identities (address, device_id) {
        address -> Text,
        key -> Binary,
        device_id -> Integer,
    }
}

diesel::table! {
    lid_pn_mapping (lid, device_id) {
        lid -> Text,
        phone_number -> Text,
        created_at -> BigInt,
        learning_source -> Text,
        updated_at -> BigInt,
        device_id -> Integer,
    }
}

diesel::table! {
    prekeys (id, device_id) {
        id -> Integer,
        key -> Binary,
        uploaded -> Bool,
        device_id -> Integer,
    }
}

diesel::table! {
    sender_key_devices (group_jid, device_jid, device_id) {
        group_jid -> Text,
        device_jid -> Text,
        has_key -> Integer,
        device_id -> Integer,
        updated_at -> BigInt,
    }
}

diesel::table! {
    sender_keys (address, device_id) {
        address -> Text,
        record -> Binary,
        device_id -> Integer,
    }
}

diesel::table! {
    sessions (address, device_id) {
        address -> Text,
        record -> Binary,
        device_id -> Integer,
    }
}

diesel::table! {
    signed_prekeys (id, device_id) {
        id -> Integer,
        record -> Binary,
        device_id -> Integer,
    }
}

diesel::table! {
    tc_tokens (jid, device_id) {
        jid -> Text,
        token -> Binary,
        token_timestamp -> BigInt,
        sender_timestamp -> Nullable<BigInt>,
        device_id -> Integer,
        updated_at -> BigInt,
    }
}

diesel::table! {
    sent_messages (chat_jid, message_id, device_id) {
        chat_jid -> Text,
        message_id -> Text,
        payload -> Binary,
        device_id -> Integer,
        created_at -> BigInt,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    app_state_keys,
    app_state_mutation_macs,
    app_state_versions,
    base_keys,
    device,
    device_registry,
    identities,
    lid_pn_mapping,
    prekeys,
    sender_key_devices,
    sender_keys,
    sent_messages,
    sessions,
    signed_prekeys,
    tc_tokens,
);
```

### Diesel type → SQLite/Postgres mapping cheat sheet
- `Binary` = SQLite BLOB → Postgres `BYTEA`.
- `Text` = TEXT → `TEXT`.
- `Integer` = i32 → `INTEGER`/`INT4`.
- `BigInt` = i64 → `BIGINT`/`INT8`.
- `Bool` = i32-ish in SQLite (0/1) → `BOOLEAN`.
- `Nullable<...>` → nullable column.

Note a schema/migration discrepancy to be aware of: the Diesel schema models `base_keys.created_at` as `Integer`, `device_registry.timestamp`/`updated_at` as `Integer`, and `sender_key_devices.updated_at`/`tc_tokens.*timestamp`/`sent_messages.created_at` as `BigInt`. The migrations declare several of these as `INTEGER` (SQLite INTEGER is dynamic width, so it accepts both i32 and i64). In Postgres pick the wider type the code actually writes: `update_device_list` writes `timestamp as i32` and `updated_at = now_secs as i32` (fits INT4); `sent_messages.created_at`/`tc_tokens` timestamps are read as i64 (use BIGINT); `sender_key_devices.updated_at` is written `now_secs` (i64 → BIGINT).

### Migrations (verbatim, all `up.sql` / `down.sql`)

`embed_migrations!("migrations")` runs them in directory order. Full files are in §below; here is each up.sql verbatim plus a column listing per final table.

**`2025-08-14-035031_initial/up.sql`**
```sql
CREATE TABLE identities (
    address TEXT PRIMARY KEY NOT NULL,
    key BLOB NOT NULL
);

CREATE TABLE sessions (
    address TEXT PRIMARY KEY NOT NULL,
    record BLOB NOT NULL
);

CREATE TABLE prekeys (
    id INTEGER PRIMARY KEY NOT NULL,
    key BLOB NOT NULL,
    uploaded BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE sender_keys (
    address TEXT PRIMARY KEY NOT NULL,
    record BLOB NOT NULL
);

CREATE TABLE app_state_keys (
    key_id BLOB PRIMARY KEY NOT NULL,
    key_data BLOB NOT NULL
);

CREATE TABLE app_state_versions (
    name TEXT PRIMARY KEY NOT NULL,
    state_data BLOB NOT NULL
);

CREATE TABLE app_state_mutation_macs (
    name TEXT NOT NULL,
    version BIGINT NOT NULL,
    index_mac BLOB NOT NULL,
    value_mac BLOB NOT NULL,
    PRIMARY KEY (name, index_mac)
);

CREATE TABLE device (
    lid TEXT PRIMARY KEY NOT NULL,
    pn TEXT NOT NULL,
    registration_id INTEGER NOT NULL,
    noise_key BLOB NOT NULL,
    identity_key BLOB NOT NULL,
    signed_pre_key BLOB NOT NULL,
    signed_pre_key_id INTEGER NOT NULL,
    signed_pre_key_signature BLOB NOT NULL,
    adv_secret_key BLOB NOT NULL,
    account BLOB,
    push_name TEXT NOT NULL DEFAULT '',
    app_version_primary INTEGER NOT NULL DEFAULT 0,
    app_version_secondary INTEGER NOT NULL DEFAULT 0,
    app_version_tertiary BIGINT NOT NULL DEFAULT 0,
    app_version_last_fetched_ms BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE signed_prekeys (
    id INTEGER PRIMARY KEY NOT NULL,
    record BLOB NOT NULL
);
```

**`2025-09-23-032232-0000_add_multi_account_support/up.sql`** — recreates `device` with `id INTEGER PRIMARY KEY AUTOINCREMENT` (existing row becomes id=1), and rebuilds every account table adding `device_id INTEGER NOT NULL DEFAULT 1` into the composite PK, plus `idx_*_device_id` indexes:
```sql
-- device_new: id INTEGER PRIMARY KEY AUTOINCREMENT, ... (same columns as initial device)
-- identities       PRIMARY KEY (address, device_id)              + idx_identities_device_id
-- sessions         PRIMARY KEY (address, device_id)              + idx_sessions_device_id
-- prekeys          PRIMARY KEY (id, device_id)                   + idx_prekeys_device_id
-- sender_keys      PRIMARY KEY (address, device_id)              + idx_sender_keys_device_id
-- signed_prekeys   PRIMARY KEY (id, device_id)                   + idx_signed_prekeys_device_id
-- app_state_keys   PRIMARY KEY (key_id, device_id)               + idx_app_state_keys_device_id
-- app_state_versions PRIMARY KEY (name, device_id)               + idx_app_state_versions_device_id
-- app_state_mutation_macs PRIMARY KEY (name, index_mac, device_id) + idx_app_state_mutation_macs_device_id
```
(Each recreated table copies old rows with `device_id = 1`.)

**`2025-12-04-000000_add_skdm_recipients/up.sql`** (later dropped by the 2026-03-26 migration):
```sql
CREATE TABLE skdm_recipients (
    group_jid TEXT NOT NULL,
    device_jid TEXT NOT NULL,
    device_id INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (group_jid, device_jid, device_id)
);
CREATE INDEX idx_skdm_recipients_group ON skdm_recipients (group_jid, device_id);
```

**`2025-12-11-000000_add_lid_pn_mapping/up.sql`**:
```sql
CREATE TABLE lid_pn_mapping (
    lid TEXT NOT NULL,
    phone_number TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    learning_source TEXT NOT NULL,
    updated_at BIGINT NOT NULL,
    device_id INTEGER NOT NULL,
    PRIMARY KEY (lid, device_id),
    FOREIGN KEY(device_id) REFERENCES device(id) ON DELETE CASCADE
);
CREATE INDEX idx_lid_pn_mapping_phone ON lid_pn_mapping(phone_number, device_id);
ALTER TABLE device ADD COLUMN edge_routing_info BLOB;
```

**`2025-12-24-000000_add_whatsapp_web_alignment/up.sql`**:
```sql
CREATE TABLE base_keys (
    address TEXT NOT NULL,
    message_id TEXT NOT NULL,
    base_key BLOB NOT NULL,
    device_id INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (address, message_id, device_id)
);
CREATE INDEX idx_base_keys_device ON base_keys (device_id);

CREATE TABLE device_registry (
    user_id TEXT NOT NULL,
    devices_json TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    phash TEXT,
    device_id INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (user_id, device_id)
);
CREATE INDEX idx_device_registry_timestamp ON device_registry (timestamp);
CREATE INDEX idx_device_registry_device ON device_registry (device_id);
CREATE INDEX idx_device_registry_updated_at ON device_registry (updated_at);

CREATE TABLE sender_key_status (   -- later dropped by 2026-03-26
    group_jid TEXT NOT NULL,
    participant TEXT NOT NULL,
    device_id INTEGER NOT NULL DEFAULT 1,
    marked_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (group_jid, participant, device_id)
);
CREATE INDEX idx_sender_key_status_group ON sender_key_status (group_jid, device_id);
```

**`2026-02-05-000000_add_props_hash/up.sql`**: `ALTER TABLE device ADD COLUMN props_hash TEXT;`

**`2026-02-12-000000_add_tc_tokens/up.sql`**:
```sql
CREATE TABLE tc_tokens (
    jid TEXT NOT NULL,
    token BLOB NOT NULL,
    token_timestamp INTEGER NOT NULL,
    sender_timestamp INTEGER,
    device_id INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (jid, device_id)
);
CREATE INDEX idx_tc_tokens_timestamp ON tc_tokens (token_timestamp, device_id);
```

**`2026-03-12-000000_add_next_pre_key_id/up.sql`**: `ALTER TABLE device ADD COLUMN next_pre_key_id INTEGER NOT NULL DEFAULT 0;`

**`2026-03-15-000000_add_sent_messages/up.sql`**:
```sql
CREATE TABLE sent_messages (
    chat_jid TEXT NOT NULL,
    message_id TEXT NOT NULL,
    payload BLOB NOT NULL,
    device_id INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (chat_jid, message_id, device_id)
);
CREATE INDEX idx_sent_messages_created ON sent_messages (created_at, device_id);
```

**`2026-03-23-000000_add_nct_salt/up.sql`**: `ALTER TABLE device ADD COLUMN nct_salt BLOB;`

**`2026-03-26-000000_unified_sender_key_devices/up.sql`** (creates `sender_key_devices`, migrates from `skdm_recipients` + `sender_key_status`, then drops both):
```sql
CREATE TABLE sender_key_devices (
    group_jid  TEXT    NOT NULL,
    device_jid TEXT    NOT NULL,
    has_key    INTEGER NOT NULL DEFAULT 0,
    device_id  INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (group_jid, device_jid, device_id)
);
CREATE INDEX idx_sender_key_devices_group ON sender_key_devices (group_jid, device_id);
INSERT OR IGNORE INTO sender_key_devices (group_jid, device_jid, device_id, has_key, updated_at)
    SELECT group_jid, device_jid, device_id, 1, created_at FROM skdm_recipients;
INSERT OR REPLACE INTO sender_key_devices (group_jid, device_jid, device_id, has_key, updated_at)
    SELECT group_jid, participant, device_id, 0, marked_at FROM sender_key_status
    WHERE participant LIKE '%:%';
DROP TABLE skdm_recipients;
DROP TABLE sender_key_status;
```

**`2026-03-31-000000_add_device_registry_raw_id/up.sql`**: `ALTER TABLE device_registry ADD COLUMN raw_id INTEGER;`

**`2026-04-06-171140_add_server_has_prekeys/up.sql`**: `ALTER TABLE device ADD COLUMN server_has_prekeys BOOLEAN NOT NULL DEFAULT 0;`

**`2026-04-26-000000_add_server_cert_chain/up.sql`**: `ALTER TABLE device ADD COLUMN server_cert_chain BLOB NULL;`

(`down.sql` files reverse each: `DROP TABLE ...`, table-rebuild to drop columns on old SQLite, or `ALTER TABLE device DROP COLUMN ...` for the last two. The full `down.sql` text for every migration is captured in the persisted tool output at `/var/home/luka/.claude/projects/-var-home-luka-Trabalho-whatsapp-v1/91475737-b972-41c8-917c-1c382d082b9a/tool-results/bdufg1cyr.txt` — the only load-bearing ones for replication are the `up.sql` shown above; down.sql do not affect the on-disk format.)

### Final table → column → type listing (post-migration, authoritative)

- **device** (PK `id`): `id INTEGER PK AUTOINCREMENT`, `lid TEXT`, `pn TEXT`, `registration_id INTEGER`, `noise_key BLOB(64)`, `identity_key BLOB(64)`, `signed_pre_key BLOB(64)`, `signed_pre_key_id INTEGER`, `signed_pre_key_signature BLOB(64)`, `adv_secret_key BLOB(32)`, `account BLOB NULL` (account_serde), `push_name TEXT DEFAULT ''`, `app_version_primary INTEGER`, `app_version_secondary INTEGER`, `app_version_tertiary BIGINT`, `app_version_last_fetched_ms BIGINT`, `edge_routing_info BLOB NULL`, `props_hash TEXT NULL`, `next_pre_key_id INTEGER DEFAULT 0`, `nct_salt BLOB NULL`, `server_has_prekeys BOOLEAN DEFAULT 0`, `server_cert_chain BLOB NULL` (bincode).
- **identities** (PK `address, device_id`): `address TEXT`, `key BLOB(32)`, `device_id INTEGER`.
- **sessions** (PK `address, device_id`): `address TEXT`, `record BLOB`, `device_id INTEGER`.
- **prekeys** (PK `id, device_id`): `id INTEGER`, `key BLOB`, `uploaded BOOLEAN DEFAULT FALSE`, `device_id INTEGER`.
- **signed_prekeys** (PK `id, device_id`): `id INTEGER`, `record BLOB`, `device_id INTEGER`.
- **sender_keys** (PK `address, device_id`): `address TEXT`, `record BLOB`, `device_id INTEGER`.
- **app_state_keys** (PK `key_id, device_id`): `key_id BLOB`, `key_data BLOB` (bincode AppStateSyncKey), `device_id INTEGER`.
- **app_state_versions** (PK `name, device_id`): `name TEXT`, `state_data BLOB` (bincode HashState), `device_id INTEGER`.
- **app_state_mutation_macs** (PK `name, index_mac, device_id`): `name TEXT`, `version BIGINT`, `index_mac BLOB`, `value_mac BLOB`, `device_id INTEGER`.
- **base_keys** (PK `address, message_id, device_id`): `address TEXT`, `message_id TEXT`, `base_key BLOB`, `device_id INTEGER`, `created_at INTEGER DEFAULT strftime('%s','now')`.
- **device_registry** (PK `user_id, device_id`): `user_id TEXT`, `devices_json TEXT` (serde_json `Vec<DeviceInfo>`), `timestamp INTEGER`, `phash TEXT NULL`, `device_id INTEGER`, `updated_at INTEGER DEFAULT strftime('%s','now')`, `raw_id INTEGER NULL`.
- **lid_pn_mapping** (PK `lid, device_id`, FK device_id→device.id ON DELETE CASCADE): `lid TEXT`, `phone_number TEXT`, `created_at BIGINT`, `learning_source TEXT`, `updated_at BIGINT`, `device_id INTEGER`.
- **sender_key_devices** (PK `group_jid, device_jid, device_id`): `group_jid TEXT`, `device_jid TEXT`, `has_key INTEGER DEFAULT 0`, `device_id INTEGER`, `updated_at INTEGER/BIGINT DEFAULT strftime('%s','now')`.
- **tc_tokens** (PK `jid, device_id`): `jid TEXT`, `token BLOB`, `token_timestamp INTEGER/BIGINT`, `sender_timestamp INTEGER/BIGINT NULL`, `device_id INTEGER`, `updated_at INTEGER/BIGINT DEFAULT strftime('%s','now')`.
- **sent_messages** (PK `chat_jid, message_id, device_id`): `chat_jid TEXT`, `message_id TEXT`, `payload BLOB`, `device_id INTEGER`, `created_at INTEGER/BIGINT DEFAULT strftime('%s','now')`.

Indexes present after all migrations: `idx_identities_device_id`, `idx_sessions_device_id`, `idx_prekeys_device_id`, `idx_sender_keys_device_id`, `idx_signed_prekeys_device_id`, `idx_app_state_keys_device_id`, `idx_app_state_versions_device_id`, `idx_app_state_mutation_macs_device_id`, `idx_base_keys_device`, `idx_device_registry_timestamp`, `idx_device_registry_device`, `idx_device_registry_updated_at`, `idx_lid_pn_mapping_phone`, `idx_sender_key_devices_group`, `idx_tc_tokens_timestamp`, `idx_sent_messages_created`.

---

## 4. Replication notes for Postgres (the critical wire-format facts)

1. **bincode 2.0 with `bincode::config::standard()`** is the only structured binary codec, used for exactly three values: `device.server_cert_chain` (`CachedServerCertChain`), `app_state_keys.key_data` (`AppStateSyncKey`), `app_state_versions.state_data` (`HashState`). To preserve compatibility, encode/decode these in Postgres with the identical bincode 2 standard config (or pass through the same opaque bytes). Standard config = little-endian, varint integer encoding, no fixed-array length prefixing differences vs legacy.
2. **serde_json** is used for exactly one value: `device_registry.devices_json` (a JSON array of `DeviceInfo { device_id, key_index }`). Stored as TEXT.
3. **Raw byte concatenation** for KeyPairs: `noise_key`/`identity_key`/`signed_pre_key` are each exactly 64 bytes = priv(32) ‖ DJB-pub(32). `signed_pre_key_signature` = 64 raw bytes, `adv_secret_key` = 32 raw bytes, `identities.key` = 32 raw bytes.
4. **No transform** (store the bytes as-is) for: session `record`, sender_key `record`, prekey `key`, signed_prekey `record`, `base_keys.base_key`, `app_state_mutation_macs.index_mac/value_mac`, `app_state_keys.key_id`, `tc_tokens.token`, `sent_messages.payload`, `device.account` (via account_serde, opaque), `device.edge_routing_info`, `device.nct_salt`.
5. **device_id scoping**: every PK except `device` includes `device_id`; in Postgres mirror this exactly. The `device` table PK is `id`; `save`/`create` write `id = self.device_id`.
6. **Conflict/upsert semantics**: SQLite `ON CONFLICT (...) DO UPDATE SET ...` maps directly to Postgres `INSERT ... ON CONFLICT (...) DO UPDATE SET ...`. `store_sent_message` uses `REPLACE INTO` (delete-then-insert) → in Postgres use `ON CONFLICT (chat_jid, message_id, device_id) DO UPDATE SET payload = EXCLUDED.payload` (note: SQLite REPLACE resets `created_at` to its DEFAULT; to match, also set `created_at = now()` on conflict).
7. **`get_latest_sync_key_id`** orders by `key_id DESC` on a BLOB — Postgres `BYTEA` ordering is byte-lexicographic and matches SQLite BLOB ordering, so this is safe.
8. **Booleans**: `prekeys.uploaded`, `device.server_has_prekeys` are SQLite BOOLEAN (0/1); `sender_key_devices.has_key` is INTEGER read as `!= 0`. In Postgres use BOOLEAN for the first two and SMALLINT/INTEGER for `has_key`.
9. Timestamps written by app code use `wacore::time::now_secs()` (Unix seconds); columns with `DEFAULT (strftime('%s','now'))` are populated by SQLite when the app omits them (notably `sent_messages.created_at`, since `store_sent_message` does not set it). Replicate that default in Postgres (`DEFAULT extract(epoch from now())::bigint`).