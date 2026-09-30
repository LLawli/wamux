-- wamux initial schema, SQLite dialect.
--
-- The Postgres port (migrations/0001_initial.sql + 0002) is the sibling of this
-- file; the two must stay structurally identical, because a store dumped from
-- one engine has to load in the other. Differences here are dialect-only:
--
--   BYTEA                  -> BLOB
--   BIGINT / SMALLINT      -> INTEGER (SQLite has one integer type)
--   BOOLEAN                -> INTEGER (0/1)
--   UUID                   -> TEXT (hyphenated, so `sqlite3` output is greppable)
--   IDENTITY               -> INTEGER PRIMARY KEY AUTOINCREMENT
--   extract(epoch FROM now()) -> unixepoch()
--
-- `connection_policy` is absent by design: it existed in the Postgres 0001 and
-- was dropped by 0002 (connect is edge-driven), so this schema never grows it.
--
-- Wire format note: blob columns hold the EXACT bytes the reference produces
-- (raw keys, bincode-standard for app-state/cert-chain/device, serde_json for
-- device_registry.devices_json). See docs/crate-notes/sqlite-reference.md.

-- device_id is the PK here (Postgres keeps `uuid` as PK and device_id as an
-- IDENTITY column) because SQLite only autoincrements the INTEGER PRIMARY KEY.
-- AUTOINCREMENT, not the implicit rowid: it guarantees a deleted account's
-- device_id is never handed to a new account, so a stale row that somehow
-- outlived the cascade can never be read as another account's Signal state.
CREATE TABLE accounts (
    device_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    uuid         TEXT NOT NULL UNIQUE,
    external_ref TEXT UNIQUE,
    push_name    TEXT,
    created_at   INTEGER NOT NULL DEFAULT (unixepoch())
);

-- The whole wacore Device serialized as one bincode-standard blob (it already
-- (de)serializes via key_pair_serde/account_serde/BigArray). PK = device_id.
CREATE TABLE device (
    device_id  INTEGER PRIMARY KEY REFERENCES accounts(device_id) ON DELETE CASCADE,
    data       BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE identities (
    address   TEXT NOT NULL,
    key       BLOB NOT NULL,             -- raw 32 bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (address, device_id)
);

CREATE TABLE sessions (
    address   TEXT NOT NULL,
    record    BLOB NOT NULL,             -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (address, device_id)
);

CREATE TABLE prekeys (
    id        INTEGER NOT NULL,
    key       BLOB NOT NULL,             -- raw bytes
    uploaded  INTEGER NOT NULL DEFAULT 0,
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (id, device_id)
);

CREATE TABLE signed_prekeys (
    id        INTEGER NOT NULL,
    record    BLOB NOT NULL,             -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (id, device_id)
);

CREATE TABLE sender_keys (
    address   TEXT NOT NULL,
    record    BLOB NOT NULL,             -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (address, device_id)
);

CREATE TABLE app_state_keys (
    key_id    BLOB NOT NULL,             -- raw bytes
    key_data  BLOB NOT NULL,             -- bincode-standard AppStateSyncKey
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (key_id, device_id)
);

CREATE TABLE app_state_versions (
    name       TEXT NOT NULL,
    state_data BLOB NOT NULL,            -- bincode-standard HashState
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (name, device_id)
);

CREATE TABLE app_state_mutation_macs (
    name      TEXT NOT NULL,
    version   INTEGER NOT NULL,
    index_mac BLOB NOT NULL,             -- raw bytes
    value_mac BLOB NOT NULL,             -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (name, index_mac, device_id)
);

CREATE TABLE base_keys (
    address    TEXT NOT NULL,
    message_id TEXT NOT NULL,
    base_key   BLOB NOT NULL,            -- raw bytes
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (address, message_id, device_id)
);

CREATE TABLE device_registry (
    user_id      TEXT NOT NULL,
    devices_json TEXT NOT NULL,          -- serde_json Vec<DeviceInfo>
    timestamp    INTEGER NOT NULL,
    phash        TEXT,
    device_id    INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    updated_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    raw_id       INTEGER,
    PRIMARY KEY (user_id, device_id)
);

CREATE TABLE lid_pn_mapping (
    lid             TEXT NOT NULL,
    phone_number    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    learning_source TEXT NOT NULL,
    updated_at      INTEGER NOT NULL,
    device_id       INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (lid, device_id)
);

CREATE TABLE sender_key_devices (
    group_jid  TEXT NOT NULL,
    device_jid TEXT NOT NULL,
    has_key    INTEGER NOT NULL DEFAULT 0,
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (group_jid, device_jid, device_id)
);

CREATE TABLE tc_tokens (
    jid              TEXT NOT NULL,
    token            BLOB NOT NULL,      -- raw bytes
    token_timestamp  INTEGER NOT NULL,
    sender_timestamp INTEGER,
    device_id        INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    updated_at       INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (jid, device_id)
);

CREATE TABLE sent_messages (
    chat_jid   TEXT NOT NULL,
    message_id TEXT NOT NULL,
    payload    BLOB NOT NULL,            -- raw bytes (protobuf Message)
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (chat_jid, message_id, device_id)
);

CREATE INDEX idx_lid_pn_mapping_phone ON lid_pn_mapping (phone_number, device_id);
CREATE INDEX idx_sender_key_devices_group ON sender_key_devices (group_jid, device_id);
CREATE INDEX idx_tc_tokens_timestamp ON tc_tokens (token_timestamp, device_id);
CREATE INDEX idx_sent_messages_created ON sent_messages (created_at, device_id);
CREATE INDEX idx_device_registry_updated_at ON device_registry (updated_at, device_id);
