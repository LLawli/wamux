//! The 13 columns that hold sealed blobs, and how each row is named in the
//! associated data (#165). One list, read by the conversion (and the decrypt
//! command) so none of them re-types a table or a column: the stores seal and
//! open row by row with `seal_at`/`open_at`, the conversion walks every row of
//! these columns, and both must name a row the same way or the converted blob
//! does not open.
//!
//! The row key of a table is the bytes of its key columns, in this order, joined
//! by a zero byte when there are several (the same rule as
//! `blob_cipher::joined_row`). A text column contributes its bytes, an INTEGER
//! id its `u32` big-endian bytes, a BLOB column its bytes. The device row has no
//! key column, so its row key is empty.

/// How one key column of a row contributes to the row key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyColumn {
    /// A TEXT column: its UTF-8 bytes.
    Text(&'static str),
    /// An INTEGER id column: the value as `u32`, big-endian (4 bytes).
    Id(&'static str),
    /// A BLOB column: its bytes.
    Bytes(&'static str),
}

/// One value read from a key column, in the order of `SealedColumn::key`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowPart {
    Text(String),
    Int(i64),
    Bytes(Vec<u8>),
}

/// A secret column: where it lives and which columns name its row (besides
/// `device_id`, which the associated data carries separately).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SealedColumn {
    pub table: &'static str,
    pub column: &'static str,
    pub key: &'static [KeyColumn],
}

const fn sealed(
    table: &'static str,
    column: &'static str,
    key: &'static [KeyColumn],
) -> SealedColumn {
    SealedColumn { table, column, key }
}

/// The 13 sealed columns, in a fixed order. The key columns are the ones the
/// stores pass to `seal_at`/`open_at` as the row key, in the same order.
static SEALED_COLUMNS: [SealedColumn; 13] = [
    sealed("device", "data", &[]),
    sealed("identities", "key", &[KeyColumn::Text("address")]),
    sealed("sessions", "record", &[KeyColumn::Text("address")]),
    sealed("prekeys", "key", &[KeyColumn::Id("id")]),
    sealed("signed_prekeys", "record", &[KeyColumn::Id("id")]),
    sealed("sender_keys", "record", &[KeyColumn::Text("address")]),
    sealed("app_state_keys", "key_data", &[KeyColumn::Bytes("key_id")]),
    sealed(
        "app_state_versions",
        "state_data",
        &[KeyColumn::Text("name")],
    ),
    sealed(
        "app_state_mutation_macs",
        "value_mac",
        &[KeyColumn::Text("name"), KeyColumn::Bytes("index_mac")],
    ),
    sealed(
        "base_keys",
        "base_key",
        &[KeyColumn::Text("address"), KeyColumn::Text("message_id")],
    ),
    sealed("tc_tokens", "token", &[KeyColumn::Text("jid")]),
    sealed(
        "msg_secrets",
        "secret",
        &[
            KeyColumn::Text("chat"),
            KeyColumn::Text("sender"),
            KeyColumn::Text("msg_id"),
        ],
    ),
    sealed(
        "sent_messages",
        "payload",
        &[KeyColumn::Text("chat_jid"), KeyColumn::Text("message_id")],
    ),
];

/// The 13 sealed columns, in a fixed order.
pub fn sealed_columns() -> &'static [SealedColumn] {
    &SEALED_COLUMNS
}

/// The row key of one row of `column`, from the values of its key columns.
/// Each part contributes by its own shape; the column only says which parts to
/// read. An id is cut to `u32` because that is what the stores seal with
/// (`id.to_be_bytes()` on a `u32`), and the schema keeps ids in that range.
pub fn row_key(column: &SealedColumn, parts: &[RowPart]) -> Vec<u8> {
    debug_assert_eq!(column.key.len(), parts.len(), "{}", column.table);
    let pieces: Vec<Vec<u8>> = parts.iter().map(part_bytes).collect();
    pieces.join(&0u8)
}

fn part_bytes(part: &RowPart) -> Vec<u8> {
    match part {
        RowPart::Text(text) => text.as_bytes().to_vec(),
        RowPart::Int(id) => (*id as u32).to_be_bytes().to_vec(),
        RowPart::Bytes(bytes) => bytes.clone(),
    }
}

#[cfg(test)]
#[path = "sealed_columns_tests.rs"]
mod tests;
