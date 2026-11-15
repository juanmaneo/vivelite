//! The overlay inside the OpenXR frame loop (feature `xr`).
//!
//! The pixel path, end to end:
//!
//! 1. wgpu creates the Vulkan instance/adapter/device/queue
//!    ([`XrGpu::new`]); only the Vulkan backend is requested.
//! 2. [`XrGpu::graphics_binding`] reads the raw `VkInstance`,
//!    `VkPhysicalDevice`, `VkDevice`, and queue indices back out of wgpu
//!    (`as_hal`) and hands them to Monado — the session's swapchain
//!    images are allocated on this same `VkDevice`.
//! 3. Per frame: the runtime's `VkImage` is wrapped as a wgpu texture
//!    (`wgpu::Device::create_texture_from_hal` over a sentinel-guarded
//!    wgpu-hal texture, so wgpu never destroys the runtime's image), the
//!    iced program is drawn with [`iced_runtime::UserInterface`] and
//!    presented by iced's wgpu engine, and the encoder is submitted on
//!    the same queue the binding named.
//! 4. The frame closes with one `XrCompositionLayerQuad` in front of the
//!    user; the runtime alpha-blends it over the passthrough view.
//!
//! Everything here needs a live Vulkan driver and OpenXR runtime, which
//! is why it is feature-gated away from `dev` and the coverage gate. The
//! decision logic it runs is the always-on [`crate::xr::plan`], and the
//! format negotiation is [`crate::xr::format`].
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fmt;
use std::thread;
use std::time::Duration;

use ash::vk::Handle as _;
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_wgpu::graphics::Viewport;
use iced_wgpu::{Engine, Renderer as WgpuRenderer};
use monado::session::{GraphicsBinding, Session, Space, Swapchain};
use monado::{
    FrameTiming, Instance, QuadSpec, ReferenceSpace, SwapchainSpec, WaitOutcome, XrError,
};
use wgpu::hal::vulkan::Api;

use crate::compositor::Phase;
use crate::model::{Message, Overlay};
use crate::xr::format;
use crate::xr::plan::{frame_action, FrameAction};

/// How long to wait for a swapchain image to become writable before
/// giving up on the frame (nanoseconds; the wrapper takes an
/// `XrDuration`).
const IMAGE_WAIT_NS: i64 = 1_000_000_000;

/// Overlay swapchain width. 16:9, large enough that 20 px text reads at
/// arm's length.
pub const WIDTH_PX: u32 = 1280;

/// Overlay swapchain height.
pub const HEIGHT_PX: u32 = 720;

/// The overlay quad in the user's local space: a 1.2 m × 0.675 m panel
/// (16:9, matching the swapchain) floating 1.75 m in front of the user.
#[must_use]
pub fn quad_spec() -> QuadSpec {
    QuadSpec {
        width_m: 1.2,
        height_m: 0.675,
        distance_m: 1.75,
    }
}

/// Everything the overlay path can fail with.
#[derive(Debug)]
pub enum OverlayError {
    /// wgpu found no Vulkan adapter.
    NoAdapter,
    /// wgpu could not create a device on the adapter.
    DeviceRequest(String),
    /// The wgpu device is not Vulkan-backed (its handles could not be
    /// extracted), or a swapchain image could not be wrapped.
    NoVulkan,
    /// The runtime offers none of the overlay's supported `VkFormat`s.
    UnsupportedFormat,
    /// The iced renderer is not the wgpu (primary) variant.
    RendererUnavailable,
    /// An OpenXR call failed.
    Xr(XrError),
}

impl From<XrError> for OverlayError {
    fn from(error: XrError) -> Self {
        OverlayError::Xr(error)
    }
}

impl fmt::Display for OverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverlayError::NoAdapter => {
                write!(f, "no Vulkan adapter found for the overlay")
            }
            OverlayError::DeviceRequest(error) => {
                write!(f, "could not create the overlay device: {error}")
            }
            OverlayError::NoVulkan => {
                write!(f, "the wgpu device is not Vulkan-backed")
            }
            OverlayError::UnsupportedFormat => {
                write!(
                    f,
                    "the runtime offers no swapchain format the overlay renders"
                )
            }
            OverlayError::RendererUnavailable => {
                write!(f, "internal: the iced renderer is not the wgpu backend")
            }
            OverlayError::Xr(error) => write!(f, "OpenXR: {error:?}"),
        }
    }
}

impl std::error::Error for OverlayError {}

/// The wgpu Vulkan device the overlay renders with.
pub struct XrGpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl XrGpu {
    /// Creates a Vulkan-only wgpu instance, adapter, device, and queue.
    ///
    /// # Errors
    ///
    /// [`OverlayError::NoAdapter`] without a Vulkan driver and
    /// [`OverlayError::DeviceRequest`] when the device request fails.
    pub fn new() -> Result<Self, OverlayError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or(OverlayError::NoAdapter)?;
        // The limits mirror what iced's own compositor requests
        // (`max_bind_groups: 2`); the overlay renders no more complex
        // pipelines than the desktop preview window does.
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("vivelite xr overlay device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits {
                    max_bind_groups: 2,
                    ..wgpu::Limits::default()
                },
            },
            None,
        ))
        .map_err(|error| OverlayError::DeviceRequest(format!("{error:?}")))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// The graphics binding for `Instance::create_session`: the raw
    /// `VkInstance`, `VkPhysicalDevice`, and `VkDevice` wgpu created, plus
    /// the family and index of the overlay's render queue. `None` when the
    /// wgpu device is not Vulkan-backed.
    ///
    /// The handles are plain pointers into wgpu's owners; a session built
    /// from them must be destroyed before this XrGpu drops. [`run`]
    /// keeps that order.
    #[must_use]
    pub fn graphics_binding(&self) -> Option<GraphicsBinding> {
        // SAFETY: the handles are only read, never freed here; wgpu keeps
        // the instance, adapter, and device alive for `self`'s lifetime,
        // and the session built from the binding is destroyed before this
        // XrGpu drops in `run`'s ownership order.
        unsafe {
            let instance = self.instance.as_hal::<Api>()?;
            let vk_instance =
                instance.shared_instance().raw_instance().handle().as_raw() as *mut c_void;
            let vk_physical_device = self.adapter.as_hal::<Api, _, _>(|adapter| {
                adapter.map(|adapter| adapter.raw_physical_device().as_raw() as *mut c_void)
            })?;
            let (vk_device, queue_family_index, queue_index) =
                self.device.as_hal::<Api, _, _>(|device| {
                    device.map(|device| {
                        (
                            device.raw_device().handle().as_raw() as *mut c_void,
                            device.queue_family_index(),
                            device.queue_index(),
                        )
                    })
                })??;
            Some(GraphicsBinding {
                instance: vk_instance,
                physical_device: vk_physical_device,
                device: vk_device,
                queue_family_index,
                queue_index,
            })
        }
    }

    /// Wraps one runtime-owned swapchain `VkImage` as a wgpu texture.
    ///
    /// `None` when the device is not Vulkan-backed. The wrapper carries a
    /// sentinel drop guard, so dropping it never destroys the runtime's
    /// image (Monado destroys it with the swapchain).
    fn wrap_image(
        &self,
        image: *mut c_void,
        extent: wgpu::Extent3d,
        texture_format: wgpu::TextureFormat,
    ) -> Option<wgpu::Texture> {
        let hal_descriptor = wgpu::hal::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::hal::TextureUses::COLOR_TARGET,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        let descriptor = wgpu::TextureDescriptor {
            label: Some("vivelite xr overlay swapchain image"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let hal_image = ash::vk::Image::from_raw(image as u64);
        // SAFETY: `image` is a live VkImage from this session's swapchain,
        // allocated on the same VkDevice that `self.device` wraps (the
        // session was created from this device's graphics binding), the
        // descriptors mirror the swapchain's real extent, format, and
        // single-sample color-attachment usage, and the sentinel drop
        // guard keeps wgpu from destroying the runtime-owned image when
        // the wrapper drops. `as_hal` is unsafe only because it exposes the
        // raw backend; the closure reads no handle beyond the device check.
        let hal_texture = unsafe {
            self.device.as_hal::<Api, _, _>(|hal_device| {
                hal_device.map(|_| {
                    wgpu::hal::vulkan::Device::texture_from_raw(
                        hal_image,
                        &hal_descriptor,
                        Some(Box::new(())),
                    )
                })
            })??
        };
        // SAFETY: the hal texture was built from this device's raw handle
        // one call above; the wgpu descriptor mirrors the hal descriptor,
        // and the render pass clears and writes the whole image before
        // the runtime reads it.
        Some(unsafe {
            self.device
                .create_texture_from_hal::<Api>(hal_texture, &descriptor)
        })
    }
}

/// The overlay's offscreen iced renderer, bound to one texture format.
#[allow(missing_debug_implementations)]
pub struct XrRenderer {
    gpu: XrGpu,
    engine: Engine,
    /// The iced renderer. Only the `Primary` (wgpu) variant is ever
    /// constructed; the tiny-skia fallback never runs on the device.
    renderer: iced::Renderer,
    format: wgpu::TextureFormat,
}

impl XrRenderer {
    /// Builds iced's wgpu engine and renderer for `format`.
    #[must_use]
    pub fn new(gpu: XrGpu, format: wgpu::TextureFormat) -> Self {
        let engine = Engine::new(&gpu.adapter, &gpu.device, &gpu.queue, format, None);
        let renderer = iced::Renderer::Primary(WgpuRenderer::new(
            &gpu.device,
            &engine,
            iced::Font::default(),
            iced::Pixels::from(16.0),
        ));
        Self {
            gpu,
            engine,
            renderer,
            format,
        }
    }

    /// Renders one frame of the overlay program into `image`.
    ///
    /// `image` must be an acquired (and waited-on) swapchain `VkImage` of
    /// this session; `width`/`height` its extent.
    ///
    /// # Errors
    ///
    /// [`OverlayError::NoVulkan`] when the image cannot be wrapped, and
    /// [`OverlayError::RendererUnavailable`] if the iced renderer is not
    /// the wgpu backend (never constructed here).
    pub fn render_into(
        &mut self,
        overlay: &Overlay,
        image: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<(), OverlayError> {
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let Some(texture) = self.gpu.wrap_image(image, extent, self.format) else {
            return Err(OverlayError::NoVulkan);
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Scale factor 1.0: the overlay lays out in physical pixels, so
        // the same view code sizes itself identically at any resolution.
        let viewport = Viewport::with_physical_size(iced_wgpu::core::Size::new(width, height), 1.0);
        let bounds = iced::Size::new(width as f32, height as f32);

        let mut user_interface = UserInterface::build(
            crate::view::view(overlay),
            bounds,
            Cache::new(),
            &mut self.renderer,
        );
        // No pointer on the device: the cursor is always unavailable, so
        // widgets draw their un-hovered state.
        let _interaction = user_interface.draw(
            &mut self.renderer,
            &iced::Theme::Dark,
            &iced_wgpu::core::renderer::Style {
                text_color: iced::Color::WHITE,
            },
            iced::mouse::Cursor::Unavailable,
        );

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vivelite xr overlay encoder"),
            });
        let no_overlay: [&str; 0] = [];
        match &mut self.renderer {
            iced::Renderer::Primary(renderer) => renderer.present(
                &mut self.engine,
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                // Clear to transparent: the runtime alpha-blends the quad
                // over the passthrough view, so the panel shows the scene
                // wherever the overlay draws nothing.
                Some(iced::Color::TRANSPARENT),
                self.format,
                &view,
                &viewport,
                &no_overlay,
            ),
            _ => return Err(OverlayError::RendererUnavailable),
        }
        // Submits the encoder on the queue the graphics binding named, so
        // the runtime's synchronization (wired to that queue) orders the
        // overlay's pixels before composition.
        self.engine.submit(&self.gpu.queue, encoder);
        Ok(())
    }
}

/// Runs the overlay against the active OpenXR runtime until the session
/// is lost or exits. Requires the OpenXR loader to be loadable in this
/// process (the same contract as `monado::Instance::create`); the wgpu
/// device, session, and their children are destroyed in spec order on
/// the way out.
///
/// # Errors
///
/// Any step can fail; see [`OverlayError`].
pub fn run() -> Result<(), OverlayError> {
    let gpu = XrGpu::new()?;
    let binding = gpu.graphics_binding().ok_or(OverlayError::NoVulkan)?;

    // SAFETY: the OpenXR loader is loadable in this process (the caller's
    // contract); the instance is destroyed last, after the session and
    // everything that borrows it.
    let instance = unsafe { Instance::create("vivelite xr overlay")? };
    let system = instance.system()?;
    let session = instance.create_session(system, &binding)?;

    // The runtime's format list is authoritative; the overlay picks the
    // best format it can also render through wgpu.
    let formats = session.swapchain_formats()?;
    let vk_format = format::pick_format(&formats).ok_or(OverlayError::UnsupportedFormat)?;
    let texture_format =
        format::texture_format(vk_format).ok_or(OverlayError::UnsupportedFormat)?;

    let mut renderer = XrRenderer::new(gpu, texture_format);
    let space = session.reference_space(ReferenceSpace::Local)?;
    let swapchain =
        session.create_swapchain(SwapchainSpec::overlay(WIDTH_PX, HEIGHT_PX, vk_format))?;
    let images = swapchain.vulkan_images()?;

    let mut overlay = Overlay::new();
    let mut phase = Phase::Connecting;
    overlay.update(Message::Compositor(phase));

    let mut session_running = false;
    loop {
        let event = instance.poll_event()?;
        phase = phase.on_event(&event);
        if overlay.update(Message::Compositor(phase)) {
            eprintln!("overlay: {}", phase.label());
        }

        match phase {
            Phase::Lost => break,
            // READY: begin the session so frames start flowing.
            Phase::Ready if !session_running => {
                session.begin()?;
                session_running = true;
            }
            // STOPPING / LOSS_PENDING: stop rendering and close the
            // session; a READY event later re-enters the loop fresh.
            Phase::Stopping if session_running => {
                session.end()?;
                session_running = false;
            }
            _ => {}
        }

        if !session_running {
            // Not begun: no frames to wait on; poll events at a gentle
            // rate instead of spinning.
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let timing = session.wait_frame()?;
        session.begin_frame()?;
        match frame_action(phase, &timing) {
            FrameAction::Render => render_one_frame(
                &session,
                &space,
                &swapchain,
                &images,
                &mut renderer,
                &mut overlay,
                &timing,
            )?,
            FrameAction::Skip => session.end_frame(&timing, &[])?,
            // Unreachable in practice — Lost breaks the loop before the
            // frame is opened — but closing the frame is never wrong.
            FrameAction::Detach => {
                session.end_frame(&timing, &[])?;
                break;
            }
        }
    }

    Ok(())
}

/// Acquires, renders, releases, and submits one quad layer. The frame
/// is already open (`wait_frame`/`begin_frame` happened in [`run`]);
/// this fills it or hands it back empty on a hiccup, then closes it.
fn render_one_frame(
    session: &Session<'_>,
    space: &Space<'_>,
    swapchain: &Swapchain<'_>,
    images: &[*mut c_void],
    renderer: &mut XrRenderer,
    overlay: &mut Overlay,
    timing: &FrameTiming,
) -> Result<(), OverlayError> {
    let quad = quad_spec();
    let index = swapchain.acquire()? as usize;
    let Some(image) = images.get(index).copied() else {
        // The runtime returned an index outside its own image list; hand
        // the frame back empty rather than guess.
        return session.end_frame(timing, &[]).map_err(OverlayError::from);
    };
    if swapchain.wait(IMAGE_WAIT_NS)? != WaitOutcome::Ready {
        // The image never became writable; release it and close the
        // frame (the compositor keeps the previous image).
        swapchain.release()?;
        return session.end_frame(timing, &[]).map_err(OverlayError::from);
    }
    renderer.render_into(overlay, image, WIDTH_PX, HEIGHT_PX)?;
    swapchain.release()?;
    let layer = quad.layer(
        swapchain.raw(),
        space.raw(),
        WIDTH_PX as i32,
        HEIGHT_PX as i32,
    );
    session.end_frame(timing, std::slice::from_ref(&layer))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_matches_the_swapchain_aspect() {
        let quad = quad_spec();
        assert!((quad.width_m / quad.height_m - 16.0 / 9.0).abs() < 0.01);
        assert_eq!((WIDTH_PX, HEIGHT_PX), (1280, 720));
        assert_eq!(quad.distance_m, 1.75);
    }

    #[test]
    fn errors_describe_their_cause() {
        let cases = [
            (OverlayError::NoAdapter, "no Vulkan adapter"),
            (
                OverlayError::DeviceRequest("boom".into()),
                "could not create the overlay device: boom",
            ),
            (OverlayError::NoVulkan, "not Vulkan-backed"),
            (
                OverlayError::UnsupportedFormat,
                "no swapchain format the overlay renders",
            ),
            (OverlayError::RendererUnavailable, "not the wgpu backend"),
            (
                OverlayError::Xr(XrError::RuntimeFailure),
                "OpenXR: RuntimeFailure",
            ),
        ];
        for (error, expected) in cases {
            let text = error.to_string();
            assert!(
                text.contains(expected),
                "{text} does not contain {expected}"
            );
        }
    }
}
