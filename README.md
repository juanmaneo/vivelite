# vivelite

> [!WARNING]
> **Work in progress.** Nothing here is stable yet: APIs, layout, and
> behavior can change at any time.

Valve's official **ARM64 SteamOS** (the Steam Frame build) on the
**HTC VIVE XR Elite** — Rust-first, inspired by
[steamac](https://github.com/fxgl/steamac).

The XR Elite exposes no `/dev/kvm` to apps. Its bootloader was not locked in
earlier firmware, but later versions lock it, so there is no microVM route and
no native SteamOS boot on current firmware. Instead, vivelite
reproduces the steamac trust model against a **proot** execution model:
Valve's signed bundle is downloaded, verified, and its components (Wine with
the FEX ARM64EC backend, SteamOS libraries) are staged and driven from the
WinlatorXR-derived app.

## Rule of the project

Rust for everything, except genuinely low-level C/C++ (Wine, FEX, Monado,
Mesa, curl, squashfs, crypto) which is wrapped behind safe crates:

| Layer | Language | Crate |
| --- | --- | --- |
| Build orchestrator | Rust | `xtask` |
| Stage/layout model | Rust | `vivelite-core` |
| Bundle fetch/verify/extract | Rust (+ C tools wrapped) | `bundle` |
| GPU robustness (HCF port) | Rust (+ Vulkan wrapped) | `hcf` |
| GPU flight recorder | Rust (+ Vulkan wrapped) | `gfr` |
| Wine embedded API | C, wrapped | `wine-sys` → `wine` |
| FEX in-Wine interfaces | C++, wrapped | `fex-sys` → `fex` |
| OpenXR loader (Monado) | C, wrapped | `monado-sys` → `monado` |

UI policy: **iced** for the on-device framebuffer/overlay UI (the VR
surface, rendered directly into the compositor's swapchain), **egui** for
normal windowed desktop tools (host-side build UI, doctor dashboard).

## Quick start

```sh
export VIVELITE_BUNDLE_URL=<Valve SteamOS ARM64 RAUC bundle URL>
cargo xtask fetch            # streamed, resumable
cargo xtask verify           # CMS signature (requires VIVELITE_RAUC_KEYRING)
cargo xtask extract <image>  # stage SteamOS components
cargo xtask doctor
cargo xtask ui              # optional egui host dashboard
cargo xtask device-ui       # preview the iced device overlay
cargo xtask device-ui --xr  # the overlay on the OpenXR compositor (opt-in)
cargo xtask gpu-stress <case>   # opt-in HCF GPU robustness workload
cargo xtask gfr-probe            # opt-in GFR marker backend probe
cargo test --workspace
cargo xtask dev             # build, test, enforce 100% coverage
```

## Safety

Every `-sys` crate is raw and documented; every wrapper crate is the safe
face. Unsafe blocks outside `-sys` crates are banned by workspace lints
(`undocumented_unsafe_blocks = deny`).

## Status

- [x] Local dev-check (`cargo xtask dev`): build, test, 100% function/line/region coverage. Verified with and without `DISPLAY`/`WAYLAND_DISPLAY` on Linux; window-launch tests run in child processes with those variables removed
- [ ] Device UI (iced): overlay compositor frontend. The preview window and the XR render path build and are tested headlessly (`cargo xtask device-ui`, `cargo xtask device-ui --xr`); the render path wraps the session's swapchain `VkImage`s as wgpu textures via a shared-device graphics binding — on-device validation is pending
- [ ] SteamOS component staging against a real Steam Frame image. Layout validation and manifest writing are tested with fixtures; a real-image run is pending. Requires `VIVELITE_RAUC_KEYRING` (Valve's RAUC CA) to verify bundles; verification fails closed without it
- [x] `lru` advisory RUSTSEC-2026-0253 (`LruCache::pop()`, fixed in 0.18.2) cleared locally: `third_party/cryoglyph` is a copy of `cryoglyph` 0.1.0 that accepts `lru` 0.18, wired in with `[patch.crates-io]`. Crates.io consumers still resolve `cryoglyph` 0.1.0 until upstream releases a newer `lru` requirement

## Ideas

- [x] Add Rust-native foundations inspired by frame-shell and frame-monado: `crates/core/src/performance.rs` models streaming/standard performance modes and one-shot battery warnings; `crates/core/src/runtime_settings.rs` validates documented passthrough, mirror, and eye-camera settings; `crates/eye` provides per-axis gaze calibration, a One Euro filter, versioned calibration persistence, OSC encoding, and configurable UDP transport. These are models/utilities, not a frame-shell port, a Monado fork, a SteamOS service integration, or a complete eye-camera client. They were implemented from the projects' public documentation without copying their source (frame-shell is GPL-3.0; frame-monado is BSL-1.0).
- [ ] Validate calibration, filter parameters, and OSC interoperability against a real tracker and VRChat; connect a hardware tracker and the sender to the device UI/runtime.
- [ ] Decide which OpenXR features are actually exposed by the XR Elite's runtime before implementing eye gaze, passthrough, mirror output, or frame-specific controls. The cited frame-monado features and hardware interfaces are Steam Frame-specific and must not be assumed available on VIVE XR Elite.
- [x] `cargo xtask gpu-stress <case>`: opt-in, outside `dev` and the
  coverage gate (the runner is built separately with the `vulkan`
  feature), with timeout and watchdog
- [x] Port Halt and Catch Fire hang cases to Rust with `ash`:
  `hang_infinite_loop`, `hang_host_event`, `hang_semaphore`, and
  `hang_timeline_semaphore_host` live in `crates/hcf` (GLSL kept in-tree,
  translated to SPIR-V at runtime with `naga`). The timeline case requires
  Vulkan 1.1+ plus timeline-semaphore support (core 1.2 or
  `VK_KHR_timeline_semaphore`); unsupported devices fail closed. GPU workloads
  remain opt-in and must only run under the watchdog.
- [ ] Port crash cases. These deliberately exercise invalid GPU behavior and
  should only be tested on an isolated device with a recovery path.
- [x] Render the iced overlay into OpenXR swapchain images: the
  `device-ui` `xr` feature creates the wgpu Vulkan device, hands its raw
  handles to the runtime as the session's graphics binding, and renders
  the iced program into each frame's `VkImage` wrapped as a wgpu texture
  (`cargo xtask device-ui --xr`; opt-in, outside `dev` and the coverage
  gate — on-device validation pending)
- [x] Rust GFR primitives and opt-in Vulkan probe (`cargo xtask gfr-probe`):
  AMD extension backend plus a portable core-Vulkan compute-marker backend
  intended for Adreno; this is not a transparent Vulkan layer
- [ ] Verify the portable GFR marker backend on XR Elite/Adreno hardware; both
  AMD and forced-portable probe backends completed on this host's AMD Radeon
  Graphics (RADV RENOIR), not on the XR Elite GPU
- [x] Evaluate FidelityFX CAS from PorQue4K as a reference for a future
  upscaling path. Findings: CAS is a contrast-adaptive sharpening compute
  pass with optional upsampling, not a full upscaler; it uses only core
  compute, so it fits Adreno without vendor extensions. The upstream
  FidelityFX code is MIT (AMD copyright), so a Rust port needs that
  attribution. PorQue4K is archived and its CAS doc is a stub, so the
  reference implementation is the GPUOpen FFX_CAS source. No port yet

## License

MIT OR Apache-2.0, same as the Rust ecosystem default.
