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
        let mut args: Vec<std::ffi::OsString> = vec![
            "--concurrency".into(),
            self.concurrency.to_string().into(),
            "--error-retry".into(),
            "10".into(),
            "--cache-repair=true".into(),
        ];
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

    fn run(
        &self,
        sub: &str,
        positional: &[std::ffi::OsString],
        tool: &'static str,
    ) -> Result<std::process::Output> {
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
        let got = std::fs::metadata(out_path).map_err(|e| {
            Error::Other(format!(
                "desync did not produce {}: {e}",
                out_path.display()
            ))
        })?;
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
        assert_eq!(json_u64("{\"k\" 1}", "k"), None);
        assert_eq!(json_u64("{\"k\": nope}", "k"), None);
    }

    #[test]
    fn default_concurrency_is_sane() {
        let r = DesyncReconstructor::default();
        assert_eq!(r.concurrency, 16);
        assert!(r.stores.is_empty());
    }

    /// Fake `desync`: records argv; `mode` bakes in a scenario (`ok`,
    /// `noinfo`, `empty`, `fail`) so parallel tests never share state.
    fn fake_desync(dir: &std::path::Path, mode: &str) -> String {
        let path = dir.join("fake-desync");
        let info_action = if mode == "noinfo" {
            "echo '{\"dedup-size-not-in-seed\": 123}'"
        } else {
            "echo '{\"unique\": 44926, \"dedup-size-not-in-seed\": 2939889664}'"
        };
        let extra = match mode {
            "ok" => "\n[ \"$1\" = extract ] && printf 'rootfs-bytes' > \"$out\"",
            "noinfo" => "",
            "empty" => "\n[ \"$1\" = extract ] && : > \"$out\"",
            "fail" => "\necho 'chunk store unreachable' >&2\nexit 9",
            other => panic!("unknown fake desync mode: {other}"),
        };
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{log}'\nout=''\nfor arg in \"$@\"; do out=\"$arg\"; done\ncase \"$1\" in\n    info) {info_action} ;;\n    extract) : ;;\nesac\n{extra}\nexit 0\n",
                log = dir.join("desync.log").display(),
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path.to_string_lossy().into_owned()
    }

    fn reconstructor(bin: &str) -> DesyncReconstructor {
        DesyncReconstructor {
            desync_bin: Some(bin.into()),
            stores: vec!["https://store.example/castr/".into()],
            cache: None,
            concurrency: 4,
        }
    }

    #[test]
    fn info_parses_the_fake_output() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_desync(dir.path(), "ok");
        let caibx = dir.path().join("rootfs.img.caibx");
        std::fs::write(&caibx, b"index").unwrap();
        let info = reconstructor(&bin).info(&caibx).unwrap();
        assert_eq!(info.unique_chunks, 44_926);
        assert_eq!(info.dedup_bytes, 2_939_889_664);
        // Flags come before positionals, stores after the global flags.
        let calls = std::fs::read_to_string(dir.path().join("desync.log")).unwrap();
        assert!(calls.contains("--concurrency 4"), "{calls}");
        assert!(calls.contains("--error-retry 10"), "{calls}");
        assert!(
            calls.contains("--store https://store.example/castr/"),
            "{calls}"
        );
        assert!(calls.contains("info --concurrency 4"), "{calls}");
    }

    #[test]
    fn cache_flag_is_passed_when_configured() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_desync(dir.path(), "ok");
        let cache = dir.path().join("cache");
        let r = DesyncReconstructor {
            desync_bin: Some(bin.clone()),
            cache: Some(cache.clone()),
            ..reconstructor(&bin)
        };
        let caibx = dir.path().join("i.caibx");
        std::fs::write(&caibx, b"index").unwrap();
        r.info(&caibx).unwrap();
        let calls = std::fs::read_to_string(dir.path().join("desync.log")).unwrap();
        assert!(
            calls.contains(&format!("--cache {}", cache.display())),
            "{calls}"
        );
    }

    #[test]
    fn info_reports_missing_unique_field() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_desync(dir.path(), "noinfo");
        let caibx = dir.path().join("i.caibx");
        std::fs::write(&caibx, b"index").unwrap();
        let err = reconstructor(&bin).info(&caibx).unwrap_err();
        assert!(err.to_string().contains("missing \"unique\""), "{err}");
    }

    #[test]
    fn reconstruct_writes_and_validates_the_image() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_desync(dir.path(), "ok");
        let caibx = dir.path().join("i.caibx");
        std::fs::write(&caibx, b"index").unwrap();
        let out = dir.path().join("rootfs.img");
        reconstructor(&bin).reconstruct(&caibx, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"rootfs-bytes");
        let calls = std::fs::read_to_string(dir.path().join("desync.log")).unwrap();
        assert!(calls.contains("--print-stats"), "{calls}");
    }

    #[test]
    fn reconstruct_rejects_a_missing_output() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        // A desync that exits successfully but produces nothing.
        let bin = dir.path().join("fake-desync-noop");
        std::fs::write(&bin, "#!/bin/sh\nexit 0").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let err = reconstructor(&bin.to_string_lossy())
            .reconstruct(&dir.path().join("i.caibx"), &dir.path().join("gone.img"))
            .unwrap_err();
        assert!(err.to_string().contains("did not produce"), "{err}");
    }

    #[test]
    fn reconstruct_rejects_an_empty_output() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_desync(dir.path(), "empty");
        let err = reconstructor(&bin)
            .reconstruct(&dir.path().join("i.caibx"), &dir.path().join("empty.img"))
            .unwrap_err();
        assert!(
            err.to_string().contains("is empty after reconstruction"),
            "{err}"
        );
    }

    #[test]
    fn reconstruct_reports_extract_failure() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_desync(dir.path(), "fail");
        let err = reconstructor(&bin)
            .reconstruct(&dir.path().join("i.caibx"), &dir.path().join("failed.img"))
            .unwrap_err();
        assert!(err.to_string().contains("chunk store unreachable"));
    }

    #[test]
    fn tool_failure_carries_stderr() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_desync(dir.path(), "fail");
        let err = reconstructor(&bin)
            .info(&dir.path().join("i.caibx"))
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("desync failed"), "{msg}");
        assert!(msg.contains("chunk store unreachable"), "{msg}");
    }

    #[test]
    #[should_panic(expected = "unknown fake desync mode")]
    fn fake_desync_rejects_unknown_mode() {
        let dir = tempfile::tempdir().unwrap();
        let _ = fake_desync(dir.path(), "invalid");
    }

    #[test]
    fn missing_binary_is_reported() {
        let err = reconstructor("/nonexistent/desync")
            .info(Path::new("i.caibx"))
            .unwrap_err();
        assert!(err.to_string().contains("could not run desync"), "{err}");
    }
}
