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

use crate::domain::media_transfer::StagedMedia;
use crate::error::WamuxError;
use crate::proto::v1 as pb;
use crate::spool::{MediaSpool, SpooledUpload, StagedUpload};

/// The header of either upload stream.
pub(super) enum MediaHeader {
    /// Boxed: the sound sticker field grew it past the other variant (clippy `large_enum_variant`).
    Send(Box<pb::SendMediaHeader>),
    Status(pb::PostStatusMediaHeader),
}

/// One chunk of either stream, as the domain sees it.
enum MediaFrame {
    /// Boxed: the headers are hundreds of bytes next to a byte chunk's 24.
    Header(Box<MediaHeader>),
    Bytes(Vec<u8>),
    /// A sound sticker's clip (#278); only SendMedia streams carry one.
    StickerAudio(Vec<u8>),
    /// A chunk with no `part` set: skipped after the header, refused as the first chunk.
    Empty,
}

impl From<pb::SendMediaChunk> for MediaFrame {
    fn from(chunk: pb::SendMediaChunk) -> Self {
        match chunk.part {
            Some(pb::send_media_chunk::Part::Header(h)) => {
                Self::Header(Box::new(MediaHeader::Send(Box::new(h))))
            }
            Some(pb::send_media_chunk::Part::Chunk(bytes)) => Self::Bytes(bytes),
            Some(pb::send_media_chunk::Part::StickerAudio(bytes)) => Self::StickerAudio(bytes),
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
            Self::Send(header) => Ok(*header),
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

    /// PostStatusMedia's upload: one file, no clip part.
    pub(super) async fn stage_upload(
        &mut self,
        spool: &MediaSpool,
        media_type: MediaType,
        max_bytes: u64,
    ) -> Result<StagedUpload, Status> {
        Ok(self
            .stage_media(spool, media_type, max_bytes, false)
            .await?
            .media)
    }

    /// Encrypt the inline byte chunks (after the header) into the spool, up
    /// to the byte limit on the plaintext (#79). Memory stays one chunk wide:
    /// the whole file is never held, and the budget is asked chunk by chunk,
    /// so a stream that does not fit is refused mid-stream with nothing sent.
    ///
    /// A sound sticker's clip (#278) is a second file in the same stream, its
    /// parts interleaved with the image's in any order, so each file has its
    /// own spooled upload, byte count and limit. `accepts_sticker_audio` is
    /// whether the header promised a clip.
    pub(super) async fn stage_media(
        &mut self,
        spool: &MediaSpool,
        media_type: MediaType,
        max_bytes: u64,
        accepts_sticker_audio: bool,
    ) -> Result<StagedMedia, Status> {
        let mut file = SpooledFile::begin(spool, media_type)?;
        let mut clip: Option<SpooledFile> = None;
        while let Some(frame) = self.next_frame().await? {
            match frame {
                MediaFrame::Bytes(bytes) => file.push(bytes, max_bytes).await?,
                MediaFrame::StickerAudio(bytes) if accepts_sticker_audio => {
                    let lane = match clip.as_mut() {
                        Some(lane) => lane,
                        None => clip.insert(SpooledFile::begin(spool, MediaType::Audio)?),
                    };
                    lane.push(bytes, max_bytes).await?;
                }
                MediaFrame::StickerAudio(_) => return Err(clip_without_header().into()),
                MediaFrame::Header(_) => {
                    return Err(WamuxError::InvalidArgument(
                        "unexpected second header".to_string(),
                    )
                    .into());
                }
                MediaFrame::Empty => {}
            }
        }
        let sticker_audio = finish_clip(clip, accepts_sticker_audio).await?;
        Ok(StagedMedia {
            media: file.upload.finish().await?,
            sticker_audio,
        })
    }
}

/// One file of the stream: its spooled upload and the plaintext bytes so far.
struct SpooledFile {
    upload: SpooledUpload,
    received: u64,
}

impl SpooledFile {
    fn begin(spool: &MediaSpool, media_type: MediaType) -> Result<Self, WamuxError> {
        Ok(Self {
            upload: spool.begin_upload(media_type)?,
            received: 0,
        })
    }

    /// The limit is per file (`media_max_bytes`); the spool budget, which is
    /// global, is what bounds the two together.
    async fn push(&mut self, bytes: Vec<u8>, max_bytes: u64) -> Result<(), WamuxError> {
        self.received += bytes.len() as u64;
        if self.received > max_bytes {
            return Err(size_limit_exceeded());
        }
        self.upload.push(bytes).await
    }
}

/// A promised clip must have arrived with at least one byte, or nothing is
/// sent: a sound sticker with an empty clip would ship a mute sticker.
async fn finish_clip(
    clip: Option<SpooledFile>,
    promised: bool,
) -> Result<Option<StagedUpload>, WamuxError> {
    match clip {
        Some(lane) if lane.received > 0 => Ok(Some(lane.upload.finish().await?)),
        _ if promised => Err(WamuxError::InvalidArgument(
            "sticker_audio in the header but the stream carried no clip bytes".to_string(),
        )),
        _ => Ok(None),
    }
}

fn clip_without_header() -> WamuxError {
    WamuxError::InvalidArgument(
        "sticker_audio part sent but the header has no sticker_audio".to_string(),
    )
}

fn size_limit_exceeded() -> WamuxError {
    WamuxError::ResourceExhausted("media exceeds size limit".to_string())
}
