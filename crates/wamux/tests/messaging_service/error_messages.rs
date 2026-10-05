//! The message of every error #114 reroutes, not only its code (#63, #114).
//!
//! The other suites pin the codes. #114 moves `WamuxError` into `wamux-types`,
//! sends every `Status` the services used to build by hand through it, and
//! makes `resolve` take a typed `AccountRef`. Nothing may change on the wire,
//! and a message is part of the wire: an edge matches on it today. Everything
//! here runs against the mock; no status is ever posted live.

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::common::{self, mock_wire::account_ref};
use crate::harness::{Fixture, MEDIA_LIMIT, fixture, jid};
use crate::media::{chunk, head, header, plaintext, send_media};

fn assert_refused<T: std::fmt::Debug>(
    result: Result<T, tonic::Status>,
    code: Code,
    message: &str,
    case: &str,
) {
    let status = result.expect_err(case);
    assert_eq!(status.code(), code, "{case}: {status:?}");
    assert_eq!(status.message(), message, "{case}");
}

fn text_to(account: Option<pb::AccountRef>, to: Option<pb::Jid>) -> pb::SendTextRequest {
    pb::SendTextRequest {
        account,
        to,
        text: "oi".into(),
        ..Default::default()
    }
}

async fn send_text(
    f: &mut Fixture,
    request: pb::SendTextRequest,
) -> Result<pb::SendResult, tonic::Status> {
    f.messages.send_text(request).await.map(|r| r.into_inner())
}

fn by_external(external: &str) -> Option<pb::AccountRef> {
    Some(pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external.into())),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn account_resolution_errors_keep_their_messages() {
    let mut f = fixture("account_resolution_errors_keep_their_messages").await;
    let to = jid(f.peer.pn());
    let unknown = uuid::Uuid::new_v4().to_string();
    let cases = [
        (
            "no account",
            None,
            Code::InvalidArgument,
            "missing account ref".to_string(),
        ),
        (
            "no ref set",
            Some(pb::AccountRef { r#ref: None }),
            Code::InvalidArgument,
            "missing account ref".to_string(),
        ),
        (
            "a bad uuid",
            Some(account_ref("nope")),
            Code::InvalidArgument,
            "bad uuid 'nope'".to_string(),
        ),
        (
            "an unknown uuid",
            Some(account_ref(&unknown)),
            Code::NotFound,
            format!("account {unknown} not found"),
        ),
        (
            "an unknown external ref",
            by_external("error-messages-nobody"),
            Code::NotFound,
            "account error-messages-nobody not found".to_string(),
        ),
        // #114 (user, 2026-10-05): the uuid is parsed, so the message names it
        // in canonical form, lowercase, whatever case the request used.
        (
            "an unknown uuid in uppercase",
            Some(account_ref(&unknown.to_uppercase())),
            Code::NotFound,
            format!("account {unknown} not found"),
        ),
        // An empty external ref is looked up like any other; #114 keeps it so.
        (
            "an empty external ref",
            by_external(""),
            Code::NotFound,
            "account  not found".to_string(),
        ),
    ];
    for (case, account, code, message) in cases {
        let result = send_text(&mut f, text_to(account, to.clone())).await;
        assert_refused(result, code, &message, case);
    }
    f.cleanup().await;
}

/// The one "not connected" (#63: `services/mod.rs` built it by hand next to
/// `WamuxError::NotConnected`), on a unary RPC and on a streaming one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnected_account_is_not_connected() {
    let mut f = fixture("a_disconnected_account_is_not_connected").await;
    let prefix = common::test_prefix("messaging_service", "error_messages_idle");
    common::sweep_orphans(f.logged.registry.storage(), &prefix).await;
    let idle = f
        .logged
        .registry
        .create_account(Some(&format!("{prefix}idle")))
        .await
        .expect("idle account");
    let idle_ref = Some(account_ref(&idle.uuid.to_string()));
    let to = jid(f.peer.pn());
    let text = send_text(&mut f, text_to(idle_ref.clone(), to)).await;
    assert_refused(
        text,
        Code::FailedPrecondition,
        "account is not connected",
        "SendText",
    );
    let mut media_header = header(&f, "image");
    media_header.account = idle_ref;
    let media = send_media(&mut f, vec![head(media_header), chunk(b"jpeg")]).await;
    assert_refused(
        media,
        Code::FailedPrecondition,
        "account is not connected",
        "SendMedia",
    );
    f.logged
        .registry
        .delete(&idle)
        .await
        .expect("delete the idle account");
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_or_bad_jid_keeps_its_message() {
    let mut f = fixture("a_missing_or_bad_jid_keeps_its_message").await;
    let account = f.a();
    let missing = send_text(&mut f, text_to(account.clone(), None)).await;
    assert_refused(missing, Code::InvalidArgument, "missing jid", "no jid");
    let empty = send_text(&mut f, text_to(account.clone(), jid(""))).await;
    assert_refused(empty, Code::InvalidArgument, "missing jid", "an empty jid");
    let bad = send_text(&mut f, text_to(account.clone(), jid("not a jid")))
        .await
        .expect_err("a bad jid");
    assert_eq!(bad.code(), Code::InvalidArgument, "{bad:?}");
    assert!(
        bad.message().starts_with("invalid jid 'not a jid': "),
        "{bad:?}"
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_stream_errors_keep_their_messages() {
    let mut f = fixture("send_media_stream_errors_keep_their_messages").await;
    let over = plaintext(MEDIA_LIMIT as usize + 1);
    let cases = [
        (
            "an empty stream",
            Vec::new(),
            Code::InvalidArgument,
            "empty media stream",
        ),
        (
            "a chunk first",
            vec![chunk(b"jpeg"), head(header(&f, "image"))],
            Code::InvalidArgument,
            "first chunk must be the header",
        ),
        (
            "a second header",
            vec![
                head(header(&f, "image")),
                chunk(b"jpeg"),
                head(header(&f, "image")),
            ],
            Code::InvalidArgument,
            "unexpected second header",
        ),
        (
            "over the limit",
            vec![head(header(&f, "image")), chunk(&over)],
            Code::ResourceExhausted,
            "media exceeds size limit",
        ),
        (
            "an unknown type",
            vec![head(header(&f, "gif")), chunk(b"gif")],
            Code::InvalidArgument,
            "unknown media_type 'gif'",
        ),
    ];
    for (case, frames, code, message) in cases {
        assert_refused(send_media(&mut f, frames).await, code, message, case);
    }
    f.cleanup().await;
}

fn status_head(f: &Fixture, media_type: &str) -> pb::PostStatusMediaChunk {
    pb::PostStatusMediaChunk {
        part: Some(pb::post_status_media_chunk::Part::Header(
            pb::PostStatusMediaHeader {
                account: f.a(),
                media_type: media_type.into(),
                mime_type: "image/jpeg".into(),
                recipients: vec![f.peer.pn().to_string()],
                ..Default::default()
            },
        )),
    }
}

fn status_chunk(bytes: &[u8]) -> pb::PostStatusMediaChunk {
    pb::PostStatusMediaChunk {
        part: Some(pb::post_status_media_chunk::Part::Chunk(bytes.to_vec())),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_status_media_stream_errors_keep_their_messages() {
    let mut f = fixture("post_status_media_stream_errors_keep_their_messages").await;
    let over = plaintext(MEDIA_LIMIT as usize + 1);
    let cases = [
        (
            "an empty stream",
            Vec::new(),
            Code::InvalidArgument,
            "empty status media stream".to_string(),
        ),
        (
            "a chunk first",
            vec![status_chunk(b"jpeg"), status_head(&f, "image")],
            Code::InvalidArgument,
            "first chunk must be the header".to_string(),
        ),
        (
            "a second header",
            vec![
                status_head(&f, "image"),
                status_chunk(b"jpeg"),
                status_head(&f, "image"),
            ],
            Code::InvalidArgument,
            "unexpected second header".to_string(),
        ),
        (
            "over the limit",
            vec![status_head(&f, "image"), status_chunk(&over)],
            Code::ResourceExhausted,
            "media exceeds size limit".to_string(),
        ),
        (
            "an audio status",
            vec![status_head(&f, "audio"), status_chunk(b"ogg")],
            Code::InvalidArgument,
            "status media_type must be image|video, got 'audio'".to_string(),
        ),
    ];
    for (case, frames, code, message) in cases {
        let result = f
            .messages
            .post_status_media(tokio_stream::iter(frames))
            .await
            .map(|r| r.into_inner());
        assert_refused(result, code, &message, case);
    }
    f.cleanup().await;
}
