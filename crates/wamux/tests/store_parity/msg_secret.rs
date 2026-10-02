//! `MsgSecretStore`: the 32-byte `messageSecret` per outbound message, with a
//! retention deadline the store must never shorten.

use std::sync::Arc;

use wacore::store::traits::MsgSecretEntry;

use crate::harness::{self, Harness};

const CHAT: &str = "5511900000001@s.whatsapp.net";
const ME: &str = "5511900000000@s.whatsapp.net";

fn entry(msg_id: &str, secret: u8, expires_at: i64, message_ts: i64) -> MsgSecretEntry {
    MsgSecretEntry {
        chat: Arc::from(CHAT),
        sender: Arc::from(ME),
        msg_id: Arc::from(msg_id),
        secret: [secret; 32],
        expires_at,
        message_ts,
    }
}

async fn msg_secrets_round_trip_with_ts_and_isolate(h: Harness) {
    let t = h.two_accounts("msg-secrets").await;
    assert_eq!(t.ba.put_msg_secrets(Vec::new()).await.unwrap(), 0);
    assert_eq!(t.ba.get_msg_secret(CHAT, ME, "M1").await.unwrap(), None);
    assert_eq!(
        t.ba.get_msg_secret_with_ts(CHAT, ME, "M1").await.unwrap(),
        None
    );

    let written =
        t.ba.put_msg_secrets(vec![entry("M1", 1, 0, 1_700), entry("M2", 2, 0, 0)])
            .await
            .unwrap();
    assert_eq!(written, 2, "the count is the entries written");
    t.bb.put_msg_secrets(vec![entry("M1", 9, 0, 9_900)])
        .await
        .unwrap();

    assert_eq!(
        t.ba.get_msg_secret(CHAT, ME, "M1").await.unwrap(),
        Some(vec![1; 32])
    );
    assert_eq!(
        t.ba.get_msg_secret_with_ts(CHAT, ME, "M1").await.unwrap(),
        Some((vec![1; 32], 1_700)),
        "the real message_ts, not the trait default's hardcoded 0"
    );
    assert_eq!(
        t.ba.get_msg_secret_with_ts(CHAT, ME, "M2").await.unwrap(),
        Some((vec![2; 32], 0))
    );
    assert_eq!(
        t.ba.get_msg_secret(CHAT, CHAT, "M1").await.unwrap(),
        None,
        "sender is part of the key"
    );
    assert_eq!(
        t.bb.get_msg_secret_with_ts(CHAT, ME, "M1").await.unwrap(),
        Some((vec![9; 32], 9_900))
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_msg_secrets_round_trip_with_ts_and_isolate() {
    msg_secrets_round_trip_with_ts_and_isolate(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_msg_secrets_round_trip_with_ts_and_isolate() {
    msg_secrets_round_trip_with_ts_and_isolate(harness::sqlite().await).await;
}

/// A redelivery merges into the existing row: the later deadline wins, `0`
/// ("never") beats any deadline, a `message_ts` of `0` ("unknown") never
/// clobbers a known one, and the secret itself is replaced. The deadline is
/// only observable through the sweep, so the sweep is the probe.
async fn msg_secret_upsert_never_shortens_retention(h: Harness) {
    let t = h.two_accounts("msg-secret-merge").await;
    t.ba.put_msg_secrets(vec![
        entry("LONGER", 1, 100, 0),
        entry("NEVER", 1, 100, 0),
        entry("TS", 1, 0, 1_700),
    ])
    .await
    .unwrap();
    t.ba.put_msg_secrets(vec![
        entry("LONGER", 2, 50, 0),
        entry("NEVER", 2, 0, 0),
        entry("TS", 3, 0, 0),
    ])
    .await
    .unwrap();

    assert_eq!(
        t.ba.get_msg_secret(CHAT, ME, "LONGER").await.unwrap(),
        Some(vec![2; 32])
    );
    assert_eq!(
        t.ba.get_msg_secret_with_ts(CHAT, ME, "TS").await.unwrap(),
        Some((vec![3; 32], 1_700)),
        "the secret moves, the known message_ts stays"
    );
    assert_eq!(
        t.ba.delete_expired_msg_secrets(75).await.unwrap(),
        0,
        "LONGER kept its 100 deadline, NEVER became 0"
    );
    assert_eq!(t.ba.delete_expired_msg_secrets(100).await.unwrap(), 1);
    assert_eq!(t.ba.get_msg_secret(CHAT, ME, "LONGER").await.unwrap(), None);
    assert_eq!(
        t.ba.get_msg_secret(CHAT, ME, "NEVER").await.unwrap(),
        Some(vec![2; 32])
    );

    // And the reverse order: a shorter deadline arriving first still loses.
    t.ba.put_msg_secrets(vec![entry("LATER", 1, 50, 0)])
        .await
        .unwrap();
    t.ba.put_msg_secrets(vec![entry("LATER", 1, 500, 0)])
        .await
        .unwrap();
    assert_eq!(t.ba.delete_expired_msg_secrets(100).await.unwrap(), 0);
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_msg_secret_upsert_never_shortens_retention() {
    msg_secret_upsert_never_shortens_retention(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_msg_secret_upsert_never_shortens_retention() {
    msg_secret_upsert_never_shortens_retention(harness::sqlite().await).await;
}

/// `expires_at = 0` is "never" and is skipped by the sweep; a deadline equal to
/// the cutoff has passed (`<=`), one after it has not.
async fn expired_msg_secrets_skip_never_and_respect_cutoff(h: Harness) {
    const CUTOFF: i64 = 1_000;
    let t = h.two_accounts("msg-secret-expiry").await;
    t.ba.put_msg_secrets(vec![
        entry("NEVER", 1, 0, 0),
        entry("BEFORE", 1, CUTOFF - 1, 0),
        entry("AT", 1, CUTOFF, 0),
        entry("AFTER", 1, CUTOFF + 1, 0),
    ])
    .await
    .unwrap();
    t.bb.put_msg_secrets(vec![entry("BEFORE", 9, CUTOFF - 1, 0)])
        .await
        .unwrap();

    assert_eq!(t.ba.delete_expired_msg_secrets(CUTOFF).await.unwrap(), 2);
    for (id, survives) in [
        ("NEVER", true),
        ("BEFORE", false),
        ("AT", false),
        ("AFTER", true),
    ] {
        assert_eq!(
            t.ba.get_msg_secret(CHAT, ME, id).await.unwrap().is_some(),
            survives,
            "{id}"
        );
    }
    assert_eq!(
        t.bb.get_msg_secret(CHAT, ME, "BEFORE").await.unwrap(),
        Some(vec![9; 32]),
        "a sweep on A must not reach B"
    );
    h.drop_accounts(t).await;
}

#[tokio::test]
async fn postgres_expired_msg_secrets_skip_never_and_respect_cutoff() {
    expired_msg_secrets_skip_never_and_respect_cutoff(harness::postgres().await).await;
}

#[tokio::test]
async fn sqlite_expired_msg_secrets_skip_never_and_respect_cutoff() {
    expired_msg_secrets_skip_never_and_respect_cutoff(harness::sqlite().await).await;
}
