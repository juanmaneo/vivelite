//! `gfr-probe [--backend auto|amd|portable]`: run a small Vulkan marker flight.
//!
//! This feature-gated diagnostic verifies that the selected marker backend
//! can submit and read back a completed flight. It is not a transparent
//! Vulkan layer and does not record arbitrary application command buffers.

use std::str::FromStr;

use gfr::BackendRequest;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

fn run(args: &[String]) -> i32 {
    let backend = match backend_arg(args) {
        Ok(backend) => backend,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("usage: gfr-probe [--backend auto|amd|portable]");
            return 1;
        }
    };
    match gfr::vulkan::run_probe(backend) {
        Ok(report) => {
            println!("device: {}", report.device_name);
            println!("backend: {:?}", report.backend);
            println!("flight: {}", report.flight);
            if report.flight.completed == report.flight.total {
                0
            } else {
                2
            }
        }
        Err(error) => {
            eprintln!("gfr probe failed: {error}");
            1
        }
    }
}

fn backend_arg(args: &[String]) -> Result<BackendRequest, String> {
    let mut selected = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] != "--backend" {
            return Err(format!("unexpected argument {:?}", args[index]));
        }
        if selected.is_some() {
            return Err("--backend may only be specified once".into());
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| "--backend needs auto, amd, or portable".to_string())?;
        selected = Some(BackendRequest::from_str(value).map_err(|error| error.to_string())?);
        index += 2;
    }
    Ok(selected.unwrap_or(BackendRequest::Auto))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_argument_defaults_and_parses() {
        assert_eq!(backend_arg(&[]), Ok(BackendRequest::Auto));
        assert_eq!(
            backend_arg(&["--backend".into(), "amd".into()]),
            Ok(BackendRequest::Amd)
        );
        assert_eq!(
            backend_arg(&["--backend".into(), "portable".into()]),
            Ok(BackendRequest::Portable)
        );
    }

    #[test]
    fn backend_argument_rejects_invalid_forms() {
        assert!(backend_arg(&["--other".into()])
            .unwrap_err()
            .contains("unexpected"));
        assert!(backend_arg(&["--backend".into()])
            .unwrap_err()
            .contains("needs"));
        assert!(backend_arg(&["--backend".into(), "bad".into()])
            .unwrap_err()
            .contains("unknown"));
        assert!(backend_arg(&[
            "--backend".into(),
            "auto".into(),
            "--backend".into(),
            "amd".into(),
        ])
        .unwrap_err()
        .contains("once"));
    }
}
