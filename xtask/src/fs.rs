//! Filesystem seam for the staging pipeline.
//!
//! `stage` and `fetch` touch the disk through [`Fs`] instead of `std::fs`
//! directly. Production uses [`RealFs`]; tests inject a wrapper that fails a
//! chosen operation, so every error branch of the pipeline is reachable
//! without a broken disk or a read-only mount.

use std::path::{Path, PathBuf};

/// The filesystem operations the pipeline needs.
pub trait Fs {
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()>;
    fn metadata_len(&self, path: &Path) -> std::io::Result<u64>;
    fn read_to_string(&self, path: &Path) -> std::io::Result<String>;
    fn exists(&self, path: &Path) -> bool;
}

/// The real filesystem.
pub struct RealFs;

impl Fs for RealFs {
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn metadata_len(&self, path: &Path) -> std::io::Result<u64> {
        std::fs::metadata(path).map(|m| m.len())
    }

    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// Maps an I/O error into the message format the CLI prints.
pub fn io_msg(context: impl AsRef<Path>, err: std::io::Error) -> String {
    format!("{}: {err}", context.as_ref().display())
}

/// Creates `path` (and parents) through `fs`, with the path in the error.
pub fn ensure_dir(fs: &dyn Fs, path: &Path) -> Result<(), String> {
    fs.create_dir_all(path).map_err(|e| io_msg(path, e))
}

/// Returns the file length through `fs`, with the path in the error.
pub fn file_len(fs: &dyn Fs, path: &Path) -> Result<u64, String> {
    fs.metadata_len(path).map_err(|e| io_msg(path, e))
}

/// Reads a text file through `fs`, with the path in the error.
pub fn read_text(fs: &dyn Fs, path: &Path) -> Result<String, String> {
    fs.read_to_string(path).map_err(|e| io_msg(path, e))
}

/// Joins a child onto `base`; kept as a helper so the stage code reads as
/// a list of named artifacts.
pub fn child(base: &Path, name: impl AsRef<Path>) -> PathBuf {
    base.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Broken;

    impl Fs for Broken {
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            Err(std::io::Error::other("no dir"))
        }
        fn metadata_len(&self, _: &Path) -> std::io::Result<u64> {
            Err(std::io::Error::other("no meta"))
        }
        fn read_to_string(&self, _: &Path) -> std::io::Result<String> {
            Err(std::io::Error::other("no read"))
        }
        fn exists(&self, _: &Path) -> bool {
            false
        }
    }

    #[test]
    fn helpers_report_path_and_cause() {
        let p = Path::new("work/x");
        assert_eq!(ensure_dir(&Broken, p).unwrap_err(), "work/x: no dir");
        assert_eq!(file_len(&Broken, p).unwrap_err(), "work/x: no meta");
        assert_eq!(read_text(&Broken, p).unwrap_err(), "work/x: no read");
        assert!(!Broken.exists(p));
    }

    #[test]
    fn real_fs_round_trips_through_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let nested = child(dir.path(), "a/b");
        ensure_dir(&RealFs, &nested).unwrap();
        let file = child(&nested, "f.txt");
        std::fs::write(&file, "hi").unwrap();
        assert_eq!(file_len(&RealFs, &file).unwrap(), 2);
        assert_eq!(read_text(&RealFs, &file).unwrap(), "hi");
        assert!(RealFs.exists(&file));
    }
}
