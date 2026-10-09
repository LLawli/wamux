-- Whether the secret columns hold sealed blobs (#164, part 1 of #76).
--
-- 'plaintext' is how every existing store starts, and a store opened with no
-- key stays that way. 'encrypted' means the 13 secret columns hold
-- XChaCha20-Poly1305 blobs (storage::blob_cipher). The daemon flips it, once,
-- when a brand-new store (no accounts) is opened with a key; converting a store
-- that already has accounts is #165.
--
-- key_id is the first 4 bytes of SHA-256 of the key. verifier is a known
-- constant sealed under the key: opening it is how a wrong key is caught when
-- the store opens, before the first account is read.
--
-- One row, pinned by the CHECK: the state is a property of the whole store.

CREATE TABLE store_encryption (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    state    TEXT NOT NULL CHECK (state IN ('plaintext', 'encrypted')),
    key_id   BYTEA,
    verifier BYTEA
);

INSERT INTO store_encryption (id, state) VALUES (1, 'plaintext');
