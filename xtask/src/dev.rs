//! Local dev-check: build, test, and enforce 100% coverage.
//!
//! Runs the same gates a contributor runs before merging, in order, and
//! stops at the first failure. The cargo binary is injected so the full
//! sequence can be exercised in tests without recursing into `cargo test`.

use std::process::Command;

/// One gate in the dev-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Human-readable name shown by the dev-check as `==> <label>`.
    pub label: &'static str,
    /// Argument vector handed to cargo for this gate.
    pub args: Vec<&'static str>,
}

/// The ordered gates: build, test, then the 100% coverage requirement.
pub fn plan() -> Vec<Step> {
    vec![
        Step {
            label: "build workspace",
            args: vec!["build", "--workspace"],
        },
        Step {
            label: "test workspace",
            args: vec!["test", "--workspace"],
        },
        Step {
            label: "coverage: require 100% functions, lines, and regions",
            args: vec![
                "llvm-cov",
                "--workspace",
                "--fail-under-functions",
                "100",
                "--fail-under-lines",
                "100",
                "--fail-under-regions",
                "100",
            ],
        },
    ]
}

/// Run every gate with `cargo_bin`, stopping at the first failure.
pub fn run(cargo_bin: &str) -> Result<String, String> {
    for step in plan() {
        println!("==> {}", step.label);
        let status = Command::new(cargo_bin)
            .args(&step.args)
            .status()
            .map_err(|e| format!("could not run {cargo_bin}: {e}"))?;
        if !status.success() {
            return Err(format!("dev-check failed at '{}': {status}", step.label));
        }
    }
    Ok("dev-check passed: build, test, 100% coverage".into())
}

/// The cargo binary to use: the one cargo set for this xtask, or `cargo`.
pub fn cargo_program() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// Puts `CARGO` back to `saved`, so a test that changed it leaves no trace.
pub fn restore_cargo_env(saved: Option<std::ffi::OsString>) {
    match saved {
        Some(value) => std::env::set_var("CARGO", value),
        None => std::env::remove_var("CARGO"),
    }
}
