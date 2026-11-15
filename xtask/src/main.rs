//! vivelite build orchestrator.
//!
//! The steamac-for-Android pattern, without the microVM: the XR Elite has no
//! `/dev/kvm` for apps, so instead of libkrun we stage Valve's verified
//! components into a proot-consumable layout that the WinlatorXR-derived
//! app runs.
//!
//! Usage:
//! ```text
//! cargo xtask fetch <bundle-url>   # streamed, resumable download
//! cargo xtask verify               # sha256 + CMS signature
//! cargo xtask extract <image>      # stage SteamOS components
//! cargo xtask doctor               # check host + drift
//! ```

use std::path::{Path, PathBuf};

use bundle::{Extractor, RootfsReconstructor, SignatureVerifier};
use vivelite_core::device_layout;

mod dev;
mod fs;
mod ui;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    };
    std::process::exit(code);
}

/// Parsed command line. Hand-rolled so the parsing is unit-testable with no
/// CLI framework on the Android side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// Download the bundle to the work directory.
    Fetch { url: String },
    /// Verify the downloaded bundle (sha256 + signature).
    Verify { sha256: Option<String> },
    /// Extract SteamOS components from a slot image.
    Extract { image: PathBuf },
    /// Full pipeline: atomupd meta → signed bundle → manifest → desync
    /// reconstruct → stage components. This is the real SteamOS staging.
    Stage { branch: String },
    /// Report host capability and component drift.
    Doctor,
    /// Open the egui host dashboard.
    Ui,
    /// Local dev-check: build, test, and enforce 100% coverage.
    Dev,
    /// Print help.
    Help,
}

/// Parse arguments into a command.
pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let Some(first) = args.first() else {
        return Ok(Cmd::Help);
    };
    let rest = &args[1..];
    match first.as_str() {
        "fetch" => {
            let url = rest
                .first()
                .cloned()
                .or_else(|| std::env::var("VIVELITE_BUNDLE_URL").ok())
                .ok_or("fetch needs <bundle-url> (or VIVELITE_BUNDLE_URL)")?;
            Ok(Cmd::Fetch { url })
        }
        "verify" => {
            let sha256 = flag_value(rest, "--sha256");
            Ok(Cmd::Verify { sha256 })
        }
        "extract" => {
            let image = rest
                .first()
                .map(PathBuf::from)
                .ok_or("extract needs <slot-image>")?;
            Ok(Cmd::Extract { image })
        }
        "stage" => {
            let branch = rest
                .first()
                .cloned()
                .or_else(|| std::env::var("VIVELITE_BRANCH").ok())
                .unwrap_or_else(|| "stable".into());
            Ok(Cmd::Stage { branch })
        }
        "doctor" => Ok(Cmd::Doctor),
        "ui" => Ok(Cmd::Ui),
        "dev" => Ok(Cmd::Dev),
        "help" | "--help" | "-h" => Ok(Cmd::Help),
        other => Err(format!("unknown command: {other}")),
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn run(args: &[String]) -> Result<String, String> {
    run_with(args, &launch_dashboard)
}

/// Launches the egui dashboard; split out so tests can fake the UI.
fn launch_dashboard() -> Result<(), String> {
    ui::launch().map_err(|e| format!("could not launch dashboard: {e}"))
}

/// Runs a parsed command. `launch_ui` is injected so the dashboard arm is
/// testable without a display.
fn run_with(args: &[String], launch_ui: &dyn Fn() -> Result<(), String>) -> Result<String, String> {
    match parse(args)? {
        Cmd::Help => Ok(usage().to_string()),
        Cmd::Ui => {
            launch_ui()?;
            Ok("dashboard closed".into())
        }
        Cmd::Fetch { url } => {
            let dest = work_root().join("bundles").join("steamos.raucb");
            fs::ensure_dir(&fs::RealFs, dest.parent().unwrap_or(Path::new(".")))?;
            // The download itself runs over HTTPS from the host; here we
            // delegate to curl for transport (C tool, wrapped, per project
            // rule) and verify with the bundle crate afterwards.
            let out = run_curl(
                "curl",
                &[
                    "--continue-at".into(),
                    "-".into(),
                    "--fail".into(),
                    "--location".into(),
                    "--output".into(),
                    dest.display().to_string(),
                    url.clone(),
                ],
            )?;
            if !out.status.success() {
                return Err("download failed".into());
            }
            let size = std::fs::metadata(&dest).map_err(|e| e.to_string())?.len();
            Ok(format!(
                "{} → {} ({} MiB); next: cargo xtask verify",
                url,
                dest.display(),
                size / (1024 * 1024)
            ))
        }
        Cmd::Verify { sha256 } => {
            let dest = work_root().join("bundles").join("steamos.raucb");
            if let Some(expected) = sha256 {
                bundle::verify_file_sha256(&dest, &expected).map_err(|e| e.to_string())?;
            }
            let verifier = bundle::RaucVerifier::default();
            verifier.verify(&dest).map_err(|e| e.to_string())?;
            Ok(format!("verified: {}", dest.display()))
        }
        Cmd::Extract { image } => {
            let out = work_root().join("staged");
            let components = bundle::steamos_components(&out);
            let extractor = bundle::SquashfsExtractor::default();
            extractor
                .extract(&image, &components)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "staged {} components under {}",
                components.len(),
                out.display()
            ))
        }
        Cmd::Stage { branch } => stage(&branch),
        Cmd::Dev => dev::run(&dev::cargo_program()),
        Cmd::Doctor => {
            let layout = device_layout(&work_root());
            let lines = doctor_checks(&layout);
            let bad = lines.iter().filter(|(ok, _)| !ok).count();
            if bad > 0 {
                for (ok, msg) in &lines {
                    if !ok {
                        eprintln!("✗ {msg}");
                    }
                }
                return Err(format!("{bad} check(s) failed"));
            }
            Ok("all checks passed".into())
        }
    }
}

/// Full SteamOS staging pipeline against the live atomupd feed:
/// meta → signed bundle → manifest → desync reconstruct → ext4 staging.
///
/// Verified against the live Valve service on 2026-11-15 (build
/// 20261022.6150929, SteamOS 0.4.5): the `.raucb` is a ~2 MiB signed
/// squashfs; the rootfs bytes come from casync chunk stores.
fn stage(branch: &str) -> Result<String, String> {
    stage_with(&fs::RealFs, branch)
}

fn stage_with(files: &dyn fs::Fs, branch: &str) -> Result<String, String> {
    let work = work_root();

    // 1. atomupd metadata: latest candidate for the branch.
    let meta_url = bundle::meta_url(branch);
    let meta = fetch_text(&meta_url)?;
    let cand = bundle::parse_candidates(&meta)
        .map_err(|e| e.to_string())?
        .pop()
        .ok_or("no candidates in the feed")?;
    eprintln!(
        "steamOS {} ({}), bundle {}",
        cand.version,
        cand.buildid,
        cand.bundle_filename()
    );

    // 2. download the signed bundle.
    let bundles = fs::child(&work, "bundles");
    fs::ensure_dir(files, &bundles)?;
    let bundle_path = fs::child(&bundles, cand.bundle_filename());
    if !files.exists(&bundle_path) {
        curl(&cand.bundle_url(), &bundle_path)?;
    }

    // 3. verify the CMS signature against Valve's RAUC CA.
    bundle::RaucVerifier::default()
        .verify(&bundle_path)
        .map_err(|e| e.to_string())?;

    // 4. unsquashfs the manifest and the casync index out of the bundle.
    let unpack = fs::child(&bundles, format!("{}.d", cand.buildid));
    fs::ensure_dir(files, &unpack)?;
    bundle::SquashfsExtractor::default()
        .extract(
            &bundle_path,
            &[
                bundle::Component {
                    inner_path: "manifest.raucm".into(),
                    dest: unpack.clone(),
                },
                bundle::Component {
                    inner_path: "rootfs.img.caibx".into(),
                    dest: unpack.clone(),
                },
            ],
        )
        .map_err(|e| e.to_string())?;
    let manifest_text = fs::read_text(files, &fs::child(&unpack, "manifest.raucm"))?;
    let manifest = bundle::parse_manifest(&manifest_text).map_err(|e| e.to_string())?;
    let compatible = manifest
        .compatible
        .as_deref()
        .ok_or("manifest lacks compatible")?;
    if compatible != "steamos-aarch64" {
        return Err(format!(
            "manifest compatible={compatible}, expected steamos-aarch64"
        ));
    }
    let rootfs = manifest
        .images
        .iter()
        .find(|i| i.slot == "rootfs")
        .ok_or("manifest lacks an [image.rootfs] section")?;
    let caibx = fs::child(&unpack, &rootfs.filename);
    if !files.exists(&caibx) {
        return Err(format!(
            "bundle does not carry {}; is this really a deckard RAUCB?",
            rootfs.filename
        ));
    }
    eprintln!(
        "manifest ok: {} / rootfs {} bytes, sha256 {}",
        compatible,
        rootfs.size.unwrap_or(0),
        rootfs.sha256.as_deref().unwrap_or("?")
    );

    // 5. reconstruct the rootfs image from Valve's chunk stores.
    let chunk_cache = fs::child(&fs::child(&work, "cache"), "desync");
    fs::ensure_dir(files, &chunk_cache)?;
    let reconstructor = bundle::DesyncReconstructor {
        stores: cand.chunk_stores(),
        cache: Some(chunk_cache),
        ..Default::default()
    };
    let rootfs_img = fs::child(&work, format!("rootfs-{}.img", cand.buildid));
    if !files.exists(&rootfs_img) {
        let info = reconstructor.info(&caibx).map_err(|e| e.to_string())?;
        eprintln!(
            "desync: {} unique chunks, {} MiB to fetch",
            info.unique_chunks,
            info.dedup_bytes / (1024 * 1024)
        );
        reconstructor
            .reconstruct(&caibx, &rootfs_img)
            .map_err(|e| e.to_string())?;
    }

    // 6. integrity: the manifest pins the full rootfs sha256.
    if let Some(expected) = &rootfs.sha256 {
        bundle::verify_file_sha256(&rootfs_img, expected).map_err(|e| e.to_string())?;
    }

    // 7. stage the userspace components (debugfs, no mount, no root).
    let staged = fs::child(&work, "staged");
    let components = bundle::steamos_components(&staged);
    bundle::Ext4Extractor::default()
        .extract(&rootfs_img, &components)
        .map_err(|e| e.to_string())?;

    Ok(format!(
        "staged {} SteamOS components under {} (rootfs {} at {})",
        components.len(),
        staged.display(),
        cand.buildid,
        rootfs_img.display()
    ))
}

/// Small text fetch via curl (transport stays a wrapped C tool).
fn fetch_text(url: &str) -> Result<String, String> {
    let out = run_curl(
        "curl",
        &[
            "--fail".into(),
            "--location".into(),
            "--silent".into(),
            "--show-error".into(),
            url.into(),
        ],
    )?;
    if !out.status.success() {
        return Err(format!(
            "could not fetch {url}: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("non-utf8 response from {url}: {e}"))
}

/// Spawns curl for transport (C tool, wrapped per the project rule). The
/// program name is a parameter so tests can exercise the spawn-failure path.
fn run_curl(program: &str, args: &[String]) -> Result<std::process::Output, String> {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run curl: {e}"))
}

fn curl(url: &str, dest: &Path) -> Result<(), String> {
    let out = run_curl(
        "curl",
        &[
            "--continue-at".into(),
            "-".into(),
            "--fail".into(),
            "--location".into(),
            "--output".into(),
            dest.display().to_string(),
            url.into(),
        ],
    )?;
    if !out.status.success() {
        return Err(format!("download failed: {url}"));
    }
    Ok(())
}

fn doctor_checks(layout: &vivelite_core::DeviceLayout) -> Vec<(bool, String)> {
    let mut v = vec![
        cmd_exists("curl", "HTTPS transport"),
        cmd_exists("rauc", "bundle verification (preferred)"),
        cmd_exists("unsquashfs", "bundle extraction"),
        cmd_exists("desync", "SteamOS rootfs reconstruction"),
        cmd_exists("debugfs", "rootfs component staging"),
        (
            layout.bundle.exists(),
            format!("bundle present at {}", layout.bundle.display()),
        ),
    ];
    v.push((
        std::env::var("VIVELITE_BUNDLE_URL").is_ok(),
        "VIVELITE_BUNDLE_URL set".to_string(),
    ));
    v
}

fn cmd_exists(bin: &str, what: &str) -> (bool, String) {
    let ok = which(bin).is_some();
    (ok, format!("{bin} available ({what})"))
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

fn work_root() -> PathBuf {
    std::env::var("VIVELITE_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new("work").to_path_buf())
}

fn usage() -> &'static str {
    "vivelite xtask — build orchestrator\n\
     \n\
     cargo xtask stage [branch]        full pipeline: meta → bundle → rootfs → staged components\n\
     cargo xtask fetch <bundle-url> [--out DIR]\n\
     cargo xtask verify [--sha256 <hex>]\n\
     cargo xtask extract <slot-image>\n\
     cargo xtask doctor\n\
     cargo xtask ui                   open the egui host dashboard\n\
     cargo xtask dev                  build, test, and enforce 100% coverage\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stage_defaults_to_stable() {
        assert_eq!(
            parse(&["stage".into()]).unwrap(),
            Cmd::Stage {
                branch: "stable".into()
            }
        );
        assert_eq!(
            parse(&["stage".into(), "beta".into()]).unwrap(),
            Cmd::Stage {
                branch: "beta".into()
            }
        );
    }

    #[test]
    fn parse_fetch_prefers_explicit_url() {
        let c = parse(&["fetch".into(), "https://example.com/b.raucb".into()]).unwrap();
        assert_eq!(
            c,
            Cmd::Fetch {
                url: "https://example.com/b.raucb".into()
            }
        );
    }

    #[test]
    fn parse_fetch_reads_env() {
        std::env::set_var("VIVELITE_BUNDLE_URL", "https://example.com/env.raucb");
        assert_eq!(
            parse(&["fetch".into()]).unwrap(),
            Cmd::Fetch {
                url: "https://example.com/env.raucb".into()
            }
        );
        std::env::remove_var("VIVELITE_BUNDLE_URL");
    }

    #[test]
    fn parse_verify_flag() {
        assert_eq!(
            parse(&["verify".into()]).unwrap(),
            Cmd::Verify { sha256: None }
        );
        assert_eq!(
            parse(&["verify".into(), "--sha256".into(), "aa".into()]).unwrap(),
            Cmd::Verify {
                sha256: Some("aa".into())
            }
        );
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(parse(&["frobnicate".into()]).is_err());
        assert!(
            parse(&["extract".into()]).is_err(),
            "extract requires an image"
        );
    }

    #[test]
    fn parse_ui_command() {
        assert_eq!(parse(&["ui".into()]).unwrap(), Cmd::Ui);
    }

    #[test]
    fn parse_dev_command() {
        assert_eq!(parse(&["dev".into()]).unwrap(), Cmd::Dev);
    }

    #[test]
    fn parse_empty_is_help() {
        assert_eq!(parse(&[]).unwrap(), Cmd::Help);
    }

    #[test]
    fn ui_arm_reports_dashboard_closed() {
        let ok = run_with(&["ui".into()], &|| Ok(()));
        assert_eq!(ok.unwrap(), "dashboard closed");
        let err = run_with(&["ui".into()], &|| Err("no display".into()));
        assert_eq!(err.unwrap_err(), "no display");
    }

    #[test]
    fn run_curl_reports_spawn_failure() {
        let err = run_curl("/nonexistent/vivelite-curl", &[]).unwrap_err();
        assert!(err.contains("could not run curl"), "{err}");
    }
}
