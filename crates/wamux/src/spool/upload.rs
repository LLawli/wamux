//! The upload side of the spool (#79): plaintext chunks go through the media
//! encryptor straight into an unnamed spool file, so the plaintext never
//! touches disk and the ciphertext never sits whole in memory.

use std::fs::File;

use anyhow::Context;
use wacore::download::MediaType;
use wacore::upload::{EncryptedMediaInfo, MediaEncryptor};

use super::{MediaSpool, SpoolReservation, SpooledCiphertext};
use crate::error::WamuxError;

/// The most the ciphertext can exceed the plaintext by: up to 16 bytes of
/// PKCS7 padding plus the 10-byte MAC. Reserved before the final write so the
/// budget never undercounts the file.
const CIPHERTEXT_OVERHEAD: u64 = 26;

/// An upload being staged. Dropping it (an error, or the client giving up
/// mid-stream) closes the unnamed file and gives the reservation back.
pub struct SpooledUpload {
    stage: Option<Stage>,
    reservation: SpoolReservation,
    /// Plaintext bytes received so far; the ciphertext is at most this plus
    /// `CIPHERTEXT_OVERHEAD`.
    plaintext: u64,
}

/// What the blocking task needs; moved into it and back, never shared.
struct Stage {
    encryptor: MediaEncryptor,
    file: File,
}

/// A finished upload: the ciphertext source, what the library needs to send
/// it, and the reservation, which has to outlive the upload to the CDN.
pub struct StagedUpload {
    pub source: SpooledCiphertext,
    pub info: EncryptedMediaInfo,
    pub reservation: SpoolReservation,
}

impl MediaSpool {
    /// Start staging an upload. The reservation starts empty and grows chunk
    /// by chunk (#79). The sidecar is wanted for AUDIO and VIDEO only, the same
    /// default `encrypt_media_with_key_and_sidecar(.., None)` applies; without
    /// it a voice note draws its bubble and will not play (#18).
    pub fn begin_upload(&self, media_type: MediaType) -> Result<SpooledUpload, WamuxError> {
        let sidecar = matches!(media_type, MediaType::Audio | MediaType::Video);
        let encryptor = MediaEncryptor::new_with_sidecar(media_type, sidecar)
            .context("starting the media encryptor")?;
        Ok(SpooledUpload {
            stage: Some(Stage {
                encryptor,
                file: self.create_file()?,
            }),
            plaintext: 0,
            reservation: self.budget().try_reserve(0)?,
        })
    }

    /// A new unnamed spool file (`O_TMPFILE`): it has no directory entry from
    /// the first byte, so a crash cannot leave one behind.
    pub(super) fn create_file(&self) -> Result<File, WamuxError> {
        let file = tempfile::tempfile_in(self.dir())
            .with_context(|| format!("creating a spool file in {}", self.dir().display()))?;
        Ok(file)
    }
}

impl SpooledUpload {
    /// Encrypt one plaintext chunk into the spool file. The budget is asked
    /// first, so a refusal happens before any disk is used.
    pub async fn push(&mut self, chunk: Vec<u8>) -> Result<(), WamuxError> {
        self.plaintext += chunk.len() as u64;
        self.cover(self.plaintext)?;
        let stage = self.take_stage()?;
        let stage = run_blocking(move || {
            let mut stage = stage;
            stage
                .encryptor
                .update_to_writer(&chunk, &mut stage.file)
                .context("writing ciphertext to the spool file")?;
            Ok(stage)
        })
        .await?;
        self.stage = Some(stage);
        Ok(())
    }

    /// Pad, MAC and seal the file; hand back everything the upload needs.
    pub async fn finish(mut self) -> Result<StagedUpload, WamuxError> {
        self.cover(self.plaintext + CIPHERTEXT_OVERHEAD)?;
        let Stage { encryptor, file } = self.take_stage()?;
        let (file, info, len) = run_blocking(move || {
            let mut file = file;
            let info = encryptor
                .finalize_to_writer(&mut file)
                .context("sealing the spool file")?;
            let len = file.metadata().context("measuring the spool file")?.len();
            Ok((file, info, len))
        })
        .await?;
        Ok(StagedUpload {
            source: SpooledCiphertext::from_file(file, len),
            info,
            reservation: self.reservation,
        })
    }

    /// Make the reservation cover `needed` bytes, growing it by the shortfall
    /// or refusing when the budget cannot give it.
    fn cover(&mut self, needed: u64) -> Result<(), WamuxError> {
        let shortfall = needed.saturating_sub(self.reservation.bytes());
        self.reservation.try_grow(shortfall)
    }

    /// Only empty after an earlier call failed, which ends the upload.
    fn take_stage(&mut self) -> Result<Stage, WamuxError> {
        self.stage.take().ok_or_else(|| {
            WamuxError::Other(anyhow::anyhow!("spooled upload used after a failure"))
        })
    }
}

/// Encryption and file writes are synchronous: keep them off the runtime.
pub(super) async fn run_blocking<T, F>(work: F) -> Result<T, WamuxError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let joined = tokio::task::spawn_blocking(work).await;
    Ok(joined.context("the spool task panicked")??)
}
