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
    count_tree(root, &mut report)?;
    write_manifest(root, &report)?;
    Ok(report)
}

fn count_tree(path: &Path, report: &mut StagingReport) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            count_tree(&entry.path(), report)?;
        } else {
            report.files += 1;
            report.bytes += metadata.len();
        }
    }
    Ok(())
}

fn write_manifest(root: &Path, report: &StagingReport) -> io::Result<()> {
    let manifest = root.join("stage-manifest.txt");
    let temporary = root.join("stage-manifest.txt.tmp");
    let mut file = fs::File::create(&temporary)?;
    writeln!(file, "format=vivelite-stage-v1")?;
    writeln!(file, "files={}", report.files)?;
    writeln!(file, "bytes={}", report.bytes)?;
    for component in REQUIRED_COMPONENTS {
        writeln!(file, "component.{}={}", component.name, component.path)?;
    }
    file.flush()?;
    fs::rename(temporary, manifest)
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
}
