//! `cargo xtask gpu-stress <case>`: opt-in GPU robustness workloads.
//!
//! The Halt and Catch Fire hang cases live in the `vivelite-hcf` crate.
//! Running one builds the `hcf-runner` binary with the `vulkan` feature
//! (ash linked against the Vulkan loader), which is why this command is
//! opt-in and outside `dev` and the coverage gate: the default workspace
//! build and tests never compile device code.
//!
//! The runner inherits stdio, so its live progress output and the
//! watchdog's last words are visible here.

use hcf::{EXIT_DEVICE_LOST, EXIT_HANG};

/// Default watchdog timeout, matching upstream HCF's termination timer.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// The cargo invocation that builds and runs one case, split out so the
/// command line is unit-testable.
pub fn runner_args(case: &str, timeout_secs: u64) -> Vec<String> {
    vec![
        "run".into(),
        "--quiet".into(),
        "--package".into(),
        "vivelite-hcf".into(),
        "--features".into(),
        "vulkan".into(),
        "--bin".into(),
        "hcf-runner".into(),
        "--".into(),
        case.into(),
        "--timeout".into(),
        timeout_secs.to_string(),
    ]
}

/// Builds and runs one case, translating the runner's exit code into a
/// report. The cargo binary is injected so tests can exercise the mapping
/// without building anything.
pub fn run(cargo_bin: &str, case: &str, timeout_secs: u64) -> Result<String, String> {
    let args = runner_args(case, timeout_secs);
    let status = std::process::Command::new(cargo_bin)
        .args(&args)
        .status()
        .map_err(|e| format!("could not run {cargo_bin}: {e}"))?;
    classify(case, timeout_secs, status.code())
}

/// Maps a runner exit code (`None` for a signal) to the report. Split out so
/// every exit class is testable without spawning a process.
pub fn classify(case: &str, timeout_secs: u64, code: Option<i32>) -> Result<String, String> {
    match code {
        Some(0) => Ok(format!("gpu-stress {case}: completed (driver survived)")),
        Some(EXIT_HANG) => Ok(format!(
            "gpu-stress {case}: hang detected (watchdog fired after {timeout_secs}s)"
        )),
        Some(EXIT_DEVICE_LOST) => Ok(format!(
            "gpu-stress {case}: crash detected (driver reported VK_ERROR_DEVICE_LOST)"
        )),
        Some(code) => Err(format!("gpu-stress {case} failed with exit code {code}")),
        None => Err(format!("gpu-stress {case} terminated by a signal")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_args_build_the_opt_in_invocation() {
        assert_eq!(
            runner_args("hang_host_event", 10),
            vec![
                "run".to_string(),
                "--quiet".to_string(),
                "--package".to_string(),
                "vivelite-hcf".to_string(),
                "--features".to_string(),
                "vulkan".to_string(),
                "--bin".to_string(),
                "hcf-runner".to_string(),
                "--".to_string(),
                "hang_host_event".to_string(),
                "--timeout".to_string(),
                "10".to_string(),
            ]
        );
    }

    #[test]
    fn classify_maps_every_exit_class() {
        let ok = classify("hang_infinite_loop", 7, Some(0)).expect("survived");
        assert!(ok.contains("completed"), "{ok}");
        let hang = classify("hang_infinite_loop", 7, Some(EXIT_HANG)).expect("hang");
        assert!(
            hang.contains("hang detected") && hang.contains("7s"),
            "{hang}"
        );
        let lost = classify("hang_infinite_loop", 7, Some(EXIT_DEVICE_LOST)).expect("lost");
        assert!(lost.contains("VK_ERROR_DEVICE_LOST"), "{lost}");
        let failed = classify("hang_infinite_loop", 7, Some(1)).expect_err("failure");
        assert!(failed.contains("exit code 1"), "{failed}");
        let signal = classify("hang_infinite_loop", 7, None).expect_err("signal");
        assert!(signal.contains("terminated by a signal"), "{signal}");
    }

    #[test]
    fn run_reports_spawn_failures() {
        let err = run("vivelite-definitely-not-a-program", "hang_host_event", 5)
            .expect_err("spawn must fail");
        assert!(err.contains("could not run"), "{err}");
    }
}
