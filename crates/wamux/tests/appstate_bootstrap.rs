//! Issue #36: the store no longer needs `bootstrapped_repair`, because the
//! library now marks a collection that is already at the server's head
//! (upstream #1545, in the pin since this test was written).
//!
//! The repair existed for one shape of row: a collection with a baseline
//! (`version > 0`) and `bootstrapped = false`, which the 0.7.0 -> main
//! migration wrote for every collection it carried over. A sync of such a
//! collection that is at its head gets an empty, final answer, and before the
//! fix nothing recorded the flag, so every app-state write to it was refused
//! with "its bootstrap has not completed".
//!
//! This runs the library's own `AppStateProcessor` over each engine's backend,
//! with the answer a sync gets at the head, and reads the row back. If it
//! fails, the pin lost the fix and the repair has to come back.

use std::sync::Arc;

use wacore::appstate::hash::HashState;
use wacore::appstate::patch_decode::{PatchList, WAPatchName};
use wacore::appstate_sync::AppStateProcessor;
use wamux::storage::StorageEngine;
use whatsapp_rust::TokioRuntime;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

const COLLECTION: &str = "regular_low";

/// A migrated row: a baseline, and the flag the migration left unset. The
/// version and ltHash are the ones the fix must leave alone.
fn unmarked_baseline() -> HashState {
    HashState {
        version: 1399,
        hash: [7; 128],
        bootstrapped: false,
        ..Default::default()
    }
}

/// The server's answer for a collection at its head, or a page of one.
fn answer(has_more_patches: bool) -> PatchList {
    PatchList {
        name: WAPatchName::RegularLow,
        has_more_patches,
        patches: Vec::new(),
        snapshot: None,
        snapshot_ref: None,
        error: None,
    }
}

/// Seed the row on a fresh account, run one answer through the library, and
/// return what the store holds afterwards.
async fn stored_after(storage: Arc<dyn StorageEngine>, list: PatchList) -> HashState {
    let account = storage
        .create_account(Some(&format!("bootstrap-{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account");
    let backend = storage.device_backend(account.device_id);
    backend
        .set_version(COLLECTION, unmarked_baseline())
        .await
        .expect("seed the migrated row");
    AppStateProcessor::new(backend.clone(), Arc::new(TokioRuntime))
        .process_patch_list(list, true)
        .await
        .expect("process the answer");
    backend
        .get_version(COLLECTION)
        .await
        .expect("read back")
        .expect("the row is still there")
}

async fn a_baseline_at_the_head_gets_marked(storage: Arc<dyn StorageEngine>) {
    let stored = stored_after(storage, answer(false)).await;
    assert!(
        stored.bootstrapped,
        "a final empty answer marks the baseline"
    );
    assert_eq!(stored.version, 1399, "the version is left alone");
    assert_eq!(stored.hash, [7; 128], "the ltHash is left alone");
}

/// The control: a page is not the head, and must not be marked. Without it the
/// test above would pass on a library that marks everything.
async fn a_page_leaves_it_unmarked(storage: Arc<dyn StorageEngine>) {
    let stored = stored_after(storage, answer(true)).await;
    assert!(!stored.bootstrapped, "a page does not finish a bootstrap");
}

#[tokio::test]
async fn postgres_marks_a_baseline_at_the_head() {
    let storage: Arc<dyn StorageEngine> = common::pg_engine(4).await;
    a_baseline_at_the_head_gets_marked(storage.clone()).await;
    a_page_leaves_it_unmarked(storage).await;
}

#[tokio::test]
async fn sqlite_marks_a_baseline_at_the_head() {
    let (storage, _dir) = common::sqlite_engine().await;
    let storage: Arc<dyn StorageEngine> = storage;
    a_baseline_at_the_head_gets_marked(storage.clone()).await;
    a_page_leaves_it_unmarked(storage).await;
}
