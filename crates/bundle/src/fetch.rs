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
        match self.end {
            Some(e) if e < self.start => Some(0),
            Some(e) => Some(e - self.start + 1),
            None => None,
        }
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
    let start = out.metadata();
    append_stream(&mut out, start, src, on_progress)
}

fn append_stream(
    out: &mut dyn Write,
    start: std::io::Result<std::fs::Metadata>,
    src: &mut dyn Read,
    on_progress: Progress,
) -> Result<u64> {
    let mut buf = [0u8; 64 * 1024];
    let mut written = start?.len();
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
    fetch_boxed(dest, Box::new(reader), on_progress)
}

// Non-generic so the logic is compiled once: each closure type passed to
// `fetch_to` would otherwise get its own copy, and tests exercise only some.
fn fetch_boxed(
    dest: &Path,
    reader: Box<dyn FnOnce(u64) -> std::io::Result<Box<dyn Read>> + '_>,
    on_progress: Progress,
) -> Result<u64> {
    let resume_from = match std::fs::metadata(dest) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e.into()),
    };
    let mut src = reader(resume_from)?;
    append_to(dest, &mut src, on_progress)
}

fn no_progress(_: u64, _: Instant) -> std::io::Result<()> {
    Ok(())
}

/// Convenience wrapper when no progress reporting is needed.
pub fn fetch_to_quiet<F>(dest: &Path, reader: F) -> Result<u64>
where
    F: FnOnce(u64) -> std::io::Result<Box<dyn Read>>,
{
    fetch_to(dest, reader, &mut no_progress)
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
        assert_eq!(
            Range {
                start: 0,
                end: Some(9)
            }
            .header(),
            "bytes=0-9"
        );
        assert_eq!(
            Range {
                start: 1024,
                end: None
            }
            .header(),
            "bytes=1024-"
        );
        assert_eq!(
            Range {
                start: 0,
                end: Some(9)
            }
            .len(),
            Some(10)
        );
        assert_eq!(
            Range {
                start: 5,
                end: None
            }
            .len(),
            None
        );
        assert!(Range {
            start: 3,
            end: Some(2)
        }
        .is_empty());
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
    fn fresh_fetch_starts_at_zero() {
        let path = tmp("fresh.bin");
        let n = fetch_to_quiet(&path, |offset| {
            assert_eq!(offset, 0);
            Ok(Box::new(std::io::Cursor::new(b"fresh".to_vec())))
        })
        .unwrap();
        assert_eq!(n, 5);
        assert_eq!(std::fs::read(&path).unwrap(), b"fresh");
    }

    #[test]
    fn io_error_helper_adds_context() {
        let err = io_err("download")(std::io::Error::other("offline"));
        assert!(err.to_string().contains("download: offline"));
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

    #[test]
    fn io_failures_are_reported_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        let mut progress = |_: u64, _: Instant| -> std::io::Result<()> { Ok(()) };
        // A directory where the destination file should be cannot be opened.
        let as_dir = dir.path().join("is-a-dir");
        std::fs::create_dir(&as_dir).unwrap();
        assert!(append_to(&as_dir, &mut std::io::Cursor::new(b"x"), &mut progress).is_err());
        assert!(
            fetch_to_quiet(&as_dir, |_| Ok(Box::new(std::io::Cursor::new(
                b"x".to_vec()
            ))))
            .is_err()
        );
        assert!(reset_partial(&as_dir).is_err());

        // The same progress callback also runs on the success path.
        let ok_path = dir.path().join("progress.bin");
        let n = append_to(&ok_path, &mut std::io::Cursor::new(b"x"), &mut progress).unwrap();
        assert_eq!(n, 1);

        // A reader that fails mid-stream surfaces the error.
        let path = dir.path().join("broken.bin");
        let broken = fetch_to_quiet(&path, |_| Ok(Box::new(FailingReader)));
        assert!(broken.is_err());

        // The reader factory itself failing is reported.
        let path = dir.path().join("no-reader.bin");
        assert!(fetch_to_quiet(&path, offline_reader).is_err());
    }

    struct FakeSink {
        flush_error: bool,
    }

    impl Write for FakeSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.flush_error {
                Err(std::io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn sink_metadata_and_flush_errors_are_reported() {
        let mut progress = |_: u64, _: Instant| -> std::io::Result<()> { Ok(()) };
        let mut src = std::io::Cursor::new(b"x");
        let mut sink = FakeSink { flush_error: false };
        assert!(append_stream(
            &mut sink,
            Err(std::io::Error::other("metadata failed")),
            &mut src,
            &mut progress,
        )
        .is_err());

        let dir = tempfile::tempdir().unwrap();
        let empty = std::fs::File::create(dir.path().join("empty.bin")).unwrap();
        let mut src = std::io::Cursor::new(b"x");
        let mut sink = FakeSink { flush_error: true };
        assert!(append_stream(&mut sink, empty.metadata(), &mut src, &mut progress).is_err());

        let mut src = std::io::Cursor::new(b"x");
        let mut sink = FakeSink { flush_error: false };
        assert_eq!(
            append_stream(&mut sink, empty.metadata(), &mut src, &mut progress).unwrap(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn self_referential_symlink_fails_before_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let looped = dir.path().join("loop.bin");
        std::os::unix::fs::symlink(&looped, &looped).unwrap();
        let err = fetch_to_quiet(&looped, offline_reader).unwrap_err();
        assert!(
            !err.to_string().contains("offline"),
            "metadata error must precede the reader: {err}"
        );
    }

    fn offline_reader(_offset: u64) -> std::io::Result<Box<dyn Read>> {
        Err(std::io::Error::other("offline"))
    }

    #[cfg(unix)]
    #[test]
    fn full_device_write_error_is_reported() {
        let err = fetch_to_quiet(Path::new("/dev/full"), |_| {
            Ok(Box::new(std::io::Cursor::new(b"x".to_vec())))
        })
        .unwrap_err();
        assert!(err.to_string().contains("No space left on device"), "{err}");
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("connection reset"))
        }
    }
}
