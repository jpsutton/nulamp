//! The spool of a finite stream (NET13): every received byte is kept in an unnamed temporary
//! file, so the decoder seeks anywhere in what has arrived without a new request, and a file
//! served without Range support (a server-side transcode) is seekable at all.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// Ranged files larger than this are not spooled: they seek with Range requests instead.
pub(crate) const MAX_RANGED: u64 = 512 * 1024 * 1024;

pub(crate) struct Spool {
    file: File,
    len: u64,
}

impl Spool {
    /// A new empty spool. The file is deleted when the spool is dropped (or the process ends).
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            file: tempfile::tempfile()?,
            len: 0,
        })
    }

    /// Bytes received so far.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(self.len))?;
        self.file.write_all(bytes)?;
        self.len += bytes.len() as u64;
        Ok(())
    }

    /// Reads from byte offset `pos`, at most up to the bytes received.
    pub fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        let n = (self.len.saturating_sub(pos)).min(buf.len() as u64) as usize;
        if n == 0 {
            return Ok(0);
        }
        self.file.seek(SeekFrom::Start(pos))?;
        self.file.read_exact(&mut buf[..n])?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_was_appended() {
        let mut s = Spool::new().unwrap();
        s.append(b"hello ").unwrap();
        s.append(b"world").unwrap();
        assert_eq!(s.len(), 11);
        let mut buf = [0u8; 8];
        assert_eq!(s.read_at(6, &mut buf).unwrap(), 5);
        assert_eq!(&buf[..5], b"world");
        assert_eq!(s.read_at(0, &mut buf).unwrap(), 8);
        assert_eq!(&buf, b"hello wo");
        assert_eq!(s.read_at(11, &mut buf).unwrap(), 0);
        assert_eq!(s.read_at(99, &mut buf).unwrap(), 0);
    }
}
