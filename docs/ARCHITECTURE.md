# vivelite architecture

North-star: Valve's official ARM64 SteamOS, verified and unmodified, running
its Windows-compatibility stack on the HTC VIVE XR Elite.

## Why not a VM (like steamac)

steamac runs Valve's image in a libkrun microVM on Apple Silicon: KVM +
virtio-gpu/Venus. On the XR Elite:

- No `/dev/kvm` exposed to apps; Android does not hand out the hypervisor.
- Later firmware versions lock the bootloader (it was not locked on earlier
  firmware); current firmware has no native SteamOS boot or mainline XR2 drivers.

So the execution model is the one WinlatorXR already proved on Android:
**proot + Wine in-process**, with Valve's own ARM64 Wine+FEX build doing the
x86 work. The stack differs from steamac only at the "where does guest code
run" layer; the trust model is identical.

## Layering

```
┌ device ──────────────────────────────────────────────────┐
│ iced overlay UI (framebuffer, compositor swapchain)      │  Rust
│ WinlatorXR-derived app shell (patched: HTC detection)    │  Java/C
│ openxr (HTC runtime: tracking) │ monado (Linux side)     │  C
├──────────────────────────────────────────────────────────┤
│ proot Linux userland (SteamOS subset)                    │
│ wine-sys/wine → Valve's ARM64 wine + FEX ARM64EC backend  │  C/C++ wrapped
│ fex-sys/fex: TSO + unaligned-atomic configuration         │  Rust
├──────────────────────────────────────────────────────────┤
└ host build: xtask (fetch/verify/extract/stage/doctor)     │  Rust
   egui desktop tools                                       │  Rust
   hcf: Halt and Catch Fire port, GPU robustness workloads  │  Rust (+ ash)
   gfr: GPU flight-recorder markers and flight analysis      │  Rust (+ ash)
```

## Trust model

1. `xtask fetch` streams Valve's signed RAUC bundle (resumable, `curl`).
2. `bundle` verifies sha256 in Rust (`sha2`) and the CMS signature via `rauc`
   (C, wrapped behind `SignatureVerifier`).
3. `bundle` extracts only the components it stages; Valve's rootfs is never
   edited. Drift across SteamOS releases is caught by `xtask doctor`.

## Component choices

- **FEX, not Box64**: Valve's ARM64 Proton embeds FEX as Wine's ARM64EC
  backtranslation layer (`Source/Windows/ARM64EC/BTInterface.h`,
  `Source/Windows/UnixLib/FEXUnixLib.h` — pinned in `fex-sys` with layout
  tests mirroring FEX's `static_assert`s). Boxing two emulators buys nothing.
- **Memory ordering**: the XR2 has no hardware TSO (unlike Apple or SD 8 Gen
  3), so `fex::TsoMode::Software` and unaligned-atomic emulation are the
  expected config; `fex::FexConfig::for_device` encodes this and `doctor`
  logs it.
- **Monado**: the Linux-side OpenXR runtime for compositing experiments;
  display and controller tracking stay on HTC's runtime (closed vendor
  stack, no upstream driver).

## UI policy

- **iced** where there is no window system: the on-device overlay renders
  into the same swapchain the OpenXR compositor consumes (framebuffer path).
- **egui** for normal windowed apps on the host: doctor dashboard, build
  progress, prefix manager.

## Rust/c boundary

- New code: Rust, always.
- Wine/FEX/Monado/Mesa, curl, unsquashfs, rauc, crypto: stay C/C++, wrapped
  in `-sys` crates with safe wrapper crates on top; ABI drift is caught by
  compile-time layout tests (see `fex-sys` and `monado-sys` tests).
- Unsafe code is allowed only in `-sys` crates; the workspace lints deny
  undocumented unsafe blocks elsewhere.

## Roadmap

1. ✅ WinlatorXR HTC detection patches (0001/0002, in the Vivie port library)
2. ✅ vivelite workspace: bundle pipeline + -sys/wrapper crates + xtask
3. ⬜ Stage a real Steam Frame image; pin component list to its layout
4. ⬜ Wine/FEX smoke test inside proot on-device
5. ⬜ iced overlay frontend on the OpenXR compositor (the `monado` wrapper's session/swapchain APIs and the overlay's compositor phase model are in place; the wgpu-into-swapchain render path is next)
6. ⬜ CI (GitHub Actions): cargo test --workspace + xtask doctor
