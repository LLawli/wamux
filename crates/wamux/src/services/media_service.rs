//! MediaService: lazy download of received media from a descriptor, streamed
//! back as a meta frame followed by byte chunks.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use wamux_types::DownloadableMedia;

use super::{client_of, missing_field};
use crate::domain::media_transfer;
use crate::proto::v1 as pb;
use crate::proto::v1::media_service_server::MediaService;
use crate::spool::{MediaSpool, SpooledDownload};
use crate::state::AccountRegistry;

const CHUNK_SIZE: usize = 64 * 1024;

pub struct MediaSvc {
    registry: Arc<AccountRegistry>,
    spool: MediaSpool,
    media_max_bytes: u64,
}

impl MediaSvc {
    pub fn new(registry: Arc<AccountRegistry>, spool: MediaSpool, media_max_bytes: u64) -> Self {
        Self {
            registry,
            spool,
            media_max_bytes,
        }
    }
}

#[tonic::async_trait]
impl MediaService for MediaSvc {
    type DownloadMediaStream = ReceiverStream<Result<pb::MediaChunk, Status>>;

    async fn download_media(
        &self,
        request: Request<pb::DownloadMediaRequest>,
    ) -> Result<Response<Self::DownloadMediaStream>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let media = DownloadableMedia::try_from(
            req.descriptor.ok_or_else(|| missing_field("descriptor"))?,
        )?;

        // Decrypted into the spool, not memory (#79). Nothing is sent until it
        // verified, so a failure still means no frame at all (#69).
        let spooled =
            media_transfer::download(&client, &self.spool, &media, self.media_max_bytes).await?;
        let (tx, rx) = mpsc::channel(2);
        tokio::spawn(stream_spooled(tx, Arc::new(spooled), media.mime_type));
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Meta frame, then the file in `CHUNK_SIZE` pieces, one read at a time so the
/// channel's depth bounds memory. The task owns the download, so its
/// reservation drops when the stream ends or the client goes away (#79).
async fn stream_spooled(
    tx: mpsc::Sender<Result<pb::MediaChunk, Status>>,
    spooled: Arc<SpooledDownload>,
    mime_type: String,
) {
    let meta = pb::MediaChunk {
        part: Some(pb::media_chunk::Part::Meta(pb::MediaMeta {
            mime_type,
            file_length: spooled.len(),
        })),
    };
    if tx.send(Ok(meta)).await.is_err() {
        return;
    }
    let mut offset: u64 = 0;
    while offset < spooled.len() {
        let frame = match spooled.read_chunk(offset, CHUNK_SIZE).await {
            Ok(bytes) => Ok(pb::MediaChunk {
                part: Some(pb::media_chunk::Part::Chunk(bytes)),
            }),
            Err(err) => Err(Status::from(err)),
        };
        let failed = frame.is_err();
        offset += CHUNK_SIZE as u64;
        if tx.send(frame).await.is_err() || failed {
            return;
        }
    }
}
