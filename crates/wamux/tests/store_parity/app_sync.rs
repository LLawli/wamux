//! `AppSyncStore`: sync keys and mutation MACs. The version state
//! (`get_version` / `set_version` / `delete_version`) is pinned by
//! `app_state_versions_distinguish_absent_from_empty` in `storage_backend.rs`.

use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::traits::AppStateSyncKey;

use crate::harness::{self, Harness};

fn sync_key(tag: u8, timestamp: i64) -> AppStateSyncKey {
    AppStateSyncKey {
        key_data: vec![tag; 32],
        fingerprint: vec![tag, 0, tag],
        timestamp,
    }
}

/// `AppStateSyncKey` has no `PartialEq`; compare every field as a tuple.
fn fields(k: &AppStateSyncKey) -> (Vec<u8>, Vec<u8>, i64) {
    (k.key_data.clone(), k.fingerprint.clone(), k.timestamp)
}

/// "Latest" is the greatest key id in byte order, which both engines must
/// agree on: BYTEA and BLOB both compare as memcmp, so `[0x02, 0x00]` sorts
/// after `[0x01, 0xff]` and the longer `[0x02, 0x00]` after its prefix `[0x02]`.
async fn sync_keys_round_trip_and_latest_id_is_highest(h: Harness) {
    let t = h.two_accounts("sync-keys").await;
    assert!(t.ba.get_sync_key(&[0x01]).await.unwrap().is_none());
    assert_eq!(t.ba.get_latest_sync_key_id().await.unwrap(), None);

    t.ba.set_sync_key(&[0x01, 0xff], sync_key(1, 100))
        .await
        .unwrap();
    t.ba.set_sync_key(&[0x02], sync_key(2, 200)).await.unwrap();
    t.ba.set_sync_key(&[0x02, 0x00], sync_key(9, 900))
        .await
        .unwrap();
    t.ba.set_sync_key(&[0x02, 0x00], sync_key(3, 300))
        .await
        .unwrap();
    t.bb.set_sync_key(&[0x7f], sync_key(7, 700)).await.unwrap();

    let back =
        t.ba.get_sync_key(&[0x02, 0x00])
            .await
            .unwrap()
            .expect("key");
    assert_eq!(
        fields(&back),
        fields(&sync_key(3, 300)),
        "a second set overwrites"
    );
    let back =
        t.ba.get_sync_key(&[0x01, 0xff])
            .await
            .unwrap()
            .expect("key");
    assert_eq!(fields(&back), fields(&sync_key(1, 100)));
    assert_eq!(
        t.ba.get_latest_sync_key_id().await.unwrap(),
        Some(vec![0x02, 0x00]),
        "B's greater id must not leak into A's answer"
    );
    assert_eq!(
        t.bb.get_latest_sync_key_id().await.unwrap(),
        Some(vec![0x7f])
    );
    assert!(t.bb.get_sync_key(&[0x02]).await.unwrap().is_none());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sync_keys_round_trip_and_latest_id_is_highest() {
    sync_keys_round_trip_and_latest_id_is_highest(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sync_keys_round_trip_and_latest_id_is_highest() {
    sync_keys_round_trip_and_latest_id_is_highest(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_sync_keys_round_trip_and_latest_id_is_highest() {
    sync_keys_round_trip_and_latest_id_is_highest(harness::turso().await).await;
}

fn mac(index: u8, value: u8) -> AppStateMutationMAC {
    AppStateMutationMAC {
        index_mac: vec![index; 32],
        value_mac: vec![value; 32],
    }
}

/// `clear_mutation_macs` runs on snapshot re-sync: a MAC left over from another
/// collection's wipe, or from another account, would corrupt the next ltHash.
async fn mutation_macs_round_trip_delete_and_clear(h: Harness) {
    const LOW: &str = "regular_low";
    const HIGH: &str = "regular_high";
    let t = h.two_accounts("mutation-macs").await;
    assert_eq!(t.ba.get_mutation_mac(LOW, &[1; 32]).await.unwrap(), None);

    t.ba.put_mutation_macs(LOW, 1, &[mac(1, 10), mac(2, 20), mac(3, 30)])
        .await
        .unwrap();
    t.ba.put_mutation_macs(LOW, 2, &[mac(1, 11)]).await.unwrap();
    t.ba.put_mutation_macs(LOW, 3, &[])
        .await
        .expect("empty is a no-op");
    t.ba.put_mutation_macs(HIGH, 1, &[mac(1, 99)])
        .await
        .unwrap();
    t.bb.put_mutation_macs(LOW, 1, &[mac(1, 50)]).await.unwrap();
    assert_eq!(
        t.ba.get_mutation_mac(LOW, &[1; 32]).await.unwrap(),
        Some(vec![11; 32]),
        "the same index MAC is overwritten"
    );
    assert_eq!(
        t.ba.get_mutation_mac(HIGH, &[1; 32]).await.unwrap(),
        Some(vec![99; 32])
    );

    t.ba.delete_mutation_macs(LOW, &[vec![2; 32], vec![42; 32]])
        .await
        .unwrap();
    assert_eq!(t.ba.get_mutation_mac(LOW, &[2; 32]).await.unwrap(), None);
    assert_eq!(
        t.ba.get_mutation_mac(LOW, &[3; 32]).await.unwrap(),
        Some(vec![30; 32])
    );

    t.ba.clear_mutation_macs(LOW).await.unwrap();
    for index in [1, 3] {
        assert_eq!(
            t.ba.get_mutation_mac(LOW, &[index; 32]).await.unwrap(),
            None
        );
    }
    assert_eq!(
        t.ba.get_mutation_mac(HIGH, &[1; 32]).await.unwrap(),
        Some(vec![99; 32]),
        "clearing one collection leaves the others"
    );
    assert_eq!(
        t.bb.get_mutation_mac(LOW, &[1; 32]).await.unwrap(),
        Some(vec![50; 32]),
        "nothing on A reaches B"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_mutation_macs_round_trip_delete_and_clear() {
    mutation_macs_round_trip_delete_and_clear(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_mutation_macs_round_trip_delete_and_clear() {
    mutation_macs_round_trip_delete_and_clear(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_mutation_macs_round_trip_delete_and_clear() {
    mutation_macs_round_trip_delete_and_clear(harness::turso().await).await;
}
