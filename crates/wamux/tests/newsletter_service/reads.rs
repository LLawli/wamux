//! The five reads, answered with the 2026-10-02 capture: what the core relays
//! by value, and the request it builds from the RPC's fields.

use tonic::Code;
use wamux::proto::v1 as pb;
use whatsapp_rust::buffa::Message as _;

use crate::captured::{
    self, CHANNEL, CREATION, DESCRIPTION, LIST_JSON, METADATA_JSON, MISSING, MISSING_CODE,
    MISSING_JSON, MISSING_TEXT, NAME, PICTURE, PREVIEW, ROWS, Row, SUBSCRIBERS, VOTE_CLEARED_AT,
    VOTED_POLL,
};
use crate::common::mock_wire::{attr, operation, sent_iq};
use crate::harness::{NEWSLETTER, fixture};

/// The capture channel as the core relays it. Tokens are the server's own,
/// lowercased; `creation_time` stays in seconds, as the proto documents.
fn captured_channel(subscriber_count: u64, picture_url: &str) -> pb::Newsletter {
    pb::Newsletter {
        jid: Some(pb::Jid {
            value: CHANNEL.to_string(),
        }),
        name: NAME.into(),
        description: DESCRIPTION.into(),
        subscriber_count,
        picture_url: picture_url.into(),
        verification: "verified".into(),
        state: "active".into(),
        role: "subscriber".into(),
        creation_time: CREATION,
    }
}

// The list entry carries no count and a null picture: the count relays as 0
// and `picture_url` falls back to the preview's path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_subscribed_relays_the_captured_channel() {
    let mut f = fixture("list_subscribed_relays_the_captured_channel").await;
    f.mock.answer_mex_with(LIST_JSON);
    let list = f
        .channels
        .list_subscribed_newsletters(f.account.clone())
        .await
        .expect("list")
        .into_inner();
    assert_eq!(list.newsletters, vec![captured_channel(0, PREVIEW)]);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_metadata_relays_the_captured_channel() {
    let mut f = fixture("get_metadata_relays_the_captured_channel").await;
    f.mock.answer_mex_with(METADATA_JSON);
    let channel = f
        .channels
        .get_newsletter_metadata(f.jid_request(CHANNEL))
        .await
        .expect("metadata")
        .into_inner();
    assert_eq!(channel, captured_channel(SUBSCRIBERS, PICTURE));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_metadata_of_a_missing_channel_is_not_found() {
    let mut f = fixture("get_metadata_of_a_missing_channel_is_not_found").await;
    f.mock.answer_mex_with(MISSING_JSON);
    let status = f
        .channels
        .get_newsletter_metadata(f.jid_request(MISSING))
        .await
        .expect_err("no channel behind the jid");
    assert_eq!(status.code(), Code::NotFound, "{status:?}");
    f.cleanup().await;
}

/// The descriptor and caption a row's body projects to: a gif is a video and a
/// ptt an audio, which carries no caption. Keyless, as channel media is.
fn relayed_media(server_id: u64, mediatype: &str) -> (pb::MediaDescriptor, String) {
    let caption = captured::media_caption(server_id);
    let (media_type, mime_type, file_length, caption) = match mediatype {
        "image" => ("image", "image/jpeg", 120_000, caption),
        "gif" => ("video", "video/mp4", 240_000, caption),
        _ => ("audio", "audio/ogg; codecs=opus", 36_000, String::new()),
    };
    let descriptor = pb::MediaDescriptor {
        direct_path: captured::media_path(server_id),
        file_sha256: vec![server_id as u8; 32],
        file_length,
        mime_type: mime_type.into(),
        media_type: media_type.into(),
        ..Default::default()
    };
    (descriptor, caption)
}

/// One captured row as the core relays it: the body in the event bus's shape,
/// the server's tallies beside it.
fn relayed_row(row: &Row) -> pb::NewsletterMessage {
    let (server_id, id, t, forwards, mediatype, reactions) = *row;
    let (media, caption) = relayed_media(server_id, mediatype);
    pb::NewsletterMessage {
        message: Some(pb::InboundMessage {
            key: Some(pb::MessageKey {
                chat: Some(pb::Jid {
                    value: CHANNEL.into(),
                }),
                id: id.into(),
                from_me: false,
                participant: None,
            }),
            chat: Some(pb::Jid {
                value: CHANNEL.into(),
            }),
            timestamp: t * 1000,
            raw_message: captured::payload(server_id, mediatype).encode_to_vec(),
            media: Some(media),
            caption,
            ..Default::default()
        }),
        server_id,
        r#type: "media".into(),
        reactions: reactions
            .iter()
            .map(|(code, count)| pb::NewsletterReactionCount {
                code: code.to_string(),
                count: *count,
            })
            .collect(),
        votes: Vec::new(),
        forwards_count: forwards,
        poll_type: String::new(),
        edit: String::new(),
        original_timestamp: 0,
        last_edit_timestamp: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_messages_relays_the_captured_page() {
    let mut f = fixture("get_messages_relays_the_captured_page").await;
    f.mock
        .answer_iq(NEWSLETTER, "get", "messages", captured::history_page());
    let page = f
        .channels
        .get_newsletter_messages(f.history(CHANNEL, 5, 0))
        .await
        .expect("history")
        .into_inner();
    let expected: Vec<pb::NewsletterMessage> = ROWS.iter().map(relayed_row).collect();
    assert_eq!(page.messages.len(), expected.len());
    for (got, want) in page.messages.iter().zip(&expected) {
        assert_eq!(got, want, "row {}", want.server_id);
    }
    f.cleanup().await;
}

// History is addressed to the server and names the channel inside (see
// `build_newsletter_messages_iq`). `before` 0 is proto3's absence: the first
// page, so no `before` goes out; any other value is the cursor, verbatim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_messages_sends_count_and_leaves_before_zero_out() {
    let mut f = fixture("get_messages_sends_count_and_leaves_before_zero_out").await;
    f.mock
        .answer_iq(NEWSLETTER, "get", "messages", captured::history_page());
    for (before, sent_before) in [(0, None), (778, Some("778"))] {
        f.channels
            .get_newsletter_messages(f.history(CHANNEL, 5, before))
            .await
            .expect("history");
        let iq = sent_iq(&f.mock, NEWSLETTER, "get", "messages").await;
        assert_eq!(attr(&iq, "to").as_deref(), Some("s.whatsapp.net"));
        let messages = operation(&iq).expect("<messages>");
        assert_eq!(attr(messages, "type").as_deref(), Some("jid"));
        assert_eq!(attr(messages, "jid").as_deref(), Some(CHANNEL));
        assert_eq!(attr(messages, "count").as_deref(), Some("5"));
        assert_eq!(
            attr(messages, "before").as_deref(),
            sent_before,
            "before {before}"
        );
    }
    f.cleanup().await;
}

// A missing channel draws an IQ 404 on history: its code maps and the raw code
// and text ride as trailers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_messages_of_a_missing_channel_relays_the_servers_code() {
    let mut f = fixture("get_messages_of_a_missing_channel_relays_the_servers_code").await;
    f.mock
        .answer_iq_error(NEWSLETTER, "get", "messages", MISSING_CODE, MISSING_TEXT);
    let status = f
        .channels
        .get_newsletter_messages(f.history(MISSING, 5, 0))
        .await
        .expect_err("no channel behind the jid");
    assert_eq!(status.code(), Code::NotFound, "{status:?}");
    let trailer = |key| status.metadata().get(key).and_then(|v| v.to_str().ok());
    assert_eq!(trailer("wa-code"), Some("404"));
    assert_eq!(trailer("wa-text"), Some(MISSING_TEXT));
    f.cleanup().await;
}

// A cleared vote stays a present `poll_vote` with no hashes, dated in ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_my_add_ons_relays_the_captured_record() {
    let mut f = fixture("get_my_add_ons_relays_the_captured_record").await;
    f.mock
        .answer_iq(NEWSLETTER, "get", "my_addons", captured::my_addons());
    let list = f
        .channels
        .get_my_newsletter_add_ons(f.add_ons(CHANNEL, 20))
        .await
        .expect("add-ons")
        .into_inner();
    let cleared = pb::NewsletterMyAddOns {
        server_id: VOTED_POLL,
        reaction: None,
        poll_vote: Some(pb::NewsletterMyPollVote {
            timestamp: VOTE_CLEARED_AT * 1000,
            option_hashes: Vec::new(),
        }),
    };
    assert_eq!(list.messages, vec![cleared]);
    let iq = sent_iq(&f.mock, NEWSLETTER, "get", "my_addons").await;
    let request = operation(&iq).expect("<my_addons>");
    assert_eq!(attr(request, "jid").as_deref(), Some(CHANNEL));
    assert_eq!(attr(request, "limit").as_deref(), Some("20"));
    f.cleanup().await;
}
