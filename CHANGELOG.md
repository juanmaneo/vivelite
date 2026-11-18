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
- Workspace with `xtask` build orchestrator and `vivelite-core` stage/layout model.
- `bundle` crate: fetch (streamed, resumable), sha256 + CMS signature verification (rauc), and SteamOS component extraction.
- `wine-sys` / `wine`: prefix management, embedded loader init, and version probe.
- `fex-sys` / `fex`: BT interface and unixlib dispatch, device capability model.
- `monado-sys` / `monado`: OpenXR loader probe and runtime endpoint detection.
- `device-ui` crate skeleton (device UI not yet implemented).
- Host egui dashboard via `cargo xtask ui`.
- `cargo xtask dev`: build, test, and 100% function/line/region coverage gate.
- `hcf` crate: Halt and Catch Fire hang cases (`hang_infinite_loop`,
  `hang_host_event`) ported to Rust with `ash`, with GLSL kept in-tree and
  translated to SPIR-V at runtime via `naga`; the device execution path
  lives behind the opt-in `vulkan` feature.
- `cargo xtask gpu-stress <case> [--timeout <secs>]`: opt-in GPU robustness
  workloads, outside `dev` and the coverage gate, with a force-exit
  watchdog (exit code 3 = hang detected, 4 = device lost).
- `gfr` crate: pure-Rust marker plans and flight analysis, plus opt-in Vulkan
  AMD-marker and portable compute-marker backends; `cargo xtask gfr-probe`
  checks marker submission/readback. The portable path is intended for Adreno;
  real XR Elite hardware validation is pending.
- `monado-sys`: session-path bindings — `XrSessionCreateInfo` +
  `XrGraphicsBindingVulkanKHR`, reference space, swapchain
  (create/acquire/wait/release/enumerate images), frame loop
  (wait/begin/end), `XrCompositionLayerQuad`, and the event buffer —
  with layout tests pinning every struct against the Khronos `openxr.h`.
- `monado`: session/swapchain safe wrapper (feature `link`):
  `Instance::create_session`/`poll_event`, `Session` (begin/end/frames,
  spaces, swapchains; children borrow their session so the compiler
  enforces the spec's destruction order), plus pure-data session state,
  event parsing, quad/swapchain specs for the overlay.
- `device-ui`: compositor phase model — OpenXR session events map onto
  the overlay's attach phase, shown on the staging page.
- `device-ui` `xr` feature: the overlay's OpenXR render path. A wgpu
  Vulkan instance/device is created once, its raw handles are handed to
  the runtime as the session's graphics binding (`wgpu::as_hal`), and
  each frame renders the iced program into the acquired swapchain
  `VkImage` (wrapped via `wgpu::Device::create_texture_from_hal` with a
  sentinel drop guard so wgpu never destroys the runtime's image), then
  submits one quad layer. Always-on pure companions: `xr::format`
  (VkFormat ↔ wgpu texture format negotiation) and `xr::plan` (per-frame
  render/skip/detach decision), covered by the `dev` gate.
- `cargo xtask device-ui --xr`: builds and runs the `xr-overlay` binary
  (opt-in like `gpu-stress`, outside `dev` and the coverage gate).
- `third_party/cryoglyph`: local copy of `cryoglyph` 0.1.0 whose only change
  is `lru` `0.16` → `0.18`, wired in through `[patch.crates-io]`.

### Changed

- `device-ui`: upgraded iced 0.13 → 0.14 (`iced::application(boot, update, view)` API), and the `xr` render path to wgpu 27 and ash 0.38 to match iced's wgpu engine and wgpu-hal. The `x11` and `wayland` iced features are enabled to avoid a `softbuffer` build failure.
- Rust toolchain pinned to 1.100.0 in `rust-toolchain.toml` (was `stable`), with `clippy`, `rustfmt`, and `llvm-tools` components for the coverage gate.

### Security

- `device-ui`: RUSTSEC-2026-0253 (`lru` `LruCache::pop()`, fixed in 0.18.2) is cleared by the local `[patch]` of `cryoglyph` 0.1.0 in `third_party/cryoglyph`, which accepts `lru` 0.18; `lru` now resolves to 0.18.5. The iced 0.14 upgrade had already cleared the earlier `lru` `IterMut` advisory (fixed in 0.16.3). The `[patch]` does not reach crates.io consumers of `vivelite-device-ui`, which still resolve `cryoglyph` 0.1.0 with `lru` 0.16 until upstream releases a newer `cryoglyph`.
- `bundle`: atomupd `buildid` and `version` are validated as single path components before they reach host paths.
- `core`: staged tree walks are depth-bounded (64 levels) so a hostile image cannot exhaust the stack.
- `bundle`: debugfs destinations are validated (no whitespace, quotes, `;`, or backslashes) before they reach the request string.
- `xtask`: `fetch`, the feed fetch, and the bundle download accept only `https://` URLs, pass `--` before the URL, and restrict curl to https (including redirects) with a 6-hour limit.
- `bundle`: `RaucVerifier` now runs `rauc info --keyring <VIVELITE_RAUC_KEYRING>` (the verifying command; the previous `rauc status` never checked the bundle signature) and fails closed when no keyring is configured.
- `bundle`: `OpensslVerifier` requires an explicit trusted CA file and never falls back to `-noverify`.
- `bundle`: `debugfs` request paths are validated before spawning; atomupd feed paths and manifest image filenames are rejected when they could traverse or escape.
- `xtask`: staging validates feed paths via `bundle` helpers before building URLs or host paths.
- `wine`: `init` leaks argv for the process lifetime (fixes a use-after-free) and reports the error string returned by `wine_init`.

### Fixed

- `wine`: `version()` reads the Wine version and build strings through the null-safe `wine_sys::cstr`.
- `device-ui`: an XR render failure releases the swapchain image and closes the frame before returning the error, keeping the runtime's frame loop balanced.
- `hcf`: the watchdog returns an error when its thread cannot start instead of silently running without one.
- `hcf`: the Vulkan output buffer offset honors the buffer's alignment requirement.
- `bundle`: tool spawn failures report "could not be started" instead of a fake exit status.
- `monado`: `VIVELITE_XR_RUNTIME` pointing at a `.json` file is classified as a runtime socket path (loader only for `XR_RUNTIME_JSON`).
- `monado-sys`: structure-type discriminators did not match `openxr.h`
  (`XR_TYPE_INSTANCE_CREATE_INFO` is 3, not 1; `SYSTEM_GET_INFO` 4, not
  2; `SESSION_CREATE_INFO` 8, not 4; `EXTENSION_PROPERTIES` 2, not 7), so
  every probe call would have failed with a structure-type error on a
  real runtime.
- `monado`: `XR_MAKE_VERSION` packed major at bit 55 instead of 48, so the
  requested API version decoded as 128.0.0 and instance creation would
  have been rejected; error mapping used invented result codes (-6/-7
  are `INITIALIZATION_FAILED`/`FUNCTION_UNSUPPORTED`, not runtime/form
  factor) — now mapped to the documented `XrResult` values.
- `monado-sys`: `XrFormFactor` was 64-bit; C enums are 32-bit in this ABI.
