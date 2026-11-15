//! `hcf-runner <case> [--timeout <secs>]`: runs one Halt and Catch Fire
//! case on the first compute-capable Vulkan device.
//!
//! Built and started by `cargo xtask gpu-stress <case>` (which enables the
//! `vulkan` feature); not part of the default workspace build, its tests,
//! or the coverage gate. A real hang never returns — the watchdog
//! terminates the process, exactly like upstream HCF's termination timer.
//!
//! Exit codes:
//!
//! - 0: the run finished and the driver stayed alive (`completed`)
//! - 1: harness failure (unknown case, no loader/device, driver error)
//! - 3: hang detected — the watchdog had to terminate the run
//! - 4: crash detected — the driver reported `VK_ERROR_DEVICE_LOST`

use std::time::Duration;

use hcf::vulkan;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

/// Parses arguments and runs one case, returning the process exit code.
fn run(args: &[String]) -> i32 {
    let Some(name) = args.first().cloned() else {
        print_usage();
        return 1;
    };
    let timeout = match flag_value(args, "--timeout") {
        Some(raw) => match raw.parse::<u64>() {
            Ok(secs) => Duration::from_secs(secs),
            Err(_) => {
                eprintln!("error: --timeout needs a number of seconds, got {raw:?}");
                return 1;
            }
        },
        None => Duration::from_secs(DEFAULT_TIMEOUT_SECS),
    };

    let Some(case) = hcf::case(&name) else {
        eprintln!(
            "error: unknown case {name:?}; known cases: {}",
            hcf::cases()
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join(" ")
        );
        return 1;
    };

    eprintln!(
        "hcf: {name} — {} (timeout {}s)",
        case.summary,
        timeout.as_secs()
    );
    let watchdog = hcf::Watchdog::arm(timeout, move || {
        eprintln!(
            "hcf: watchdog fired after {}s: GPU hang detected",
            timeout.as_secs()
        );
        std::process::exit(hcf::EXIT_HANG);
    });

    let result = vulkan::execute(case);
    watchdog.disarm();

    match result {
        Ok(outcome) => {
            println!("hcf: {name}: {outcome}");
            match outcome {
                vulkan::Outcome::Completed => 0,
                vulkan::Outcome::HangDetected => hcf::EXIT_HANG,
                vulkan::Outcome::DeviceLost => hcf::EXIT_DEVICE_LOST,
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Upstream HCF's default termination timer, in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// The value after `flag`, if present.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Prints the one-line usage.
fn print_usage() {
    eprintln!("usage: hcf-runner <case> [--timeout <secs>]");
}
