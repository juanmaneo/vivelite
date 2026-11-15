//! Safe wrapper around FEX's Wine-facing interfaces.
//!
//! FEX inside Wine is not driven by a public "start emulation" call: Wine's
//! ARM64EC frontend calls the BT hooks while loading x86 code. What vivelite
//! controls is the *configuration* around that: TSO mode, unaligned-atomic
//! behaviour, and the CPUID mask the guest sees — all via the unixlib
//! dispatch. This crate builds those calls safely and records what the
//! device can and cannot accelerate.

use fex_sys::{unalign_atomic, FexUnixLibFunctions};

/// What the SoC offers for x86 memory-ordering emulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsoMode {
    /// Hardware TSO (Apple silicon, Snapdragon 8 Gen 3 in later frames).
    /// One `prctl` away; near-native x86 atomics.
    Hardware,
    /// Software TSO in the JIT (the XR Elite's Snapdragon XR2 falls here).
    /// Correct, but every x86 atomic is instrumented.
    Software,
}

/// Known-capability snapshot of a device, used to pick config defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceCapabilities {
    /// Hardware TSO available.
    pub hardware_tso: bool,
    /// Kernel supports the unaligned-atomic prctl.
    pub kernel_unaligned_atomic: bool,
}

impl DeviceCapabilities {
    /// The VIVE XR Elite (Snapdragon XR2, stock Android kernel):
    /// no hardware TSO, no unaligned-atomic prctl.
    pub const VIVE_XR_ELITE: DeviceCapabilities = DeviceCapabilities {
        hardware_tso: false,
        kernel_unaligned_atomic: false,
    };
}

/// Emulation knobs, resolved from device capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FexConfig {
    /// Which TSO implementation to request.
    pub tso: TsoMode,
    /// Unaligned-atomic flags (`FEX_UNALIGN_ATOMIC_*`).
    pub unaligned_flags: u64,
}

impl FexConfig {
    /// Config for a device, choosing the fastest correct options.
    pub fn for_device(d: DeviceCapabilities) -> FexConfig {
        FexConfig {
            tso: if d.hardware_tso { TsoMode::Hardware } else { TsoMode::Software },
            unaligned_flags: if d.kernel_unaligned_atomic {
                unalign_atomic::EMULATE | unalign_atomic::BACKPATCH
            } else {
                0
            },
        }
    }

    /// The unixlib dispatch index for the TSO request.
    pub fn tso_dispatch(self) -> FexUnixLibFunctions {
        FexUnixLibFunctions::SetHardwareTsoControl
    }

    /// True when hardware TSO will actually be requested.
    pub fn requests_hardware_tso(self) -> bool {
        self.tso == TsoMode::Hardware
    }

    /// Human-readable summary for the doctor log.
    pub fn summary(self) -> String {
        format!(
            "tso={} unaligned=0x{:x}",
            match self.tso {
                TsoMode::Hardware => "hardware",
                TsoMode::Software => "software(jit)",
            },
            self.unaligned_flags
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xr_elite_runs_software_tso() {
        let c = FexConfig::for_device(DeviceCapabilities::VIVE_XR_ELITE);
        assert_eq!(c.tso, TsoMode::Software);
        assert!(!c.requests_hardware_tso());
        assert_eq!(c.unaligned_flags, 0);
        assert!(c.summary().contains("software(jit)"));
    }

    #[test]
    fn capable_device_uses_hardware_tso() {
        let d = DeviceCapabilities { hardware_tso: true, kernel_unaligned_atomic: true };
        let c = FexConfig::for_device(d);
        assert!(c.requests_hardware_tso());
        assert_eq!(c.unaligned_flags, 0x3);
    }

    #[test]
    fn tso_dispatch_is_the_c_index() {
        assert_eq!(FexConfig::for_device(DeviceCapabilities::VIVE_XR_ELITE).tso_dispatch() as u32, 0);
    }
}
