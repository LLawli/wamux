-- Which byte format the three structured BLOB columns hold (#31):
-- device.data, app_state_versions.state_data, app_state_keys.key_data.
--
-- They were bincode-standard, which is positional: every field whatsapp-rust
-- appended to Device or HashState made every stored blob unreadable. They are
-- now protobuf (proto/store/blobs.proto). SQL cannot run that conversion, so
-- this migration only records where the store stands: every store that reaches
-- it holds bincode, including a brand-new one, whose zero rows convert
-- trivially. The daemon then converts on open (storage::bincode_upgrade) and
-- flips the marker in the same transaction as the rows.
--
-- One row, pinned by the CHECK: the marker is a property of the whole store.

CREATE TABLE blob_format (
    id     INTEGER PRIMARY KEY CHECK (id = 1),
    format TEXT NOT NULL
);

INSERT INTO blob_format (id, format) VALUES (1, 'bincode');
