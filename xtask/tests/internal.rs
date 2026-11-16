//! Unit-level tests for the xtask library internals, run as an integration
//! test so the library compiles exactly once (single coverage record).

use std::ffi::OsString;
use std::path::Path;

use xtask::fs::{self, Fs, RealFs};
use xtask::{
    curl, doctor_report, launch_device_overlay, launch_device_overlay_with, parse, read_manifest,
    require_https, run_curl, run_with, stage_with, which_in, Cmd,
};

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

/// The three fetch URL sources in one test, so the process env is touched
/// from a single test and never races the others.
#[test]
fn parse_fetch_url_sources() {
    // Explicit URL wins and never consults the environment.
    let c = parse(&["fetch".into(), "https://example.com/b.raucb".into()]).unwrap();
    assert_eq!(
        c,
        Cmd::Fetch {
            url: "https://example.com/b.raucb".into()
        }
    );

    // No argument: fall back to VIVELITE_BUNDLE_URL.
    let saved = std::env::var_os("VIVELITE_BUNDLE_URL");
    std::env::set_var("VIVELITE_BUNDLE_URL", "https://example.com/env.raucb");
    assert_eq!(
        parse(&["fetch".into()]).unwrap(),
        Cmd::Fetch {
            url: "https://example.com/env.raucb".into()
        }
    );

    // Neither argument nor environment: a parse error.
    std::env::remove_var("VIVELITE_BUNDLE_URL");
    let err = parse(&["fetch".into()]).unwrap_err();
    assert!(err.contains("fetch needs"), "{err}");

    match saved {
        Some(value) => std::env::set_var("VIVELITE_BUNDLE_URL", value),
        None => std::env::remove_var("VIVELITE_BUNDLE_URL"),
    }
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
fn parse_device_ui_command() {
    assert_eq!(
        parse(&["device-ui".into()]).unwrap(),
        Cmd::DeviceUi { xr: false }
    );
    assert_eq!(
        parse(&["device-ui".into(), "--xr".into()]).unwrap(),
        Cmd::DeviceUi { xr: true }
    );
}

#[test]
fn ui_arm_reports_dashboard_closed() {
    let ok = || Ok(());
    let fail = || Err("no display".into());
    let closed = run_with(&["ui".into()], &ok, &ok, "cargo");
    assert_eq!(closed.unwrap(), "dashboard closed");
    let err = run_with(&["ui".into()], &fail, &ok, "cargo");
    assert_eq!(err.unwrap_err(), "no display");
}

#[test]
fn device_ui_arm_reports_overlay_closed() {
    let ok = || Ok(());
    let fail = || Err("no display".into());
    let closed = run_with(&["device-ui".into()], &ok, &ok, "cargo");
    assert_eq!(closed.unwrap(), "overlay closed");
    let err = run_with(&["device-ui".into()], &ok, &fail, "cargo");
    assert_eq!(err.unwrap_err(), "no display");
}

/// The device-ui --xr arm maps the runner's exit codes; `true` stands in
/// for a runner that exits 0, `false` for one that exits 1.
#[test]
fn device_ui_xr_arm_reports_the_runner_outcome() {
    let ok = || Ok(());
    let msg = run_with(&["device-ui".into(), "--xr".into()], &ok, &ok, "true").unwrap();
    assert!(msg.contains("session lost"), "{msg}");

    let err = run_with(&["device-ui".into(), "--xr".into()], &ok, &ok, "false").unwrap_err();
    assert!(err.contains("OpenXR runtime"), "{err}");
}

/// A parse failure inside `run_with` surfaces as the parse error itself.
#[test]
fn run_with_reports_parse_errors() {
    let ok = || Ok(());
    let err = run_with(&["frobnicate".into()], &ok, &ok, "cargo").unwrap_err();
    assert!(err.contains("unknown command"), "{err}");
}

#[test]
fn parse_gpu_stress_case_and_timeout() {
    assert_eq!(
        parse(&["gpu-stress".into(), "hang_infinite_loop".into()]).unwrap(),
        Cmd::GpuStress {
            case: "hang_infinite_loop".into(),
            timeout_secs: xtask::gpu::DEFAULT_TIMEOUT_SECS
        }
    );
    assert_eq!(
        parse(&[
            "gpu-stress".into(),
            "hang_host_event".into(),
            "--timeout".into(),
            "5".into()
        ])
        .unwrap(),
        Cmd::GpuStress {
            case: "hang_host_event".into(),
            timeout_secs: 5
        }
    );
}

#[test]
fn parse_gpu_stress_rejects_missing_case_and_bad_timeout() {
    let missing = parse(&["gpu-stress".into()]).unwrap_err();
    assert!(missing.contains("gpu-stress needs"), "{missing}");
    assert!(missing.contains("hang_infinite_loop"), "{missing}");
    assert!(missing.contains("hang_host_event"), "{missing}");

    let bad = parse(&[
        "gpu-stress".into(),
        "hang_infinite_loop".into(),
        "--timeout".into(),
        "soon".into(),
    ])
    .unwrap_err();
    assert!(bad.contains("--timeout needs"), "{bad}");
}

/// The gpu-stress arm maps the runner's exit codes; `true` stands in for a
/// runner that exits 0, and a nonexistent program for a spawn failure.
#[test]
fn gpu_stress_arm_reports_the_runner_outcome() {
    let ok = || Ok(());
    let msg = run_with(
        &["gpu-stress".into(), "hang_infinite_loop".into()],
        &ok,
        &ok,
        "true",
    )
    .unwrap();
    assert!(msg.contains("completed"), "{msg}");

    let err = run_with(
        &["gpu-stress".into(), "hang_infinite_loop".into()],
        &ok,
        &ok,
        "vivelite-definitely-not-a-program",
    )
    .unwrap_err();
    assert!(err.contains("could not run"), "{err}");
}

/// The gfr-probe arm reports the probe's exit status; `true` stands in for a
/// successful probe, and a nonexistent program for a spawn failure.
#[test]
fn gfr_probe_arm_reports_the_probe_outcome() {
    let ok = || Ok(());
    let msg = run_with(
        &["gfr-probe".into(), "--backend".into(), "portable".into()],
        &ok,
        &ok,
        "true",
    )
    .unwrap();
    assert!(msg.contains("completed"), "{msg}");

    let err = run_with(
        &["gfr-probe".into()],
        &ok,
        &ok,
        "vivelite-definitely-not-a-program",
    )
    .unwrap_err();
    assert!(err.contains("could not run"), "{err}");
}

#[test]
fn which_resolves_only_existing_files_on_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tool"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.path().join("tool"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let path =
        std::env::join_paths([dir.path().join("missing"), dir.path().to_path_buf()]).unwrap();
    assert_eq!(which_in(Some(path), "tool"), Some(dir.path().join("tool")));
    assert_eq!(which_in(Some(OsString::from(dir.path())), "absent"), None);
    assert_eq!(which_in(None, "tool"), None);
}

#[test]
fn curl_reports_spawn_failure_before_download() {
    let err = curl(
        "/nonexistent/vivelite-curl",
        "https://example.invalid/b.raucb",
        Path::new("unused.raucb"),
    )
    .unwrap_err();
    assert!(err.contains("could not run curl"), "{err}");
}

#[test]
fn require_https_accepts_only_https_urls() {
    assert!(require_https("https://example.com/a.raucb").is_ok());
    for bad in [
        "http://example.com/a.raucb",
        "--config=/tmp/x",
        "file:///etc/passwd",
        "https://",
        "https:///path",
        "",
    ] {
        assert!(require_https(bad).is_err(), "accepted {bad:?}");
    }
}

#[test]
fn fetch_command_refuses_plain_http_before_spawning() {
    let ok = || Ok(());
    let err = run_with(
        &["fetch".into(), "http://example.com/a.raucb".into()],
        &ok,
        &ok,
        "cargo",
    )
    .unwrap_err();
    assert!(err.contains("refusing non-https URL"), "{err}");
}

#[test]
fn curl_fetch_refuses_non_https_before_spawning() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out");
    let err = curl("/nonexistent/vivelite-curl", "--config=/tmp/x", &dest).unwrap_err();
    assert!(err.contains("refusing non-https URL"), "{err}");
    assert!(!dest.exists());
}

#[test]
fn run_curl_reports_spawn_failure() {
    let err = run_curl("/nonexistent/vivelite-curl", &[]).unwrap_err();
    assert!(err.contains("could not run curl"), "{err}");
}

#[test]
fn read_manifest_reports_read_and_parse_failures() {
    let dir = tempfile::tempdir().unwrap();
    let read_fail = FailingFs {
        fail: "read_to_string",
    };
    let err = read_manifest(&read_fail, dir.path()).unwrap_err();
    assert!(err.contains("injected read failure"), "{err}");

    std::fs::write(dir.path().join("manifest.raucm"), "[broken\n").unwrap();
    let err = read_manifest(&RealFs, dir.path()).unwrap_err();
    assert!(err.contains("bad manifest"), "{err}");

    std::fs::write(
        dir.path().join("manifest.raucm"),
        "[update]\ncompatible=steamos-aarch64\n",
    )
    .unwrap();
    let manifest = read_manifest(&RealFs, dir.path()).unwrap();
    assert_eq!(manifest.compatible.as_deref(), Some("steamos-aarch64"));
}

#[test]
fn doctor_report_passes_when_every_check_passes() {
    let lines = vec![(true, "curl available".to_string())];
    assert_eq!(doctor_report(&lines).unwrap(), "all checks passed");
}

#[test]
fn doctor_report_fails_and_counts_bad_checks() {
    let lines = vec![
        (true, "curl available".to_string()),
        (false, "rauc available".to_string()),
    ];
    assert_eq!(doctor_report(&lines).unwrap_err(), "1 check(s) failed");
}

#[test]
fn device_overlay_launcher_reports_success() {
    assert!(launch_device_overlay_with(|| Ok(())).is_ok());
}

#[test]
fn device_overlay_launcher_reports_window_failure() {
    let err = launch_device_overlay_with(|| {
        Err(device_ui::Error::WindowCreationFailed(Box::new(
            std::io::Error::other("no compositor"),
        )))
    })
    .unwrap_err();
    assert!(err.contains("could not launch device overlay"), "{err}");
}

#[test]
fn device_overlay_launcher_turns_a_panic_into_an_error() {
    fn panics() -> Result<(), device_ui::Error> {
        panic!("XOpenDisplayFailed")
    }
    let err = launch_device_overlay_with(panics).unwrap_err();
    assert!(err.contains("no display available"), "{err}");
}

/// Runs the real `device_ui::run` in a child with display variables removed,
/// so the headless error path is exercised against the actual iced runtime.
#[test]
fn device_overlay_launcher_handles_the_real_headless_runtime() {
    const CHILD: &str = "VIVELITE_XTASK_OVERLAY_HEADLESS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        assert!(launch_device_overlay().is_err());
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "device_overlay_launcher_handles_the_real_headless_runtime",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .status()
        .unwrap();
    assert!(status.success());
}

/// Wraps a real filesystem and fails one chosen operation, so each
/// `Fs` error branch of the stage pipeline can be reached in a test.
struct FailingFs {
    fail: &'static str,
}

impl Fs for FailingFs {
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        if self.fail == "create_dir_all" {
            return Err(std::io::Error::other("injected mkdir failure"));
        }
        RealFs.create_dir_all(path)
    }
    fn metadata_len(&self, path: &Path) -> std::io::Result<u64> {
        RealFs.metadata_len(path)
    }
    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        if self.fail == "read_to_string" {
            return Err(std::io::Error::other("injected read failure"));
        }
        RealFs.read_to_string(path)
    }
    fn exists(&self, path: &Path) -> bool {
        RealFs.exists(path)
    }
}

#[test]
fn stage_pipeline_surfaces_injected_faults() {
    const FEED: &str = r#"{"candidates":[{"buildid":"b1","version":"0.4.5","update_path":"vr/b1/deckard.raucb","chunks_store_path":"vr/chunks.castr"}]}"#;
    let work = tempfile::tempdir().unwrap();
    // Fails the first directory the pipeline creates: the bundles dir.
    let err = stage_with(
        &FailingFs {
            fail: "create_dir_all",
        },
        work.path(),
        FEED,
    )
    .unwrap_err();
    assert!(err.contains("injected mkdir failure"), "{err}");
}

#[test]
fn filesystem_faults_are_reported_with_context() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = FailingFs {
        fail: "create_dir_all",
    };
    let err = fs::ensure_dir(&blocked, &dir.path().join("bundles")).unwrap_err();
    assert!(err.contains("injected mkdir failure"), "{err}");
    let reader = FailingFs {
        fail: "read_to_string",
    };
    let err = fs::read_text(&reader, &dir.path().join("manifest.raucm")).unwrap_err();
    assert!(err.contains("injected read failure"), "{err}");
}
