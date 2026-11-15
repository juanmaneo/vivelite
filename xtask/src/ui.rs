use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use vivelite_core::device_layout;

use crate::{doctor_checks, work_root};

struct Dashboard {
    checks: Vec<(bool, String)>,
    output: String,
    branch: String,
    bundle_url: String,
    busy: bool,
    last_refresh: Instant,
    task_tx: Sender<String>,
    task_rx: Receiver<String>,
}

impl Dashboard {
    fn new() -> Self {
        let (task_tx, task_rx) = mpsc::channel();
        let mut dashboard = Self {
            checks: Vec::new(),
            output: "Review host readiness, then run a workflow.".into(),
            branch: "stable".into(),
            bundle_url: std::env::var("VIVELITE_BUNDLE_URL").unwrap_or_default(),
            busy: false,
            last_refresh: Instant::now(),
            task_tx,
            task_rx,
        };
        dashboard.refresh_checks();
        dashboard
    }

    fn refresh_checks(&mut self) {
        self.checks = doctor_checks(&device_layout(&work_root()));
        self.last_refresh = Instant::now();
    }

    fn start_task(&mut self, label: &str, args: Vec<String>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.output = format!("Running {label}…");
        let tx = self.task_tx.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(|| crate::run(&args))
                .unwrap_or_else(|_| Err("task stopped unexpectedly".into()));
            let message = match result {
                Ok(message) => format!("✓ {message}"),
                Err(error) => format!("✗ {error}"),
            };
            let _ = tx.send(message);
        });
    }
}

impl eframe::App for Dashboard {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            ui.label(format!("Workspace: {}", work_root().display()));
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

pub fn launch() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 620.0])
            .with_min_inner_size([600.0, 480.0])
            .with_title("vivelite — Host dashboard"),
        ..Default::default()
    };
    eframe::run_native(
        "vivelite — Host dashboard",
        options,
        Box::new(|_cc| Ok(Box::new(Dashboard::new()))),
    )
}
