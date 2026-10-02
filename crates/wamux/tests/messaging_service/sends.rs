//! The DM and group sends: what the peer opens is what the RPC was asked to
//! send, and the stanza around it is the one the library addresses.

use wamux::proto::v1 as pb;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::{attr, sent_message};
use crate::harness::{GROUP, fixture, jid, key, send_to_peer, sender, text};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_text_reaches_the_peer_as_the_text_sent() {
    let mut f = fixture("send_text_reaches_the_peer_as_the_text_sent").await;
    let request = text(&f, f.peer.pn(), "bom dia");
    let answer = f
        .messages
        .send_text(request)
        .await
        .expect("send")
        .into_inner();
    let sent = answer.key.clone().expect("key");
    assert_eq!(sent.remote_jid, f.peer.pn().to_string());
    assert!(sent.from_me && !sent.id.is_empty(), "{sent:?}");
    let fanout = answer.recipient_fanout.expect("a DM reports its fan-out");
    assert!(fanout.addressed >= 1, "{fanout:?}");
    assert_eq!(fanout.encrypted, fanout.addressed, "{fanout:?}");
    assert!(!fanout.skipped_primary, "{fanout:?}");

    let stanza = sent_message(&f.mock, &sent.id).await;
    assert_eq!(attr(&stanza, "type").as_deref(), Some("text"));
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    assert_eq!(opened.conversation.as_deref(), Some("bom dia"));
    f.cleanup().await;
}

// Anything beyond the text upgrades it to an extendedTextMessage, and every
// part rides as the edge sent it: mentions verbatim, the quote by id and
// participant, the preview's fields, the ephemeral timer as expiration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_text_composes_mentions_quote_preview_and_ephemeral() {
    let mut f = fixture("send_text_composes_mentions_quote_preview_and_ephemeral").await;
    let peer = f.peer.pn().to_string();
    let request = pb::SendTextRequest {
        mentions: vec![pb::Mention { jid: peer.clone() }],
        quote: Some(pb::QuoteContext {
            quoted: Some(key(&peer, "3EB0QUOTED")),
            participant: peer.clone(),
        }),
        link_preview: Some(pb::LinkPreview {
            matched_text: "https://example.com/a".into(),
            title: "A title".into(),
            description: "A description".into(),
            jpeg_thumbnail: vec![0xff, 0xd8, 0xff],
            preview_type: 0,
        }),
        ephemeral_seconds: 86_400,
        ..text(&f, &peer, "veja https://example.com/a")
    };
    let sent = f
        .messages
        .send_text(request)
        .await
        .expect("send")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let extended = opened.extended_text_message.as_option().expect("extended");
    assert_eq!(extended.text.as_deref(), Some("veja https://example.com/a"));
    assert_eq!(
        extended.matched_text.as_deref(),
        Some("https://example.com/a")
    );
    assert_eq!(extended.title.as_deref(), Some("A title"));
    assert_eq!(extended.description.as_deref(), Some("A description"));
    assert_eq!(
        extended.jpeg_thumbnail.as_deref(),
        Some(&[0xff, 0xd8, 0xff][..])
    );
    assert_eq!(extended.preview_type, None, "0 relays as absent");
    let context = extended.context_info.as_option().expect("context");
    assert_eq!(context.mentioned_jid, vec![peer.clone()]);
    assert_eq!(context.stanza_id.as_deref(), Some("3EB0QUOTED"));
    assert_eq!(context.participant.as_deref(), Some(peer.as_str()));
    assert_eq!(context.expiration, Some(86_400));
    f.cleanup().await;
}

// A linked device's send reaches the account's own phone too, wrapped as a
// deviceSentMessage naming where it went.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_text_fans_out_to_own_phone() {
    let mut f = fixture("send_text_fans_out_to_own_phone").await;
    let sent = send_to_peer(&mut f, "bom dia").await;
    let stanza = sent_message(&f.mock, &sent.id).await;
    let opened = f
        .phone
        .open_dm(&stanza, &sender())
        .await
        .expect("phone opens");
    let device_sent = opened
        .device_sent_message
        .as_option()
        .expect("deviceSentMessage");
    assert_eq!(
        device_sent.destination_jid.as_deref(),
        Some(f.peer.pn().to_string().as_str())
    );
    let inner = device_sent.message.as_option().expect("inner message");
    assert_eq!(inner.conversation.as_deref(), Some("bom dia"));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_reaction_reaches_the_peer() {
    let mut f = fixture("send_reaction_reaches_the_peer").await;
    let target = send_to_peer(&mut f, "bom dia").await;
    let request = pb::SendReactionRequest {
        account: f.a(),
        target: Some(target.clone()),
        emoji: "\u{1F44D}".into(),
    };
    let sent = f
        .messages
        .send_reaction(request)
        .await
        .expect("react")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    assert_eq!(attr(&stanza, "type").as_deref(), Some("reaction"));
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let reaction = opened.reaction_message.as_option().expect("reaction");
    assert_eq!(reaction.text.as_deref(), Some("\u{1F44D}"));
    assert_eq!(
        wa_key_of(reaction.key.as_option()),
        (target.remote_jid, target.id, true)
    );
    f.cleanup().await;
}

/// `(remote_jid, id, from_me)` of a wa key, for one comparison.
fn wa_key_of(key: Option<&wa::MessageKey>) -> (String, String, bool) {
    let key = key.expect("a key");
    (
        key.remote_jid.clone().unwrap_or_default(),
        key.id.clone().unwrap_or_default(),
        key.from_me.unwrap_or_default(),
    )
}

// The edit is its own stanza (`edit="1"`) carrying a MESSAGE_EDIT protocol
// message. The answer names the chat as the caller wrote it and the edit
// stanza's own id.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_message_reaches_the_peer_as_an_edit() {
    let mut f = fixture("edit_message_reaches_the_peer_as_an_edit").await;
    let target = send_to_peer(&mut f, "bom dia").await;
    let request = pb::EditMessageRequest {
        account: f.a(),
        target: Some(target.clone()),
        new_text: "boa tarde".into(),
    };
    let answer = f
        .messages
        .edit_message(request)
        .await
        .expect("edit")
        .into_inner();
    let answered = answer.key.expect("key");
    assert_eq!(answered.remote_jid, target.remote_jid);
    assert_ne!(answered.id, target.id, "the edit stanza has its own id");
    let stanza = sent_message(&f.mock, &answered.id).await;
    assert_eq!(attr(&stanza, "edit").as_deref(), Some("1"));
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let protocol = opened
        .protocol_message
        .as_option()
        .expect("protocolMessage");
    assert_eq!(
        protocol.r#type,
        Some(wa::message::protocol_message::Type::MessageEdit)
    );
    assert_eq!(wa_key_of(protocol.key.as_option()).1, target.id);
    let edited = protocol.edited_message.as_option().expect("editedMessage");
    assert_eq!(edited.conversation.as_deref(), Some("boa tarde"));
    f.cleanup().await;
}

// A revoke for everyone is a stanza with `edit="7"` and a REVOKE protocol
// message naming the target. The answer is the target's own key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_message_for_everyone_reaches_the_peer_as_a_revoke() {
    let mut f = fixture("delete_message_for_everyone_reaches_the_peer_as_a_revoke").await;
    let target = send_to_peer(&mut f, "bom dia").await;
    let before = f.mock.client_messages().len();
    let request = pb::DeleteMessageRequest {
        account: f.a(),
        target: Some(target.clone()),
        for_everyone: true,
    };
    let answer = f
        .messages
        .delete_message(request)
        .await
        .expect("revoke")
        .into_inner();
    assert_eq!(answer.key, Some(target.clone()));
    assert!(answer.recipient_fanout.is_some(), "a revoke is a send");
    let stanza = crate::harness::newest_stanza(&f.mock, "message", before).await;
    assert_eq!(attr(&stanza, "edit").as_deref(), Some("7"));
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let protocol = opened
        .protocol_message
        .as_option()
        .expect("protocolMessage");
    assert_eq!(
        protocol.r#type,
        Some(wa::message::protocol_message::Type::Revoke)
    );
    assert_eq!(wa_key_of(protocol.key.as_option()).1, target.id);
    f.cleanup().await;
}

// A group send is one skmsg for everyone, with the sender key distributed to
// each member in its own pkmsg: both members open the same text.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_text_to_a_group_reaches_every_member() {
    let mut f = fixture("send_text_to_a_group_reaches_every_member").await;
    let request = pb::SendTextRequest {
        to: jid(GROUP),
        ..text(&f, GROUP, "bom dia, grupo")
    };
    let sent = f
        .messages
        .send_text(request)
        .await
        .expect("send")
        .into_inner();
    let sent = sent.key.expect("key");
    assert_eq!(sent.remote_jid, GROUP);
    let stanza = sent_message(&f.mock, &sent.id).await;
    assert_eq!(attr(&stanza, "to").as_deref(), Some(GROUP));
    for member in [&f.peer, &f.other] {
        let opened = member.open_group(&stanza, &sender()).await.expect("open");
        assert_eq!(opened.conversation.as_deref(), Some("bom dia, grupo"));
    }
    f.cleanup().await;
}
