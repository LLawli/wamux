//! The `IN` list text the batched reads are built from (#104).

use super::{READ_CHUNK, in_placeholders};

#[test]
fn placeholders_start_at_the_first_index_and_count_up() {
    assert_eq!(in_placeholders(2, 3), "$2, $3, $4");
    assert_eq!(in_placeholders(1, 1), "$1");
}

#[test]
fn a_full_chunk_has_read_chunk_placeholders() {
    let text = in_placeholders(3, READ_CHUNK);
    let parts: Vec<&str> = text.split(", ").collect();
    assert_eq!(parts.len(), READ_CHUNK);
    assert_eq!(parts.first(), Some(&"$3"));
    assert_eq!(
        parts.last().map(|p| p.to_string()),
        Some(format!("${}", READ_CHUNK + 2))
    );
}
