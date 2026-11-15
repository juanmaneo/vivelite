//! Tests for the local dev-check gates.

use xtask::dev::{cargo_program, plan, restore_cargo_env, run};

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
    restore_cargo_env(Some("vivelite-test-cargo".into()));
    assert_eq!(cargo_program(), "vivelite-test-cargo");
    restore_cargo_env(None);
    assert_eq!(cargo_program(), "cargo");
    restore_cargo_env(saved);
}
