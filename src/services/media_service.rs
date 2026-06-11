//! MediaService: lazy download of received media from a descriptor, streamed
//! back as a meta frame followed by byte chunks.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use super::{client_of, require_field};
use crate::domain::media_transfer;
use crate::proto::v1 as pb;
use crate::proto::v1::media_service_server::MediaService;
use crate::state::AccountRegistry;

const CHUNK_SIZE: usize = 64 * 1024;

pub struct MediaSvc {
    registry: Arc<AccountRegistry>,
}

impl MediaSvc {
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self { registry }
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
        let descriptor = require_field(req.descriptor, "descriptor")?;

        // Eager decrypt-to-memory, then stream out in chunks.
        let data = media_transfer::download(&client, &descriptor).await?;
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let meta = pb::MediaChunk {
                part: Some(pb::media_chunk::Part::Meta(pb::MediaMeta {
                    mime_type: descriptor.mime_type.clone(),
                    file_length: data.len() as u64,
                })),
            };
            if tx.send(Ok(meta)).await.is_err() {
                return;
            }
            for chunk in data.chunks(CHUNK_SIZE) {
                let frame = pb::MediaChunk {
                    part: Some(pb::media_chunk::Part::Chunk(chunk.to_vec())),
                };
                if tx.send(Ok(frame)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
