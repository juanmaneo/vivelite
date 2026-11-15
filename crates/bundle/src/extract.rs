//! Extraction of the RAUC bundle's squashfs payload.
//!
//! `unsquashfs` (C) does the heavy lifting; this module plans and validates
//! the extraction so the orchestrator can stage selected components only
//! (Wine, FEX, Mesa, SteamOS libraries) rather than flashing slot images.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Error, Result};

/// A component to extract from the bundle's filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// Absolute path inside the bundle's root filesystem, e.g. `/usr/lib/steam`.
    pub inner_path: String,
    /// Destination directory on the host build tree.
    pub dest: PathBuf,
}

/// Something that can extract paths from a squashfs image.
pub trait Extractor {
    /// Extract `components` from the image at `image` on disk.
    fn extract(&self, image: &Path, components: &[Component]) -> Result<()>;
}

/// Drives `unsquashfs -d <dest> <image> <paths...>`.
#[derive(Debug, Clone, Default)]
pub struct SquashfsExtractor {
    /// Path to `unsquashfs`; defaults to `unsquashfs` on PATH.
    pub unsquashfs_bin: Option<String>,
    /// Extra args, e.g. `-n` to skip POSIX attrs on exotic filesystems.
    pub extra_args: Vec<String>,
}

impl SquashfsExtractor {
    fn cmd(&self) -> Command {
        Command::new(self.unsquashfs_bin.as_deref().unwrap_or("unsquashfs"))
    }
}

impl Extractor for SquashfsExtractor {
    fn extract(&self, image: &Path, components: &[Component]) -> Result<()> {
        // unsquashfs extracts multiple paths into one destination tree, so
        // group components by destination directory and run once per group.
        let mut by_dest: std::collections::BTreeMap<PathBuf, Vec<&Component>> = Default::default();
        for c in components {
            by_dest.entry(c.dest.clone()).or_default().push(c);
        }
        for (dest, group) in by_dest {
            std::fs::create_dir_all(&dest)?;
            let mut cmd = self.cmd();
            cmd.arg("-d").arg(&dest);
            for a in &self.extra_args {
                cmd.arg(a);
            }
            cmd.arg(image);
            for c in group {
                cmd.arg(&c.inner_path);
            }
            let out = cmd.output().map_err(|e| Error::Tool {
                tool: "unsquashfs",
                status: None,
                stderr: format!("could not run unsquashfs: {e}"),
            })?;
            if !out.status.success() {
                return Err(Error::Tool {
                    tool: "unsquashfs",
                    status: Some(out.status),
                    stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Stages components out of the reconstructed SteamOS rootfs (ext4).
///
/// `debugfs` from e2fsprogs reads ext2/3/4 images directly, with **no mount
/// and no privileges** — the only viable option on a locked-bootloader XR
/// Elite where the process runs unprivileged under proot. Directories are
/// recursively dumped with `rdump`, regular files with `dump`.
///
/// The rootfs image is produced by [`crate::desync::RootfsReconstructor`]; the
/// SteamOS slot is a raw 10 GiB ext4 image per the bundle manifest.
#[derive(Debug, Clone, Default)]
pub struct Ext4Extractor {
    /// Path to `debugfs`; defaults to `debugfs` on PATH.
    pub debugfs_bin: Option<String>,
    /// Extra args appended to every `debugfs` invocation.
    pub extra_args: Vec<String>,
}

impl Ext4Extractor {
    fn cmd(&self) -> Command {
        Command::new(self.debugfs_bin.as_deref().unwrap_or("debugfs"))
    }

    /// One `debugfs -R <request>` call; returns stdout.
    fn request(&self, image: &Path, request: &str) -> Result<String> {
        let mut cmd = self.cmd();
        cmd.arg("-R").arg(request).arg(image);
        for a in &self.extra_args {
            cmd.arg(a);
        }
        let out = cmd.output().map_err(|e| Error::Tool {
            tool: "debugfs",
            status: None,
            stderr: format!("could not run debugfs: {e}"),
        })?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() || stdout.contains("File not found by ext2_lookup") {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            return Err(Error::Tool {
                tool: "debugfs",
                status: Some(out.status),
                stderr: if stderr.is_empty() { stdout } else { stderr },
            });
        }
        Ok(stdout)
    }

    /// `stat` output distinguishes directories from regular files.
    fn is_directory(&self, image: &Path, inner: &str) -> Result<bool> {
        let st = self.request(image, &format!("stat {inner}"))?;
        Ok(st.contains("directory") && !st.contains("regular"))
    }
}

fn validate_debugfs_path(path: &str) -> Result<()> {
    let inner = path
        .strip_prefix('/')
        .ok_or_else(|| Error::Other(format!("debugfs path must be absolute: {path:?}")))?;
    if inner.is_empty()
        || inner
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || inner
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
        || inner.contains(['"', '\'', '`', ';', '\\'])
    {
        return Err(Error::Other(format!("unsafe debugfs path: {path:?}")));
    }
    Ok(())
}

impl Extractor for Ext4Extractor {
    fn extract(&self, image: &Path, components: &[Component]) -> Result<()> {
        for c in components {
            validate_debugfs_path(&c.inner_path)?;
            std::fs::create_dir_all(&c.dest)?;
            // Paths inside the image are absolute without the leading slash
            // for debugfs requests; strip it.
            let inner = c.inner_path.trim_start_matches('/');
            let request = if self.is_directory(image, inner)? {
                // rdump <dir> <destdir> copies the whole subtree.
                format!("rdump {inner} {}", c.dest.display())
            } else {
                // dump <file> <destfile>: name the output like the source.
                let dest_file = c.dest.join(inner.rsplit('/').next().unwrap_or(inner));
                format!("dump {inner} {}", dest_file.display())
            };
            self.request(image, &request)?;
        }
        Ok(())
    }
}

/// The components vivelite stages out of Valve's SteamOS ARM64 image.
///
/// Paths follow the Steam Frame image layout; they are validated at extract
/// time and the list is expected to drift with Valve releases — `xtask doctor`
/// checks for the drift.
pub fn steamos_components(out_root: &Path) -> Vec<Component> {
    let lib = out_root.join("steamos/lib");
    let bin = out_root.join("steamos/bin");
    let share = out_root.join("steamos/share");
    vec![
        // Wine with the ARM64EC backend and FEX built in.
        Component {
            inner_path: "/usr/lib/wine".into(),
            dest: lib.clone(),
        },
        // Proton's runtime pieces referenced by toolmanifests.
        Component {
            inner_path: "/usr/lib/pressure-vessel".into(),
            dest: lib.clone(),
        },
        // FEX assets: rootfs defaults and config schema.
        Component {
            inner_path: "/usr/share/fex".into(),
            dest: share,
        },
        // Valve's gamescope compositor, reused for the flat 2D stage.
        Component {
            inner_path: "/usr/bin/gamescope".into(),
            dest: bin.clone(),
        },
        // The Steam client and its helper binaries.
        Component {
            inner_path: "/usr/bin/steam".into(),
            dest: bin,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_cover_the_core_stack() {
        let comps = steamos_components(Path::new("/out"));
        let paths: Vec<&str> = comps.iter().map(|c| c.inner_path.as_str()).collect();
        assert!(paths.contains(&"/usr/lib/wine"));
        assert!(paths.contains(&"/usr/share/fex"));
        assert!(
            paths.iter().all(|p| p.starts_with('/')),
            "inner paths must be absolute"
        );
    }

    #[test]
    fn extractor_groups_by_destination() {
        // Building the grouping is internal; here we at least validate that
        // distinct destinations do not collide in the planned layout.
        let comps = steamos_components(Path::new("/out"));
        let mut dests: Vec<&PathBuf> = comps.iter().map(|c| &c.dest).collect();
        dests.sort();
        dests.dedup();
        assert!(dests.len() >= 2, "expected multiple destination roots");
    }

    /// Write an executable stand-in for a C tool. Appends its argv to `log`
    /// so tests can assert how the crate drives the real tool.
    fn fake_tool(dir: &Path, name: &str, log: &Path, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho \"$@\" >> '{}'\n{body}", log.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn tool_error_message(err: crate::Error) -> String {
        err.to_string()
    }

    #[test]
    fn squashfs_extractor_extracts_and_groups() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("squashfs.log");
        let unsq = fake_tool(
            dir.path(),
            "fake-unsquashfs",
            &log,
            r#"
dest=""
while [ $# -gt 0 ]; do
    case "$1" in
        -d) dest="$2"; shift 2 ;;
        -*) shift ;;
        *) break ;;
    esac
done
image="$1"; shift
for p in "$@"; do
    mkdir -p "$(dirname "$dest/$p")"
    echo "payload of $p" > "$dest/$p"
done
"#,
        );
        let out_a = dir.path().join("out-a");
        let out_b = dir.path().join("out-b");
        let comps = vec![
            Component {
                inner_path: "/usr/bin/steam".into(),
                dest: out_a.clone(),
            },
            Component {
                inner_path: "/usr/bin/gamescope".into(),
                dest: out_a.clone(),
            },
            Component {
                inner_path: "/usr/share/fex".into(),
                dest: out_b.clone(),
            },
        ];
        let extractor = SquashfsExtractor {
            unsquashfs_bin: Some(unsq.to_string_lossy().into_owned()),
            extra_args: vec!["-n".into()],
        };
        extractor
            .extract(dir.path().join("bundle.raucb").as_path(), &comps)
            .unwrap();
        // Same destination grouped into one call, distinct ones separate.
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.matches("-d").count(),
            2,
            "one call per destination: {calls}"
        );
        assert!(
            calls.contains("/usr/bin/steam /usr/bin/gamescope"),
            "{calls}"
        );
        assert!(calls.contains("-n"), "extra args forwarded: {calls}");
        assert!(calls.contains("bundle.raucb"), "image passed: {calls}");
        assert_eq!(
            std::fs::read_to_string(out_a.join("usr/bin/steam")).unwrap(),
            "payload of /usr/bin/steam\n"
        );
        assert!(out_b.join("usr/share/fex").exists());
    }

    #[test]
    fn squashfs_extractor_tool_failure_is_reported() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let unsq = fake_tool(
            dir.path(),
            "fake-unsquashfs",
            &dir.path().join("unused.log"),
            "echo boom >&2; exit 3",
        );
        let extractor = SquashfsExtractor {
            unsquashfs_bin: Some(unsq.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
        };
        let err = extractor
            .extract(
                dir.path().join("b.raucb").as_path(),
                &[Component {
                    inner_path: "/usr/bin/steam".into(),
                    dest: dir.path().join("out"),
                }],
            )
            .unwrap_err();
        let msg = tool_error_message(err);
        assert!(msg.contains("unsquashfs failed"), "{msg}");
        assert!(msg.contains("boom"), "{msg}");
    }

    #[test]
    fn squashfs_extractor_reports_destination_creation_failure() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("not-a-directory");
        std::fs::write(&parent, b"file").unwrap();
        let extractor = SquashfsExtractor::default();
        let err = extractor
            .extract(
                Path::new("image.raucb"),
                &[Component {
                    inner_path: "/usr/bin/steam".into(),
                    dest: parent.join("out"),
                }],
            )
            .unwrap_err();
        assert!(matches!(err, crate::Error::Io(_)));
    }

    #[test]
    fn squashfs_extractor_missing_binary_is_reported() {
        let extractor = SquashfsExtractor {
            unsquashfs_bin: Some("/nonexistent/unsquashfs".into()),
            extra_args: Vec::new(),
        };
        let err = extractor
            .extract(
                Path::new("/dev/null"),
                &[Component {
                    inner_path: "/usr/bin/steam".into(),
                    dest: std::env::temp_dir().join("vivelite-missing-bin-dest"),
                }],
            )
            .unwrap_err();
        let msg = tool_error_message(err);
        assert!(msg.contains("could not run unsquashfs"), "{msg}");
    }

    #[test]
    fn ext4_extractor_dumps_directories_and_files() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("debugfs.log");
        let debugfs = fake_tool(
            dir.path(),
            "fake-debugfs",
            &log,
            r#"
req=""
while [ $# -gt 0 ]; do
    case "$1" in
        -R) req="$2"; shift 2 ;;
        -*) shift ;;
        *) shift ;;
    esac
done
case "$req" in
    stat\ *)
        inner=${req#stat }
        case "$inner" in
            usr/lib/wine|usr/lib/pressure-vessel) echo " Mode: 040755 Type: directory" ;;
            *) echo " Mode: 0100755 Type: regular file" ;;
        esac
        ;;
    rdump\ *)
        inner=$(echo "$req" | cut -d' ' -f2)
        dest=$(echo "$req" | cut -d' ' -f3)
        mkdir -p "$dest/$inner/sub"
        echo inner > "$dest/$inner/sub/libwine.so"
        ;;
    dump\ *)
        inner=$(echo "$req" | cut -d' ' -f2)
        dest=$(echo "$req" | cut -d' ' -f3)
        echo binary > "$dest"
        ;;
    *) echo "unexpected request: $req" >&2; exit 1 ;;
esac
"#,
        );
        let out = dir.path().join("staged");
        let comps = vec![
            Component {
                inner_path: "/usr/lib/wine".into(),
                dest: out.clone(),
            },
            Component {
                inner_path: "/usr/bin/steam".into(),
                dest: out.clone(),
            },
        ];
        let extractor = Ext4Extractor {
            debugfs_bin: Some(debugfs.to_string_lossy().into_owned()),
            extra_args: vec!["-f".into(), "fake".into()],
        };
        extractor
            .extract(dir.path().join("rootfs.img").as_path(), &comps)
            .unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("stat usr/lib/wine"), "{calls}");
        assert!(calls.contains("rdump usr/lib/wine"), "{calls}");
        assert!(calls.contains("dump usr/bin/steam"), "{calls}");
        assert!(
            out.join("usr/lib/wine/sub/libwine.so").exists(),
            "rdump copies the tree"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("steam")).unwrap(),
            "binary\n"
        );
    }

    #[test]
    fn ext4_extractor_rejects_unsafe_debugfs_paths_before_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let extractor = Ext4Extractor::default();
        for path in [
            "relative/path",
            "/usr/lib/../etc",
            "/usr/lib with spaces",
            "/usr/lib;quit",
        ] {
            let err = extractor
                .extract(
                    Path::new("rootfs.img"),
                    &[Component {
                        inner_path: path.into(),
                        dest: dir.path().join("out"),
                    }],
                )
                .unwrap_err();
            assert!(err.to_string().contains("debugfs path"), "{err}");
        }
    }

    #[test]
    fn ext4_extractor_reports_tool_failure() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let debugfs = fake_tool(
            dir.path(),
            "fake-debugfs",
            &dir.path().join("unused.log"),
            "echo kaboom >&2; exit 1",
        );
        let extractor = Ext4Extractor {
            debugfs_bin: Some(debugfs.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
        };
        let err = extractor
            .extract(
                Path::new("rootfs.img"),
                &[Component {
                    inner_path: "/usr/lib/wine".into(),
                    dest: dir.path().join("out"),
                }],
            )
            .unwrap_err();
        let msg = tool_error_message(err);
        assert!(msg.contains("debugfs failed"), "{msg}");
        assert!(msg.contains("kaboom"), "{msg}");
    }

    #[test]
    fn ext4_extractor_reports_destination_creation_failure() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("not-a-directory");
        std::fs::write(&parent, b"file").unwrap();
        let extractor = Ext4Extractor::default();
        let err = extractor
            .extract(
                Path::new("rootfs.img"),
                &[Component {
                    inner_path: "/usr/lib/wine".into(),
                    dest: parent.join("out"),
                }],
            )
            .unwrap_err();
        assert!(matches!(err, crate::Error::Io(_)));
    }

    #[test]
    fn ext4_extractor_reports_request_failure_after_stat() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let debugfs = fake_tool(
            dir.path(),
            "fake-debugfs",
            &dir.path().join("unused.log"),
            r#"
req=""
while [ $# -gt 0 ]; do
    case "$1" in
        -R) req="$2"; shift 2 ;;
        *) shift ;;
    esac
done
case "$req" in
    stat\ *) echo "Type: directory" ;;
    rdump\ *) echo "extract failed" >&2; exit 1 ;;
esac
"#,
        );
        let extractor = Ext4Extractor {
            debugfs_bin: Some(debugfs.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
        };
        let err = extractor
            .extract(
                Path::new("rootfs.img"),
                &[Component {
                    inner_path: "/usr/lib/wine".into(),
                    dest: dir.path().join("out"),
                }],
            )
            .unwrap_err();
        assert!(err.to_string().contains("extract failed"));
    }

    #[test]
    fn ext4_extractor_treats_missing_inode_as_error() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        // debugfs exits 0 but prints the sentinel on stdout (no stderr).
        let debugfs = fake_tool(
            dir.path(),
            "fake-debugfs",
            &dir.path().join("unused.log"),
            "echo 'File not found by ext2_lookup'; echo ' Mode: 0100644 regular'",
        );
        let extractor = Ext4Extractor {
            debugfs_bin: Some(debugfs.to_string_lossy().into_owned()),
            extra_args: Vec::new(),
        };
        let err = extractor
            .extract(
                Path::new("rootfs.img"),
                &[Component {
                    inner_path: "/gone".into(),
                    dest: dir.path().join("out"),
                }],
            )
            .unwrap_err();
        let msg = tool_error_message(err);
        assert!(msg.contains("File not found by ext2_lookup"), "{msg}");
    }

    #[test]
    fn ext4_extractor_missing_binary_is_reported() {
        let extractor = Ext4Extractor {
            debugfs_bin: Some("/nonexistent/debugfs".into()),
            extra_args: Vec::new(),
        };
        let err = extractor
            .extract(
                Path::new("rootfs.img"),
                &[Component {
                    inner_path: "/usr/lib/wine".into(),
                    dest: std::env::temp_dir().join("vivelite-missing-debugfs-dest"),
                }],
            )
            .unwrap_err();
        let msg = tool_error_message(err);
        assert!(msg.contains("could not run debugfs"), "{msg}");
    }
}
