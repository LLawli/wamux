//! The inline media upload streams of SendMedia and PostStatusMedia (#63, #114).
//!
//! The two RPCs stream different generated chunk types with the same shape: a
//! header first, then raw bytes. Each chunk is mapped to one `MediaFrame` and
//! `MediaChunks` reads either stream through one code path, so the header
//! check and the size limit are written once. A concrete enum over the two
//! streams, not a generic over the chunk type: prost generates no shared trait
//! for them, and there are exactly two.

use tonic::{Status, Streaming};
use wacore::download::MediaType;

use crate::error::WamuxError;
use crate::proto::v1 as pb;
use crate::spool::{MediaSpool, StagedUpload};

/// The header of either upload stream.
pub(super) enum MediaHeader {
    Send(pb::SendMediaHeader),
    Status(pb::PostStatusMediaHeader),
}

/// One chunk of either stream, as the domain sees it.
enum MediaFrame {
    /// Boxed: the headers are hundreds of bytes next to a byte chunk's 24.
    Header(Box<MediaHeader>),
    Bytes(Vec<u8>),
    /// A chunk with no `part` set: skipped after the header, refused as the first chunk.
    Empty,
}

impl From<pb::SendMediaChunk> for MediaFrame {
    fn from(chunk: pb::SendMediaChunk) -> Self {
        match chunk.part {
            Some(pb::send_media_chunk::Part::Header(h)) => {
                Self::Header(Box::new(MediaHeader::Send(h)))
            }
            Some(pb::send_media_chunk::Part::Chunk(bytes)) => Self::Bytes(bytes),
            None => Self::Empty,
        }
    }
}

impl From<pb::PostStatusMediaChunk> for MediaFrame {
    fn from(chunk: pb::PostStatusMediaChunk) -> Self {
        match chunk.part {
            Some(pb::post_status_media_chunk::Part::Header(h)) => {
                Self::Header(Box::new(MediaHeader::Status(h)))
            }
            Some(pb::post_status_media_chunk::Part::Chunk(bytes)) => Self::Bytes(bytes),
            None => Self::Empty,
        }
    }
}

impl MediaHeader {
    /// The SendMedia header. The other variant cannot come out of a SendMedia
    /// stream, so reaching it is a bug here, answered as an internal error.
    pub(super) fn into_send(self) -> Result<pb::SendMediaHeader, WamuxError> {
        match self {
            Self::Send(header) => Ok(header),
            Self::Status(_) => Err(header_mismatch("SendMedia")),
        }
    }

    pub(super) fn into_status(self) -> Result<pb::PostStatusMediaHeader, WamuxError> {
        match self {
            Self::Status(header) => Ok(header),
            Self::Send(_) => Err(header_mismatch("PostStatusMedia")),
        }
    }
}

fn header_mismatch(rpc: &str) -> WamuxError {
    WamuxError::Other(anyhow::anyhow!(
        "{rpc} stream carried the other RPC's header"
    ))
}

/// An upload stream of either RPC.
pub(super) enum MediaChunks {
    Send(Streaming<pb::SendMediaChunk>),
    Status(Streaming<pb::PostStatusMediaChunk>),
}

impl MediaChunks {
    async fn next_frame(&mut self) -> Result<Option<MediaFrame>, Status> {
        match self {
            Self::Send(stream) => Ok(stream.message().await?.map(MediaFrame::from)),
            Self::Status(stream) => Ok(stream.message().await?.map(MediaFrame::from)),
        }
    }

    /// The first chunk, which must be the header. `empty_stream` is the
    /// message of a stream with no chunk at all, which differs per RPC.
    pub(super) async fn read_header(
        &mut self,
        empty_stream: &'static str,
    ) -> Result<MediaHeader, Status> {
        match self.next_frame().await? {
            None => Err(WamuxError::InvalidArgument(empty_stream.to_string()).into()),
            Some(MediaFrame::Header(header)) => Ok(*header),
            Some(_) => Err(WamuxError::InvalidArgument(
                "first chunk must be the header".to_string(),
            )
            .into()),
        }
    }

    /// Encrypt the inline byte chunks (after the header) into the spool, up
    /// to the byte limit on the plaintext (#79). Memory stays one chunk wide:
    /// the whole file is never held, and the budget is asked chunk by chunk,
    /// so a stream that does not fit is refused mid-stream with nothing sent.
    pub(super) async fn stage_upload(
        &mut self,
        spool: &MediaSpool,
        media_type: MediaType,
        max_bytes: u64,
    ) -> Result<StagedUpload, Status> {
        let mut upload = spool.begin_upload(media_type)?;
        let mut received: u64 = 0;
        while let Some(frame) = self.next_frame().await? {
            match frame {
                MediaFrame::Bytes(bytes) => {
                    received += bytes.len() as u64;
                    if received > max_bytes {
                        return Err(size_limit_exceeded().into());
                    }
                    upload.push(bytes).await?;
                }
                MediaFrame::Header(_) => {
                    return Err(WamuxError::InvalidArgument(
                        "unexpected second header".to_string(),
                    )
                    .into());
                }
                MediaFrame::Empty => {}
            }
        }
        Ok(upload.finish().await?)
    }
}

fn size_limit_exceeded() -> WamuxError {
    WamuxError::ResourceExhausted("media exceeds size limit".to_string())
}
