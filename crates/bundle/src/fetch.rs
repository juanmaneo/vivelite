//! Streaming, resumable bundle download.
//!
//! Intentionally std-only for the transport layer: on the desktop this runs
//! over HTTPS via `ureq` (enabled in `xtask`), while on-device the same file
//! layout is produced by the host build and sideloaded. What lives here is
//! the byte-level part that is unit-testable: resumable writes and Range
//! header construction.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use crate::{Error, Result};

/// An HTTP byte range, used to resume partial downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// First byte, inclusive.
    pub start: u64,
    /// End byte, inclusive; `None` means "to the end of the file".
    pub end: Option<u64>,
}

impl Range {
    /// Build a `bytes=a-b` / `bytes=a-` header value.
    ///
    /// ```ignore
    /// assert_eq!(Range { start: 1024, end: None }.header(), "bytes=1024-");
    /// ```
    pub fn header(self) -> String {
        match self.end {
            Some(e) => format!("bytes={}-{}", self.start, e),
            None => format!("bytes={}-", self.start),
        }
    }

    /// Number of bytes this range covers, when known.
    pub fn len(self) -> Option<u64> {
        self.end.map(|e| e.saturating_sub(self.start) + 1)
    }

    /// True when the range covers zero bytes.
    pub fn is_empty(self) -> bool {
        self.len() == Some(0)
    }
}

/// Progress callback receives (bytes_so_far, elapsed) and may abort by
/// returning `Err`.
pub type Progress<'a> = &'a mut dyn FnMut(u64, Instant) -> std::io::Result<()>;

/// Append `src` to the partial file at `dest`, resuming from its current
/// length. Used both by real downloads and by tests that simulate a
/// connection that dropped mid-transfer.
pub fn append_to(dest: &Path, src: &mut dyn Read, on_progress: Progress) -> Result<u64> {
    let mut out = OpenOptions::new().create(true).append(true).open(dest)?;
    let mut buf = [0u8; 64 * 1024];
    let start = out.metadata()?.len();
    let mut written = start;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        written += n as u64;
        on_progress(written, Instant::now())?;
    }
    out.flush()?;
    Ok(written)
}

/// Fetch `url` into `dest` in full, calling `fetch_to`'s transport hook.
///
/// The actual HTTPS client is injected so the download logic stays testable:
/// `reader` returns a fresh byte source, optionally for a resumed Range.
pub fn fetch_to<F>(dest: &Path, reader: F, on_progress: Progress) -> Result<u64>
where
    F: FnOnce(u64) -> std::io::Result<Box<dyn Read>>,
{
    let resume_from = if dest.exists() {
        std::fs::metadata(dest)?.len()
    } else {
        0
    };
    let mut src = reader(resume_from)?;
    append_to(dest, &mut src, on_progress)
}

/// Convenience wrapper when no progress reporting is needed.
pub fn fetch_to_quiet<F>(dest: &Path, reader: F) -> Result<u64>
where
    F: FnOnce(u64) -> std::io::Result<Box<dyn Read>>,
{
    fetch_to(dest, reader, &mut |_, _| Ok(()))
}

/// Truncate a partially downloaded file so a retry starts over.
pub fn reset_partial(dest: &Path) -> Result<()> {
    File::create(dest)?;
    Ok(())
}

/// Wrap std errors into the crate error type with context.
pub fn io_err(ctx: &str) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Other(format!("{ctx}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(name);
        std::mem::forget(d); // keep for the test
        p
    }

    #[test]
    fn range_headers() {
        assert_eq!(Range { start: 0, end: Some(9) }.header(), "bytes=0-9");
        assert_eq!(Range { start: 1024, end: None }.header(), "bytes=1024-");
        assert_eq!(Range { start: 0, end: Some(9) }.len(), Some(10));
        assert_eq!(Range { start: 5, end: None }.len(), None);
        assert!(Range { start: 3, end: Some(2) }.is_empty());
    }

    #[test]
    fn appends_and_resumes() {
        let path = tmp("part.bin");
        let first: &[u8] = b"hello ";
        let mut r = std::io::Cursor::new(first);
        append_to(&path, &mut r, &mut |_, _| Ok(())).unwrap();
        let second: &[u8] = b"world";
        let mut r = std::io::Cursor::new(second);
        let n = append_to(&path, &mut r, &mut |_, _| Ok(())).unwrap();
        assert_eq!(n, 11);
        let got = std::fs::read(&path).unwrap();
        assert_eq!(got, b"hello world");
    }

    #[test]
    fn fetch_passes_resume_offset() {
        let path = tmp("resume.bin");
        std::fs::write(&path, b"12345678").unwrap();
        let n = fetch_to_quiet(&path, |offset| {
            assert_eq!(offset, 8);
            Ok(Box::new(std::io::Cursor::new(b"90".to_vec())))
        })
        .unwrap();
        assert_eq!(n, 10);
        assert_eq!(std::fs::read(&path).unwrap(), b"1234567890");
    }

    #[test]
    fn progress_can_abort() {
        let path = tmp("abort.bin");
        let mut r = std::io::Cursor::new(vec![0u8; 128 * 1024]);
        let err = append_to(&path, &mut r, &mut |_, _| {
            Err(std::io::Error::other("user cancelled"))
        });
        assert!(err.is_err());
        reset_partial(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }
}
