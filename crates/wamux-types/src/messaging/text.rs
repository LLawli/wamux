//! An outgoing text message (#115).

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::messaging::key::OutgoingContext;

/// A link preview the edge fetched. Relayed field for field: an empty string
/// or a zero is the absent field, which the domain's builder decides.
/// `preview_type` stays a raw number here; the domain checks it against the
/// library's enum (#73 owns closed enums).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkPreview {
    pub matched_text: String,
    pub title: String,
    pub description: String,
    pub jpeg_thumbnail: Vec<u8>,
    pub preview_type: i32,
}

/// The content of a SendText: its routing (`account`, `to`) is the service's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutgoingText {
    pub text: String,
    pub context: OutgoingContext,
    pub link_preview: Option<LinkPreview>,
}

impl From<pb::LinkPreview> for LinkPreview {
    fn from(preview: pb::LinkPreview) -> Self {
        Self {
            matched_text: preview.matched_text,
            title: preview.title,
            description: preview.description,
            jpeg_thumbnail: preview.jpeg_thumbnail,
            preview_type: preview.preview_type,
        }
    }
}

impl TryFrom<pb::SendTextRequest> for OutgoingText {
    type Error = WamuxError;

    fn try_from(request: pb::SendTextRequest) -> Result<Self, WamuxError> {
        Ok(Self {
            context: OutgoingContext::from_proto(
                request.mentions,
                request.quote,
                request.ephemeral_seconds,
            )?,
            link_preview: request.link_preview.map(LinkPreview::from),
            text: request.text,
        })
    }
}
