//! #84: the marker's two values, and nothing else, map to a format.

use super::{StoreBlobFormat, parse_blob_format};
use crate::storage::bincode_upgrade::{
    BLOB_FORMAT_BINCODE, BLOB_FORMAT_PROTOBUF, BincodeUpgradeError,
};

#[test]
fn parse_blob_format_maps_the_two_markers() {
    assert_eq!(
        parse_blob_format(BLOB_FORMAT_BINCODE).unwrap(),
        StoreBlobFormat::Bincode
    );
    assert_eq!(
        parse_blob_format(BLOB_FORMAT_PROTOBUF).unwrap(),
        StoreBlobFormat::Protobuf
    );
}

#[test]
fn parse_blob_format_refuses_an_unknown_marker() {
    let err = parse_blob_format("json").unwrap_err();
    assert!(
        matches!(&err, BincodeUpgradeError::UnknownFormat(marker) if marker == "json"),
        "{err:?}"
    );
}
