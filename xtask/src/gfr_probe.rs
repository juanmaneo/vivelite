//! `cargo xtask gfr-probe`: run the opt-in Vulkan flight-recorder probe.
//!
//! The Vulkan path is feature-gated and intentionally excluded from the
//! default build and coverage gate. It verifies marker write/readback on the
//! current device; it does not intercept another application's submissions.

use gfr::BackendRequest;

/// The CLI spelling of an explicit backend request; `None` for `Auto`, which
/// needs no `--backend` flag.
fn backend_name(backend: BackendRequest) -> Option<&'static str> {
    match backend {
        BackendRequest::Auto => None,
        BackendRequest::Amd => Some("amd"),
        BackendRequest::Portable => Some("portable"),
    }
}

/// Builds the cargo invocation for the feature-gated probe.
pub fn runner_args(backend: BackendRequest) -> Vec<String> {
    let mut args = vec![
        "run".into(),
        "--quiet".into(),
        "--package".into(),
        "vivelite-gfr".into(),
        "--features".into(),
        "vulkan".into(),
        "--bin".into(),
        "gfr-probe".into(),
    ];
    if let Some(name) = backend_name(backend) {
        args.extend(["--".into(), "--backend".into(), name.into()]);
    }
    args
}

/// Builds and runs the probe, translating its status into a user-facing report.
pub fn run(cargo_bin: &str, backend: BackendRequest) -> Result<String, String> {
    let status = std::process::Command::new(cargo_bin)
        .args(runner_args(backend))
        .status()
        .map_err(|error| format!("could not run {cargo_bin}: {error}"))?;
    classify(status.code())
}

/// Maps a process exit status (`None` means terminated by a signal).
pub fn classify(code: Option<i32>) -> Result<String, String> {
    match code {
        Some(0) => Ok("gfr-probe: marker flight completed".into()),
        Some(code) => Err(format!("gfr-probe failed with exit code {code}")),
        None => Err("gfr-probe terminated by a signal".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_args_cover_auto_and_explicit_backend() {
        assert_eq!(
            runner_args(BackendRequest::Auto),
            vec![
                "run",
                "--quiet",
                "--package",
                "vivelite-gfr",
                "--features",
                "vulkan",
                "--bin",
                "gfr-probe",
            ]
        );
        assert_eq!(
            runner_args(BackendRequest::Amd),
            vec![
                "run",
                "--quiet",
                "--package",
                "vivelite-gfr",
                "--features",
                "vulkan",
                "--bin",
                "gfr-probe",
                "--",
                "--backend",
                "amd",
            ]
        );
        assert_eq!(
            runner_args(BackendRequest::Portable)
                .last()
                .map(String::as_str),
            Some("portable")
        );
    }

    #[test]
    fn classify_maps_all_process_outcomes() {
        assert_eq!(
            classify(Some(0)),
            Ok("gfr-probe: marker flight completed".into())
        );
        assert!(classify(Some(1)).unwrap_err().contains("exit code 1"));
        assert!(classify(None).unwrap_err().contains("signal"));
    }

    #[test]
    fn run_reports_spawn_failures() {
        let error = run("vivelite-definitely-not-a-program", BackendRequest::Auto).unwrap_err();
        assert!(error.contains("could not run"));
    }
}
