//! Status / story posting. Thin relays over `client.status()`: the edge picks
//! the recipient device set and supplies any media bytes + thumbnail; the core
//! uploads and posts, deciding no privacy policy of its own.

use wamux_types::{Jid, MediaKind, StatusMedia, StatusRevoke, StatusText};
use whatsapp_rust::buffa::Enumeration;
use whatsapp_rust::waproto::whatsapp::message::extended_text_message::FontType;
use whatsapp_rust::{Client, Jid as LibJid, SendResult, StatusSendOptions};

use crate::domain::account_setting_update::status_privacy_of;
use crate::domain::media_transfer;
use crate::domain::messaging::lib_message_id;
use crate::domain::wire_defaults::nonempty_string;
use crate::error::{WamuxError, client_err};
use crate::spool::StagedUpload;

/// Post a text status. `background_argb`/`font` relay verbatim (0 is a valid
/// transparent background, so it is NOT mapped away). Recipients are the device
/// set the status is encrypted to — the edge composes that list.
pub async fn post_status_text(
    client: &Client,
    status: &StatusText,
) -> Result<SendResult, WamuxError> {
    let recipients = lib_recipients(&status.recipients);
    // The font is a closed enum, not a bare i32. Reject an out-of-schema number
    // rather than quietly falling back to SYSTEM: the edge asked for a specific
    // font and deserves to hear that it does not exist.
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
            // Privacy defaults to "contacts" and the operation id is generated
            // by the library; per-status privacy lists are edge policy, not yet
            // a wire field, so the core posts with the lib default.
            StatusSendOptions::default(),
        )
        .await
        .map_err(client_err)
}

/// Post an image or video status. Uploads the spooled bytes (same path as
/// `media_transfer::send_media`), then posts via the lib's typed status sender.
/// `seconds` is the video duration; it is ignored for an image.
pub async fn post_status_media(
    client: &Client,
    status: &StatusMedia,
    staged: StagedUpload,
) -> Result<SendResult, WamuxError> {
    let kind = status.kind;
    let recipients = lib_recipients(&status.recipients);
    let upload = media_transfer::upload_staged(client, staged, kind.media_type()).await?;
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
        // match exhaustive over all seven `MediaKind` values (#114).
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
    let message_id = lib_message_id(&revoke.message_id.to_string())?;
    client
        .status()
        .revoke(
            message_id,
            &lib_recipients(&revoke.recipients),
            StatusSendOptions::default(),
        )
        .await
        .map_err(client_err)
}

/// The status audience the library last synced (#89). A sync read of the
/// device snapshot; `None` (no audience yet) crosses as an unset `audience`,
/// never as a default that would read as "all contacts".
pub fn status_audience(client: &Client) -> crate::proto::v1::StatusAudienceResponse {
    crate::proto::v1::StatusAudienceResponse {
        audience: client
            .status()
            .audience()
            .map(|action| status_privacy_of(&action)),
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
