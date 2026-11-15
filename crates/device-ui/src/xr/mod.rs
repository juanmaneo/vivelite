//! OpenXR compositor render path: the overlay inside the session's frame.
//!
//! Split by what each piece needs:
//!
//! - [`format`] and [`plan`] are pure data, always compiled and covered by
//!   the `dev` gate: which `VkFormat` the overlay can render into, and
//!   whether one frame should render, skip, or detach.
//! - [`overlay`] (feature `xr`) is the pixel path: it creates the wgpu
//!   Vulkan device, hands its raw handles to the runtime as the session's
//!   graphics binding, and renders the iced program into the swapchain
//!   images Monado allocated on that same device.

pub mod format;
pub mod plan;

#[cfg(feature = "xr")]
pub mod overlay;
