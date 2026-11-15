//! Rust-native GPU flight recorder primitives and Vulkan marker backends.
//!
//! This is not a transparent Vulkan layer: applications must place markers
//! in their command streams and inspect the resulting [`FlightReport`]. The
//! [`backend::Backend::Portable`] path uses core Vulkan compute dispatches
//! for marker writes and is intended for GPUs such as Qualcomm Adreno that do
//! not expose `VK_AMD_buffer_marker`. [`backend::Backend::Amd`] uses the AMD
//! marker extension where available.
//!
//! Device execution is opt-in behind the `vulkan` feature; marker planning
//! and post-run analysis are pure Rust and usable without a GPU.

pub mod backend;
pub mod marker;
pub mod report;
#[cfg(feature = "vulkan")]
pub mod vulkan;

pub use backend::{Backend, BackendRequest, ParseBackendError};
pub use marker::{Marker, MarkerPlan};
pub use report::{analyze, FlightReport};

#[cfg(test)]
mod tests {
    #[test]
    fn portable_marker_path_is_documented_as_core_vulkan() {
        assert!(include_str!("lib.rs").contains("core Vulkan compute dispatches"));
    }
}
