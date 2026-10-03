//! `SignalStore` batch methods (#104): 250 items, so a batched read crosses
//! three 100-value chunks and pads the last one. A repeated key in one write
//! batch keeps the last value, as the per-row loop it replaces did.

use std::sync::Arc;

use bytes::Bytes;

use crate::harness::{self, Harness};

const MANY: usize = 250;

fn address(i: usize) -> Arc<str> {
    Arc::from(format!("55119{i:08}.0"))
}

fn record(i: usize) -> Bytes {
    Bytes::from(format!("record-{i}").into_bytes())
}

async fn identities_batch_round_trips_and_deletes(h: Harness) {
    let t = h.two_accounts("batch-identities").await;
    t.ba.put_identities_batch(&[])
        .await
        .expect("empty is a no-op");
    let batch = [
        (address(1), [1u8; 32]),
        (address(2), [2u8; 32]),
        (address(1), [7u8; 32]),
    ];
    t.ba.put_identities_batch(&batch).await.unwrap();
    assert_eq!(
        t.ba.load_identity(&address(1)).await.unwrap(),
        Some([7u8; 32]),
        "last wins"
    );
    assert_eq!(
        t.ba.load_identity(&address(2)).await.unwrap(),
        Some([2u8; 32])
    );
    assert_eq!(t.bb.load_identity(&address(1)).await.unwrap(), None);

    t.ba.delete_identities_batch(&[])
        .await
        .expect("empty is a no-op");
    t.ba.delete_identities_batch(&[address(1), address(3)])
        .await
        .unwrap();
    assert_eq!(t.ba.load_identity(&address(1)).await.unwrap(), None);
    assert_eq!(
        t.ba.load_identity(&address(2)).await.unwrap(),
        Some([2u8; 32])
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_identities_batch_round_trips_and_deletes() {
    identities_batch_round_trips_and_deletes(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_identities_batch_round_trips_and_deletes() {
    identities_batch_round_trips_and_deletes(harness::sqlite().await).await;
}

async fn sessions_batch_round_trips_and_deletes(h: Harness) {
    let t = h.two_accounts("batch-sessions").await;
    let mut batch: Vec<(Arc<str>, Bytes)> = (0..MANY).map(|i| (address(i), record(i))).collect();
    batch.push((address(0), Bytes::from_static(b"last")));
    t.ba.put_sessions_batch(&batch).await.unwrap();
    t.bb.put_session(&address(0), b"other account")
        .await
        .unwrap();

    let mut asked: Vec<Arc<str>> = (0..MANY).map(address).collect();
    asked.push(Arc::from("absent.0"));
    let mut got = t.ba.get_sessions_batch(&asked).await.unwrap();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got.len(), MANY, "absent addresses are left out");
    assert_eq!(&got[0].1[..], b"last", "last wins");
    for (i, (addr, bytes)) in got.iter().enumerate().skip(1) {
        assert_eq!((addr, bytes), (&address(i), &record(i)));
    }
    assert!(t.ba.get_sessions_batch(&[]).await.unwrap().is_empty());

    let gone: Vec<Arc<str>> = (0..MANY - 1).map(address).collect();
    t.ba.delete_sessions_batch(&gone).await.unwrap();
    let left = t.ba.get_sessions_batch(&asked).await.unwrap();
    assert_eq!(left, vec![(address(MANY - 1), record(MANY - 1))]);
    assert!(t.bb.get_session(&address(0)).await.unwrap().is_some());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sessions_batch_round_trips_and_deletes() {
    sessions_batch_round_trips_and_deletes(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sessions_batch_round_trips_and_deletes() {
    sessions_batch_round_trips_and_deletes(harness::sqlite().await).await;
}

async fn prekeys_batch_round_trips_and_removes(h: Harness) {
    let t = h.two_accounts("batch-prekeys").await;
    let uploaded: Vec<(u32, Bytes)> = (1..=200u32).map(|i| (i, record(i as usize))).collect();
    let pending: Vec<(u32, Bytes)> = (201..=MANY as u32)
        .map(|i| (i, record(i as usize)))
        .collect();
    t.ba.store_prekeys_batch(&uploaded, true).await.unwrap();
    t.ba.store_prekeys_batch(&pending, false).await.unwrap();
    t.ba.store_prekeys_batch(&[], true)
        .await
        .expect("empty is a no-op");
    assert_eq!(h.raw.prekey_uploaded(t.a.device_id, 1).await, Some(true));
    assert_eq!(h.raw.prekey_uploaded(t.a.device_id, 250).await, Some(false));

    let mut asked: Vec<u32> = (1..=MANY as u32).collect();
    asked.push(9999);
    let mut got = t.ba.load_prekeys_batch(&asked).await.unwrap();
    got.sort_by_key(|(id, _)| *id);
    assert_eq!(got.len(), MANY, "absent ids are left out");
    assert_eq!(got[124], (125, record(125)));
    assert!(t.bb.load_prekeys_batch(&asked).await.unwrap().is_empty());

    let removed: Vec<u32> = (2..=MANY as u32).collect();
    t.ba.remove_prekeys_batch(&removed).await.unwrap();
    assert_eq!(
        t.ba.load_prekeys_batch(&asked).await.unwrap(),
        vec![(1, record(1))]
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_prekeys_batch_round_trips_and_removes() {
    prekeys_batch_round_trips_and_removes(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_prekeys_batch_round_trips_and_removes() {
    prekeys_batch_round_trips_and_removes(harness::sqlite().await).await;
}

async fn sender_keys_batch_round_trips_and_deletes(h: Harness) {
    let t = h.two_accounts("batch-sender-keys").await;
    let batch = [
        (address(1), record(1)),
        (address(2), record(2)),
        (address(1), Bytes::from_static(b"last")),
    ];
    t.ba.put_sender_keys_batch(&batch).await.unwrap();
    t.ba.put_sender_keys_batch(&[])
        .await
        .expect("empty is a no-op");
    assert_eq!(
        t.ba.get_sender_key(&address(1)).await.unwrap().as_deref(),
        Some(&b"last"[..])
    );
    assert_eq!(t.bb.get_sender_key(&address(1)).await.unwrap(), None);

    t.ba.delete_sender_keys_batch(&[address(1), address(9)])
        .await
        .unwrap();
    t.ba.delete_sender_keys_batch(&[])
        .await
        .expect("empty is a no-op");
    assert_eq!(t.ba.get_sender_key(&address(1)).await.unwrap(), None);
    assert_eq!(
        t.ba.get_sender_key(&address(2)).await.unwrap(),
        Some(record(2).to_vec())
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sender_keys_batch_round_trips_and_deletes() {
    sender_keys_batch_round_trips_and_deletes(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sender_keys_batch_round_trips_and_deletes() {
    sender_keys_batch_round_trips_and_deletes(harness::sqlite().await).await;
}
