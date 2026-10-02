//! `SignalStore`: identities, sessions, prekeys, signed prekeys, sender keys.

use bytes::Bytes;

use crate::harness::{self, Harness};

const ALICE: &str = "alice@s.whatsapp.net";
const BOB: &str = "bob@s.whatsapp.net";

async fn identities_round_trip_overwrite_delete_and_isolate(h: Harness) {
    let t = h.two_accounts("identities").await;
    assert_eq!(t.ba.load_identity(ALICE).await.unwrap(), None);

    t.ba.put_identity(ALICE, [1u8; 32]).await.unwrap();
    t.ba.put_identity(ALICE, [2u8; 32]).await.unwrap();
    t.ba.put_identity(BOB, [3u8; 32]).await.unwrap();
    t.bb.put_identity(ALICE, [9u8; 32]).await.unwrap();
    assert_eq!(
        t.ba.load_identity(ALICE).await.unwrap(),
        Some([2u8; 32]),
        "a second put overwrites"
    );

    t.ba.delete_identity(ALICE).await.unwrap();
    assert_eq!(t.ba.load_identity(ALICE).await.unwrap(), None);
    assert_eq!(t.ba.load_identity(BOB).await.unwrap(), Some([3u8; 32]));
    assert_eq!(
        t.bb.load_identity(ALICE).await.unwrap(),
        Some([9u8; 32]),
        "a delete on A must not reach B's row for the same address"
    );
    t.ba.delete_identity(ALICE)
        .await
        .expect("deleting an absent identity is a no-op");
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_identities_round_trip_overwrite_delete_and_isolate() {
    identities_round_trip_overwrite_delete_and_isolate(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_identities_round_trip_overwrite_delete_and_isolate() {
    identities_round_trip_overwrite_delete_and_isolate(harness::sqlite().await).await;
}

async fn sessions_round_trip_overwrite_delete_and_isolate(h: Harness) {
    let t = h.two_accounts("sessions").await;
    assert_eq!(t.ba.get_session(ALICE).await.unwrap(), None);
    assert!(!t.ba.has_session(ALICE).await.unwrap());

    t.ba.put_session(ALICE, b"first").await.unwrap();
    t.ba.put_session(ALICE, b"second").await.unwrap();
    t.bb.put_session(ALICE, b"bs-own").await.unwrap();
    assert_eq!(
        t.ba.get_session(ALICE).await.unwrap(),
        Some(Bytes::from_static(b"second"))
    );
    assert!(t.ba.has_session(ALICE).await.unwrap());

    t.ba.delete_session(ALICE).await.unwrap();
    assert_eq!(t.ba.get_session(ALICE).await.unwrap(), None);
    assert!(!t.ba.has_session(ALICE).await.unwrap());
    assert_eq!(
        t.bb.get_session(ALICE).await.unwrap(),
        Some(Bytes::from_static(b"bs-own"))
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sessions_round_trip_overwrite_delete_and_isolate() {
    sessions_round_trip_overwrite_delete_and_isolate(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sessions_round_trip_overwrite_delete_and_isolate() {
    sessions_round_trip_overwrite_delete_and_isolate(harness::sqlite().await).await;
}

async fn prekeys_store_load_remove_and_max_are_scoped(h: Harness) {
    let t = h.two_accounts("prekeys").await;
    assert_eq!(t.ba.get_max_prekey_id().await.unwrap(), 0, "0 when empty");

    t.ba.store_prekey(5, b"pk-5", false).await.unwrap();
    t.ba.store_prekey(9, b"pk-9-old", false).await.unwrap();
    t.ba.store_prekey(9, b"pk-9", true).await.unwrap();
    t.bb.store_prekey(5, b"pk-5-b", false).await.unwrap();
    assert_eq!(
        t.ba.load_prekey(9).await.unwrap().as_deref(),
        Some(&b"pk-9"[..])
    );
    assert_eq!(t.ba.get_max_prekey_id().await.unwrap(), 9);
    assert_eq!(t.bb.get_max_prekey_id().await.unwrap(), 5);

    t.ba.remove_prekey(9).await.unwrap();
    assert_eq!(t.ba.load_prekey(9).await.unwrap(), None);
    assert_eq!(
        t.ba.get_max_prekey_id().await.unwrap(),
        5,
        "the max follows the rows, it is not a high-water mark"
    );
    t.ba.remove_prekey(5).await.unwrap();
    assert_eq!(t.ba.get_max_prekey_id().await.unwrap(), 0);
    assert_eq!(
        t.bb.load_prekey(5).await.unwrap().as_deref(),
        Some(&b"pk-5-b"[..]),
        "a remove on A must not reach B's prekey with the same id"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_prekeys_store_load_remove_and_max_are_scoped() {
    prekeys_store_load_remove_and_max_are_scoped(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_prekeys_store_load_remove_and_max_are_scoped() {
    prekeys_store_load_remove_and_max_are_scoped(harness::sqlite().await).await;
}

/// UPDATE, never upsert: a prekey consumed between the upload snapshot and this
/// call must stay deleted, or the server is handed a key we cannot answer.
async fn mark_prekeys_uploaded_updates_without_resurrecting(h: Harness) {
    let t = h.two_accounts("prekeys-uploaded").await;
    let (a, b) = (t.a.device_id, t.b.device_id);
    for id in [1, 2, 3] {
        t.ba.store_prekey(id, b"pk", false).await.unwrap();
    }
    t.bb.store_prekey(1, b"pk", false).await.unwrap();

    t.ba.mark_prekeys_uploaded(&[])
        .await
        .expect("empty is a no-op");
    t.ba.mark_prekeys_uploaded(&[1, 3, 77]).await.unwrap();

    assert_eq!(h.raw.prekey_uploaded(a, 1).await, Some(true));
    assert_eq!(h.raw.prekey_uploaded(a, 2).await, Some(false));
    assert_eq!(h.raw.prekey_uploaded(a, 3).await, Some(true));
    assert_eq!(h.raw.prekey_uploaded(a, 77).await, None, "no resurrection");
    assert_eq!(t.ba.load_prekey(77).await.unwrap(), None);
    assert_eq!(
        h.raw.prekey_uploaded(b, 1).await,
        Some(false),
        "marking on A must not touch B's prekey with the same id"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_mark_prekeys_uploaded_updates_without_resurrecting() {
    mark_prekeys_uploaded_updates_without_resurrecting(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_mark_prekeys_uploaded_updates_without_resurrecting() {
    mark_prekeys_uploaded_updates_without_resurrecting(harness::sqlite().await).await;
}

async fn signed_prekeys_round_trip_list_remove_and_isolate(h: Harness) {
    let t = h.two_accounts("signed-prekeys").await;
    assert_eq!(t.ba.load_signed_prekey(1).await.unwrap(), None);
    assert!(t.ba.load_all_signed_prekeys().await.unwrap().is_empty());

    t.ba.store_signed_prekey(2, b"spk-2").await.unwrap();
    t.ba.store_signed_prekey(1, b"spk-1-old").await.unwrap();
    t.ba.store_signed_prekey(1, b"spk-1").await.unwrap();
    t.bb.store_signed_prekey(1, b"spk-1-b").await.unwrap();
    assert_eq!(
        t.ba.load_signed_prekey(1).await.unwrap(),
        Some(b"spk-1".to_vec())
    );

    let mut all = t.ba.load_all_signed_prekeys().await.unwrap();
    all.sort();
    assert_eq!(all, vec![(1, b"spk-1".to_vec()), (2, b"spk-2".to_vec())]);

    t.ba.remove_signed_prekey(1).await.unwrap();
    assert_eq!(t.ba.load_signed_prekey(1).await.unwrap(), None);
    assert_eq!(
        t.ba.load_all_signed_prekeys().await.unwrap(),
        vec![(2, b"spk-2".to_vec())]
    );
    assert_eq!(
        t.bb.load_all_signed_prekeys().await.unwrap(),
        vec![(1, b"spk-1-b".to_vec())],
        "B lists only its own, and A's remove did not reach it"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_signed_prekeys_round_trip_list_remove_and_isolate() {
    signed_prekeys_round_trip_list_remove_and_isolate(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_signed_prekeys_round_trip_list_remove_and_isolate() {
    signed_prekeys_round_trip_list_remove_and_isolate(harness::sqlite().await).await;
}

async fn sender_keys_round_trip_overwrite_delete_and_isolate(h: Harness) {
    const GROUP_ADDR: &str = "120363000000000000@g.us::alice@s.whatsapp.net";
    let t = h.two_accounts("sender-keys").await;
    assert_eq!(t.ba.get_sender_key(GROUP_ADDR).await.unwrap(), None);

    t.ba.put_sender_key(GROUP_ADDR, b"sk-old").await.unwrap();
    t.ba.put_sender_key(GROUP_ADDR, b"sk").await.unwrap();
    t.bb.put_sender_key(GROUP_ADDR, b"sk-b").await.unwrap();
    assert_eq!(
        t.ba.get_sender_key(GROUP_ADDR).await.unwrap(),
        Some(b"sk".to_vec())
    );

    t.ba.delete_sender_key(GROUP_ADDR).await.unwrap();
    assert_eq!(t.ba.get_sender_key(GROUP_ADDR).await.unwrap(), None);
    assert_eq!(
        t.bb.get_sender_key(GROUP_ADDR).await.unwrap(),
        Some(b"sk-b".to_vec())
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sender_keys_round_trip_overwrite_delete_and_isolate() {
    sender_keys_round_trip_overwrite_delete_and_isolate(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sender_keys_round_trip_overwrite_delete_and_isolate() {
    sender_keys_round_trip_overwrite_delete_and_isolate(harness::sqlite().await).await;
}
