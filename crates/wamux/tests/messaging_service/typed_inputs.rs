//! What #115 changes on the wire, and only for input that was malformed or
//! spelled the legacy way: the messaging domain's inputs became typed, so a
//! jid that used to relay unparsed now parses, and an empty id is refused
//! (decided 2026-10-05). Well-formed input goes out as it always did; the
//! rest of this suite pins that.

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::sent_message;
use crate::harness::{
    Fixture, PEER_PN_USER, assert_only_the_probe_was_sent, fixture, key, sender, text, wire_count,
};

/// Each call refused with `code`, and when `message` is given, with exactly it.
fn assert_each_refused(results: Vec<(String, Result<(), tonic::Status>)>, message: Option<&str>) {
    for (case, result) in results {
        let status = result.expect_err(&case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
        if let Some(message) = message {
            assert_eq!(status.message(), message, "{case}");
        }
    }
}

/// The five calls that act on a message by its key: reaction, edit, revoke,
/// delete for me, star.
async fn act_on(
    f: &mut Fixture,
    target: pb::MessageKey,
) -> Vec<(String, Result<(), tonic::Status>)> {
    let a = f.a();
    let m = &mut f.messages;
    let t = || Some(target.clone());
    vec![
        (
            "SendReaction".to_string(),
            m.send_reaction(pb::SendReactionRequest {
                account: a.clone(),
                target: t(),
                emoji: "x".into(),
            })
            .await
            .map(drop),
        ),
        (
            "EditMessage".to_string(),
            m.edit_message(pb::EditMessageRequest {
                account: a.clone(),
                target: t(),
                new_text: "x".into(),
            })
            .await
            .map(drop),
        ),
        (
            "DeleteMessage for everyone".to_string(),
            m.delete_message(pb::DeleteMessageRequest {
                account: a.clone(),
                target: t(),
                for_everyone: true,
            })
            .await
            .map(drop),
        ),
        (
            "DeleteMessage for me".to_string(),
            m.delete_message(pb::DeleteMessageRequest {
                account: a.clone(),
                target: t(),
                for_everyone: false,
            })
            .await
            .map(drop),
        ),
        (
            "StarMessage".to_string(),
            m.star_message(pb::StarMessageRequest {
                account: a.clone(),
                target: t(),
                starred: true,
            })
            .await
            .map(drop),
        ),
    ]
}

// Before #115 an empty id reached the library, which acted on a message named
// "". Now it is the edge's mistake, said before anything reaches the wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_target_with_an_empty_id_is_refused() {
    let mut f = fixture("a_target_with_an_empty_id_is_refused").await;
    let before = wire_count(&f);
    let peer = f.peer.pn().to_string();
    let results = act_on(&mut f, key(&peer, "")).await;
    assert_each_refused(results, Some("empty message id"));
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// Mentions and participants relayed unparsed before #115. They are jids, so
// a malformed one is now InvalidArgument, and nothing is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_mention_or_participant_is_refused() {
    let mut f = fixture("a_malformed_mention_or_participant_is_refused").await;
    let before = wire_count(&f);
    let peer = f.peer.pn().to_string();
    let mention = pb::SendTextRequest {
        mentions: vec![pb::Mention {
            jid: "not a jid".into(),
        }],
        ..text(&f, &peer, "@x")
    };
    let quote_participant = pb::SendTextRequest {
        quote: Some(pb::QuoteContext {
            quoted: Some(key(&peer, "3EB0QUOTED")),
            participant: "not a jid".into(),
        }),
        ..text(&f, &peer, "re")
    };
    let mut results = vec![
        (
            "a malformed mention".to_string(),
            f.messages.send_text(mention).await.map(drop),
        ),
        (
            "a malformed quote participant".to_string(),
            f.messages.send_text(quote_participant).await.map(drop),
        ),
    ];
    let target = pb::MessageKey {
        participant: "not a jid".into(),
        ..key(&peer, "3EB0TARGET")
    };
    for (rpc, result) in act_on(&mut f, target).await {
        results.push((format!("{rpc}: a malformed target participant"), result));
    }
    assert_each_refused(results, None);
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// A quote that has a key must name the quoted message: an empty id quoted
// nothing and still upgraded the text before #115.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quote_with_an_empty_id_is_refused() {
    let mut f = fixture("a_quote_with_an_empty_id_is_refused").await;
    let before = wire_count(&f);
    let peer = f.peer.pn().to_string();
    let request = pb::SendTextRequest {
        quote: Some(pb::QuoteContext {
            quoted: Some(key(&peer, "")),
            participant: String::new(),
        }),
        ..text(&f, &peer, "re")
    };
    let result = f.messages.send_text(request).await.map(drop);
    assert_each_refused(
        vec![("SendText".to_string(), result)],
        Some("empty quote.quoted.id; expected the id of the quoted message"),
    );
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// The library's parse is the one normalization (#114): a `@c.us` mention
// reaches the peer in the phone namespace, as a `@c.us` recipient already did.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_mention_relays_in_the_phone_namespace() {
    let mut f = fixture("a_legacy_mention_relays_in_the_phone_namespace").await;
    let peer = f.peer.pn().to_string();
    let request = pb::SendTextRequest {
        mentions: vec![pb::Mention {
            jid: format!("{PEER_PN_USER}@c.us"),
        }],
        ..text(&f, &peer, "@peer")
    };
    let sent = f
        .messages
        .send_text(request)
        .await
        .expect("a legacy mention is still a mention")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let extended = opened.extended_text_message.as_option().expect("extended");
    let context = extended.context_info.as_option().expect("context");
    assert_eq!(
        context.mentioned_jid,
        vec![format!("{PEER_PN_USER}@s.whatsapp.net")]
    );
    f.cleanup().await;
}

// A quote with no key quotes nothing. Before #115 it still upgraded the text
// to an extendedTextMessage with no context; now it is no quote at all, and
// the text goes as the plain conversation it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quote_without_a_key_sends_plain_text() {
    let mut f = fixture("a_quote_without_a_key_sends_plain_text").await;
    let peer = f.peer.pn().to_string();
    let request = pb::SendTextRequest {
        quote: Some(pb::QuoteContext {
            quoted: None,
            participant: String::new(),
        }),
        ..text(&f, &peer, "sem citação")
    };
    let sent = f
        .messages
        .send_text(request)
        .await
        .expect("send")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    assert_eq!(opened.conversation.as_deref(), Some("sem citação"));
    assert!(opened.extended_text_message.is_unset());
    f.cleanup().await;
}
