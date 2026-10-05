//! Status posts and their revoke (#115). Validated by tests and the mock
//! only: no live test posts a status.

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::jid::Jid;
use crate::media_kind::MediaKind;
use crate::message_id::MessageId;

/// A text status. `background_argb` 0 is a valid transparent background, and
/// `font` stays a raw number the domain checks against the library's enum.
/// `recipients` is the device set the status is encrypted to; an empty one is
/// the library's to refuse (#101).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusText {
    pub text: String,
    pub background_argb: u32,
    pub font: i32,
    pub recipients: Vec<Jid>,
}

/// An image or video status. `seconds` is a video's duration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusMedia {
    pub kind: MediaKind,
    pub recipients: Vec<Jid>,
    pub caption: String,
    pub thumbnail: Vec<u8>,
    pub seconds: u32,
}

/// A status revoke, checked for shape before any account is looked up (#41).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusRevoke {
    pub message_id: MessageId,
    pub recipients: Vec<Jid>,
}

impl TryFrom<pb::PostStatusTextRequest> for StatusText {
    type Error = WamuxError;

    fn try_from(request: pb::PostStatusTextRequest) -> Result<Self, WamuxError> {
        Ok(Self {
            recipients: parse_recipients(&request.recipients)?,
            text: request.text,
            background_argb: request.background_argb,
            font: request.font,
        })
    }
}

/// The kind first (`MediaKind::parse_status`), then the recipients.
impl TryFrom<pb::PostStatusMediaHeader> for StatusMedia {
    type Error = WamuxError;

    fn try_from(header: pb::PostStatusMediaHeader) -> Result<Self, WamuxError> {
        let kind = MediaKind::parse_status(&header.media_type)?;
        Ok(Self {
            kind,
            recipients: parse_recipients(&header.recipients)?,
            caption: header.caption,
            thumbnail: header.thumbnail,
            seconds: header.seconds,
        })
    }
}

/// An empty id or recipient list would reach the library and come back as an
/// opaque client error, which maps to Unavailable; the edge sent a malformed
/// request and hears that, with today's messages:
/// `"message_id is empty; expected the status's own id (PostStatus* key.id)"`,
/// `"recipients is empty; expected the device set the status was posted to"`.
impl TryFrom<pb::RevokeStatusRequest> for StatusRevoke {
    type Error = WamuxError;

    fn try_from(request: pb::RevokeStatusRequest) -> Result<Self, WamuxError> {
        if request.message_id.is_empty() {
            return Err(WamuxError::InvalidArgument(
                "message_id is empty; expected the status's own id (PostStatus* key.id)"
                    .to_string(),
            ));
        }
        if request.recipients.is_empty() {
            return Err(WamuxError::InvalidArgument(
                "recipients is empty; expected the device set the status was posted to".to_string(),
            ));
        }
        Ok(Self {
            message_id: MessageId::new(request.message_id)?,
            recipients: parse_recipients(&request.recipients)?,
        })
    }
}

fn parse_recipients(values: &[String]) -> Result<Vec<Jid>, WamuxError> {
    values.iter().map(|value| Jid::parse(value)).collect()
}
