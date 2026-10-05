//! #64: a send passes on delivery, never on the ack (CLAUDE.md, #47).

use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux_tools::delivery::{EventTap, is_delivery_receipt, send_reached_phone};

fn sent(id: &str, fanout: Option<pb::RecipientFanout>) -> pb::SendResult {
    pb::SendResult {
        key: Some(pb::MessageKey {
            chat: Some(pb::Jid {
                value: "5561900000001@s.whatsapp.net".into(),
            }),
            id: id.into(),
            from_me: true,
            participant: None,
        }),
        server_timestamp: 1,
        recipient_fanout: fanout,
    }
}

fn fanout(addressed: u32, encrypted: u32, skipped_primary: bool) -> pb::RecipientFanout {
    pb::RecipientFanout {
        addressed,
        encrypted,
        skipped_primary,
        had_unregistered_device: false,
    }
}

fn receipt(kind: &str, ids: &[&str]) -> pb::EventEnvelope {
    pb::EventEnvelope {
        account_uuid: "acct".into(),
        event: Some(pb::event_envelope::Event::Receipt(pb::ReceiptEvent {
            chat: "5561900000001@s.whatsapp.net".into(),
            sender: "5561900000001@s.whatsapp.net".into(),
            message_ids: ids.iter().map(|s| s.to_string()).collect(),
            r#type: kind.into(),
            timestamp: 1,
        })),
        ..Default::default()
    }
}

#[test]
fn a_fanout_that_reached_the_phone_returns_the_id() {
    assert_eq!(
        send_reached_phone(&sent("ABC", Some(fanout(2, 2, false)))),
        Ok("ABC".to_string())
    );
}

#[test]
fn a_partial_fanout_that_still_reached_the_phone_passes() {
    assert_eq!(
        send_reached_phone(&sent("ABC", Some(fanout(3, 1, false)))),
        Ok("ABC".to_string())
    );
}

#[test]
fn a_missing_fanout_is_not_a_dm_delivery() {
    let why = send_reached_phone(&sent("ABC", None)).unwrap_err();
    assert!(why.contains("recipient_fanout"), "{why}");
}

#[test]
fn a_fanout_that_encrypted_for_nobody_fails() {
    assert!(send_reached_phone(&sent("ABC", Some(fanout(0, 0, false)))).is_err());
}

#[test]
fn a_fanout_that_skipped_the_phone_fails() {
    let why = send_reached_phone(&sent("ABC", Some(fanout(2, 1, true)))).unwrap_err();
    assert!(why.contains("phone") || why.contains("primary"), "{why}");
}

#[test]
fn an_empty_message_id_fails_even_with_a_good_fanout() {
    assert!(send_reached_phone(&sent("", Some(fanout(1, 1, false)))).is_err());
}

#[test]
fn a_missing_key_fails() {
    let mut result = sent("ABC", Some(fanout(1, 1, false)));
    result.key = None;
    assert!(send_reached_phone(&result).is_err());
}

#[test]
fn delivered_read_and_played_receipts_carrying_the_id_count() {
    for kind in ["delivered", "read", "played"] {
        assert!(
            is_delivery_receipt(&receipt(kind, &["X", "ABC"]), "ABC"),
            "{kind}"
        );
    }
}

#[test]
fn a_receipt_for_another_id_does_not_count() {
    assert!(!is_delivery_receipt(
        &receipt("delivered", &["OTHER"]),
        "ABC"
    ));
}

#[test]
fn a_sender_or_retry_receipt_does_not_count() {
    for kind in ["sender", "retry", "server-error", "read-self"] {
        assert!(
            !is_delivery_receipt(&receipt(kind, &["ABC"]), "ABC"),
            "{kind}"
        );
    }
}

#[test]
fn a_non_receipt_event_does_not_count() {
    let envelope = pb::EventEnvelope {
        event: Some(pb::event_envelope::Event::Message(
            pb::InboundMessage::default(),
        )),
        ..Default::default()
    };
    assert!(!is_delivery_receipt(&envelope, "ABC"));
}

#[tokio::test(start_paused = true)]
async fn the_tap_finds_a_receipt_that_arrived_before_the_wait() {
    let events = vec![Ok(receipt("delivered", &["ABC"]))];
    let tap = EventTap::spawn(tokio_stream::iter(events));
    assert!(tap.delivered("ABC", Duration::from_secs(5)).await);
}

#[tokio::test(start_paused = true)]
async fn the_tap_finds_a_receipt_that_arrives_during_the_wait() {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let tap = EventTap::spawn(tokio_stream::wrappers::ReceiverStream::new(rx));
    tokio::spawn(async move {
        // not a sync point: the paused clock stands in for the receipt arriving 2 s into the wait, in virtual time
        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = tx.send(Ok(receipt("delivered", &["ABC"]))).await;
    });
    assert!(tap.delivered("ABC", Duration::from_secs(10)).await);
}

#[tokio::test(start_paused = true)]
async fn the_tap_gives_up_when_the_window_closes() {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<pb::EventEnvelope, tonic::Status>>(4);
    let tap = EventTap::spawn(tokio_stream::wrappers::ReceiverStream::new(rx));
    let started = tokio::time::Instant::now();
    assert!(!tap.delivered("ABC", Duration::from_secs(30)).await);
    assert!(started.elapsed() >= Duration::from_secs(30));
    drop(tx);
}

#[tokio::test(start_paused = true)]
async fn the_tap_keeps_every_event_in_arrival_order() {
    let events = vec![
        Ok(receipt("delivered", &["A"])),
        Ok(receipt("read", &["B"])),
    ];
    let tap = EventTap::spawn(tokio_stream::iter(events));
    let found = tap
        .wait_for(Duration::from_secs(5), |e| is_delivery_receipt(e, "B"))
        .await;
    assert!(found.is_some());
    assert_eq!(tap.seen().len(), 2);
    assert!(is_delivery_receipt(&tap.seen()[0], "A"));
}
