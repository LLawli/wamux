//! #79: the spooled ciphertext as an `UploadSource`. The library opens one
//! reader per attempt, at the offset a resume names; readers must not share a
//! position, or a retry would read from wherever the last one stopped.

use std::io::{Read, Write};

use wacore::upload::UploadSource;

use super::SpooledCiphertext;

/// A spool-like file (unlinked) holding `0, 1, 2, ...` mod 256.
fn spooled(len: usize) -> (SpooledCiphertext, Vec<u8>) {
    let bytes: Vec<u8> = (0..len).map(|i| (i % 256) as u8).collect();
    let mut file = tempfile::tempfile().expect("tempfile");
    file.write_all(&bytes).expect("write");
    (SpooledCiphertext::from_file(file, len as u64), bytes)
}

fn read_n(reader: &mut dyn Read, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    reader.read_exact(&mut out).expect("read");
    out
}

#[test]
fn reader_from_an_offset_reads_the_tail() {
    let (source, bytes) = spooled(1000);
    let mut tail = Vec::new();
    source
        .reader_from(600)
        .expect("reader")
        .read_to_end(&mut tail)
        .expect("read");
    assert_eq!(tail, bytes[600..]);
    let mut whole = Vec::new();
    source
        .reader_from(0)
        .expect("reader")
        .read_to_end(&mut whole)
        .expect("read");
    assert_eq!(whole, bytes, "a second attempt starts over");
}

#[test]
fn two_readers_do_not_share_a_position() {
    let (source, bytes) = spooled(1000);
    let mut first = source.reader_from(0).expect("first");
    assert_eq!(read_n(&mut *first, 10), bytes[0..10]);
    let mut second = source.reader_from(500).expect("second");
    assert_eq!(read_n(&mut *second, 10), bytes[500..510]);
    assert_eq!(
        read_n(&mut *first, 10),
        bytes[10..20],
        "the first reader kept its own place"
    );
}

#[test]
fn len_is_the_ciphertext_length() {
    let (source, _) = spooled(4096);
    assert_eq!(source.len(), 4096);
    assert!(!source.is_empty());
}
