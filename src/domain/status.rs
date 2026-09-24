//! Status / story posting. Thin relays over `client.status()`: the edge picks
//! the recipient device set and supplies any media bytes + thumbnail; the core
//! uploads and posts, deciding no privacy policy of its own.

use wacore::download::MediaType;
use whatsapp_rust::buffa::Enumeration;
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::waproto::whatsapp::message::extended_text_message::FontType;
use whatsapp_rust::{Client, Jid, SendResult, StatusSendOptions};

use crate::domain::jid_parse::parse_jids;
use crate::domain::wire_defaults::nonempty_string;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Post a text status. `background_argb`/`font` relay verbatim (0 is a valid
/// transparent background, so it is NOT mapped away). Recipients are the device
/// set the status is encrypted to — the edge composes that list.
pub async fn post_status_text(
    client: &Client,
    req: &pb::PostStatusTextRequest,
) -> Result<SendResult, WamuxError> {
    let recipients = parse_jids(&req.recipients)?;
    // 0.7 types the font as a closed enum instead of a bare i32. Reject an
    // out-of-schema number rather than quietly falling back to SYSTEM: the edge
    // asked for a specific font and deserves to hear that it does not exist.
    let font = FontType::from_i32(req.font).ok_or_else(|| {
        WamuxError::InvalidArgument(format!(
            "unknown status font {}; expected a wa FontType value",
            req.font
        ))
    })?;
    client
        .status()
        .send_text(
            &req.text,
            req.background_argb,
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

/// The two media kinds a status can carry. Status supports image and video
/// only (no audio/document/sticker), so this is a deliberately smaller set than
/// `media_transfer::MediaKind`.
enum StatusMediaKind {
    Image,
    Video,
}

impl StatusMediaKind {
    fn parse(value: &str) -> Result<Self, WamuxError> {
        match value {
            "image" => Ok(Self::Image),
            "video" => Ok(Self::Video),
            other => Err(WamuxError::InvalidArgument(format!(
                "status media_type must be image|video, got '{other}'"
            ))),
        }
    }

    fn upload_type(&self) -> MediaType {
        match self {
            Self::Image => MediaType::Image,
            Self::Video => MediaType::Video,
        }
    }
}

/// Post an image or video status. Uploads the streamed bytes (same path as
/// `media_transfer::send_media`), then posts via the lib's typed status sender.
/// `seconds` is the video duration; it is ignored for an image.
pub async fn post_status_media(
    client: &Client,
    header: &pb::PostStatusMediaHeader,
    data: Vec<u8>,
) -> Result<SendResult, WamuxError> {
    let kind = StatusMediaKind::parse(&header.media_type)?;
    let recipients = parse_jids(&header.recipients)?;
    let upload = client
        .upload(data, kind.upload_type(), UploadOptions::new())
        .await
        .map_err(client_err)?;
    let caption = nonempty_string(&header.caption);
    let status = client.status();
    let result = match kind {
        StatusMediaKind::Image => {
            status
                .send_image(
                    upload,
                    header.thumbnail.clone(),
                    caption.as_deref(),
                    &recipients,
                    StatusSendOptions::default(),
                )
                .await
        }
        StatusMediaKind::Video => {
            status
                .send_video(
                    upload,
                    header.thumbnail.clone(),
                    header.seconds,
                    caption.as_deref(),
                    &recipients,
                    StatusSendOptions::default(),
                )
                .await
        }
    };
    result.map_err(client_err)
}

/// A status revoke, checked for shape before any account is looked up.
pub struct StatusRevoke {
    pub message_id: String,
    pub recipients: Vec<Jid>,
}

/// Check a RevokeStatus request (issue #41). An empty id or recipient list
/// would reach the library and come back as an opaque client error, which maps
/// to `Unavailable`; the edge sent a malformed request and should hear that.
pub fn parse_status_revoke(req: &pb::RevokeStatusRequest) -> Result<StatusRevoke, WamuxError> {
    if req.message_id.is_empty() {
        return Err(WamuxError::InvalidArgument(
            "message_id is empty; expected the status's own id (PostStatus* key.id)".to_string(),
        ));
    }
    if req.recipients.is_empty() {
        return Err(WamuxError::InvalidArgument(
            "recipients is empty; expected the device set the status was posted to".to_string(),
        ));
    }
    Ok(StatusRevoke {
        message_id: req.message_id.clone(),
        recipients: parse_jids(&req.recipients)?,
    })
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
            revoke.message_id,
            &revoke.recipients,
            StatusSendOptions::default(),
        )
        .await
        .map_err(client_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_media_kind_parses_image_and_video() {
        assert!(matches!(
            StatusMediaKind::parse("image").unwrap().upload_type(),
            MediaType::Image
        ));
        assert!(matches!(
            StatusMediaKind::parse("video").unwrap().upload_type(),
            MediaType::Video
        ));
    }

    // Audio/document/sticker are valid for a normal message but NOT for a
    // status; the edge sending one is an InvalidArgument, not a silent fallback.
    #[test]
    fn status_media_kind_rejects_non_status_kinds() {
        for bad in ["audio", "document", "sticker", "gif"] {
            assert!(
                matches!(
                    StatusMediaKind::parse(bad),
                    Err(WamuxError::InvalidArgument(_))
                ),
                "expected {bad} to be rejected"
            );
        }
    }

    fn revoke_request(message_id: &str, recipients: &[&str]) -> pb::RevokeStatusRequest {
        pb::RevokeStatusRequest {
            account: None,
            message_id: message_id.to_string(),
            recipients: recipients.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn status_revoke_keeps_the_id_and_every_recipient() {
        let req = revoke_request(
            "3EB0STATUS",
            &["5511999000111@s.whatsapp.net", "169815004184633@lid"],
        );
        let revoke = parse_status_revoke(&req).unwrap();
        assert_eq!(revoke.message_id, "3EB0STATUS");
        assert_eq!(revoke.recipients.len(), 2);
    }

    #[test]
    fn status_revoke_refuses_an_empty_id_or_recipient_list() {
        let bad = [
            revoke_request("", &["5511999000111@s.whatsapp.net"]),
            revoke_request("3EB0STATUS", &[]),
            revoke_request("3EB0STATUS", &[""]),
        ];
        for req in bad {
            assert!(
                matches!(
                    parse_status_revoke(&req),
                    Err(WamuxError::InvalidArgument(_))
                ),
                "expected {req:?} to be refused"
            );
        }
    }
}
