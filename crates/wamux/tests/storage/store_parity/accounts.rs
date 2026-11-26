//! `StorageEngine::create_account` with a taken `external_ref` (#269): every
//! engine reports `ExternalRefTaken`, not a generic database error, so the
//! service can answer `ALREADY_EXISTS` and the edge can adopt the account.

use wamux::storage::CreateAccountError;

use crate::common;
use crate::store_parity::harness::{self, Harness};

/// Unique to this file: `sweep_orphans` deletes every account under it.
const PREFIX: &str = "parity-taken/";

/// The second create of one ref is `ExternalRefTaken(ref)`; the failed insert
/// leaves the store usable (on SQLite and Turso it was a job of a group-commit
/// batch) and the first row as the only one holding the ref.
async fn taken_external_ref_is_reported(h: Harness) {
    common::sweep_orphans(&h.storage, PREFIX).await;
    let taken = format!("{PREFIX}x");
    let first = h
        .storage
        .create_account(Some(&taken))
        .await
        .expect("first create");

    let again = h.storage.create_account(Some(&taken)).await;
    match again {
        Err(CreateAccountError::ExternalRefTaken(reported)) => assert_eq!(reported, taken),
        other => panic!("a taken external_ref must be ExternalRefTaken, got {other:?}"),
    }

    let other = h
        .storage
        .create_account(Some(&format!("{PREFIX}y")))
        .await
        .expect("a create after the failed one must still succeed");
    assert_ne!(other.device_id, first.device_id);

    let holders: Vec<_> = h
        .storage
        .list_accounts()
        .await
        .expect("list")
        .into_iter()
        .filter(|row| row.external_ref.as_deref() == Some(taken.as_str()))
        .collect();
    assert_eq!(holders.len(), 1, "exactly one row holds {taken}");
    assert_eq!(holders[0].uuid, first.uuid);

    common::sweep_orphans(&h.storage, PREFIX).await;
}

#[tokio::test]
async fn postgres_taken_external_ref_is_reported() {
    taken_external_ref_is_reported(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_taken_external_ref_is_reported() {
    taken_external_ref_is_reported(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_taken_external_ref_is_reported() {
    taken_external_ref_is_reported(harness::turso().await).await;
}
