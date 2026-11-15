//! Headless egui dashboard tests, driven through `egui_kittest`.
//!
//! Lives in its own integration-test binary so the library compiles once
//! for coverage and so process-env changes here stay isolated from the
//! other test binaries.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use xtask::ui::{new_app, run_cli_task, Dashboard, TaskRunner};

fn work_dir() -> PathBuf {
    tempfile::tempdir().unwrap().path().to_path_buf()
}

/// Harness driving the real `eframe::App::update` implementation.
fn harness(work: PathBuf, runner: TaskRunner) -> Harness<'static, Dashboard> {
    Harness::builder()
        .with_size(egui::vec2(760.0, 620.0))
        .build_eframe(move |_cc| Dashboard::with_runner(work, runner))
}

fn ok_runner() -> TaskRunner {
    Arc::new(|args: &[String]| Ok(format!("ok: {}", args.join(" "))))
}

/// Steps until the background task result has been applied.
fn drain_until_idle(h: &mut Harness<'static, Dashboard>) {
    let mut steps = 0;
    while h.state().busy && steps < 200 {
        h.run_steps(1);
        steps += 1;
    }
    assert!(!h.state().busy, "dashboard task never finished");
}

#[test]
fn default_dashboard_uses_production_defaults() {
    let dashboard = Dashboard::default();
    assert_eq!(
        dashboard.output,
        "Review host readiness, then run a workflow."
    );
}

#[test]
fn dashboard_renders_and_reports_closed_status() {
    let mut h = harness(work_dir(), ok_runner());
    h.run_steps(2);
    h.get_by_label_contains("System readiness");
    h.get_by_label_contains("Setup workflows");
    h.get_by_label_contains("Review host readiness");

    h.get_by_label("Doctor").click();
    h.run_steps(1);
    drain_until_idle(&mut h);
    assert!(h.state().output.starts_with("OK: "), "{}", h.state().output);
}

#[test]
fn refresh_button_and_periodic_refresh_reset_checks() {
    let mut h = harness(work_dir(), ok_runner());
    h.get_by_label("Refresh checks").click();
    h.run_steps(2);

    // Both check colors must be exercised deterministically, regardless
    // of which tools happen to be installed on the build host.
    h.state_mut().checks = vec![(true, "good check".into()), (false, "bad check".into())];
    h.run_steps(2);

    // Aging past the refresh interval triggers an automatic refresh.
    let stale = Instant::now() - Duration::from_secs(30);
    h.state_mut().last_refresh = stale;
    h.run_steps(2);
    assert!(h.state().last_refresh > stale);
}

#[test]
fn busy_dashboard_blocks_new_tasks_and_reports_errors() {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let runner: TaskRunner = Arc::new(move |args: &[String]| {
        release_rx.lock().unwrap().recv().expect("release signal");
        Err(format!("boom: {}", args.join(" ")))
    });
    let mut h = harness(work_dir(), runner);

    // A blank branch falls back to "stable".
    h.state_mut().branch = "  ".into();
    h.get_by_label("Stage SteamOS").click();
    h.run_steps(2);
    assert!(h.state().busy);
    assert_eq!(h.state().output, "Running stage…");

    // While busy: the refresh click is ignored and other buttons are
    // disabled, so no second task can start.
    h.get_by_label("Refresh checks").click();
    h.state_mut().bundle_url = "https://example.invalid/x.raucb".into();
    h.get_by_label("Fetch bundle").click();
    h.run_steps(2);
    assert_eq!(h.state().output, "Running stage…");

    release_tx.send(()).expect("runner alive");
    drain_until_idle(&mut h);
    assert_eq!(h.state().output, "ERROR: boom: stage stable");
}

#[test]
fn workflow_buttons_run_their_commands() {
    let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&recorded);
    let runner: TaskRunner = Arc::new(move |args: &[String]| {
        sink.lock().unwrap().push(args.to_vec());
        Ok("done".into())
    });
    let mut h = harness(work_dir(), runner);

    h.state_mut().branch = "beta".into();
    h.state_mut().bundle_url = "https://example.invalid/b.raucb".into();
    for label in ["Stage SteamOS", "Fetch bundle", "Verify bundle", "Doctor"] {
        h.get_by_label(label).click();
        h.run_steps(1);
        drain_until_idle(&mut h);
    }

    assert_eq!(
        *recorded.lock().unwrap(),
        vec![
            vec!["stage".to_string(), "beta".to_string()],
            vec![
                "fetch".to_string(),
                "https://example.invalid/b.raucb".to_string()
            ],
            vec!["verify".to_string()],
            vec!["doctor".to_string()],
        ]
    );
    assert_eq!(h.state().output, "OK: done");
}

#[test]
fn panicking_runner_is_reported_as_an_error() {
    let runner: TaskRunner = Arc::new(|_args: &[String]| panic!("runner exploded"));
    let mut h = harness(work_dir(), runner);
    h.get_by_label("Doctor").click();
    h.run_steps(1);
    drain_until_idle(&mut h);
    assert_eq!(h.state().output, "ERROR: task stopped unexpectedly");
}

#[test]
fn busy_flag_blocks_start_task_without_a_thread() {
    let mut dashboard = Dashboard::with_runner(work_dir(), ok_runner());
    dashboard.busy = true;
    let before = dashboard.output.clone();
    dashboard.start_task("doctor", vec!["doctor".into()]);
    assert!(dashboard.busy);
    assert_eq!(dashboard.output, before);
}

#[test]
fn new_app_builds_the_dashboard_from_the_creation_context() {
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    assert!(new_app(&cc).is_ok());
}

/// Run native launch in a child with display variables removed, so this
/// test reaches the no-display error path without opening a real window.
#[test]
fn launch_without_a_display_reports_an_error() {
    const CHILD: &str = "VIVELITE_XTASK_UI_HEADLESS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let outcome = std::panic::catch_unwind(xtask::ui::launch);
        assert!(matches!(outcome, Ok(Err(_)) | Err(_)));
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "launch_without_a_display_reports_an_error",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn new_dashboard_reads_env_and_cli_runner_still_works() {
    let _dashboard = Dashboard::new();
    let help = run_cli_task(&["help".to_string()]).unwrap();
    assert!(help.contains("cargo xtask dev"), "{help}");
}
