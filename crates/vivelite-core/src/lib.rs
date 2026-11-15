//! vivelite — Rust tooling around Valve's official ARM64 SteamOS (Steam Frame build)
//! for the HTC VIVE XR Elite.
//!
//! Rust-first everywhere; C/C++ only where it is genuinely low level
//! (Wine, FEX, Monado, Mesa), wrapped behind safe crates:
//!
//! - `bundle`  — fetch + verify Valve's signed RAUC bundle, extract components.
//! - `xtask`   — build orchestrator (fetch / verify / extract / stage / doctor).
//! - `wine-sys`/`wine`     — raw FFI to Wine's embedded C API + safe wrapper.
//! - `fex-sys`/`fex`       — raw FFI to FEX's Wine unixlib / ARM64EC interfaces + safe wrapper.
//! - `monado-sys`/`monado` — raw FFI to the OpenXR loader (Monado) + safe wrapper.

use std::path::Path;

/// Build stages, in the order `xtask` runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Download Valve's RAUC bundle (streamed, resumable).
    Fetch,
    /// Verify sha256 + CMS signature.
    Verify,
    /// Extract bundle components (squashfs).
    Extract,
    /// Stage components into the on-device layout.
    Stage,
}

impl Stage {
    /// Stable machine-readable name, also used as the `xtask` subcommand.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Fetch => "fetch",
            Stage::Verify => "verify",
            Stage::Extract => "extract",
            Stage::Stage => "stage",
        }
    }
}

/// Default on-device layout under the app's private storage.
///
/// Mirrors the component layout of the steamac project but targets proot
/// instead of libkrun: the XR Elite exposes no `/dev/kvm` to apps.
pub fn device_layout(root: &Path) -> DeviceLayout {
    DeviceLayout {
        bundle: root.join("bundles").join("steamos.raucb"),
        rootfs: root.join("rootfs"),
        wine_prefix: root.join("prefix"),
        fex_rootfs: root.join("fex-rootfs"),
        assets: root.join("assets"),
        logs: root.join("logs"),
    }
}

/// Where each component lives on device.
#[derive(Debug, Clone)]
pub struct DeviceLayout {
    /// Valve's downloaded RAUC bundle.
    pub bundle: std::path::PathBuf,
    /// Unpacked SteamOS root filesystem subset used by proot.
    pub rootfs: std::path::PathBuf,
    /// Wine (Proton) prefix for Windows games.
    pub wine_prefix: std::path::PathBuf,
    /// FEX guest rootfs (x86_64 libraries FEX maps).
    pub fex_rootfs: std::path::PathBuf,
    /// Staged assets (fonts, drivers, configs).
    pub assets: std::path::PathBuf,
    /// Logs written by the orchestrator.
    pub logs: std::path::PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_names_are_stable() {
        assert_eq!(Stage::Fetch.name(), "fetch");
        assert_eq!(Stage::Verify.name(), "verify");
        assert_eq!(Stage::Extract.name(), "extract");
        assert_eq!(Stage::Stage.name(), "stage");
    }

    #[test]
    fn stages_are_ordered() {
        assert!(Stage::Fetch < Stage::Verify);
        assert!(Stage::Verify < Stage::Extract);
        assert!(Stage::Extract < Stage::Stage);
    }

    #[test]
    fn device_layout_paths() {
        let l = device_layout(Path::new("/data/vivelite"));
        assert_eq!(l.bundle, Path::new("/data/vivelite/bundles/steamos.raucb"));
        assert_eq!(l.wine_prefix, Path::new("/data/vivelite/prefix"));
    }
}
