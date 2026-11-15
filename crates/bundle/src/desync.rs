//! Rootfs reconstruction from the bundle's casync index via `desync`.
//!
//! The `.raucb` bundle does not contain the rootfs bytes. It contains
//! `rootfs.img.caibx`, a casync index of ~64 KiB content-addressed chunks;
//! the bytes live in Valve's chunk stores (see [`crate::atomupd`]).
//! `desync` (Go, static binary, Apache-2.0/BSD-3) is the reference client —
//! the same one the steamac project drives — and is wrapped here like every
//! other external C/Go tool: invoked as a subprocess, planned and validated
//! from Rust.
//!
//! Verified invocation shape (mirrors steamac's `DiskCreator.reconstruct`):
//!
//! ```text
//! desync info <caibx>                     → JSON: unique, dedup-size-not-in-seed
//! desync extract [options] <caibx> <out>  → reconstructs the image
//! ```
//!
//! On-device this runs inside proot: desync needs no privileges, no mounts,
//! no KVM — only network, disk and CPU, which is precisely what the XR Elite
//! offers an unprivileged app process.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Error, Result};

/// Statistics about an index, from `desync info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconstructInfo {
    /// Number of unique chunks to download.
    pub unique_chunks: u64,
    /// Bytes of unique data to download (`dedup-size-not-in-seed`).
    pub dedup_bytes: u64,
}

/// Something that can turn a `.caibx` index into a raw image file.
pub trait RootfsReconstructor {
    /// Chunk/download statistics for planning and disk-space checks.
    fn info(&self, caibx: &Path) -> Result<ReconstructInfo>;
    /// Reconstruct the image at `caibx` into `out`.
    fn reconstruct(&self, caibx: &Path, out: &Path) -> Result<()>;
}

/// Drives the `desync` binary against remote casync stores.
#[derive(Debug, Clone)]
pub struct DesyncReconstructor {
    /// Path to `desync`; defaults to `desync` on PATH.
    pub desync_bin: Option<String>,
    /// Chunk store URLs, searched in order (per-image first, shared second).
    pub stores: Vec<String>,
    /// Local chunk cache directory; reused across runs.
    pub cache: Option<PathBuf>,
    /// Parallel chunk fetches.
    pub concurrency: usize,
}

impl Default for DesyncReconstructor {
    fn default() -> Self {
        DesyncReconstructor {
            desync_bin: None,
            stores: Vec::new(),
            cache: None,
            concurrency: 16,
        }
    }
}

impl DesyncReconstructor {
    /// Full argument vector for a desync invocation: global flags first,
    /// positional arguments last (desync's cobra CLI expects flags before
    /// positionals, like steamac passes them).
    fn args(&self, positional: &[std::ffi::OsString]) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        args.push("--concurrency".into());
        args.push(self.concurrency.to_string().into());
        args.push("--error-retry".into());
        args.push("10".into());
        args.push("--cache-repair=true".into());
        if let Some(cache) = &self.cache {
            args.push("--cache".into());
            args.push(cache.as_os_str().into());
        }
        for s in &self.stores {
            args.push("--store".into());
            args.push(s.into());
        }
        args.extend(positional.iter().cloned());
        args
    }

    fn run(&self, sub: &str, positional: &[std::ffi::OsString], tool: &'static str) -> Result<std::process::Output> {
        let mut cmd = Command::new(self.desync_bin.as_deref().unwrap_or("desync"));
        cmd.arg(sub).args(self.args(positional));
        let out = cmd.output().map_err(|e| Error::Tool {
            tool,
            status: std::process::ExitStatus::default(),
            stderr: format!("could not run {tool}: {e}"),
        })?;
        if !out.status.success() {
            return Err(Error::Tool {
                tool,
                status: out.status,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        Ok(out)
    }
}

impl RootfsReconstructor for DesyncReconstructor {
    fn info(&self, caibx: &Path) -> Result<ReconstructInfo> {
        let out = self.run("info", &[caibx.as_os_str().to_owned()], "desync")?;
        let json = String::from_utf8_lossy(&out.stdout).into_owned();
        Ok(ReconstructInfo {
            unique_chunks: json_u64(&json, "unique")
                .ok_or_else(|| Error::Other("desync info: missing \"unique\" in output".into()))?,
            dedup_bytes: json_u64(&json, "dedup-size-not-in-seed").unwrap_or(0),
        })
    }

    fn reconstruct(&self, caibx: &Path, out_path: &Path) -> Result<()> {
        self.run(
            "extract",
            &[
                std::ffi::OsString::from("--print-stats"),
                caibx.as_os_str().to_owned(),
                out_path.as_os_str().to_owned(),
            ],
            "desync",
        )?;
        let got = std::fs::metadata(out_path).map_err(|e| Error::Other(format!(
            "desync did not produce {}: {e}",
            out_path.display()
        )))?;
        if got.len() == 0 {
            return Err(Error::Other(format!(
                "{} is empty after reconstruction",
                out_path.display()
            )));
        }
        Ok(())
    }
}

/// Read a `"key": number` field from a small JSON document.
fn json_u64(json: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\"");
    let i = json.find(&pat)?;
    let rest = &json[i + pat.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_desync_info_output() {
        let j = r#"{"unique": 44926, "dedup-size-not-in-seed": 2939889664, "size": 10737418240}"#;
        let info = ReconstructInfo {
            unique_chunks: json_u64(j, "unique").unwrap(),
            dedup_bytes: json_u64(j, "dedup-size-not-in-seed").unwrap(),
        };
        assert_eq!(info.unique_chunks, 44_926);
        assert_eq!(info.dedup_bytes, 2_939_889_664);
        assert_eq!(json_u64(j, "size"), Some(10_737_418_240));
        assert_eq!(json_u64(j, "missing"), None);
    }

    #[test]
    fn default_concurrency_is_sane() {
        let r = DesyncReconstructor::default();
        assert_eq!(r.concurrency, 16);
        assert!(r.stores.is_empty());
    }
}
