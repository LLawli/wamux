//! `ProtocolStore` methods the wacore traits ship with a default body that is
//! wrong for a real backend, overridden by both engines (#93).
//!
//! - `get_sent_message`: the default errors, so a group repair or resend that
//!   misses the in-memory cache finds no payload.
//! - `delete_expired_base_keys`: the default is `Ok(0)`, so the keepalive sweep
//!   never pruned `base_keys`.
//! - the two tc-token writers: the defaults are a read-modify-write, which the
//!   trait doc says must be atomic against each other. The last test is the one
//!   that fails on the default: concurrent writers drop a field.
//!
//! `docs/store-trait-defaults.md` lists every default and why it is kept or not.

use std::sync::Arc;

use wacore::store::traits::{Backend, TcTokenEntry};

use crate::harness::{self, Harness, now_secs};

/// `TcTokenEntry` has no `PartialEq`; compare every field as a tuple.
fn fields(e: &TcTokenEntry) -> (Vec<u8>, i64, Option<i64>) {
    (e.token.clone(), e.token_timestamp, e.sender_timestamp)
}

async fn tc_fields(backend: &dyn Backend, jid: &str) -> (Vec<u8>, i64, Option<i64>) {
    let entry = backend
        .get_tc_token(jid)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("tc token row for {jid}"));
    fields(&entry)
}

async fn get_sent_message_reads_without_consuming(h: Harness) {
    const CHAT: &str = "120363000000000001@g.us";
    let t = h.two_accounts("sent-get").await;
    assert_eq!(
        t.ba.get_sent_message(CHAT, "M1").await.unwrap(),
        None,
        "an absent row is Ok(None), not the default's Unsupported error"
    );

    t.ba.store_sent_message(CHAT, "M1", b"payload-a")
        .await
        .unwrap();
    t.bb.store_sent_message(CHAT, "M2", b"payload-b")
        .await
        .unwrap();
    for read in ["first", "second"] {
        assert_eq!(
            t.ba.get_sent_message(CHAT, "M1").await.unwrap(),
            Some(b"payload-a".to_vec()),
            "the {read} read returns the payload"
        );
    }
    assert_eq!(
        t.ba.take_sent_message(CHAT, "M1").await.unwrap(),
        Some(b"payload-a".to_vec()),
        "reads leave the row for the retry that takes it"
    );
    assert_eq!(t.ba.get_sent_message(CHAT, "M1").await.unwrap(), None);
    assert_eq!(
        t.ba.get_sent_message(CHAT, "M2").await.unwrap(),
        None,
        "A must not read B's row"
    );
    assert_eq!(
        t.bb.get_sent_message(CHAT, "M2").await.unwrap(),
        Some(b"payload-b".to_vec())
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_get_sent_message_reads_without_consuming() {
    get_sent_message_reads_without_consuming(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_get_sent_message_reads_without_consuming() {
    get_sent_message_reads_without_consuming(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_get_sent_message_reads_without_consuming() {
    get_sent_message_reads_without_consuming(harness::turso().await).await;
}

/// `created_at` is the wall clock at save, so the cutoffs sit an hour either
/// side of now: no flake from a second ticking over.
async fn delete_expired_base_keys_prunes_before_cutoff(h: Harness) {
    const ADDR: &str = "5511900000001.0:1";
    let t = h.two_accounts("base-key-expiry").await;
    for id in ["M1", "M2"] {
        t.ba.save_base_key(ADDR, id, b"key-a").await.unwrap();
    }
    t.bb.save_base_key(ADDR, "M1", b"key-b").await.unwrap();

    let past = now_secs() - 3_600;
    assert_eq!(t.ba.delete_expired_base_keys(past).await.unwrap(), 0);
    assert!(
        t.ba.has_same_base_key(ADDR, "M1", b"key-a").await.unwrap(),
        "a cutoff in the past removes nothing"
    );

    let future = now_secs() + 3_600;
    assert_eq!(
        t.ba.delete_expired_base_keys(future).await.unwrap(),
        2,
        "the count is A's rows only"
    );
    for id in ["M1", "M2"] {
        assert!(!t.ba.has_same_base_key(ADDR, id, b"key-a").await.unwrap());
    }
    assert!(
        t.bb.has_same_base_key(ADDR, "M1", b"key-b").await.unwrap(),
        "a sweep on A must not reach B"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_delete_expired_base_keys_prunes_before_cutoff() {
    delete_expired_base_keys_prunes_before_cutoff(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_delete_expired_base_keys_prunes_before_cutoff() {
    delete_expired_base_keys_prunes_before_cutoff(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_delete_expired_base_keys_prunes_before_cutoff() {
    delete_expired_base_keys_prunes_before_cutoff(harness::turso().await).await;
}

async fn touch_tc_token_sender_timestamp_only_advances(h: Harness) {
    const NEW: &str = "100000000000001@lid";
    const REAL: &str = "100000000000002@lid";
    let t = h.two_accounts("tc-touch").await;

    t.ba.touch_tc_token_sender_timestamp(NEW, 50).await.unwrap();
    assert_eq!(
        tc_fields(&*t.ba, NEW).await,
        (Vec::new(), 50, Some(50)),
        "an absent row becomes a byte-less placeholder"
    );
    t.ba.touch_tc_token_sender_timestamp(NEW, 40).await.unwrap();
    assert_eq!(
        tc_fields(&*t.ba, NEW).await,
        (Vec::new(), 50, Some(50)),
        "an older timestamp never regresses the sender bucket"
    );
    t.ba.touch_tc_token_sender_timestamp(NEW, 70).await.unwrap();
    assert_eq!(
        tc_fields(&*t.ba, NEW).await,
        (Vec::new(), 50, Some(70)),
        "a newer one advances it and leaves token_timestamp alone"
    );

    let real = TcTokenEntry {
        token: b"real".to_vec(),
        token_timestamp: 10,
        sender_timestamp: None,
    };
    t.ba.put_tc_token(REAL, &real).await.unwrap();
    t.ba.touch_tc_token_sender_timestamp(REAL, 30)
        .await
        .unwrap();
    assert_eq!(
        tc_fields(&*t.ba, REAL).await,
        (b"real".to_vec(), 10, Some(30)),
        "a NULL sender bucket takes the timestamp; the real token stays"
    );
    assert!(t.bb.get_all_tc_token_jids().await.unwrap().is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_touch_tc_token_sender_timestamp_only_advances() {
    touch_tc_token_sender_timestamp_only_advances(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_touch_tc_token_sender_timestamp_only_advances() {
    touch_tc_token_sender_timestamp_only_advances(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_touch_tc_token_sender_timestamp_only_advances() {
    touch_tc_token_sender_timestamp_only_advances(harness::turso().await).await;
}

async fn store_received_tc_token_is_newer_wins(h: Harness) {
    const NEW: &str = "100000000000001@lid";
    const HELD: &str = "100000000000002@lid";
    let t = h.two_accounts("tc-received").await;

    t.ba.store_received_tc_token(NEW, b"first", 100)
        .await
        .unwrap();
    assert_eq!(
        tc_fields(&*t.ba, NEW).await,
        (b"first".to_vec(), 100, None),
        "an absent row is stored with no sender bucket"
    );
    t.ba.store_received_tc_token(NEW, b"stale", 90)
        .await
        .unwrap();
    assert_eq!(
        tc_fields(&*t.ba, NEW).await,
        (b"first".to_vec(), 100, None),
        "an older token never clobbers a fresher real one"
    );
    t.ba.store_received_tc_token(NEW, b"same", 100)
        .await
        .unwrap();
    assert_eq!(
        tc_fields(&*t.ba, NEW).await,
        (b"same".to_vec(), 100, None),
        "an equal timestamp overwrites"
    );

    t.ba.touch_tc_token_sender_timestamp(HELD, 500)
        .await
        .unwrap();
    t.ba.store_received_tc_token(HELD, b"real", 20)
        .await
        .unwrap();
    assert_eq!(
        tc_fields(&*t.ba, HELD).await,
        (b"real".to_vec(), 20, Some(500)),
        "a placeholder never blocks the first real token, and the sender bucket survives"
    );
    t.ba.store_received_tc_token(HELD, b"newer", 30)
        .await
        .unwrap();
    assert_eq!(
        tc_fields(&*t.ba, HELD).await,
        (b"newer".to_vec(), 30, Some(500))
    );
    assert!(t.bb.get_all_tc_token_jids().await.unwrap().is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_store_received_tc_token_is_newer_wins() {
    store_received_tc_token_is_newer_wins(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_store_received_tc_token_is_newer_wins() {
    store_received_tc_token_is_newer_wins(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_store_received_tc_token_is_newer_wins() {
    store_received_tc_token_is_newer_wins(harness::turso().await).await;
}

/// The trait's own example of the race: history sync stores a received token
/// while the send path touches the sender bucket of the same contact. With a
/// read-modify-write both read "absent" and the second write drops the first
/// one's field. Each writer owns one field, so after both every row must hold
/// the real token AND the sender timestamp, whatever the interleaving.
async fn tc_token_writers_converge_under_concurrency(h: Harness) {
    const CONTACTS: usize = 64;
    let t = h.two_accounts("tc-concurrent").await;
    let jids: Vec<String> = (0..CONTACTS)
        .map(|i| format!("1000000000{i:05}@lid"))
        .collect();

    let mut writers = tokio::task::JoinSet::new();
    for jid in &jids {
        let (backend, contact) = (Arc::clone(&t.ba), jid.clone());
        writers.spawn(async move {
            backend
                .store_received_tc_token(&contact, b"real", 100)
                .await
        });
        let (backend, contact) = (Arc::clone(&t.ba), jid.clone());
        writers.spawn(async move { backend.touch_tc_token_sender_timestamp(&contact, 200).await });
    }
    while let Some(joined) = writers.join_next().await {
        joined.expect("writer task").expect("tc token write");
    }

    let mut lost = Vec::new();
    for jid in &jids {
        let got = tc_fields(&*t.ba, jid).await;
        if got != (b"real".to_vec(), 100, Some(200)) {
            lost.push((jid.clone(), got));
        }
    }
    assert!(
        lost.is_empty(),
        "{} of {CONTACTS} rows lost a field: {lost:?}",
        lost.len()
    );
    assert!(t.bb.get_all_tc_token_jids().await.unwrap().is_empty());
    h.drop_accounts(t).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_tc_token_writers_converge_under_concurrency() {
    tc_token_writers_converge_under_concurrency(harness::postgres().await).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_tc_token_writers_converge_under_concurrency() {
    tc_token_writers_converge_under_concurrency(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_tc_token_writers_converge_under_concurrency() {
    tc_token_writers_converge_under_concurrency(harness::turso().await).await;
}
