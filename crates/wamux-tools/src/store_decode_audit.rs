//! Decode every stored Signal and device blob with the pinned library, the way
//! a restart reads them. Run against a copy of a real store before a
//! whatsapp-rust bump ships: the library's decoders change between pins (a
//! stricter one rejects a record the old one kept, and that session is
//! dropped), and no test holds production records.
//!
//! Counts only. A blob is key material and an address is a phone number, so
//! neither is ever printed; a failure is reported by table and error text.

use std::collections::BTreeMap;

use wacore::libsignal::protocol::{
    PreKeyRecord, SenderKeyRecord, SessionRecord, SignedPreKeyRecord,
};
use wamux::storage::blob_codec::decode_device;

/// One store table holding blobs, and the statement that reads them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlobTable {
    Device,
    Sessions,
    SenderKeys,
    Prekeys,
    SignedPrekeys,
    Identities,
}

impl BlobTable {
    pub const ALL: [BlobTable; 6] = [
        BlobTable::Device,
        BlobTable::Sessions,
        BlobTable::SenderKeys,
        BlobTable::Prekeys,
        BlobTable::SignedPrekeys,
        BlobTable::Identities,
    ];

    pub fn select_all(self) -> &'static str {
        match self {
            BlobTable::Device => "SELECT data FROM device",
            BlobTable::Sessions => "SELECT record FROM sessions",
            BlobTable::SenderKeys => "SELECT record FROM sender_keys",
            BlobTable::Prekeys => "SELECT key FROM prekeys",
            BlobTable::SignedPrekeys => "SELECT record FROM signed_prekeys",
            BlobTable::Identities => "SELECT key FROM identities",
        }
    }
}

/// Decode one blob as the library's load path does. Sessions and sender keys
/// go through `deserialize_for_store` with a fresh incarnation, which is what
/// a new process presents: a reservation written by the old one is stale.
pub fn decode_blob(table: BlobTable, bytes: &[u8]) -> Result<(), String> {
    const FRESH_INCARNATION: [u8; 16] = [0xA5; 16];
    match table {
        BlobTable::Device => decode_device(bytes).map(drop).map_err(|e| e.to_string()),
        BlobTable::Sessions => SessionRecord::deserialize_for_store(bytes, &FRESH_INCARNATION)
            .map(drop)
            .map_err(|e| e.to_string()),
        BlobTable::SenderKeys => SenderKeyRecord::deserialize_for_store(bytes, &FRESH_INCARNATION)
            .map(drop)
            .map_err(|e| e.to_string()),
        BlobTable::Prekeys => PreKeyRecord::deserialize(bytes)
            .map(drop)
            .map_err(|e| e.to_string()),
        BlobTable::SignedPrekeys => SignedPreKeyRecord::deserialize(bytes)
            .map(drop)
            .map_err(|e| e.to_string()),
        BlobTable::Identities => identity_key(bytes),
    }
}

/// `put_identity` stores the raw 32-byte key, nothing else.
fn identity_key(bytes: &[u8]) -> Result<(), String> {
    match bytes.len() {
        32 => Ok(()),
        len => Err(format!("identity key is {len} bytes, expected 32")),
    }
}

/// Per table: how many blobs decoded, and each distinct error with its count.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TableTally {
    pub decoded: usize,
    pub failures: BTreeMap<String, usize>,
}

impl TableTally {
    pub fn record(&mut self, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => self.decoded += 1,
            Err(error) => *self.failures.entry(error).or_default() += 1,
        }
    }

    pub fn failed(&self) -> usize {
        self.failures.values().sum()
    }
}
