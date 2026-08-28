-- `MessageContextInfo.messageSecret`, keyed by the outbound message it rode on.
-- Needed to decrypt poll votes, edits, and msmsg bot replies: the receive path
-- looks the secret up by (chat, target_sender, target_id), where target_id is
-- our own original message id.
--
-- Still relay-pure: this is protocol key material, the same class as sessions
-- and prekeys, not business message content. Only the 32-byte secret is stored,
-- never the message.
--
-- `MsgSecretStore` is new in whatsapp-rust 0.7 and is a required supertrait of
-- `Backend`, so this table has no 0.6 counterpart and starts empty.

CREATE TABLE msg_secrets (
    chat       TEXT NOT NULL,
    sender     TEXT NOT NULL,
    msg_id     TEXT NOT NULL,
    secret     BYTEA NOT NULL,            -- raw 32 bytes
    -- Absolute unix seconds; 0 means "never expire" and is kept by the pruner.
    expires_at BIGINT NOT NULL DEFAULT 0,
    -- Parent message event time, 0 when unknown. Backs the edit-processing window.
    message_ts BIGINT NOT NULL DEFAULT 0,
    device_id  INTEGER NOT NULL REFERENCES accounts(device_id) ON DELETE CASCADE,
    PRIMARY KEY (chat, sender, msg_id, device_id)
);

CREATE INDEX idx_msg_secrets_expires ON msg_secrets (device_id, expires_at);
