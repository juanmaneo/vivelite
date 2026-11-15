//! Host dashboard UI (egui) for the desktop side.
//!
//! The window wiring lives in [`launch`]; everything the dashboard draws is
//! in [`Dashboard::draw`], which is driven headlessly by `egui_kittest` in
//! the tests below. Workflow commands run on a background thread through an
//! injectable [`TaskRunner`] so UI tests never touch the network or the
//! staging tools.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use vivelite_core::device_layout;

use crate::{doctor_checks, work_root};

/// Runs one dashboard workflow command. Production wires this to the xtask
/// CLI; tests inject a fake so UI tests stay hermetic.
type TaskRunner = Arc<dyn Fn(&[String]) -> Result<String, String> + Send + Sync>;

struct Dashboard {
    checks: Vec<(bool, String)>,
    output: String,
    branch: String,
    bundle_url: String,
    busy: bool,
    last_refresh: Instant,
    task_tx: Sender<String>,
    task_rx: Receiver<String>,
    work: PathBuf,
    runner: TaskRunner,
}

impl Dashboard {
    fn new() -> Self {
        let mut dashboard = Self::with_runner(work_root(), Arc::new(run_cli_task));
        dashboard.bundle_url = std::env::var("VIVELITE_BUNDLE_URL").unwrap_or_default();
        dashboard
    }

    fn with_runner(work: PathBuf, runner: TaskRunner) -> Self {
        let (task_tx, task_rx) = mpsc::channel();
        let mut dashboard = Self {
            checks: Vec::new(),
            output: "Review host readiness, then run a workflow.".into(),
            branch: "stable".into(),
            bundle_url: String::new(),
            busy: false,
            last_refresh: Instant::now(),
            task_tx,
            task_rx,
            work,
            runner,
        };
        dashboard.refresh_checks();
        dashboard
    }

    fn refresh_checks(&mut self) {
        self.checks = doctor_checks(&device_layout(&self.work));
        self.last_refresh = Instant::now();
    }

    fn start_task(&mut self, label: &str, args: Vec<String>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.output = format!("Running {label}…");
        let tx = self.task_tx.clone();
        let runner = Arc::clone(&self.runner);
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner(&args)))
                .unwrap_or_else(|_| Err("task stopped unexpectedly".into()));
            let message = match result {
                Ok(message) => format!("✓ {message}"),
                Err(error) => format!("✗ {error}"),
            };
            let _ = tx.send(message);
        });
    }

    fn draw(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.task_rx.try_recv() {
            self.output = message;
            self.busy = false;
            self.refresh_checks();
        }
        if self.last_refresh.elapsed() > Duration::from_secs(15) {
            self.refresh_checks();
        }
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("vivelite");
                ui.label("Host dashboard");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Refresh checks").clicked() && !self.busy {
                        self.refresh_checks();
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("System readiness");
            ui.label(format!("Workspace: {}", self.work.display()));
            ui.add_space(8.0);

            egui::Frame::group(ui.style()).show(ui, |ui| {
                for (ok, message) in &self.checks {
                    ui.horizontal(|ui| {
                        let color = if *ok {
                            egui::Color32::from_rgb(90, 200, 130)
                        } else {
                            egui::Color32::from_rgb(235, 100, 100)
                        };
                        ui.colored_label(color, "●");
                        ui.label(message);
                    });
                }
            });

            ui.add_space(18.0);
            ui.heading("Setup workflows");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Branch");
                ui.text_edit_singleline(&mut self.branch);
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Stage SteamOS"))
                    .clicked()
                {
                    let branch = if self.branch.trim().is_empty() {
                        "stable"
                    } else {
                        self.branch.trim()
                    };
                    self.start_task("stage", vec!["stage".into(), branch.into()]);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Bundle URL");
                ui.text_edit_singleline(&mut self.bundle_url);
                if ui
                    .add_enabled(
                        !self.busy && !self.bundle_url.trim().is_empty(),
                        egui::Button::new("Fetch bundle"),
                    )
                    .clicked()
                {
                    self.start_task("fetch", vec!["fetch".into(), self.bundle_url.trim().into()]);
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Verify bundle"))
                    .clicked()
                {
                    self.start_task("verify", vec!["verify".into()]);
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Doctor"))
                    .clicked()
                {
                    self.start_task("doctor", vec!["doctor".into()]);
                }
            });

            ui.add_space(18.0);
            ui.heading("Activity");
            if self.busy {
                ui.spinner();
            }
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(&self.output);
            });
        });
    }
}

/// Production task runner: reuse the xtask command implementations.
fn run_cli_task(args: &[String]) -> Result<String, String> {
    crate::run(args)
}

impl eframe::App for Dashboard {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw(ctx);
    }
}

pub fn launch() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 620.0])
            .with_min_inner_size([600.0, 480.0])
            .with_title("vivelite — Host dashboard"),
        ..Default::default()
    };
    eframe::run_native("vivelite — Host dashboard", options, Box::new(new_app))
}

fn new_app(
    _cc: &eframe::CreationContext<'_>,
) -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Box::new(Dashboard::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

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
        for _ in 0..200 {
            h.run_steps(1);
            if !h.state().busy {
                break;
            }
        }
        assert!(!h.state().busy, "dashboard task never finished");
    }

    #[test]
    fn dashboard_renders_and_reports_closed_status() {
        let mut h = harness(work_dir(), ok_runner());
        h.run_steps(2);
        h.get_by_label_contains("System readiness");
        h.get_by_label_contains("Setup workflows");
        h.get_by_label_contains("Review host readiness");
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
        assert_eq!(h.state().output, "✗ boom: stage stable");
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
        assert_eq!(h.state().output, "✓ done");
    }

    #[test]
    fn panicking_runner_is_reported_as_an_error() {
        let runner: TaskRunner = Arc::new(|_args: &[String]| panic!("runner exploded"));
        let mut h = harness(work_dir(), runner);
        h.get_by_label("Doctor").click();
        h.run_steps(1);
        drain_until_idle(&mut h);
        assert_eq!(h.state().output, "✗ task stopped unexpectedly");
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

    /// Opening a real window needs a display. Hosts without one take the
    /// error path; hosts with one skip so the test never opens a window.
    #[test]
    fn launch_without_a_display_reports_an_error() {
        if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return;
        }
        assert!(launch().is_err());
    }

    #[test]
    fn new_dashboard_reads_env_and_cli_runner_still_works() {
        let _dashboard = Dashboard::new();
        let help = run_cli_task(&["help".to_string()]).unwrap();
        assert!(help.contains("cargo xtask dev"), "{help}");
    }
}
