//! Tests for the filesystem seam used by the staging pipeline.

use std::path::{Path, PathBuf};

use xtask::fs::{child, ensure_dir, file_len, read_text, Fs, RealFs};

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
    let nested: PathBuf = child(dir.path(), "a/b");
    ensure_dir(&RealFs, &nested).unwrap();
    let file = child(&nested, "f.txt");
    std::fs::write(&file, "hi").unwrap();
    assert_eq!(file_len(&RealFs, &file).unwrap(), 2);
    assert_eq!(read_text(&RealFs, &file).unwrap(), "hi");
    assert!(RealFs.exists(&file));
}
