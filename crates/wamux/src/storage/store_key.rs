//! The key that seals the store's secrets at rest (#164): 64 hex characters in
//! a file only its owner can read.
//!
//! A file, never an environment variable or the TOML body: an environment is
//! readable through `/proc/<pid>/environ` and `docker inspect`, and systemd
//! `LoadCredential=` and Docker secrets both deliver files. The refusal of a
//! group- or world-readable file is the same rule the store file itself follows
//! (#75): a secret other users can read is not a secret.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Why a store key could not be loaded. No variant carries the key or any part
/// of the file's content.
#[derive(Debug, thiserror::Error)]
pub enum StoreKeyError {
    #[error("cannot read store key file {path}: {source}")]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "store key file {path} has mode {mode:o}, readable by group or others; run: chmod 600 {path}"
    )]
    LooseMode { path: PathBuf, mode: u32 },
    #[error(
        "store key must be exactly 64 hex characters (generate one with: openssl rand -hex 32)"
    )]
    BadFormat,
}

/// A 256-bit key. `Debug` never prints it.
#[derive(Clone)]
pub struct StoreKey {
    bytes: [u8; 32],
}

impl std::fmt::Debug for StoreKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StoreKey(id={})", key_id_hex(&self.id()))
    }
}

/// The key id as lowercase hex, for logs and error messages.
pub fn key_id_hex(id: &[u8; 4]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

impl StoreKey {
    /// 64 hex characters (either case), with at most one trailing `\n` or
    /// `\r\n`. Anything else is `BadFormat`.
    pub fn parse_hex(text: &str) -> Result<Self, StoreKeyError> {
        let digits = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        let raw = digits.as_bytes();
        if raw.len() != 64 {
            return Err(StoreKeyError::BadFormat);
        }
        let mut bytes = [0u8; 32];
        for (slot, pair) in bytes.iter_mut().zip(raw.chunks_exact(2)) {
            *slot = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
        }
        Ok(Self { bytes })
    }

    /// Read and parse a key file. Refuses a mode with any group or other bit
    /// (`mode & 0o077 != 0`) before reading it.
    pub fn from_file(path: &Path) -> Result<Self, StoreKeyError> {
        let unreadable = |source| StoreKeyError::Unreadable {
            path: path.to_path_buf(),
            source,
        };
        // Check the mode of the handle we read from, not of the path: no gap
        // for the file to be swapped between the check and the read.
        let mut file = std::fs::File::open(path).map_err(unreadable)?;
        let mode = file.metadata().map_err(unreadable)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(StoreKeyError::LooseMode {
                path: path.to_path_buf(),
                mode: mode & 0o7777,
            });
        }
        let mut text = String::new();
        file.read_to_string(&mut text).map_err(|source| {
            // Not UTF-8 is a format problem, never worth echoing the bytes.
            if source.kind() == std::io::ErrorKind::InvalidData {
                return StoreKeyError::BadFormat;
            }
            unreadable(source)
        })?;
        Self::parse_hex(&text)
    }

    /// The first four bytes of SHA-256 of the key: written in every sealed
    /// blob's header and in the `store_encryption` row, so a blob says which key
    /// sealed it without revealing anything about the key.
    pub fn id(&self) -> [u8; 4] {
        let digest = Sha256::digest(self.bytes);
        [digest[0], digest[1], digest[2], digest[3]]
    }

    /// The raw key bytes, for the cipher only.
    pub(crate) fn expose(&self) -> &[u8; 32] {
        &self.bytes
    }
}

fn hex_nibble(digit: u8) -> Result<u8, StoreKeyError> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        b'A'..=b'F' => Ok(digit - b'A' + 10),
        _ => Err(StoreKeyError::BadFormat),
    }
}

#[cfg(test)]
#[path = "store_key_tests.rs"]
mod tests;
