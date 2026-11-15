//! The overlay's OpenXR session path: session, reference space, swapchain,
//! and frame loop.
//!
//! The device-ui overlay composites as one quad layer: render iced into a
//! swapchain image, then submit the image as an `XrCompositionLayerQuad`
//! in front of the user. This module owns that surface as data plus (behind
//! the `link` feature) the `ash`-friendly handles that drive it:
//!
//! - [`SessionState`] / [`Event`]: the runtime lifecycle, mapped from
//!   `xrPollEvent` into plain data the overlay model can react to.
//! - [`QuadSpec`]: the overlay quad as data (size, distance), turned into
//!   the composition-layer struct when a frame is submitted.
//! - [`SwapchainSpec`]: the overlay swapchain's shape.
//! - [`Session`] / [`Space`] / [`Swapchain`] (feature `link`): the safe
//!   owners. Children borrow their session so Rust's borrow checker
//!   enforces the spec's destruction order (spaces and swapchains die
//!   before the session, the session before the instance).

use monado_sys::{
    XrCompositionLayerQuad, XrEventDataBuffer, XrExtent2Df, XrOffset2Di, XrPosef, XrQuaternionf,
    XrRect2Di, XrSession, XrSpace, XrStructureType, XrSwapchain, XrSwapchainSubImage, XrTime,
    XrVector3f, XR_EYE_VISIBILITY_BOTH, XR_REFERENCE_SPACE_TYPE_LOCAL,
    XR_REFERENCE_SPACE_TYPE_STAGE, XR_SESSION_STATE_EXITING, XR_SESSION_STATE_FOCUSED,
    XR_SESSION_STATE_IDLE, XR_SESSION_STATE_LOSS_PENDING, XR_SESSION_STATE_READY,
    XR_SESSION_STATE_STOPPING, XR_SESSION_STATE_SYNCHRONIZED, XR_SESSION_STATE_UNKNOWN,
    XR_SESSION_STATE_VISIBLE, XR_TYPE_COMPOSITION_LAYER_QUAD,
    XR_TYPE_EVENT_DATA_INSTANCE_LOSS_PENDING, XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED,
};

/// The OpenXR session lifecycle, from the `XrSessionState` enum in
/// openxr.h. The overlay model mirrors this to decide when to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// `XR_SESSION_STATE_UNKNOWN`: not yet enumerated, or an unmapped code.
    Unknown,
    /// `XR_SESSION_STATE_IDLE`: running but rendering nothing.
    Idle,
    /// `XR_SESSION_STATE_READY`: `begin()` may be called.
    Ready,
    /// `XR_SESSION_STATE_SYNCHRONIZED`: frame timing is meaningful.
    Synchronized,
    /// `XR_SESSION_STATE_VISIBLE`: visible but not focused.
    Visible,
    /// `XR_SESSION_STATE_FOCUSED`: the user is looking at the overlay.
    Focused,
    /// `XR_SESSION_STATE_STOPPING`: call `end()`, then wind down.
    Stopping,
    /// `XR_SESSION_STATE_LOSS_PENDING`: the session is about to be lost.
    LossPending,
    /// `XR_SESSION_STATE_EXITING`: the application should exit.
    Exiting,
}

impl SessionState {
    /// Maps a raw `XrSessionState` value; `None` for codes outside the
    /// 1.0 enum.
    pub fn from_raw(raw: i32) -> Option<SessionState> {
        Some(match raw {
            XR_SESSION_STATE_UNKNOWN => SessionState::Unknown,
            XR_SESSION_STATE_IDLE => SessionState::Idle,
            XR_SESSION_STATE_READY => SessionState::Ready,
            XR_SESSION_STATE_SYNCHRONIZED => SessionState::Synchronized,
            XR_SESSION_STATE_VISIBLE => SessionState::Visible,
            XR_SESSION_STATE_FOCUSED => SessionState::Focused,
            XR_SESSION_STATE_STOPPING => SessionState::Stopping,
            XR_SESSION_STATE_LOSS_PENDING => SessionState::LossPending,
            XR_SESSION_STATE_EXITING => SessionState::Exiting,
            _ => return None,
        })
    }

    /// The raw `XrSessionState` value, for logging and tests.
    pub fn as_raw(&self) -> i32 {
        match self {
            SessionState::Unknown => XR_SESSION_STATE_UNKNOWN,
            SessionState::Idle => XR_SESSION_STATE_IDLE,
            SessionState::Ready => XR_SESSION_STATE_READY,
            SessionState::Synchronized => XR_SESSION_STATE_SYNCHRONIZED,
            SessionState::Visible => XR_SESSION_STATE_VISIBLE,
            SessionState::Focused => XR_SESSION_STATE_FOCUSED,
            SessionState::Stopping => XR_SESSION_STATE_STOPPING,
            SessionState::LossPending => XR_SESSION_STATE_LOSS_PENDING,
            SessionState::Exiting => XR_SESSION_STATE_EXITING,
        }
    }
}

/// One polled runtime event, as plain data. The overlay's compositor state
/// machine consumes these; see `device_ui::compositor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Nothing was pending at poll time.
    None,
    /// The session changed state; the important one for the overlay.
    SessionStateChanged {
        /// The new state.
        state: SessionState,
        /// The raw session handle that changed.
        session: XrSession,
        /// When the change happened.
        time: XrTime,
    },
    /// The runtime is about to go away; re-detect the endpoint.
    InstanceLossPending,
    /// An event type this wrapper does not model.
    Other(XrStructureType),
}

/// Reads one event out of a polled [`XrEventDataBuffer`].
///
/// The buffer is the union every OpenXR event arrives in: its leading
/// `type_`/`next` fields overlay the concrete event's header, so the
/// payload is read by casting the buffer itself, per the spec.
pub fn parse_event(buffer: &XrEventDataBuffer) -> Event {
    match buffer.type_ {
        0 => Event::None,
        XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED => {
            // SAFETY: the runtime reported this buffer as a session-state
            // event, so a valid `XrEventDataSessionStateChanged` overlays
            // it; reading through the cast is within the buffer's bounds.
            let event = unsafe {
                &*(buffer as *const XrEventDataBuffer
                    as *const monado_sys::XrEventDataSessionStateChanged)
            };
            Event::SessionStateChanged {
                state: SessionState::from_raw(event.state).unwrap_or(SessionState::Unknown),
                session: event.session,
                time: event.time,
            }
        }
        XR_TYPE_EVENT_DATA_INSTANCE_LOSS_PENDING => Event::InstanceLossPending,
        other => Event::Other(other),
    }
}

/// The reference space the overlay's quad is posed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceSpace {
    /// `XR_REFERENCE_SPACE_TYPE_LOCAL`: origin at the headset's starting
    /// pose; the right default for an overlay panel.
    Local,
    /// `XR_REFERENCE_SPACE_TYPE_STAGE`: origin on the floor of a bounded
    /// play space.
    Stage,
}

impl ReferenceSpace {
    /// The raw `XrReferenceSpaceType` value.
    pub fn as_raw(&self) -> i32 {
        match self {
            ReferenceSpace::Local => XR_REFERENCE_SPACE_TYPE_LOCAL,
            ReferenceSpace::Stage => XR_REFERENCE_SPACE_TYPE_STAGE,
        }
    }
}

/// The overlay quad as data: a flat panel `distance_m` in front of the
/// user, `width_m` × `height_m` in size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadSpec {
    /// Panel width in meters.
    pub width_m: f32,
    /// Panel height in meters.
    pub height_m: f32,
    /// Distance in front of the user (along −Z) in meters.
    pub distance_m: f32,
}

impl QuadSpec {
    /// The quad's pose: identity orientation, centered, at `distance_m`.
    pub fn pose(&self) -> XrPosef {
        XrPosef {
            orientation: XrQuaternionf {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            position: XrVector3f {
                x: 0.0,
                y: 0.0,
                z: -self.distance_m,
            },
        }
    }

    /// Builds the composition-layer struct for one frame: the whole
    /// `swapchain` image (`width_px` × `height_px`, the swapchain's size),
    /// shown to both eyes, at [`QuadSpec::pose`].
    pub fn layer(
        &self,
        swapchain: XrSwapchain,
        space: XrSpace,
        width_px: i32,
        height_px: i32,
    ) -> XrCompositionLayerQuad {
        XrCompositionLayerQuad {
            type_: XR_TYPE_COMPOSITION_LAYER_QUAD,
            next: std::ptr::null(),
            layer_flags: 0,
            space,
            eye_visibility: XR_EYE_VISIBILITY_BOTH,
            sub_image: XrSwapchainSubImage {
                swapchain,
                image_rect: XrRect2Di {
                    offset: XrOffset2Di { x: 0, y: 0 },
                    extent: monado_sys::XrExtent2Di {
                        width: width_px,
                        height: height_px,
                    },
                },
                image_array_index: 0,
            },
            pose: self.pose(),
            size: XrExtent2Df {
                width: self.width_m,
                height: self.height_m,
            },
        }
    }
}

/// The overlay swapchain's shape. Format is a `VkFormat` as `i64`, taken
/// from the session's supported formats (`Session::swapchain_formats`
/// under the `link` feature; the runtime's list is authoritative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapchainSpec {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `VkFormat` as `i64`, from the session's supported formats.
    pub format: i64,
    /// `XrSwapchainUsageFlags`; the overlay only writes.
    pub usage_flags: u32,
    /// Samples per pixel.
    pub sample_count: u32,
    /// Array size; 1 unless the runtime requires arrays.
    pub array_size: u32,
    /// Mip levels; the overlay renders 1:1.
    pub mip_count: u32,
}

impl SwapchainSpec {
    /// A color-attachment swapchain for the overlay: single-sample,
    /// single-layer, single-mip.
    pub fn overlay(width: u32, height: u32, format: i64) -> SwapchainSpec {
        SwapchainSpec {
            width,
            height,
            format,
            usage_flags: monado_sys::XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT,
            sample_count: 1,
            array_size: 1,
            mip_count: 1,
        }
    }
}

/// The result of waiting for a swapchain image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The acquired image is ready to render into.
    Ready,
    /// The wait timed out; release and retry, or give up.
    TimedOut,
}

/// One frame's timing, from `xrWaitFrame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    /// The `XrTime` to stamp `end_frame` with.
    pub display_time: XrTime,
    /// Whether the compositor will use this frame; skip rendering when
    /// false.
    pub should_render: bool,
}

// ---------------------------------------------------------------------------
// Device execution (feature `link`): the safe owners.
// ---------------------------------------------------------------------------

#[cfg(feature = "link")]
mod device {
    //! `ash`-friendly owners for the session path. Vulkan handles arrive
    //! as raw pointers from `ash::vk::Handle::as_raw()`; nothing here links
    //! against Vulkan itself.

    use std::os::raw::c_void;

    use monado_sys::{
        VkDevice, VkImage, VkInstance, VkPhysicalDevice, XrCompositionLayerQuad, XrFrameBeginInfo,
        XrFrameEndInfo, XrFrameState, XrFrameWaitInfo, XrGraphicsBindingVulkanKHR, XrPosef,
        XrQuaternionf, XrReferenceSpaceCreateInfo, XrSessionBeginInfo, XrSpace, XrSwapchain,
        XrSwapchainCreateInfo, XrSwapchainImageAcquireInfo, XrSwapchainImageReleaseInfo,
        XrSwapchainImageVulkanKHR, XrSwapchainImageWaitInfo, XrVector3f,
        XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND, XR_TIMEOUT_EXPIRED, XR_TYPE_FRAME_BEGIN_INFO,
        XR_TYPE_FRAME_END_INFO, XR_TYPE_FRAME_STATE, XR_TYPE_FRAME_WAIT_INFO,
        XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR, XR_TYPE_REFERENCE_SPACE_CREATE_INFO,
        XR_TYPE_SESSION_BEGIN_INFO, XR_TYPE_SWAPCHAIN_CREATE_INFO,
        XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO, XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO,
        XR_TYPE_SWAPCHAIN_IMAGE_VULKAN_KHR, XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO,
        XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
    };

    use super::{FrameTiming, ReferenceSpace, SwapchainSpec, WaitOutcome};
    use crate::check;
    use crate::Instance;
    use crate::XrError;

    /// The Vulkan device the overlay renders with, handed to
    /// `Instance::create_session`. Every handle comes from
    /// `ash::vk::Handle::as_raw()` on the compositor's device.
    #[derive(Debug, Clone, Copy)]
    pub struct GraphicsBinding {
        /// The shared `VkInstance`.
        pub instance: VkInstance,
        /// The `VkPhysicalDevice` the runtime picked.
        pub physical_device: VkPhysicalDevice,
        /// The `VkDevice` the overlay renders with.
        pub device: VkDevice,
        /// The queue family that submits.
        pub queue_family_index: u32,
        /// The queue within the family.
        pub queue_index: u32,
    }

    impl GraphicsBinding {
        /// The raw `XrGraphicsBindingVulkanKHR` this wraps.
        pub(crate) fn raw(&self) -> XrGraphicsBindingVulkanKHR {
            XrGraphicsBindingVulkanKHR {
                type_: XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR,
                next: std::ptr::null(),
                instance: self.instance,
                physical_device: self.physical_device,
                device: self.device,
                queue_family_index: self.queue_family_index,
                queue_index: self.queue_index,
            }
        }
    }

    /// A live OpenXR session. Spaces and swapchains borrow the session, so
    /// the compiler enforces the spec's destruction order.
    #[derive(Debug)]
    pub struct Session<'instance> {
        raw: monado_sys::XrSession,
        _instance: &'instance Instance,
    }

    impl<'instance> Session<'instance> {
        /// Wraps a session handle returned by the loader. Only
        /// `Instance::create_session` calls this.
        pub(crate) fn from_raw(
            raw: monado_sys::XrSession,
            instance: &'instance Instance,
        ) -> Session<'instance> {
            Session {
                raw,
                _instance: instance,
            }
        }

        /// Transition a READY session to RUNNING.
        ///
        /// # Errors
        ///
        /// `SessionNotReady` if the session has not reached READY;
        /// `SessionRunning` if it already runs — see [`XrError`].
        pub fn begin(&self) -> Result<(), XrError> {
            let info = XrSessionBeginInfo {
                type_: XR_TYPE_SESSION_BEGIN_INFO,
                next: std::ptr::null(),
                primary_view_configuration_type: XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
            };
            // SAFETY: `self.raw` is a live session handle and `info` is
            // initialized and outlives the call.
            check(unsafe { monado_sys::xrBeginSession(self.raw, &info) })
        }

        /// Transition a STOPPING session back to IDLE.
        pub fn end(&self) -> Result<(), XrError> {
            // SAFETY: `self.raw` is a live session handle.
            check(unsafe { monado_sys::xrEndSession(self.raw) })
        }

        /// Ask the runtime to move the session to STOPPING (the runtime
        /// then sends a state event; call `end` when it arrives).
        pub fn request_exit(&self) -> Result<(), XrError> {
            // SAFETY: `self.raw` is a live session handle.
            check(unsafe { monado_sys::xrRequestExitSession(self.raw) })
        }

        /// Creates a reference space with an identity pose.
        pub fn reference_space(&self, kind: ReferenceSpace) -> Result<Space<'_>, XrError> {
            let info = XrReferenceSpaceCreateInfo {
                type_: XR_TYPE_REFERENCE_SPACE_CREATE_INFO,
                next: std::ptr::null(),
                reference_space_type: kind.as_raw(),
                pose_in_reference_space: XrPosef {
                    orientation: XrQuaternionf {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 1.0,
                    },
                    position: XrVector3f {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
            };
            let mut raw = 0;
            // SAFETY: `self.raw` is a live session, `info` is initialized
            // and outlives the call, and `raw` is a valid out-pointer.
            check(unsafe { monado_sys::xrCreateReferenceSpace(self.raw, &info, &mut raw) })?;
            Ok(Space {
                raw,
                _session: self,
            })
        }

        /// The session's supported swapchain formats, `VkFormat` values as
        /// `i64`. Pick one before building a [`SwapchainSpec`].
        pub fn swapchain_formats(&self) -> Result<Vec<i64>, XrError> {
            let mut count = 0;
            // SAFETY: `self.raw` is a live session; the two-call pattern is
            // the documented way to size the output buffer.
            check(unsafe {
                monado_sys::xrEnumerateSwapchainFormats(
                    self.raw,
                    0,
                    &mut count,
                    std::ptr::null_mut(),
                )
            })?;
            let mut formats = vec![0i64; count as usize];
            // SAFETY: `formats` holds `count` valid slots.
            check(unsafe {
                monado_sys::xrEnumerateSwapchainFormats(
                    self.raw,
                    count,
                    &mut count,
                    formats.as_mut_ptr(),
                )
            })?;
            Ok(formats)
        }

        /// Creates the overlay's swapchain.
        pub fn create_swapchain(&self, spec: SwapchainSpec) -> Result<Swapchain<'_>, XrError> {
            let info = XrSwapchainCreateInfo {
                type_: XR_TYPE_SWAPCHAIN_CREATE_INFO,
                next: std::ptr::null(),
                create_flags: 0,
                usage_flags: spec.usage_flags,
                format: spec.format,
                sample_count: spec.sample_count,
                width: spec.width,
                height: spec.height,
                face_count: 1,
                array_size: spec.array_size,
                mip_count: spec.mip_count,
            };
            let mut raw = 0;
            // SAFETY: `self.raw` is a live session, `info` is initialized
            // and outlives the call, and `raw` is a valid out-pointer.
            check(unsafe { monado_sys::xrCreateSwapchain(self.raw, &info, &mut raw) })?;
            Ok(Swapchain {
                raw,
                _session: self,
            })
        }

        /// Blocks until the next frame's timing is known.
        pub fn wait_frame(&self) -> Result<FrameTiming, XrError> {
            let wait = XrFrameWaitInfo {
                type_: XR_TYPE_FRAME_WAIT_INFO,
                next: std::ptr::null(),
            };
            let mut state = XrFrameState {
                type_: XR_TYPE_FRAME_STATE,
                next: std::ptr::null_mut(),
                predicted_display_time: 0,
                predicted_display_period: 0,
                should_render: 0,
            };
            // SAFETY: `self.raw` is a live session; `wait` and `state` are
            // initialized and outlive the call.
            check(unsafe { monado_sys::xrWaitFrame(self.raw, &wait, &mut state) })?;
            Ok(FrameTiming {
                display_time: state.predicted_display_time,
                should_render: state.should_render != 0,
            })
        }

        /// Opens the frame; rendering may start.
        pub fn begin_frame(&self) -> Result<(), XrError> {
            let info = XrFrameBeginInfo {
                type_: XR_TYPE_FRAME_BEGIN_INFO,
                next: std::ptr::null(),
            };
            // SAFETY: `self.raw` is a live session; `info` is initialized.
            check(unsafe { monado_sys::xrBeginFrame(self.raw, &info) })
        }

        /// Submits the frame: the overlay always alpha-blends (so the VR
        /// scene shows through the semi-transparent panel).
        pub fn end_frame(
            &self,
            timing: &FrameTiming,
            layers: &[XrCompositionLayerQuad],
        ) -> Result<(), XrError> {
            // The layer array is passed as base-header pointers; the spec's
            // polymorphism. `XrCompositionLayerQuad` is repr(C) with the
            // base header (type/next/flags/space) as its prefix.
            let pointers: Vec<*const c_void> = layers
                .iter()
                .map(|layer| layer as *const XrCompositionLayerQuad as *const c_void)
                .collect();
            let info = XrFrameEndInfo {
                type_: XR_TYPE_FRAME_END_INFO,
                next: std::ptr::null(),
                display_time: timing.display_time,
                environment_blend_mode: XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND,
                layer_count: layers.len() as u32,
                layers: pointers.as_ptr(),
            };
            // SAFETY: `self.raw` is a live session; `info` is initialized,
            // and `pointers` (borrowed by `info.layers`) outlives the call.
            check(unsafe { monado_sys::xrEndFrame(self.raw, &info) })
        }
    }

    impl Drop for Session<'_> {
        fn drop(&mut self) {
            // SAFETY: `self.raw` came from a successful `xrCreateSession`;
            // all spaces and swapchains are dropped first (their borrows
            // guarantee it), and the session is destroyed exactly once here.
            unsafe {
                monado_sys::xrDestroySession(self.raw);
            }
        }
    }

    /// A reference space; destroyed on drop, before its session.
    #[derive(Debug)]
    pub struct Space<'session> {
        raw: XrSpace,
        _session: &'session Session<'session>,
    }

    impl Space<'_> {
        /// The raw handle, for building layer structs.
        pub fn raw(&self) -> XrSpace {
            self.raw
        }
    }

    impl Drop for Space<'_> {
        fn drop(&mut self) {
            // SAFETY: `self.raw` came from a successful
            // `xrCreateReferenceSpace` and is destroyed exactly once, before
            // the session (the borrow guarantees the drop order).
            unsafe {
                monado_sys::xrDestroySpace(self.raw);
            }
        }
    }

    /// The overlay's swapchain; destroyed on drop, before its session.
    #[derive(Debug)]
    pub struct Swapchain<'session> {
        raw: XrSwapchain,
        _session: &'session Session<'session>,
    }

    impl Swapchain<'_> {
        /// Acquires the next image index for writing.
        pub fn acquire(&self) -> Result<u32, XrError> {
            let info = XrSwapchainImageAcquireInfo {
                type_: XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO,
                next: std::ptr::null(),
            };
            let mut index = 0;
            // SAFETY: `self.raw` is a live swapchain; `info` is initialized
            // and `index` is a valid out-pointer.
            check(unsafe { monado_sys::xrAcquireSwapchainImage(self.raw, &info, &mut index) })?;
            Ok(index)
        }

        /// Waits for the acquired image to become writable. A timeout of
        /// `XR_INFINITE_DURATION` (`i64::MAX`) blocks forever.
        pub fn wait(&self, timeout: i64) -> Result<WaitOutcome, XrError> {
            let info = XrSwapchainImageWaitInfo {
                type_: XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO,
                next: std::ptr::null(),
                timeout,
            };
            // SAFETY: `self.raw` is a live swapchain; `info` is initialized.
            let r = unsafe { monado_sys::xrWaitSwapchainImage(self.raw, &info) };
            if r == XR_TIMEOUT_EXPIRED {
                return Ok(WaitOutcome::TimedOut);
            }
            check(r)?;
            Ok(WaitOutcome::Ready)
        }

        /// Releases the acquired image back to the compositor.
        pub fn release(&self) -> Result<(), XrError> {
            let info = XrSwapchainImageReleaseInfo {
                type_: XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO,
                next: std::ptr::null(),
            };
            // SAFETY: `self.raw` is a live swapchain; `info` is initialized.
            check(unsafe { monado_sys::xrReleaseSwapchainImage(self.raw, &info) })
        }

        /// The `VkImage`s behind the swapchain, indexed by [`Swapchain::acquire`]'s
        /// returned index; render into these via ash with external-memory
        /// import (the remaining piece of the compositor path).
        pub fn vulkan_images(&self) -> Result<Vec<VkImage>, XrError> {
            let mut count = 0;
            // SAFETY: `self.raw` is a live swapchain; two-call pattern.
            check(unsafe {
                monado_sys::xrEnumerateSwapchainImages(
                    self.raw,
                    0,
                    &mut count,
                    std::ptr::null_mut(),
                )
            })?;
            // The runtime writes each image into caller-typed structs; the
            // type must be set before the call, per the spec.
            let mut images = vec![
                XrSwapchainImageVulkanKHR {
                    type_: XR_TYPE_SWAPCHAIN_IMAGE_VULKAN_KHR,
                    next: std::ptr::null_mut(),
                    image: std::ptr::null_mut(),
                };
                count as usize
            ];
            // SAFETY: `images` holds `count` caller-initialized structs.
            check(unsafe {
                monado_sys::xrEnumerateSwapchainImages(
                    self.raw,
                    count,
                    &mut count,
                    images.as_mut_ptr(),
                )
            })?;
            Ok(images.into_iter().map(|image| image.image).collect())
        }

        /// The raw handle, for building layer structs.
        pub fn raw(&self) -> XrSwapchain {
            self.raw
        }
    }

    impl Drop for Swapchain<'_> {
        fn drop(&mut self) {
            // SAFETY: `self.raw` came from a successful `xrCreateSwapchain`
            // and is destroyed exactly once, before the session (the borrow
            // guarantees the drop order).
            unsafe {
                monado_sys::xrDestroySwapchain(self.raw);
            }
        }
    }
}

#[cfg(feature = "link")]
pub use device::{GraphicsBinding, Session, Space, Swapchain};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_states_roundtrip_the_documented_values() {
        for raw in 0..=8 {
            let state = SessionState::from_raw(raw).expect("documented value");
            assert_eq!(state.as_raw(), raw, "roundtrip for {raw}");
        }
        assert_eq!(SessionState::from_raw(0), Some(SessionState::Unknown));
        assert_eq!(SessionState::from_raw(5), Some(SessionState::Focused));
        assert_eq!(SessionState::from_raw(8), Some(SessionState::Exiting));
        assert_eq!(SessionState::from_raw(9), None, "outside the 1.0 enum");
        assert_eq!(SessionState::from_raw(-1), None);
    }

    #[test]
    fn parse_event_returns_none_on_an_empty_buffer() {
        // SAFETY: all-zero is a valid bit pattern for this plain C struct;
        // nothing was polled into it.
        let buffer: XrEventDataBuffer = unsafe { std::mem::zeroed() };
        assert_eq!(parse_event(&buffer), Event::None);
    }

    #[test]
    fn parse_event_reads_session_state_changes() {
        // SAFETY: all-zero is a valid bit pattern for this plain C struct.
        let mut buffer: XrEventDataBuffer = unsafe { std::mem::zeroed() };
        // The event overlays the whole buffer, per the spec; write it that
        // way and let `parse_event` read it back.
        // SAFETY: `XrEventDataSessionStateChanged` (40 bytes) fits in
        // `XrEventDataBuffer` (4016 bytes), and both are plain C structs.
        let event = unsafe {
            &mut *(&mut buffer as *mut XrEventDataBuffer
                as *mut monado_sys::XrEventDataSessionStateChanged)
        };
        event.type_ = XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED;
        event.session = 7;
        event.state = monado_sys::XR_SESSION_STATE_FOCUSED;
        event.time = 99;
        assert_eq!(
            parse_event(&buffer),
            Event::SessionStateChanged {
                state: SessionState::Focused,
                session: 7,
                time: 99,
            }
        );
    }

    #[test]
    fn parse_event_maps_unknown_states_and_other_types() {
        // SAFETY: all-zero is a valid bit pattern for this plain C struct.
        let mut buffer: XrEventDataBuffer = unsafe { std::mem::zeroed() };
        // SAFETY: same overlay as the test above.
        let event = unsafe {
            &mut *(&mut buffer as *mut XrEventDataBuffer
                as *mut monado_sys::XrEventDataSessionStateChanged)
        };
        event.type_ = XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED;
        event.state = 1234; // not in the 1.0 enum
        assert_eq!(
            parse_event(&buffer),
            Event::SessionStateChanged {
                state: SessionState::Unknown,
                session: 0,
                time: 0,
            }
        );

        // SAFETY: all-zero is a valid bit pattern for this plain C struct.
        let mut loss: XrEventDataBuffer = unsafe { std::mem::zeroed() };
        loss.type_ = XR_TYPE_EVENT_DATA_INSTANCE_LOSS_PENDING;
        assert_eq!(parse_event(&loss), Event::InstanceLossPending);

        // SAFETY: all-zero is a valid bit pattern for this plain C struct.
        let mut other: XrEventDataBuffer = unsafe { std::mem::zeroed() };
        other.type_ = 200;
        assert_eq!(parse_event(&other), Event::Other(200));
    }

    #[test]
    fn quad_spec_builds_the_composition_layer() {
        let spec = QuadSpec {
            width_m: 1.0,
            height_m: 0.5,
            distance_m: 2.0,
        };
        let layer = spec.layer(5, 9, 1024, 768);
        assert_eq!(layer.type_, XR_TYPE_COMPOSITION_LAYER_QUAD);
        assert_eq!(layer.space, 9);
        assert_eq!(layer.sub_image.swapchain, 5);
        assert_eq!(layer.sub_image.image_array_index, 0);
        assert_eq!(
            layer.sub_image.image_rect.offset,
            XrOffset2Di { x: 0, y: 0 }
        );
        assert_eq!(
            layer.sub_image.image_rect.extent,
            monado_sys::XrExtent2Di {
                width: 1024,
                height: 768
            }
        );
        assert_eq!(layer.eye_visibility, monado_sys::XR_EYE_VISIBILITY_BOTH);
        // Identity orientation, centered, 2 m in front (−Z).
        assert_eq!(layer.pose.orientation.w, 1.0);
        assert_eq!(layer.pose.position.z, -2.0);
        assert_eq!(layer.size.width, 1.0);
        assert_eq!(layer.size.height, 0.5);
    }

    #[test]
    fn overlay_swapchain_spec_sets_usage_and_shape() {
        let spec = SwapchainSpec::overlay(1024, 768, 37);
        assert_eq!(spec.width, 1024);
        assert_eq!(spec.height, 768);
        assert_eq!(spec.format, 37);
        assert_eq!(
            spec.usage_flags,
            monado_sys::XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT
        );
        assert_eq!(spec.sample_count, 1);
        assert_eq!(spec.array_size, 1);
        assert_eq!(spec.mip_count, 1);
    }

    #[test]
    fn wait_outcomes_and_frame_timing_carry_plain_data() {
        assert_eq!(WaitOutcome::Ready, WaitOutcome::Ready);
        assert_ne!(WaitOutcome::Ready, WaitOutcome::TimedOut);
        let timing = FrameTiming {
            display_time: 42,
            should_render: true,
        };
        assert_eq!(timing.display_time, 42);
        assert!(timing.should_render);
        assert_eq!(
            FrameTiming {
                display_time: 1,
                should_render: false
            },
            FrameTiming {
                display_time: 1,
                should_render: false
            }
        );
    }

    #[test]
    fn reference_spaces_map_to_their_raw_values() {
        assert_eq!(
            ReferenceSpace::Local.as_raw(),
            monado_sys::XR_REFERENCE_SPACE_TYPE_LOCAL
        );
        assert_eq!(
            ReferenceSpace::Stage.as_raw(),
            monado_sys::XR_REFERENCE_SPACE_TYPE_STAGE
        );
    }

    /// `Session::end_frame` passes layer structs as base-header pointers;
    /// this pins the prefix layout the cast relies on.
    #[test]
    fn quad_layer_prefix_matches_the_base_header() {
        let spec = QuadSpec {
            width_m: 1.0,
            height_m: 1.0,
            distance_m: 1.0,
        };
        let layer = spec.layer(5, 9, 64, 64);
        // The base header is type/next/layer_flags/space; the quad's first
        // fields must overlay them exactly (repr(C), same types).
        let base = &layer as *const XrCompositionLayerQuad as *const monado_sys::XrEventDataBuffer;
        // SAFETY: the cast is only read through the first 16 bytes, which
        // both structs share as repr(C) prefixes; nothing is written.
        let prefix = unsafe { std::slice::from_raw_parts(base as *const u8, 8) };
        let expected: [u8; 4] = XR_TYPE_COMPOSITION_LAYER_QUAD.to_ne_bytes();
        assert_eq!(prefix[0..4], expected);
    }
}
