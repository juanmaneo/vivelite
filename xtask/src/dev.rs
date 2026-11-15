//! Local dev-check: build, test, and enforce 100% coverage.
//!
//! Runs the same gates a contributor runs before merging, in order, and
//! stops at the first failure. The cargo binary is injected so the full
//! sequence can be exercised in tests without recursing into `cargo test`.

use std::process::Command;

/// One gate in the dev-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub label: &'static str,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_builds_tests_then_enforces_full_coverage() {
        let steps = plan();
        assert_eq!(steps[0].args, ["build", "--workspace"]);
        assert_eq!(steps[1].args, ["test", "--workspace"]);
        let cov = &steps[2].args;
        assert_eq!(cov[0], "llvm-cov");
        for gate in [
            "--fail-under-functions",
            "--fail-under-lines",
            "--fail-under-regions",
        ] {
            let i = cov.iter().position(|a| *a == gate).expect("gate present");
            assert_eq!(cov[i + 1], "100");
        }
    }

    #[test]
    fn run_passes_when_every_gate_passes() {
        assert!(run("true").is_ok());
    }

    #[test]
    fn run_reports_the_failing_gate() {
        let err = run("false").unwrap_err();
        assert!(err.contains("build workspace"), "{err}");
    }

    #[test]
    fn run_reports_missing_binary() {
        let err = run("definitely-not-a-real-cargo-binary").unwrap_err();
        assert!(err.contains("could not run"), "{err}");
    }

    #[test]
    fn cargo_program_reads_and_restores_cargo_env() {
        let saved = std::env::var_os("CARGO");
        std::env::set_var("CARGO", "vivelite-test-cargo");
        assert_eq!(cargo_program(), "vivelite-test-cargo");
        std::env::remove_var("CARGO");
        assert_eq!(cargo_program(), "cargo");
        match saved {
            Some(value) => std::env::set_var("CARGO", value),
            None => std::env::remove_var("CARGO"),
        }
    }
}
