//! `ProtocolStore`, second half: tc tokens and the sent-message retry cache,
//! the two tables with expiry sweeps.

use wacore::store::traits::TcTokenEntry;

use crate::harness::{self, Harness, now_secs};

fn token(bytes: &[u8], token_timestamp: i64, sender_timestamp: Option<i64>) -> TcTokenEntry {
    TcTokenEntry {
        token: bytes.to_vec(),
        token_timestamp,
        sender_timestamp,
    }
}

/// `TcTokenEntry` has no `PartialEq`; compare every field as a tuple.
fn fields(e: &TcTokenEntry) -> (Vec<u8>, i64, Option<i64>) {
    (e.token.clone(), e.token_timestamp, e.sender_timestamp)
}

async fn sorted_jids(backend: &dyn wacore::store::traits::Backend) -> Vec<String> {
    let mut jids = backend.get_all_tc_token_jids().await.unwrap();
    jids.sort();
    jids
}

async fn tc_tokens_round_trip_list_and_delete(h: Harness) {
    let t = h.two_accounts("tc-tokens").await;
    assert!(
        t.ba.get_tc_token("100000000000001@lid")
            .await
            .unwrap()
            .is_none()
    );
    assert!(t.ba.get_all_tc_token_jids().await.unwrap().is_empty());

    t.ba.put_tc_token("100000000000002@lid", &token(b"t2", 20, Some(25)))
        .await
        .unwrap();
    t.ba.put_tc_token("100000000000001@lid", &token(b"old", 1, Some(1)))
        .await
        .unwrap();
    t.ba.put_tc_token("100000000000001@lid", &token(b"t1", 10, None))
        .await
        .unwrap();
    t.bb.put_tc_token("100000000000009@lid", &token(b"tb", 90, None))
        .await
        .unwrap();

    let back =
        t.ba.get_tc_token("100000000000001@lid")
            .await
            .unwrap()
            .expect("row");
    assert_eq!(
        fields(&back),
        fields(&token(b"t1", 10, None)),
        "a put replaces all three fields, None included"
    );
    let back =
        t.ba.get_tc_token("100000000000002@lid")
            .await
            .unwrap()
            .expect("row");
    assert_eq!(fields(&back), fields(&token(b"t2", 20, Some(25))));
    assert_eq!(
        sorted_jids(&*t.ba).await,
        vec!["100000000000001@lid", "100000000000002@lid"]
    );

    t.ba.delete_tc_token("100000000000001@lid").await.unwrap();
    assert!(
        t.ba.get_tc_token("100000000000001@lid")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(sorted_jids(&*t.ba).await, vec!["100000000000002@lid"]);
    assert_eq!(sorted_jids(&*t.bb).await, vec!["100000000000009@lid"]);
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_tc_tokens_round_trip_list_and_delete() {
    tc_tokens_round_trip_list_and_delete(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_tc_tokens_round_trip_list_and_delete() {
    tc_tokens_round_trip_list_and_delete(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_tc_tokens_round_trip_list_and_delete() {
    tc_tokens_round_trip_list_and_delete(harness::turso().await).await;
}

/// A row goes only when BOTH windows are stale: the received token (older than
/// `token_cutoff`, or byte-empty) AND the sender bucket (older than
/// `sender_cutoff`, or never set). Both comparisons are strict.
async fn expired_tc_tokens_need_both_windows_stale(h: Harness) {
    const TOKEN_CUTOFF: i64 = 1_000;
    const SENDER_CUTOFF: i64 = 2_000;
    let cases: [(&str, TcTokenEntry, bool); 8] = [
        ("old-token-no-sender", token(b"t", 500, None), true),
        ("old-token-old-sender", token(b"t", 500, Some(1_500)), true),
        (
            "old-token-fresh-sender",
            token(b"t", 500, Some(2_500)),
            false,
        ),
        ("fresh-token-no-sender", token(b"t", 1_500, None), false),
        ("empty-token-no-sender", token(b"", 1_500, None), true),
        (
            "empty-token-fresh-sender",
            token(b"", 1_500, Some(2_500)),
            false,
        ),
        ("token-at-cutoff", token(b"t", TOKEN_CUTOFF, None), false),
        (
            "sender-at-cutoff",
            token(b"t", 500, Some(SENDER_CUTOFF)),
            false,
        ),
    ];
    let t = h.two_accounts("tc-expiry").await;
    for (jid, entry, _) in &cases {
        t.ba.put_tc_token(jid, entry).await.unwrap();
    }
    t.bb.put_tc_token("old-token-no-sender", &token(b"t", 500, None))
        .await
        .unwrap();

    let deleted =
        t.ba.delete_expired_tc_tokens(TOKEN_CUTOFF, SENDER_CUTOFF)
            .await
            .unwrap();
    let mut kept: Vec<String> = cases
        .iter()
        .filter(|(_, _, goes)| !goes)
        .map(|(jid, _, _)| jid.to_string())
        .collect();
    kept.sort();
    assert_eq!(deleted, 3, "the count is the rows removed");
    assert_eq!(sorted_jids(&*t.ba).await, kept);
    assert_eq!(
        sorted_jids(&*t.bb).await,
        vec!["old-token-no-sender"],
        "a sweep on A must not reach B"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_expired_tc_tokens_need_both_windows_stale() {
    expired_tc_tokens_need_both_windows_stale(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_expired_tc_tokens_need_both_windows_stale() {
    expired_tc_tokens_need_both_windows_stale(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_expired_tc_tokens_need_both_windows_stale() {
    expired_tc_tokens_need_both_windows_stale(harness::turso().await).await;
}

/// Retry receipts consume the payload: a second take must find nothing, or a
/// duplicate receipt would resend the message.
async fn sent_messages_take_removes_what_it_returns(h: Harness) {
    const CHAT: &str = "5511900000001@s.whatsapp.net";
    let t = h.two_accounts("sent-take").await;
    assert_eq!(t.ba.take_sent_message(CHAT, "M1").await.unwrap(), None);

    t.ba.store_sent_message(CHAT, "M1", b"payload-old")
        .await
        .unwrap();
    t.ba.store_sent_message(CHAT, "M1", b"payload")
        .await
        .unwrap();
    t.ba.store_sent_message(CHAT, "M2", b"other").await.unwrap();
    t.bb.store_sent_message(CHAT, "M1", b"payload-b")
        .await
        .unwrap();

    assert_eq!(
        t.ba.take_sent_message(CHAT, "M1").await.unwrap(),
        Some(b"payload".to_vec()),
        "a second store overwrites the payload"
    );
    assert_eq!(t.ba.take_sent_message(CHAT, "M1").await.unwrap(), None);
    assert_eq!(
        t.ba.take_sent_message(CHAT, "M2").await.unwrap(),
        Some(b"other".to_vec()),
        "taking M1 left M2 alone"
    );
    assert_eq!(
        t.bb.take_sent_message(CHAT, "M1").await.unwrap(),
        Some(b"payload-b".to_vec()),
        "A's take must not consume B's row"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_sent_messages_take_removes_what_it_returns() {
    sent_messages_take_removes_what_it_returns(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_sent_messages_take_removes_what_it_returns() {
    sent_messages_take_removes_what_it_returns(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_sent_messages_take_removes_what_it_returns() {
    sent_messages_take_removes_what_it_returns(harness::turso().await).await;
}

/// The store stamps `created_at` with the wall clock, so the cutoffs sit an
/// hour either side of now: no flake from a second ticking over.
async fn expired_sent_messages_respect_cutoff(h: Harness) {
    const CHAT: &str = "5511900000001@s.whatsapp.net";
    let t = h.two_accounts("sent-expiry").await;
    for id in ["M1", "M2", "M3"] {
        t.ba.store_sent_message(CHAT, id, b"p").await.unwrap();
    }
    t.bb.store_sent_message(CHAT, "M1", b"p").await.unwrap();

    let past = now_secs() - 3_600;
    assert_eq!(t.ba.delete_expired_sent_messages(past).await.unwrap(), 0);
    assert_eq!(
        t.ba.take_sent_message(CHAT, "M3").await.unwrap(),
        Some(b"p".to_vec()),
        "a cutoff in the past removes nothing"
    );

    let future = now_secs() + 3_600;
    assert_eq!(t.ba.delete_expired_sent_messages(future).await.unwrap(), 2);
    assert_eq!(t.ba.take_sent_message(CHAT, "M1").await.unwrap(), None);
    assert_eq!(t.ba.take_sent_message(CHAT, "M2").await.unwrap(), None);
    assert_eq!(
        t.bb.take_sent_message(CHAT, "M1").await.unwrap(),
        Some(b"p".to_vec()),
        "a sweep on A must not reach B"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_expired_sent_messages_respect_cutoff() {
    expired_sent_messages_respect_cutoff(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_expired_sent_messages_respect_cutoff() {
    expired_sent_messages_respect_cutoff(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_expired_sent_messages_respect_cutoff() {
    expired_sent_messages_respect_cutoff(harness::turso().await).await;
}
