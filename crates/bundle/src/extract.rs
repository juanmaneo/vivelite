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
                status: std::process::ExitStatus::default(),
                stderr: format!("could not run unsquashfs: {e}"),
            })?;
            if !out.status.success() {
                return Err(Error::Tool {
                    tool: "unsquashfs",
                    status: out.status,
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
            status: std::process::ExitStatus::default(),
            stderr: format!("could not run debugfs: {e}"),
        })?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() || stdout.contains("File not found by ext2_lookup") {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            return Err(Error::Tool {
                tool: "debugfs",
                status: out.status,
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

impl Extractor for Ext4Extractor {
    fn extract(&self, image: &Path, components: &[Component]) -> Result<()> {
        for c in components {
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
}
