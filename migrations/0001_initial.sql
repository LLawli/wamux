-- wamux initial schema.
--
-- Mirrors the whatsapp-rust-sqlite-storage reference layout (15 store tables),
-- ported to Postgres, plus a wamux-specific `accounts` table that maps the
-- canonical UUID / external_ref to the integer device_id used to scope every
-- other table. One shared database; every store row carries device_id.
--
-- Wire format note: blob columns hold the EXACT bytes the reference produces
-- (raw keys, bincode-standard for app-state/cert-chain/device, serde_json for
-- device_registry.devices_json). See docs/crate-notes/sqlite-reference.md.

CREATE TABLE accounts (
    uuid              UUID PRIMARY KEY,
    external_ref      TEXT UNIQUE,
    device_id         INTEGER GENERATED ALWAYS AS IDENTITY UNIQUE,
    connection_policy SMALLINT NOT NULL DEFAULT 1, -- 1=on_demand, 2=always_on
    push_name         TEXT,
    created_at        BIGINT NOT NULL DEFAULT extract(epoch FROM now())::bigint
);

-- The whole wacore Device serialized as one bincode-standard blob (it already
-- (de)serializes via key_pair_serde/account_serde/BigArray). PK = device_id.
CREATE TABLE device (
    device_id  INTEGER PRIMARY KEY REFERENCES accounts(device_id) ON DELETE CASCADE,
    data       BYTEA NOT NULL,
    created_at BIGINT NOT NULL DEFAULT extract(epoch FROM now())::bigint
);

CREATE TABLE identities (
    address   TEXT NOT NULL,
    key       BYTEA NOT NULL,            -- raw 32 bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (address, device_id)
);

CREATE TABLE sessions (
    address   TEXT NOT NULL,
    record    BYTEA NOT NULL,            -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (address, device_id)
);

CREATE TABLE prekeys (
    id        INTEGER NOT NULL,
    key       BYTEA NOT NULL,            -- raw bytes
    uploaded  BOOLEAN NOT NULL DEFAULT FALSE,
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (id, device_id)
);

CREATE TABLE signed_prekeys (
    id        INTEGER NOT NULL,
    record    BYTEA NOT NULL,            -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (id, device_id)
);

CREATE TABLE sender_keys (
    address   TEXT NOT NULL,
    record    BYTEA NOT NULL,            -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (address, device_id)
);

CREATE TABLE app_state_keys (
    key_id    BYTEA NOT NULL,            -- raw bytes
    key_data  BYTEA NOT NULL,            -- bincode-standard AppStateSyncKey
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (key_id, device_id)
);

CREATE TABLE app_state_versions (
    name       TEXT NOT NULL,
    state_data BYTEA NOT NULL,           -- bincode-standard HashState
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (name, device_id)
);

CREATE TABLE app_state_mutation_macs (
    name      TEXT NOT NULL,
    version   BIGINT NOT NULL,
    index_mac BYTEA NOT NULL,            -- raw bytes
    value_mac BYTEA NOT NULL,            -- raw bytes
    device_id INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (name, index_mac, device_id)
);

CREATE TABLE base_keys (
    address    TEXT NOT NULL,
    message_id TEXT NOT NULL,
    base_key   BYTEA NOT NULL,           -- raw bytes
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL DEFAULT extract(epoch FROM now())::bigint,
    PRIMARY KEY (address, message_id, device_id)
);

CREATE TABLE device_registry (
    user_id      TEXT NOT NULL,
    devices_json TEXT NOT NULL,          -- serde_json Vec<DeviceInfo>
    timestamp    BIGINT NOT NULL,
    phash        TEXT,
    device_id    INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    updated_at   BIGINT NOT NULL DEFAULT extract(epoch FROM now())::bigint,
    raw_id       INTEGER,
    PRIMARY KEY (user_id, device_id)
);

CREATE TABLE lid_pn_mapping (
    lid             TEXT NOT NULL,
    phone_number    TEXT NOT NULL,
    created_at      BIGINT NOT NULL,
    learning_source TEXT NOT NULL,
    updated_at      BIGINT NOT NULL,
    device_id       INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (lid, device_id)
);

CREATE TABLE sender_key_devices (
    group_jid  TEXT NOT NULL,
    device_jid TEXT NOT NULL,
    has_key    INTEGER NOT NULL DEFAULT 0,
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    updated_at BIGINT NOT NULL DEFAULT extract(epoch FROM now())::bigint,
    PRIMARY KEY (group_jid, device_jid, device_id)
);

CREATE TABLE tc_tokens (
    jid              TEXT NOT NULL,
    token            BYTEA NOT NULL,      -- raw bytes
    token_timestamp  BIGINT NOT NULL,
    sender_timestamp BIGINT,
    device_id        INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    updated_at       BIGINT NOT NULL DEFAULT extract(epoch FROM now())::bigint,
    PRIMARY KEY (jid, device_id)
);

CREATE TABLE sent_messages (
    chat_jid   TEXT NOT NULL,
    message_id TEXT NOT NULL,
    payload    BYTEA NOT NULL,           -- raw bytes (protobuf Message)
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL DEFAULT extract(epoch FROM now())::bigint,
    PRIMARY KEY (chat_jid, message_id, device_id)
);

CREATE INDEX idx_lid_pn_mapping_phone ON lid_pn_mapping (phone_number, device_id);
CREATE INDEX idx_sender_key_devices_group ON sender_key_devices (group_jid, device_id);
CREATE INDEX idx_tc_tokens_timestamp ON tc_tokens (token_timestamp, device_id);
CREATE INDEX idx_sent_messages_created ON sent_messages (created_at, device_id);
CREATE INDEX idx_device_registry_updated_at ON device_registry (updated_at, device_id);
