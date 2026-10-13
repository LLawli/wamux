//! The download side of the spool (#79): the library decrypts into an unnamed
//! spool file, and only after it verified the media does anything go out.

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;

use anyhow::Context;
use wacore::download::DownloadWriter;

use super::upload::run_blocking;
use super::{MediaSpool, SpoolReservation};
use crate::error::WamuxError;

/// A spool file that refuses to grow past what was reserved for it, so a CDN
/// that sends more than the descriptor promised cannot outgrow the budget.
pub struct BoundedFile {
    file: File,
    position: u64,
    limit: u64,
}

impl BoundedFile {
    fn new(file: File, limit: u64) -> Self {
        Self {
            file,
            position: 0,
            limit,
        }
    }

    pub fn into_file(self) -> File {
        self.file
    }
}

impl Write for BoundedFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let end = self.position.saturating_add(buf.len() as u64);
        if end > self.limit {
            return Err(io::Error::other(format!(
                "media is larger than the {} bytes reserved for it",
                self.limit
            )));
        }
        let written = self.file.write(buf)?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for BoundedFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.position = self.file.seek(to)?;
        Ok(self.position)
    }
}

impl DownloadWriter for BoundedFile {
    fn truncate(&mut self, len: u64) -> io::Result<()> {
        self.file.set_len(len)
    }
}

/// A download's spool file and the bytes reserved for it.
pub struct DownloadSlot {
    pub writer: BoundedFile,
    pub reservation: SpoolReservation,
}

/// A verified download, staged: read it back in pieces.
pub struct SpooledDownload {
    file: File,
    len: u64,
    // Held until the stream ends: the file still occupies the disk.
    _reservation: SpoolReservation,
}

impl MediaSpool {
    /// Reserve `bytes`, then open a file that may hold no more than that. The
    /// reservation comes first so a refusal for the budget costs no file and no
    /// I/O; the file is created off the runtime, being disk I/O.
    pub async fn begin_download(&self, bytes: u64) -> Result<DownloadSlot, WamuxError> {
        let reservation = self.budget().try_reserve(bytes)?;
        let spool = self.clone();
        let file = run_blocking(move || Ok(spool.create_file()?)).await?;
        Ok(DownloadSlot {
            writer: BoundedFile::new(file, bytes),
            reservation,
        })
    }
}

impl SpooledDownload {
    /// The finished download: the writer the library handed back, measured.
    pub fn seal(writer: BoundedFile, reservation: SpoolReservation) -> Result<Self, WamuxError> {
        let file = writer.into_file();
        let len = file.metadata().context("measuring the spool file")?.len();
        Ok(Self {
            file,
            len,
            _reservation: reservation,
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Up to `max` bytes at `offset`; empty at the end. Positioned, so the
    /// reads need no seek and no shared offset.
    pub async fn read_chunk(
        self: &std::sync::Arc<Self>,
        offset: u64,
        max: usize,
    ) -> Result<Vec<u8>, WamuxError> {
        let spooled = std::sync::Arc::clone(self);
        run_blocking(move || {
            let want = (spooled.len.saturating_sub(offset)).min(max as u64) as usize;
            let mut chunk = vec![0u8; want];
            spooled
                .file
                .read_exact_at(&mut chunk, offset)
                .context("reading the spool file")?;
            Ok(chunk)
        })
        .await
    }
}
