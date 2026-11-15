//! Raw FFI bindings to FEX-Emu's in-Wine interfaces.
//!
//! These declarations mirror FEX main branch, `Source/Windows/ARM64EC/BTInterface.h`
//! and `Source/Windows/UnixLib/FEXUnixLib.h` — the interfaces Wine's ARM64EC
//! frontend calls into, which is how Valve's ARM64 SteamOS runs x86 Windows
//! code. Symbols live in the unixlib object Wine loads (`fex_wine`), so this
//! crate is compiled with `-lfex_wine` by build.rs; there is no stable
//! standalone C API to bind.
//!
//! Cross-check against the FEX tree pinned by your SteamOS bundle version:
//! `Source/Windows/ARM64EC/BTInterface.h` is the contract of record.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use std::os::raw::{c_int, c_ulong, c_void};

/// NTSTATUS from the NT headers: success is 0.
pub type Ntstatus = i32;

/// `HANDLE` is a pointer-width integer on Windows ABIs.
pub type Handle = *mut c_void;

/// Opaque exception record (Windows `EXCEPTION_RECORD`).
pub type ExceptionRecord = c_void;
/// Opaque guest context (Windows x86_64 `CONTEXT`).
pub type GuestContext = c_void;
/// Opaque native ARM64EC context (Windows `ARM64_NT_CONTEXT`).
pub type Arm64NtContext = c_void;
/// Opaque native TEB.
pub type Teb = c_void;
/// `SYSTEM_CPU_INFORMATION`.
pub type SystemCpuInformation = c_void;
/// Windows `ULONG`-typed booleans: 0 is false.
pub type NtBool = c_int;
/// `SIZE_T`.
pub type SizeT = usize;
/// `ULONG` protection flags.
pub type Prot = c_ulong;

#[cfg(feature = "link")]
extern "C" {
    /// Per-process init of the FEX backtranslation layer. Called by Wine at
    /// process start; returns NTSTATUS.
    pub fn ProcessInit() -> Ntstatus;

    /// Per-process teardown, after the exit status is known.
    pub fn ProcessTerm(handle: Handle, after: NtBool, status: Ntstatus);

    /// Per-thread init; must run on each Wine thread that will execute
    /// x86 code. Returns NTSTATUS.
    pub fn ThreadInit() -> Ntstatus;

    /// Per-thread teardown.
    pub fn ThreadTerm(handle: Handle, exit_code: i32);

    /// Recover a consistent state after a guest fault, then re-execute.
    pub fn ResetToConsistentState(
        exception: *mut ExceptionRecord,
        guest_context: *mut GuestContext,
        native_context: *mut Arm64NtContext,
    );

    /// Memory-alloc notification (Wine intercepts NtAllocateVirtualMemory).
    pub fn NotifyMemoryAlloc(
        address: *mut c_void,
        size: SizeT,
        alloc_type: c_ulong,
        prot: Prot,
        after: NtBool,
        status: Ntstatus,
    );

    /// Memory-free notification.
    pub fn NotifyMemoryFree(
        address: *mut c_void,
        size: SizeT,
        free_type: c_ulong,
        after: NtBool,
        status: Ntstatus,
    );

    /// Memory-protect notification (NtProtectVirtualMemory).
    pub fn NotifyMemoryProtect(
        address: *mut c_void,
        size: SizeT,
        new_prot: Prot,
        after: NtBool,
        status: Ntstatus,
    );

    /// MapViewOfSection notification; returns NTSTATUS.
    pub fn NotifyMapViewOfSection(
        unk1: *mut c_void,
        address: *mut c_void,
        unk2: *mut c_void,
        size: SizeT,
        alloc_type: c_ulong,
        prot: Prot,
    ) -> Ntstatus;

    /// UnmapViewOfSection notification.
    pub fn NotifyUnmapViewOfSection(address: *mut c_void, after: NtBool, status: Ntstatus);

    /// Heavy instruction-cache flush (whole-address-space coherence work).
    pub fn FlushInstructionCacheHeavy(address: *const c_void, size: SizeT);

    /// Targeted instruction-cache flush for a range the guest rewrote.
    pub fn BTCpu64FlushInstructionCache(address: *const c_void, size: SizeT);

    /// A range the guest dirtied (e.g. after a self-modifying write).
    pub fn BTCpu64NotifyMemoryDirty(address: *const c_void, size: SizeT);

    /// Guest read through a file handle into a dirty-tracked buffer.
    pub fn BTCpu64NotifyReadFile(
        handle: Handle,
        address: *mut c_void,
        size: SizeT,
        after: NtBool,
        status: Ntstatus,
    );

    /// CPUID-style feature probe, ARM64EC flavour.
    pub fn BTCpu64IsProcessorFeaturePresent(feature: c_ulong) -> NtBool;

    /// Fill `SYSTEM_CPU_INFORMATION` for the emulated CPU.
    pub fn UpdateProcessorInformation(info: *mut SystemCpuInformation);
}

// ---------------------------------------------------------------------------
// unixlib dispatch — FEXUnixLib.h
// ---------------------------------------------------------------------------

/// `FEXUnixLibFunctions`: the dispatch table index passed to the unixlib
/// entry point. Mirrors `enum class FEXUnixLibFunctions`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FexUnixLibFunctions {
    /// Hardware TSO control on supported cores (XR2: unsupported).
    SetHardwareTsoControl = 0,
    /// Kernel unaligned-atomic control flags.
    SetKernelUnalignedAtomicControl = 1,
    /// madvise from the Wine side.
    Madvise = 2,
    /// prctl PR_SET_VMA_ANON_NAME.
    SetVmaName = 3,
    /// SHM stats query.
    GetShmStatsVma = 4,
    /// SHM stats file cleanup.
    DeleteShmStatsFile = 5,
    /// Map an fd into FEX's SHM.
    MapFile = 6,
    /// Get the serving process id.
    GetPid = 7,
    /// Probe TPIDRRO support (Apple cores).
    CheckSupportsTpidrroCpu = 8,
}

/// `struct FEXUnixLib_SetHardwareTSOControlArgs`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SetHardwareTsoControlArgs {
    /// Enable/disable hardware TSO.
    pub enable: bool,
}

/// `struct FEXUnixLib_SetKernelUnalignedAtomicControl`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SetKernelUnalignedAtomicControl {
    /// One of the `FEX_UNALIGN_ATOMIC_*` flags.
    pub flags: u64,
}

/// `struct FEXUnixLib_Madvise`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MadviseArgs {
    /// Address to advise.
    pub addr: *const c_void,
    /// Length to advise.
    pub size: usize,
    /// POSIX advice value.
    pub advice: i32,
    /// ABI padding.
    pub pad: u32,
}

/// `struct FEXUnixLib_SetVMAName`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SetVmaNameArgs {
    /// Range start.
    pub addr: *const c_void,
    /// Range length.
    pub size: usize,
    /// NUL-terminated name.
    pub name: *const c_char,
}

/// `struct FEXUnixLib_MapFile`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MapFileArgs {
    /// File descriptor to map.
    pub fd: i32,
    /// Size to map.
    pub map_size: u64,
    /// Offset into the file.
    pub offset: u64,
}

/// Bit values for `SetKernelUnalignedAtomicControl::flags`
/// (`PR_ARM64_SET_UNALIGN_ATOMIC`, from FEXUnixLib.h).
pub mod unalign_atomic {
    /// Emulate unaligned atomics in the JIT.
    pub const EMULATE: u64 = 1 << 0;
    /// Back-patch unaligned accesses into the code stream.
    pub const BACKPATCH: u64 = 1 << 1;
    /// Fault on split locks instead of emulating.
    pub const STRICT_SPLIT_LOCKS: u64 = 1 << 2;
}

use std::os::raw::c_char;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unixlib_layout_matches_c() {
        // FEXUnixLib.h static_asserts, mirrored so a mismatch fails the build.
        assert_eq!(std::mem::size_of::<SetHardwareTsoControlArgs>(), 1);
        assert_eq!(std::mem::size_of::<SetKernelUnalignedAtomicControl>(), 8);
        assert_eq!(std::mem::size_of::<MadviseArgs>(), 24);
        assert_eq!(std::mem::size_of::<SetVmaNameArgs>(), 24);
        assert_eq!(std::mem::size_of::<MapFileArgs>(), 24);
    }

    #[test]
    fn dispatch_indices_match_c() {
        // The dispatch enum is ABI: index order must not drift.
        assert_eq!(FexUnixLibFunctions::SetHardwareTsoControl as u32, 0);
        assert_eq!(
            FexUnixLibFunctions::SetKernelUnalignedAtomicControl as u32,
            1
        );
        assert_eq!(FexUnixLibFunctions::Madvise as u32, 2);
        assert_eq!(FexUnixLibFunctions::SetVmaName as u32, 3);
        assert_eq!(FexUnixLibFunctions::GetShmStatsVma as u32, 4);
        assert_eq!(FexUnixLibFunctions::DeleteShmStatsFile as u32, 5);
        assert_eq!(FexUnixLibFunctions::MapFile as u32, 6);
        assert_eq!(FexUnixLibFunctions::GetPid as u32, 7);
        assert_eq!(FexUnixLibFunctions::CheckSupportsTpidrroCpu as u32, 8);
    }

    #[test]
    fn unalign_flags_match_c() {
        assert_eq!(unalign_atomic::EMULATE, 0x1);
        assert_eq!(unalign_atomic::BACKPATCH, 0x2);
        assert_eq!(unalign_atomic::STRICT_SPLIT_LOCKS, 0x4);
    }
}
