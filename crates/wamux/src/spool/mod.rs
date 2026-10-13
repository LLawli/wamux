//! The media spool (#79): SendMedia, PostStatusMedia and DownloadMedia stage
//! a transfer in a file here instead of holding the whole file in memory, so
//! memory per transfer is constant. Spool files never have a name (unlinked
//! from creation), and a global byte budget bounds what all transfers in
//! flight may hold on disk.

pub mod budget;
pub mod ciphertext;
pub mod download;
pub mod upload;

use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use budget::{SpoolBudget, SpoolReservation};
pub use ciphertext::SpooledCiphertext;
pub use download::{BoundedFile, DownloadSlot, SpooledDownload};
pub use upload::{SpooledUpload, StagedUpload};

/// Where transfers are staged and how much they may hold at once. Built once
/// from the config and cloned into the services that transfer media.
#[derive(Clone)]
pub struct MediaSpool {
    dir: PathBuf,
    budget: Arc<SpoolBudget>,
}

impl MediaSpool {
    /// `max_bytes` bounds the bytes all transfers hold at once; 0 = unlimited.
    pub fn new(dir: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            dir: dir.into(),
            budget: SpoolBudget::new(max_bytes),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn budget(&self) -> &Arc<SpoolBudget> {
        &self.budget
    }
}

/// Why the spool directory cannot be used; startup stops on either.
#[derive(Debug, thiserror::Error)]
pub enum SpoolDirError {
    #[error("cannot create the media spool directory {path}")]
    Create {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("the media spool directory {path} is not writable")]
    NotWritable {
        path: String,
        #[source]
        source: io::Error,
    },
}

/// Create the missing levels of `dir` as 0700 (an existing directory keeps its
/// mode) and prove a spool file can be created in it.
pub fn prepare_spool_dir(dir: &Path) -> Result<(), SpoolDirError> {
    let create = |source| SpoolDirError::Create {
        path: dir.display().to_string(),
        source,
    };
    // `recursive` leaves an existing directory (and its mode) alone; only the
    // levels it has to create get 0700.
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(create)?;
    // The probe file is unlinked from creation, so nothing is left behind.
    tempfile::tempfile_in(dir).map_err(|source| SpoolDirError::NotWritable {
        path: dir.display().to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "dir_tests.rs"]
mod dir_tests;
