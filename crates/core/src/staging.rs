//! Validation and manifest generation for extracted SteamOS components.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A component required by the on-device SteamOS layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredComponent {
    /// Stable manifest identifier.
    pub name: &'static str,
    /// Path below the extracted SteamOS root.
    pub path: &'static str,
    /// Whether the component must be a directory (otherwise a regular file).
    pub directory: bool,
}

/// Required paths in the SteamOS tree produced by `bundle::steamos_components`.
pub const REQUIRED_COMPONENTS: [RequiredComponent; 5] = [
    RequiredComponent {
        name: "wine",
        path: "steamos/lib/wine",
        directory: true,
    },
    RequiredComponent {
        name: "pressure-vessel",
        path: "steamos/lib/pressure-vessel",
        directory: true,
    },
    RequiredComponent {
        name: "fex-assets",
        path: "steamos/share/fex",
        directory: true,
    },
    RequiredComponent {
        name: "gamescope",
        path: "steamos/bin/gamescope",
        directory: false,
    },
    RequiredComponent {
        name: "steam",
        path: "steamos/bin/steam",
        directory: false,
    },
];

/// Information about a successfully inspected staged layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingReport {
    /// Root directory containing the `steamos/` tree.
    pub root: PathBuf,
    /// Number of regular files and symlinks under the root.
    pub files: u64,
    /// Total bytes reported by filesystem metadata.
    pub bytes: u64,
    /// Number of required components validated.
    pub components: usize,
}

/// Validates all required paths, counts the extracted files, then writes a
/// deterministic `stage-manifest.txt` beneath `root`.
pub fn validate_and_write_manifest(root: &Path) -> io::Result<StagingReport> {
    for required in REQUIRED_COMPONENTS {
        let path = root.join(required.path);
        let metadata = fs::symlink_metadata(&path)?;
        let kind_ok = if required.directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        };
        if !kind_ok {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} has the wrong file type", path.display()),
            ));
        }
    }

    let mut report = StagingReport {
        root: root.to_path_buf(),
        files: 0,
        bytes: 0,
        components: REQUIRED_COMPONENTS.len(),
    };
    count_tree(root, &mut report, 0)?;
    write_manifest(root, &report)?;
    Ok(report)
}

/// Deepest directory nesting the staged tree may have; real SteamOS trees
/// stay far below this, and the bound keeps a hostile image from exhausting
/// the stack.
const MAX_TREE_DEPTH: usize = 64;

fn count_tree(path: &Path, report: &mut StagingReport, depth: usize) -> io::Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is nested deeper than {MAX_TREE_DEPTH}", path.display()),
        ));
    }
    for entry in fs::read_dir(path)? {
        count_entry(entry, report, depth)?;
    }
    Ok(())
}

fn count_entry(
    entry: io::Result<fs::DirEntry>,
    report: &mut StagingReport,
    depth: usize,
) -> io::Result<()> {
    let entry = entry?;
    let path = entry.path();
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.is_dir() {
        count_tree(&path, report, depth + 1)?;
    } else {
        report.files += 1;
        report.bytes += metadata.len();
    }
    Ok(())
}

fn write_manifest(root: &Path, report: &StagingReport) -> io::Result<()> {
    let manifest = root.join("stage-manifest.txt");
    let temporary = root.join("stage-manifest.txt.tmp");
    let mut file = fs::File::create(&temporary)?;
    write_manifest_contents(&mut file, report)?;
    fs::rename(temporary, manifest)
}

fn write_manifest_contents(writer: &mut impl Write, report: &StagingReport) -> io::Result<()> {
    writeln!(writer, "format=vivelite-stage-v1")?;
    writeln!(writer, "files={}", report.files)?;
    writeln!(writer, "bytes={}", report.bytes)?;
    for component in REQUIRED_COMPONENTS {
        writeln!(writer, "component.{}={}", component.name, component.path)?;
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        for component in REQUIRED_COMPONENTS {
            let path = temp.path().join(component.path);
            if component.directory {
                fs::create_dir_all(&path).unwrap();
                fs::write(path.join("payload"), b"steam").unwrap();
            } else {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, b"binary").unwrap();
            }
        }
        temp
    }

    #[test]
    fn validates_layout_and_writes_manifest() {
        let temp = fixture();
        let report = validate_and_write_manifest(temp.path()).unwrap();
        assert_eq!(report.components, 5);
        assert_eq!(report.files, 5);
        assert_eq!(report.bytes, 3 * 5 + 2 * 6);
        let manifest = fs::read_to_string(temp.path().join("stage-manifest.txt")).unwrap();
        assert!(manifest.contains("format=vivelite-stage-v1"));
        assert!(manifest.contains("component.wine=steamos/lib/wine"));
        assert!(manifest.contains("files=5"));
    }

    #[test]
    fn missing_component_is_reported() {
        let temp = fixture();
        fs::remove_file(temp.path().join("steamos/bin/steam")).unwrap();
        assert_eq!(
            validate_and_write_manifest(temp.path()).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn wrong_component_type_is_reported() {
        let temp = fixture();
        let path = temp.path().join("steamos/bin/steam");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_eq!(
            validate_and_write_manifest(temp.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn counts_symlinks_without_following_them() {
        let temp = fixture();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/outside", temp.path().join("steamos/bin/link")).unwrap();
        let report = validate_and_write_manifest(temp.path()).unwrap();
        #[cfg(unix)]
        assert_eq!(report.files, 6);
        #[cfg(not(unix))]
        assert_eq!(report.files, 5);
    }

    #[test]
    fn tree_counting_reports_read_and_entry_errors() {
        let temp = fixture();
        let report = StagingReport {
            root: temp.path().to_path_buf(),
            files: 0,
            bytes: 0,
            components: REQUIRED_COMPONENTS.len(),
        };
        let file = temp.path().join("not-a-directory");
        fs::write(&file, b"x").unwrap();
        assert!(count_tree(&file, &mut report.clone(), 0).is_err());

        assert!(count_entry(
            Err(io::Error::other("entry failed")),
            &mut report.clone(),
            0
        )
        .is_err());

        let stale = temp.path().join("stale-entry");
        fs::write(&stale, b"x").unwrap();
        let entry = fs::read_dir(temp.path())
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path() == stale)
            .unwrap();
        fs::remove_file(stale).unwrap();
        assert!(count_entry(Ok(entry), &mut report.clone(), 0).is_err());
    }

    #[test]
    fn tree_counting_rejects_excessive_nesting() {
        let temp = fixture();
        let mut deep = temp.path().join("deep");
        for _ in 0..=MAX_TREE_DEPTH {
            deep.push("d");
        }
        fs::create_dir_all(&deep).unwrap();
        let err = validate_and_write_manifest(temp.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("nested deeper"), "{err}");
    }

    #[test]
    fn manifest_write_failures_are_reported() {
        let temp = fixture();
        let report = StagingReport {
            root: temp.path().to_path_buf(),
            files: 5,
            bytes: 27,
            components: REQUIRED_COMPONENTS.len(),
        };

        let temporary = temp.path().join("stage-manifest.txt.tmp");
        fs::create_dir(&temporary).unwrap();
        assert!(write_manifest(temp.path(), &report).is_err());
        fs::remove_dir(temporary).unwrap();

        #[cfg(unix)]
        {
            let temporary = temp.path().join("stage-manifest.txt.tmp");
            std::os::unix::fs::symlink("/dev/full", &temporary).unwrap();
            let err = write_manifest(temp.path(), &report).unwrap_err();
            assert!(err.to_string().contains("No space left"), "{err}");
        }

        struct FailingWriter {
            writes_before_failure: usize,
            writes: usize,
            fail_flush: bool,
        }
        impl Write for FailingWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if self.writes == self.writes_before_failure {
                    return Err(io::Error::other("write failed"));
                }
                self.writes += 1;
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                if self.fail_flush {
                    Err(io::Error::other("flush failed"))
                } else {
                    Ok(())
                }
            }
        }
        for writes_before_failure in 0..8 {
            let mut writer = FailingWriter {
                writes_before_failure,
                writes: 0,
                fail_flush: false,
            };
            assert!(write_manifest_contents(&mut writer, &report).is_err());
        }
        let mut writer = FailingWriter {
            writes_before_failure: usize::MAX,
            writes: 0,
            fail_flush: true,
        };
        assert!(write_manifest_contents(&mut writer, &report).is_err());
        let mut writer = FailingWriter {
            writes_before_failure: usize::MAX,
            writes: 0,
            fail_flush: false,
        };
        assert!(write_manifest_contents(&mut writer, &report).is_ok());

        let manifest = temp.path().join("stage-manifest.txt");
        fs::create_dir(&manifest).unwrap();
        assert!(write_manifest(temp.path(), &report).is_err());
    }

    #[test]
    fn validation_reports_tree_and_manifest_failures() {
        let temp = fixture();
        let nested = temp.path().join("steamos/lib/wine");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&nested, fs::Permissions::from_mode(0o000)).unwrap();
            let err = validate_and_write_manifest(temp.path()).unwrap_err();
            fs::set_permissions(&nested, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        }

        let temp = fixture();
        fs::create_dir(temp.path().join("stage-manifest.txt.tmp")).unwrap();
        assert!(validate_and_write_manifest(temp.path()).is_err());
    }
}
