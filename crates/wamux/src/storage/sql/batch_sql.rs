//! Batched reads with a fixed-size `IN` list (#104).
//!
//! One statement text per method: every chunk binds exactly `READ_CHUNK`
//! values, and a short last chunk is padded by repeating one of its values
//! (a repeat inside `IN` changes nothing), so the driver's statement cache
//! holds one entry, not one per batch size.

use std::collections::HashSet;
use std::hash::Hash;

/// Values bound per batched read: one `IN` list of this many placeholders.
pub(super) const READ_CHUNK: usize = 100;

/// `$first, $first+1, ..., $first+count-1`, comma-separated.
pub(super) fn in_placeholders(first: usize, count: usize) -> String {
    let numbered: Vec<String> = (first..first + count).map(|n| format!("${n}")).collect();
    numbered.join(", ")
}

/// Split `items` into chunks of exactly `READ_CHUNK` values, the last one padded
/// by repeating its own last value. Duplicates are dropped first, so a value
/// asked for twice cannot come back twice from two different chunks; the
/// caller maps results back to the request itself when it needs positions.
pub(super) fn padded_chunks<T: Clone + Eq + Hash>(items: &[T]) -> Vec<Vec<T>> {
    let mut seen: HashSet<T> = HashSet::new();
    let unique: Vec<T> = items
        .iter()
        .filter(|item| seen.insert((*item).clone()))
        .cloned()
        .collect();
    unique.chunks(READ_CHUNK).map(pad_to_read_chunk).collect()
}

fn pad_to_read_chunk<T: Clone>(chunk: &[T]) -> Vec<T> {
    let mut padded: Vec<T> = chunk.to_vec();
    if let Some(last) = chunk.last() {
        padded.resize(READ_CHUNK, last.clone());
    }
    padded
}

#[cfg(test)]
mod padding_tests {
    use super::*;

    #[test]
    fn every_chunk_is_full_and_duplicates_are_dropped() {
        let items: Vec<u32> = (0..250).chain(0..10).collect();
        let chunks = padded_chunks(&items);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.len() == READ_CHUNK));
        assert_eq!(chunks[2][49], 249);
        assert_eq!(chunks[2][99], 249);
    }

    #[test]
    fn an_empty_request_has_no_chunks() {
        assert!(padded_chunks::<u32>(&[]).is_empty());
    }
}

#[cfg(test)]
#[path = "batch_sql_tests.rs"]
mod tests;
