# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> [!WARNING]
> Work in progress. Nothing here is stable yet; APIs may change at any time.

## [Unreleased]

### Added

- `LICENSE-MIT` and `LICENSE-APACHE` license texts (dual-licensed `MIT OR Apache-2.0`).
- `CHANGELOG.md`.

## [0.1.1]

### Added

- Workspace with `xtask` build orchestrator and `vivelite-core` stage/layout model.
- `bundle` crate: fetch (streamed, resumable), sha256 + CMS signature verification (rauc), and SteamOS component extraction.
- `wine-sys` / `wine`: prefix management, embedded loader init, and version probe.
- `fex-sys` / `fex`: BT interface and unixlib dispatch, device capability model.
- `monado-sys` / `monado`: OpenXR loader probe and runtime endpoint detection.
- `device-ui` crate skeleton (device UI not yet implemented).
- Host egui dashboard via `cargo xtask ui`.
- `cargo xtask dev`: build, test, and 100% function/line/region coverage gate.
