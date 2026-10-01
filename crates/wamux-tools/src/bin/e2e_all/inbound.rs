//! The opt-in reception window (`WAMUX_E2E_INBOUND=1`): the person at
//! WAMUX_LIVE_DEST sends this account a text and an image, then DownloadMedia fetches the image. Off by
//! default, so a run with nobody at the phone neither waits nor fails here.

use std::time::Duration;

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::media_service_client::MediaServiceClient;
use wamux_tools::live_env::same_user;
use wamux_tools::report::Report;

use crate::E2eCtx;

const WINDOW: Duration = Duration::from_secs(60);

pub async fn run_inbound_window(channel: &Channel, report: &mut Report, ctx: &E2eCtx) {
    println!(
        "\n>>> From {}, send this account a TEXT and an IMAGE now. Waiting up to 60s ...\n",
        ctx.dest
    );
    let found = ctx
        .tap
        .wait_for(WINDOW, |e| inbound_media_of(e, &ctx.dest).is_some())
        .await;
    let any_inbound = found.is_some() || has_inbound_message(ctx);
    report.verify(
        "Event reception (inbound msg)",
        any_inbound,
        format!("inbound message seen={any_inbound}"),
    );
    let descriptor = found.as_ref().and_then(|e| inbound_media_of(e, &ctx.dest));
    let Some(descriptor) = descriptor else {
        return report.fail(
            "Media.DownloadMedia",
            "no inbound media received in the window",
        );
    };
    let mut media = MediaServiceClient::new(channel.clone());
    download(&mut media, report, ctx, descriptor).await;
}

/// A message the person at WAMUX_LIVE_DEST sent in their direct chat. Our own
/// sends echo back as messages too (issue #22), so `from_me` ones never count,
/// and neither does any other chat's traffic (a group or a channel posting
/// during the window is unrelated). A `@lid` sender is matched through
/// `sender_alt`, the phone jid the stanza itself carried; when the stanza has
/// no counterpart the message does not match, because the core relays the
/// pair verbatim and guessing a lid-to-phone mapping here would be inventing
/// one.
fn received_message<'a>(
    envelope: &'a pb::EventEnvelope,
    dest: &str,
) -> Option<&'a pb::InboundMessage> {
    let Some(pb::event_envelope::Event::Message(m)) = &envelope.event else {
        return None;
    };
    let from_other = m.key.as_ref().is_some_and(|key| !key.from_me);
    let direct = !m.chat.ends_with("@g.us") && !m.chat.ends_with("@newsletter");
    let from_dest = same_user(&m.sender, dest) || same_user(&m.sender_alt, dest);
    (from_other && direct && from_dest).then_some(m)
}

fn inbound_media_of(envelope: &pb::EventEnvelope, dest: &str) -> Option<pb::MediaDescriptor> {
    received_message(envelope, dest).and_then(|m| m.media.clone())
}

fn has_inbound_message(ctx: &E2eCtx) -> bool {
    ctx.tap
        .seen()
        .iter()
        .any(|e| received_message(e, &ctx.dest).is_some())
}

async fn download(
    media: &mut MediaServiceClient<Channel>,
    report: &mut Report,
    ctx: &E2eCtx,
    descriptor: pb::MediaDescriptor,
) {
    let request = pb::DownloadMediaRequest {
        account: Some(ctx.acct.clone()),
        descriptor: Some(descriptor),
    };
    let mut stream = match media.download_media(request).await {
        Ok(response) => response.into_inner(),
        Err(e) => return report.fail("Media.DownloadMedia", e.to_string()),
    };
    let mut total = 0usize;
    loop {
        match stream.message().await {
            Ok(Some(chunk)) => {
                if let Some(pb::media_chunk::Part::Chunk(bytes)) = chunk.part {
                    total += bytes.len();
                }
            }
            Ok(None) => break,
            Err(e) => return report.fail("Media.DownloadMedia", e.to_string()),
        }
    }
    report.verify("Media.DownloadMedia", total > 0, format!("{total} bytes"));
}
