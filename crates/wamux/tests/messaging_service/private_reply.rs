//! A reply that quotes a message from another chat carries that chat as the
//! context's `remoteJid` (#217): a private reply to a group message, sent to
//! the author in a direct chat. A reply in the same chat carries none, the
//! way WhatsApp's clients send it. Proved on what the peer opened.

use wamux::proto::v1 as pb;

use crate::common::mock_wire::sent_message;
use crate::harness::{Fixture, fixture, jid, sender, text};
use crate::media::{chunk, head, header, plaintext, send_media};

const GROUP: &str = "120363041234567217@g.us";
const QUOTED_ID: &str = "3EB0QUOTED217";

/// A quote of `QUOTED_ID`, written by the peer, in `chat`.
fn quoting(f: &Fixture, chat: &str) -> Option<pb::QuoteContext> {
    Some(pb::QuoteContext {
        quoted: Some(pb::MessageKey {
            chat: jid(chat),
            id: QUOTED_ID.to_string(),
            from_me: false,
            participant: jid(f.peer.pn()),
        }),
        participant: None,
    })
}

/// Send a text quoting a message in `quoted_chat` to the peer and return the
/// context the peer opened.
async fn text_context(
    f: &mut Fixture,
    quoted_chat: &str,
) -> whatsapp_rust::waproto::whatsapp::ContextInfo {
    let request = pb::SendTextRequest {
        quote: quoting(f, quoted_chat),
        ..text(f, f.peer.pn(), "respondendo")
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
    extended.context_info.as_option().expect("context").clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_reply_reaches_the_peer_with_remote_jid() {
    let mut f = fixture("private_reply_reaches_the_peer_with_remote_jid").await;
    let context = text_context(&mut f, GROUP).await;
    assert_eq!(context.remote_jid.as_deref(), Some(GROUP));
    assert_eq!(context.stanza_id.as_deref(), Some(QUOTED_ID));
    assert_eq!(
        context.participant.as_deref(),
        Some(f.peer.pn().to_string().as_str())
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_chat_reply_carries_no_remote_jid() {
    let mut f = fixture("same_chat_reply_carries_no_remote_jid").await;
    let dm = f.peer.pn().to_string();
    let context = text_context(&mut f, &dm).await;
    assert_eq!(
        context.remote_jid, None,
        "the quoted message is in this chat"
    );
    assert_eq!(context.stanza_id.as_deref(), Some(QUOTED_ID));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_reply_with_private_quote_carries_remote_jid() {
    let mut f = fixture("media_reply_with_private_quote_carries_remote_jid").await;
    let quote = quoting(&f, GROUP);
    let with_quote = pb::SendMediaHeader {
        quote,
        ..header(&f, pb::MediaType::Image)
    };
    let plain = plaintext(10_000);
    let sent = send_media(&mut f, vec![head(with_quote), chunk(&plain)])
        .await
        .expect("send media");
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let image = opened.image_message.as_option().expect("imageMessage");
    let context = image.context_info.as_option().expect("context");
    assert_eq!(context.remote_jid.as_deref(), Some(GROUP));
    assert_eq!(context.stanza_id.as_deref(), Some(QUOTED_ID));
    f.cleanup().await;
}
