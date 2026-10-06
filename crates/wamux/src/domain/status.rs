//! Status / story posting. Thin relays over `client.status()`: the edge picks
//! the recipient device set and supplies any media bytes + thumbnail; the core
//! uploads and posts, deciding no privacy policy of its own.

use wamux_types::{Jid, MediaKind, StatusMedia, StatusRevoke, StatusText};
use whatsapp_rust::buffa::Enumeration;
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::waproto::whatsapp::message::extended_text_message::FontType;
use whatsapp_rust::{Client, Jid as LibJid, SendResult, StatusSendOptions};

use crate::domain::wire_defaults::nonempty_string;
use crate::error::{WamuxError, client_err};

/// Post a text status. `background_argb`/`font` relay verbatim (0 is a valid
/// transparent background, so it is NOT mapped away). Recipients are the device
/// set the status is encrypted to — the edge composes that list.
pub async fn post_status_text(
    client: &Client,
    status: &StatusText,
) -> Result<SendResult, WamuxError> {
    let recipients = lib_recipients(&status.recipients);
    // 0.7 types the font as a closed enum instead of a bare i32. Reject an
    // out-of-schema number rather than quietly falling back to SYSTEM: the edge
    // asked for a specific font and deserves to hear that it does not exist.
    let font = FontType::from_i32(status.font).ok_or_else(|| {
        WamuxError::InvalidArgument(format!(
            "unknown status font {}; expected a wa FontType value",
            status.font
        ))
    })?;
    client
        .status()
        .send_text(
            &status.text,
            status.background_argb,
            font,
            &recipients,
            // Privacy is the only StatusSendOptions field and defaults to
            // "contacts"; per-status privacy lists are edge policy, not yet a
            // wire field, so the core posts with the lib default.
            StatusSendOptions::default(),
        )
        .await
        .map_err(client_err)
}

/// Post an image or video status. Uploads the streamed bytes (same path as
/// `media_transfer::send_media`), then posts via the lib's typed status sender.
/// `seconds` is the video duration; it is ignored for an image.
pub async fn post_status_media(
    client: &Client,
    status: &StatusMedia,
    data: Vec<u8>,
) -> Result<SendResult, WamuxError> {
    let kind = status.kind;
    let recipients = lib_recipients(&status.recipients);
    let upload = client
        .upload(data, kind.media_type(), UploadOptions::new())
        .await
        .map_err(client_err)?;
    let caption = nonempty_string(&status.caption);
    let sender = client.status();
    let result = match kind {
        MediaKind::Image => {
            sender
                .send_image(
                    upload,
                    status.thumbnail.clone(),
                    caption.as_deref(),
                    &recipients,
                    StatusSendOptions::default(),
                )
                .await
        }
        MediaKind::Video => {
            sender
                .send_video(
                    upload,
                    status.thumbnail.clone(),
                    status.seconds,
                    caption.as_deref(),
                    &recipients,
                    StatusSendOptions::default(),
                )
                .await
        }
        // `parse_status` only yields the two kinds above; the arm keeps the
        // match exhaustive now that `MediaKind` has seven (#114).
        other => {
            return Err(other.refused_as_status());
        }
    };
    result.map_err(client_err)
}

/// The library borrows the recipients as its own type.
fn lib_recipients(recipients: &[Jid]) -> Vec<LibJid> {
    recipients.iter().map(|jid| jid.as_lib().clone()).collect()
}

/// Revoke a status this account posted. Not the chat revoke: the library's
/// `send_message` refuses `status@broadcast`, and a status revoke is encrypted
/// to the status's own recipients, which only the edge knows (issue #41).
pub async fn revoke_status(
    client: &Client,
    revoke: StatusRevoke,
) -> Result<SendResult, WamuxError> {
    client
        .status()
        .revoke(
            revoke.message_id.to_string(),
            &lib_recipients(&revoke.recipients),
            StatusSendOptions::default(),
        )
        .await
        .map_err(client_err)
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
