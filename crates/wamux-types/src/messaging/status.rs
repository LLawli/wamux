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
/// refused here, before any account is looked up (#101).
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
            recipients: require_recipients(&request.recipients, POSTED_TO)?,
            text: request.text,
            background_argb: request.background_argb,
            font: request.font,
        })
    }
}

/// The kind first (`MediaKind::parse_status`), then the recipients (#101).
impl TryFrom<pb::PostStatusMediaHeader> for StatusMedia {
    type Error = WamuxError;

    fn try_from(header: pb::PostStatusMediaHeader) -> Result<Self, WamuxError> {
        let kind = MediaKind::parse_status(header.media_type)?;
        Ok(Self {
            kind,
            recipients: require_recipients(&header.recipients, POSTED_TO)?,
            caption: header.caption,
            thumbnail: header.thumbnail,
            seconds: header.seconds,
        })
    }
}

/// The id first, then the recipients: an empty one of either is the edge's
/// malformed request and is refused here as InvalidArgument.
impl TryFrom<pb::RevokeStatusRequest> for StatusRevoke {
    type Error = WamuxError;

    fn try_from(request: pb::RevokeStatusRequest) -> Result<Self, WamuxError> {
        if request.message_id.is_empty() {
            return Err(WamuxError::InvalidArgument(
                "message_id is empty; expected the status's own id (PostStatus* key.id)"
                    .to_string(),
            ));
        }
        let recipients = require_recipients(&request.recipients, WAS_POSTED_TO)?;
        Ok(Self {
            message_id: MessageId::new(request.message_id)?,
            recipients,
        })
    }
}

const POSTED_TO: &str = "is posted to";
const WAS_POSTED_TO: &str = "was posted to";

/// A status with no recipients encrypts for nobody; the lib refuses it with an
/// error that read as Unavailable (#101), so the shape check lives here.
/// `verb` keeps the revoke's past tense ("the status was posted to").
fn require_recipients(values: &[pb::Jid], verb: &str) -> Result<Vec<Jid>, WamuxError> {
    if values.is_empty() {
        return Err(WamuxError::InvalidArgument(format!(
            "recipients is empty; expected the device set the status {verb}"
        )));
    }
    parse_recipients(values)
}

fn parse_recipients(values: &[pb::Jid]) -> Result<Vec<Jid>, WamuxError> {
    // Each recipient is required: an unset or empty one is "missing jid" (#120).
    values
        .iter()
        .map(|value| Jid::from_required_wire(Some(value.clone())))
        .collect()
}
