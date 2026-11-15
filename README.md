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
cargo xtask verify           # sha256 + CMS signature (rauc)
cargo xtask extract <image>  # stage SteamOS components
cargo xtask doctor
cargo xtask ui              # optional egui host dashboard
cargo xtask device-ui       # preview the iced device overlay
cargo xtask gpu-stress <case>   # opt-in HCF GPU robustness workload
cargo test --workspace
cargo xtask dev             # build, test, enforce 100% coverage
```

## Safety

Every `-sys` crate is raw and documented; every wrapper crate is the safe
face. Unsafe blocks outside `-sys` crates are banned by workspace lints
(`undocumented_unsafe_blocks = deny`).

## Status

- [x] Local dev-check (`cargo xtask dev`): build, test, 100% function/line/region coverage. Verified with and without `DISPLAY`/`WAYLAND_DISPLAY` on Linux; window-launch tests run in child processes with those variables removed
- [ ] Device UI (iced): overlay compositor frontend. Preview window builds and is tested headlessly (`cargo xtask device-ui`); the `monado` wrapper now carries the session/space/swapchain/frame-loop APIs and the overlay tracks the compositor phase — the wgpu-into-swapchain render path is pending
- [ ] SteamOS component staging against a real Steam Frame image. Layout validation and manifest writing are tested with fixtures; a real-image run is pending

## Ideas

- [x] `cargo xtask gpu-stress <case>`: opt-in, outside `dev` and the
  coverage gate (the runner is built separately with the `vulkan`
  feature), with timeout and watchdog
- [x] Port Halt and Catch Fire hang cases to Rust with `ash`:
  `hang_infinite_loop` and `hang_host_event` live in `crates/hcf`
  (GLSL kept in-tree, translated to SPIR-V at runtime with `naga`)
- [ ] Port more HCF cases as needed (`hang_semaphore`,
  `hang_timeline_semaphore_host`, the crash cases)
- [ ] Render the iced overlay into OpenXR swapchain images (wgpu/Vulkan
  external-memory import): the session/swapchain wrapper APIs and the
  compositor phase model are in place, the pixel path is the missing piece
- [ ] Graphics Flight Recorder (GFR) is blocked on the XR Elite: it
  requires the AMD-only `VK_AMD_buffer_marker`, and the GPU is Qualcomm
  Adreno
- [ ] Evaluate FidelityFX CAS from PorQue4K as a reference for a future
  upscaling path

## License

MIT OR Apache-2.0, same as the Rust ecosystem default.
