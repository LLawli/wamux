//! `store_decode_audit`: what decodes, what fails, and how the tally counts.

use wacore::libsignal::protocol::SessionRecord;
use wacore::store::Device;
use wamux::storage::blob_codec::encode_device;
use wamux_tools::store_decode_audit::{BlobTable, TableTally, decode_blob};

#[test]
fn blobs_the_library_wrote_decode() {
    let session = SessionRecord::new_fresh().serialize().unwrap();
    assert_eq!(decode_blob(BlobTable::Sessions, &session), Ok(()));
    assert_eq!(
        decode_blob(BlobTable::Device, &encode_device(&Device::new())),
        Ok(())
    );
    assert_eq!(decode_blob(BlobTable::Identities, &[7; 32]), Ok(()));
}

#[test]
fn garbage_fails_in_every_table() {
    let garbage = [0xFF_u8; 9];
    for table in BlobTable::ALL {
        assert!(
            decode_blob(table, &garbage).is_err(),
            "{table:?} accepted garbage"
        );
    }
}

#[test]
fn an_identity_of_the_wrong_length_names_its_length() {
    assert_eq!(
        decode_blob(BlobTable::Identities, &[7; 33]),
        Err("identity key is 33 bytes, expected 32".to_string())
    );
}

#[test]
fn the_tally_counts_each_distinct_error() {
    let mut tally = TableTally::default();
    tally.record(Ok(()));
    tally.record(Err("a".into()));
    tally.record(Err("a".into()));
    tally.record(Err("b".into()));
    assert_eq!(tally.decoded, 1);
    assert_eq!(tally.failed(), 3);
    assert_eq!(tally.failures.get("a"), Some(&2));
}
