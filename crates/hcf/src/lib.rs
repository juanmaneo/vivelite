//! Halt and Catch Fire, ported to Rust with `ash`.
//!
//! The upstream project ([googlestadia/halt_and_catch_fire][hcf], Apache-2.0)
//! is a collection of Vulkan programs that hang or crash the GPU, used to test
//! that tooling (Graphics Flight Recorder, drivers, robustness paths) detects
//! known classes of defects. vivelite ports the hang cases to Rust as GPU
//! robustness workloads for the Adreno-backed compositor path.
//!
//! This crate is the HCF equivalent of a `-sys` wrapper plus its safe face in
//! one place: [`vulkan`] (behind the `vulkan` feature) drives the GPU through
//! `ash`, everything else is plain Rust with no device dependency, so the
//! default workspace build and the coverage gate stay GPU-free.
//!
//! # Layout
//!
//! - [`case`]: the case model — a shader, a buffer initialization policy, and
//!   a command-buffer recipe, as data. Ports `hang_infinite_loop` and
//!   `hang_host_event` faithfully (256-entry SSBO pairs, local size 4).
//! - [`spirv`]: GLSL → SPIR-V translation at runtime via `naga` (pure Rust,
//!   no glslang dependency; the SPIR-V is never checked in).
//! - [`watchdog`]: a force-exit watchdog; a real hang may wedge the queue
//!   forever, so termination must not depend on Vulkan returning.
//! - [`vulkan`]: `ash`-based device execution (feature `vulkan`), including
//!   HCF's crash check (empty submit + fenced 30 s wait) and the
//!   never-signaled host event.
//!
//! # Differences from upstream
//!
//! - GLSL `#version 450` instead of 430: the `naga` frontend accepts
//!   440/450/460 only, and 450 is a superset for SSBO compute.
//! - Anonymous SSBO interface blocks (`buffer b { ... };`) instead of named
//!   instances: same SPIR-V, same bindings, friendlier to the translator.
//! - `float(i + j + k)` instead of the implicit uint→float conversion in
//!   `sum += i + j + k`: identical semantics (the uint sum is computed in
//!   uint, then converted), explicit for the translator.
//! - The never-signaled event is never destroyed, exactly like upstream;
//!   `vkDestroyDevice` reclaims it.
//!
//! [hcf]: https://github.com/googlestadia/halt_and_catch_fire

pub mod case;
pub mod spirv;
#[cfg(feature = "vulkan")]
pub mod vulkan;
pub mod watchdog;

pub use case::{buffer_init_words, case, cases, BufferInit, Case, Recipe};
pub use spirv::{translate, TranslateError};
pub use watchdog::Watchdog;

/// Process exit code for "the watchdog had to kill the run": the workload
/// hung and the harness terminated it. Distinct from 0 (an outcome was
/// observed) and 1 (the harness itself failed).
pub const EXIT_HANG: i32 = 3;

/// Process exit code for "the driver reported `VK_ERROR_DEVICE_LOST`": the
/// crash was detected, which is an observation, not a harness failure.
pub const EXIT_DEVICE_LOST: i32 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct() {
        assert_eq!(EXIT_HANG, 3);
        assert_eq!(EXIT_DEVICE_LOST, 4);
        assert_ne!(EXIT_HANG, EXIT_DEVICE_LOST);
        assert_ne!(EXIT_HANG, 0);
        assert_ne!(EXIT_DEVICE_LOST, 0);
    }
}
