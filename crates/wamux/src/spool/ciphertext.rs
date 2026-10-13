//! The encrypted upload body, staged in an unlinked spool file (#79), as the
//! library's `UploadSource`: each upload attempt (and a server-driven resume)
//! asks for a reader at an offset, and each reader is independent.

use std::fs::File;
use std::io::{self, Read};
use std::os::unix::fs::FileExt;
use std::sync::Arc;

use wacore::upload::UploadSource;

/// A spool file holding exactly `len` bytes of ciphertext.
pub struct SpooledCiphertext {
    file: Arc<File>,
    len: u64,
}

impl SpooledCiphertext {
    pub fn from_file(file: File, len: u64) -> Self {
        Self {
            file: Arc::new(file),
            len,
        }
    }
}

impl UploadSource for SpooledCiphertext {
    fn len(&self) -> u64 {
        self.len
    }

    fn reader_from(&self, offset: u64) -> std::io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(PositionedReader {
            file: Arc::clone(&self.file),
            offset,
        }))
    }
}

/// A reader with a position of its own. It reads with `read_at` (pread), which
/// never touches the file's shared offset: a `try_clone` plus `seek` would,
/// because clones share one offset, and a retry would start where the last
/// attempt stopped.
struct PositionedReader {
    file: Arc<File>,
    offset: u64,
}

impl Read for PositionedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read_at(buf, self.offset)?;
        self.offset += read as u64;
        Ok(read)
    }
}

#[cfg(test)]
#[path = "ciphertext_tests.rs"]
mod tests;
