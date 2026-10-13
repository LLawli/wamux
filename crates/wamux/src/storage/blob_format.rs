//! What the `blob_format` marker says (#31), as a type the health probe can
//! report (#84). Outlives `bincode_upgrade`: 0.3.0 deletes the conversion but
//! still reads the marker, to refuse a store that is still bincode.

use wacore::store::error::{Result as StoreResult, StoreError};

use super::bincode_upgrade::{BLOB_FORMAT_BINCODE, BLOB_FORMAT_PROTOBUF, BincodeUpgradeError};

/// The format the store's structured blobs are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreBlobFormat {
    Bincode,
    Protobuf,
}

/// The marker's text as a format. A marker this build does not know (written by
/// a newer build) is an error, never a guess.
pub fn parse_blob_format(marker: &str) -> Result<StoreBlobFormat, BincodeUpgradeError> {
    match marker {
        BLOB_FORMAT_BINCODE => Ok(StoreBlobFormat::Bincode),
        BLOB_FORMAT_PROTOBUF => Ok(StoreBlobFormat::Protobuf),
        other => Err(BincodeUpgradeError::UnknownFormat(other.to_string())),
    }
}

/// The marker row as the engines' `blob_format` answers it. Shared so both
/// families map a missing row and an unknown marker the same way: a
/// `StoreError::Database` whose source names the offending value.
pub fn store_blob_format(marker: Option<String>) -> StoreResult<StoreBlobFormat> {
    let marker = marker.ok_or_else(|| {
        StoreError::Database(Box::new(BincodeUpgradeError::UnknownFormat(
            "<no blob_format row>".to_string(),
        )))
    })?;
    parse_blob_format(&marker).map_err(|e| StoreError::Database(Box::new(e)))
}

#[cfg(test)]
#[path = "blob_format_tests.rs"]
mod tests;
