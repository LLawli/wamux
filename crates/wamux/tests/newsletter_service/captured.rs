//! The channel reads as WhatsApp's server answered them on 2026-10-02,
//! transcribed from `captured-2026-10-02.xml` (anonymized; see its header).
//! Each function builds the CHILD of the `<iq>`, which is what the mock wraps.
//! `captured_answers_render_exactly_as_received` proves every transcription
//! renders back to its captured line, so a typo here cannot pass for a server
//! answer.

use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::assert_transcribes;

pub const CHANNEL: &str = "120363000000000070@newsletter";
/// A well-formed channel jid with no channel behind it.
pub const MISSING: &str = "120363000000000099@newsletter";
pub const NAME: &str = "wamux capture 70";
pub const DESCRIPTION: &str = "Anonymized channel for the wamux #70 capture.";
pub const PICTURE: &str = "/m1/v/t24/anonymized-picture?stp=dst-jpg_tt6";
pub const PREVIEW: &str = "/m1/v/t24/anonymized-picture?stp=dst-jpg_s192x192_tt6";
pub const CREATION: i64 = 1_688_746_895;
pub const SUBSCRIBERS: u64 = 234_310_777;
/// The poll this account voted in and then cleared, as `my_addons` reports it.
pub const VOTED_POLL: u64 = 777;
pub const VOTE_CLEARED_AT: i64 = 1_790_454_233;
pub const LIVE_SECONDS: u64 = 90;
/// The history IQ for a channel that does not exist.
pub const MISSING_CODE: u16 = 404;
pub const MISSING_TEXT: &str = "item-not-found";

/// ListSubscribedNewsletters (`w:mex`), trimmed to the capture channel. The
/// list entry carries no `subscribers_count` and a null `picture`.
pub const LIST_JSON: &str = r#"{"data":{"xwa2_newsletter_subscribed":[{"id":"120363000000000070@newsletter","state":{"type":"ACTIVE"},"thread_metadata":{"creation_time":"1688746895","description":{"id":"1689653839450668","text":"Anonymized channel for the wamux #70 capture.","update_time":"1689653839450668"},"handle":null,"invite":"0029AbCdEfGhIjKlMnOpQrSt","name":{"id":"1688746895480511","text":"wamux capture 70","update_time":"1688746895480511"},"picture":null,"preview":{"direct_path":"\/m1\/v\/t24\/anonymized-picture?stp=dst-jpg_s192x192_tt6","id":"1790189174236026","type":"PREVIEW"},"settings":{"reaction_codes":{"value":"ALL"}},"verification":"VERIFIED"},"viewer_metadata":{"role":"SUBSCRIBER","settings":[{"type":"MUTE_FOLLOWER_ACTIVITY","value":"OFF"},{"type":"MUTE_ADMIN_ACTIVITY","value":"ON"}]}}]}}"#;

/// GetNewsletterMetadata (`w:mex`) for the capture channel.
pub const METADATA_JSON: &str = r#"{"data":{"xwa2_newsletter":{"id":"120363000000000070@newsletter","state":{"type":"ACTIVE"},"thread_metadata":{"creation_time":"1688746895","description":{"id":"1689653839450668","text":"Anonymized channel for the wamux #70 capture.","update_time":"1689653839450668"},"handle":null,"invite":"0029AbCdEfGhIjKlMnOpQrSt","name":{"id":"1688746895480511","text":"wamux capture 70","update_time":"1688746895480511"},"picture":{"direct_path":"\/m1\/v\/t24\/anonymized-picture?stp=dst-jpg_tt6","id":"1790189174236026","type":"IMAGE"},"preview":{"direct_path":"\/m1\/v\/t24\/anonymized-picture?stp=dst-jpg_s192x192_tt6","id":"1790189174236026","type":"PREVIEW"},"settings":{"reaction_codes":{"value":"ALL"}},"subscribers_count":"234310777","verification":"VERIFIED"},"viewer_metadata":{"role":"SUBSCRIBER","settings":[{"type":"MUTE_FOLLOWER_ACTIVITY","value":"OFF"},{"type":"MUTE_ADMIN_ACTIVITY","value":"ON"}]}}}}"#;

/// GetNewsletterMetadata (`w:mex`) for a jid with no channel behind it.
pub const MISSING_JSON: &str = r#"{"data":{"xwa2_newsletter":{"id":null,"state":{"type":"NON_EXISTING"},"thread_metadata":null,"viewer_metadata":null}}}"#;

/// The `<result>` a MEX answer is: its JSON as bytes.
pub fn mex_result(json: &str) -> Node {
    NodeBuilder::new("result")
        .bytes(json.as_bytes().to_vec())
        .build()
}

/// One history row as captured: `(server_id, id, t, forwards, mediatype,
/// three reactions)`. The capture kept 301 reactions a row; three are enough.
pub type Row = (
    u64,
    &'static str,
    i64,
    u64,
    &'static str,
    [(&'static str, u64); 3],
);

pub const ROWS: [Row; 5] = [
    (
        778,
        "AC09908EB2D2E7DADB3067915EED323E",
        1_790_172_737,
        7638,
        "gif",
        [
            ("\u{1F621}", 437),
            ("\u{1F981}", 11),
            ("\u{1F1EC}\u{1F1F3}", 13),
        ],
    ),
    (
        779,
        "ACD54555CDFD6916C76B62D556EAD3F3",
        1_790_359_379,
        11107,
        "image",
        [("\u{2721}", 18), ("\u{1F621}", 646), ("\u{1F628}", 18)],
    ),
    (
        780,
        "3A12626C3E0F2F0C07DF",
        1_790_704_027,
        4032,
        "image",
        [
            ("\u{1F621}", 109),
            ("\u{1F981}", 9),
            ("\u{1F1EC}\u{1F1F3}", 21),
        ],
    ),
    (
        781,
        "3AB6E59C3F41D3197CC1",
        1_790_704_079,
        49068,
        "ptt",
        [
            ("\u{1F621}", 987),
            ("\u{1F1EC}\u{1F1F3}", 196),
            ("\u{1FA93}", 44),
        ],
    ),
    (
        782,
        "AC91B00AE2C1F81CAE82AD0A44348C55",
        1_790_784_528,
        4913,
        "gif",
        [("\u{1F621}", 262), ("\u{1F981}", 6), ("\u{1F694}", 7)],
    ),
];

pub fn media_path(server_id: u64) -> String {
    format!("/m1/v/t62.7161-24/anonymized-{server_id}")
}

pub fn media_caption(server_id: u64) -> String {
    format!("wamux capture 70: {server_id}")
}

/// The body of a row. The log keeps only a `<plaintext>`'s length, so each one
/// is a neutral message of the row's `mediatype`, keyless as channel media is.
pub fn payload(server_id: u64, mediatype: &str) -> wa::Message {
    let path = Some(media_path(server_id));
    let sha = Some(vec![server_id as u8; 32]);
    let caption = Some(media_caption(server_id));
    match mediatype {
        "image" => wa::Message {
            image_message: MessageField::some(wa::message::ImageMessage {
                mimetype: Some("image/jpeg".into()),
                direct_path: path,
                file_sha256: sha,
                file_length: Some(120_000),
                caption,
                ..Default::default()
            }),
            ..Default::default()
        },
        "gif" => wa::Message {
            video_message: MessageField::some(wa::message::VideoMessage {
                mimetype: Some("video/mp4".into()),
                direct_path: path,
                file_sha256: sha,
                file_length: Some(240_000),
                caption,
                gif_playback: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        },
        _ => wa::Message {
            audio_message: MessageField::some(wa::message::AudioMessage {
                mimetype: Some("audio/ogg; codecs=opus".into()),
                direct_path: path,
                file_sha256: sha,
                file_length: Some(36_000),
                ptt: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        },
    }
}

fn row_node(row: &Row) -> Node {
    let (server_id, id, t, forwards, mediatype, reactions) = *row;
    let reactions = reactions.iter().map(|(code, count)| {
        NodeBuilder::new("reaction")
            .attr("code", *code)
            .attr("count", count.to_string())
            .build()
    });
    NodeBuilder::new("message")
        .attr("server_id", server_id.to_string())
        .attr("id", id)
        .attr("type", "media")
        .attr("t", t.to_string())
        .children([
            NodeBuilder::new("forwards_count")
                .attr("count", forwards.to_string())
                .build(),
            NodeBuilder::new("reactions").children(reactions).build(),
            NodeBuilder::new("plaintext")
                .attr("mediatype", mediatype)
                .bytes(payload(server_id, mediatype).encode_to_vec())
                .build(),
        ])
        .build()
}

/// GetNewsletterMessages: the five newest rows of the capture channel.
pub fn history_page() -> Node {
    NodeBuilder::new("messages")
        .attr("jid", CHANNEL)
        .children(ROWS.iter().map(row_node))
        .build()
}

/// GetMyNewsletterAddOns: one poll vote, cleared (a dated `<votes/>` with no
/// `<vote>` under it).
pub fn my_addons() -> Node {
    NodeBuilder::new("my_addons")
        .children([NodeBuilder::new("messages")
            .attr("jid", CHANNEL)
            .children([NodeBuilder::new("message")
                .attr("server_id", VOTED_POLL.to_string())
                .children([NodeBuilder::new("votes")
                    .attr("t", VOTE_CLEARED_AT.to_string())
                    .build()])
                .build()])
            .build()])
        .build()
}

/// SubscribeNewsletterLiveUpdates.
pub fn live_updates() -> Node {
    NodeBuilder::new("live_updates")
        .attr("duration", LIVE_SECONDS.to_string())
        .build()
}

/// The `<error>` the history IQ drew for a missing channel, as the mock's
/// `answer_iq_error` writes it.
fn missing_history_error() -> Node {
    NodeBuilder::new("error")
        .attr("code", MISSING_CODE.to_string())
        .attr("text", MISSING_TEXT)
        .build()
}

#[test]
fn captured_answers_render_exactly_as_received() {
    let file = include_str!("captured-2026-10-02.xml");
    let lines: Vec<&str> = file.lines().filter(|l| l.starts_with("<iq ")).collect();
    let built = [
        mex_result(LIST_JSON),
        mex_result(METADATA_JSON),
        history_page(),
        my_addons(),
        live_updates(),
        mex_result(MISSING_JSON),
        missing_history_error(),
    ];
    assert_eq!(lines.len(), built.len(), "one captured <iq> per read");
    for (line, node) in lines.iter().zip(built.iter()) {
        assert_transcribes(line, node);
    }
}
