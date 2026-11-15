//! `cargo xtask device-ui --xr`: the overlay against the real compositor.
//!
//! The XR render path lives in the `vivelite-device-ui` crate behind the
//! `xr` feature (iced's wgpu engine, the OpenXR `link` wrapper, ash),
//! which is why this arm is opt-in like `gpu-stress`: the default
//! workspace build and the coverage gate never compile device code. The
//! runner inherits stdio, so the overlay's phase log is visible here.

/// The cargo invocation that builds and runs the overlay, split out so
/// the command line is unit-testable.
pub fn runner_args() -> Vec<String> {
    vec![
        "run".into(),
        "--quiet".into(),
        "--package".into(),
        "vivelite-device-ui".into(),
        "--features".into(),
        "xr".into(),
        "--bin".into(),
        "xr-overlay".into(),
    ]
}

/// Builds and runs the overlay, translating the runner's exit code into
/// a report. The cargo binary is injected so tests can exercise the
/// mapping without building anything.
pub fn run(cargo_bin: &str) -> Result<String, String> {
    let args = runner_args();
    let status = std::process::Command::new(cargo_bin)
        .args(&args)
        .status()
        .map_err(|e| format!("could not run {cargo_bin}: {e}"))?;
    classify(status.code())
}

/// Maps a runner exit code (`None` for a signal) to the report. Split out
/// so every exit class is testable without spawning a process.
pub fn classify(code: Option<i32>) -> Result<String, String> {
    match code {
        Some(0) => Ok("xr overlay exited (session lost or runtime exit)".into()),
        Some(1) => {
            Err("xr overlay failed; is an OpenXR runtime active (monado)? see output".into())
        }
        Some(code) => Err(format!("xr overlay failed with exit code {code}")),
        None => Err("xr overlay terminated by a signal".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_args_build_the_opt_in_invocation() {
        assert_eq!(
            runner_args(),
            vec![
                "run".to_string(),
                "--quiet".to_string(),
                "--package".to_string(),
                "vivelite-device-ui".to_string(),
                "--features".to_string(),
                "xr".to_string(),
                "--bin".to_string(),
                "xr-overlay".to_string(),
            ]
        );
    }

    #[test]
    fn classify_maps_every_exit_class() {
        let ok = classify(Some(0)).expect("clean exit");
        assert!(ok.contains("session lost"), "{ok}");
        let failure = classify(Some(1)).expect_err("runtime failure");
        assert!(failure.contains("OpenXR runtime"), "{failure}");
        let failed = classify(Some(2)).expect_err("unexpected code");
        assert!(failed.contains("exit code 2"), "{failed}");
        let signal = classify(None).expect_err("signal");
        assert!(signal.contains("terminated by a signal"), "{signal}");
    }

    #[test]
    fn run_classifies_the_runner_exit_status() {
        let ok = run("true").expect("a clean runner exit is reported");
        assert!(ok.contains("session lost"), "{ok}");
    }

    #[test]
    fn run_reports_spawn_failures() {
        let err = run("vivelite-definitely-not-a-program").expect_err("spawn must fail");
        assert!(err.contains("could not run"), "{err}");
    }
}
