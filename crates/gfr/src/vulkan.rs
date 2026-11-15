//! Vulkan execution for the AMD and portable marker backends.
//!
//! The probe submits a small marker-only flight. Applications can use the
//! same marker plan and report types to instrument their own command buffers;
//! transparent layer interception is intentionally out of scope here.
//!
//! Compiled only with the `vulkan` feature, so the default build, its tests,
//! and the coverage gate never link a Vulkan loader. Run it with
//! `cargo xtask gfr-probe`.

#![allow(unsafe_code)]
// The unsafe blocks below all carry SAFETY comments; the allow only silences
// the workspace's `unsafe_code = warn` lint, matching the `-sys` crates.

use std::ffi::c_char;

use ash::{vk, Entry};

use crate::backend::{self, Backend, BackendRequest};
use crate::marker::MarkerPlan;
use crate::report::{analyze, FlightReport};

/// Marker writer: each dispatch stores `value` into word `slot` of the buffer.
const MARKER_SHADER: &str = "#version 450
layout(local_size_x = 1) in;
layout(std430, set = 0, binding = 0) buffer MarkerWords { uint words[]; };
layout(push_constant) uniform MarkerPush { uint slot; uint value; } pc;
void main() { words[pc.slot] = pc.value; }
";

/// Fence wait bound in nanoseconds (5 s). A wait that exceeds it means the
/// probe's own work is wedged, which is reported as an error.
const FENCE_WAIT_NS: u64 = 5_000_000_000;

/// Failure while setting up or running a marker probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The system Vulkan loader could not be loaded.
    Loader(String),
    /// No compute-capable physical device was found.
    NoDevice,
    /// The requested marker backend cannot run on the selected device.
    Backend(String),
    /// No host-visible coherent memory is available.
    NoMemoryType,
    /// Marker shader translation failed.
    Shader(String),
    /// Vulkan returned an error.
    Vulkan(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loader(message) => write!(f, "Vulkan loader: {message}"),
            Self::NoDevice => f.write_str("no compute-capable Vulkan device"),
            Self::Backend(message) => write!(f, "marker backend: {message}"),
            Self::NoMemoryType => f.write_str("no HOST_VISIBLE|HOST_COHERENT memory type"),
            Self::Shader(message) => write!(f, "marker shader: {message}"),
            Self::Vulkan(message) => write!(f, "Vulkan: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Result of a marker-only probe and the selected device backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// Selected marker implementation.
    pub backend: Backend,
    /// Device-reported Vulkan physical-device name.
    pub device_name: String,
    /// Interpreted marker flight.
    pub flight: FlightReport,
}

/// Runs a marker-only flight. `Auto` prefers AMD markers, then uses the
/// core-Vulkan shader path, which is the intended Adreno implementation.
pub fn run_probe(request: BackendRequest) -> Result<ProbeReport, Error> {
    // SAFETY: ash dlopens the system loader; `entry` outlives the instance and
    // every call made through it below.
    let entry = unsafe { Entry::load() }.map_err(|error| Error::Loader(error.to_string()))?;
    let app_name = c"vivelite-gfr-probe";
    let app = vk::ApplicationInfo::default()
        .application_name(app_name)
        .engine_name(app_name)
        .api_version(vk::API_VERSION_1_0);
    let create = vk::InstanceCreateInfo::default().application_info(&app);
    // SAFETY: no layers or extensions are requested; the instance is
    // destroyed below, before `entry` is dropped.
    let instance = unsafe { entry.create_instance(&create, None) }.map_err(vk_error)?;
    let result = run_on_instance(&instance, request);
    // SAFETY: `run_on_instance` has destroyed every object it created.
    unsafe { instance.destroy_instance(None) };
    result
}

fn run_on_instance(
    instance: &ash::Instance,
    request: BackendRequest,
) -> Result<ProbeReport, Error> {
    // SAFETY: enumeration on a live instance.
    let physicals = unsafe { instance.enumerate_physical_devices() }.map_err(vk_error)?;
    let (physical, family) = physicals
        .into_iter()
        .find_map(|physical| {
            // SAFETY: querying a physical device enumerated by this instance.
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(physical) };
            families
                .iter()
                .position(|props| props.queue_flags.contains(vk::QueueFlags::COMPUTE))
                .map(|family| (physical, family as u32))
        })
        .ok_or(Error::NoDevice)?;

    // SAFETY: extension enumeration for the physical device chosen above.
    let extensions =
        unsafe { instance.enumerate_device_extension_properties(physical) }.map_err(vk_error)?;
    let amd_extension = extensions.iter().any(|extension| {
        c_name(&extension.extension_name) == vk::AMD_BUFFER_MARKER_NAME.to_bytes()
    });
    let backend = backend::select(request, amd_extension).map_err(Error::Backend)?;
    let name = device_name(instance, physical);
    let flight = run_on_new_device(instance, physical, family, backend)?;
    Ok(ProbeReport {
        backend,
        device_name: name,
        flight,
    })
}

fn run_on_new_device(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    family: u32,
    backend: Backend,
) -> Result<FlightReport, Error> {
    let priorities = [1.0];
    let queue_infos = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(family)
        .queue_priorities(&priorities)];
    let amd_extension: [*const c_char; 1] = [vk::AMD_BUFFER_MARKER_NAME.as_ptr()];
    let extensions: &[*const c_char] = match backend {
        Backend::Amd => &amd_extension,
        Backend::Portable => &[],
    };
    let device_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_infos)
        .enabled_extension_names(extensions);
    // SAFETY: the queue family, and the AMD extension when selected, were
    // advertised by this physical device.
    let device =
        unsafe { instance.create_device(physical, &device_info, None) }.map_err(vk_error)?;
    let result = run_flight(instance, physical, &device, family, backend);
    // SAFETY: `run_flight` released every object it created. If its fence
    // timed out the device may still be busy; destroying it then is the
    // probe's last act before reporting that failure.
    unsafe { device.destroy_device(None) };
    result
}

/// Cleanup actions run in reverse registration order when dropped, including
/// on error paths, so each object is destroyed after the objects using it.
#[derive(Default)]
struct Cleanup<'a> {
    actions: Vec<Box<dyn FnOnce() + 'a>>,
}

impl<'a> Cleanup<'a> {
    fn defer(&mut self, action: impl FnOnce() + 'a) {
        self.actions.push(Box::new(action));
    }
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        while let Some(action) = self.actions.pop() {
            action();
        }
    }
}

/// How one marker is written into the flight buffer.
enum Recorder {
    Amd(ash::amd::buffer_marker::Device),
    Portable(MarkerPipeline),
}

fn run_flight(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    family: u32,
    backend: Backend,
) -> Result<FlightReport, Error> {
    let plan = MarkerPlan::new(["probe begin", "probe marker write", "probe complete"]);
    let bytes = (plan.len() * 4) as vk::DeviceSize;
    let mut cleanup = Cleanup::default();

    // SAFETY: queue 0 of the family was requested at device creation.
    let queue = unsafe { device.get_device_queue(family, 0) };
    // SAFETY: memory properties of an enumerated physical device.
    let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical) };

    let buffer_info = vk::BufferCreateInfo::default()
        .size(bytes)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    // SAFETY: buffer creation on a live device.
    let buffer = unsafe { device.create_buffer(&buffer_info, None) }.map_err(vk_error)?;
    // SAFETY: requirements of a live buffer.
    let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    let Some(memory_index) = memory_type(&memory_properties, requirements.memory_type_bits) else {
        // SAFETY: nothing references the buffer yet; destroyed on this failure path.
        unsafe { device.destroy_buffer(buffer, None) };
        return Err(Error::NoMemoryType);
    };
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_index);
    // SAFETY: allocation with the memory type reported for this buffer.
    let memory = match unsafe { device.allocate_memory(&alloc_info, None) } {
        Ok(memory) => memory,
        Err(error) => {
            // SAFETY: nothing references the buffer yet; destroyed on this failure path.
            unsafe { device.destroy_buffer(buffer, None) };
            return Err(vk_error(error));
        }
    };
    // Actions run in reverse: the buffer is destroyed before its memory is freed.
    cleanup.defer(move || {
        // SAFETY: the memory is freed after the buffer bound to it is destroyed.
        unsafe { device.free_memory(memory, None) }
    });
    cleanup.defer(move || {
        // SAFETY: the buffer is destroyed once nothing in this flight uses it.
        unsafe { device.destroy_buffer(buffer, None) }
    });
    // SAFETY: offset 0 satisfies the buffer alignment for a fresh allocation.
    unsafe { device.bind_buffer_memory(buffer, memory, 0) }.map_err(vk_error)?;
    // SAFETY: the memory type is HOST_VISIBLE and mapping covers the whole allocation.
    let mapped =
        unsafe { device.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()) }
            .map_err(vk_error)?;
    cleanup.defer(move || {
        // SAFETY: this is the mapping returned above and cleanup runs before
        // the allocation is freed.
        unsafe { device.unmap_memory(memory) }
    });
    // SAFETY: the mapping spans at least `bytes` bytes; clearing it makes an
    // unwritten marker read as zero.
    unsafe { std::ptr::write_bytes(mapped.cast::<u8>(), 0, bytes as usize) };

    let recorder = match backend {
        Backend::Amd => Recorder::Amd(ash::amd::buffer_marker::Device::new(instance, device)),
        Backend::Portable => {
            Recorder::Portable(create_marker_pipeline(device, buffer, &mut cleanup)?)
        }
    };

    let pool_info = vk::CommandPoolCreateInfo::default().queue_family_index(family);
    // SAFETY: command pool for the queue family the device was created with.
    let pool = unsafe { device.create_command_pool(&pool_info, None) }.map_err(vk_error)?;
    cleanup.defer(move || {
        // SAFETY: the pool is destroyed once its command buffer is no longer in use.
        unsafe { device.destroy_command_pool(pool, None) }
    });
    let alloc = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    // SAFETY: one primary command buffer from a pool of this device.
    let command = unsafe { device.allocate_command_buffers(&alloc) }.map_err(vk_error)?[0];

    let begin =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    // SAFETY: the command buffer was just allocated and is not pending.
    unsafe { device.begin_command_buffer(command, &begin) }.map_err(vk_error)?;
    for marker in plan.markers() {
        match &recorder {
            Recorder::Amd(extension) => {
                // SAFETY: the buffer has TRANSFER_DST, the offset is 4-byte
                // aligned and inside the buffer, and the extension is enabled
                // on this device (the backend was selected from its support).
                unsafe {
                    extension.cmd_write_buffer_marker(
                        command,
                        vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                        buffer,
                        u64::from(marker.id - 1) * 4,
                        marker.id,
                    );
                }
            }
            Recorder::Portable(pipeline) => {
                record_marker_dispatch(device, command, pipeline, marker.id)
            }
        }
    }
    // SAFETY: the command buffer is in the recording state.
    unsafe { device.end_command_buffer(command) }.map_err(vk_error)?;

    let fence_info = vk::FenceCreateInfo::default();
    // SAFETY: fence creation on a live device.
    let fence = unsafe { device.create_fence(&fence_info, None) }.map_err(vk_error)?;
    cleanup.defer(move || {
        // SAFETY: the fence is destroyed once nothing references it.
        unsafe { device.destroy_fence(fence, None) }
    });
    let command_buffers = [command];
    let submit = vk::SubmitInfo::default().command_buffers(&command_buffers);
    // SAFETY: a fully recorded command buffer submitted to the queue of its own family.
    unsafe { device.queue_submit(queue, &[submit], fence) }.map_err(vk_error)?;
    // SAFETY: waits only on the fence submitted above, with a bounded timeout.
    unsafe { device.wait_for_fences(&[fence], true, FENCE_WAIT_NS) }
        .map_err(|error| Error::Vulkan(format!("marker fence did not signal: {error}")))?;

    // SAFETY: the fence has signaled, so the device's writes are visible through
    // this HOST_COHERENT mapping, which covers `plan.len()` words.
    let words = unsafe { std::slice::from_raw_parts(mapped.cast::<u32>(), plan.len()) }.to_vec();
    Ok(analyze(&words, &plan))
}

/// A compute pipeline that stores one marker word per dispatch.
struct MarkerPipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    set: vk::DescriptorSet,
}

fn create_marker_pipeline<'a>(
    device: &'a ash::Device,
    buffer: vk::Buffer,
    cleanup: &mut Cleanup<'a>,
) -> Result<MarkerPipeline, Error> {
    let binding = [vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::COMPUTE)];
    let set_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&binding);
    // SAFETY: layout creation on a live device.
    let set_layout =
        unsafe { device.create_descriptor_set_layout(&set_layout_info, None) }.map_err(vk_error)?;
    cleanup.defer(move || {
        // SAFETY: destroyed after the pipeline layout that references it.
        unsafe { device.destroy_descriptor_set_layout(set_layout, None) }
    });

    let pool_sizes = [vk::DescriptorPoolSize {
        ty: vk::DescriptorType::STORAGE_BUFFER,
        descriptor_count: 1,
    }];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(&pool_sizes)
        .max_sets(1);
    // SAFETY: pool creation on a live device.
    let pool = unsafe { device.create_descriptor_pool(&pool_info, None) }.map_err(vk_error)?;
    cleanup.defer(move || {
        // SAFETY: destroying the pool frees the descriptor set allocated from it.
        unsafe { device.destroy_descriptor_pool(pool, None) }
    });
    let set_layouts = [set_layout];
    let alloc = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&set_layouts);
    // SAFETY: one set from a pool sized for exactly one storage buffer.
    let set = unsafe { device.allocate_descriptor_sets(&alloc) }.map_err(vk_error)?[0];
    let buffer_infos = [vk::DescriptorBufferInfo::default()
        .buffer(buffer)
        .offset(0)
        .range(vk::WHOLE_SIZE)];
    let writes = [vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .buffer_info(&buffer_infos)];
    // SAFETY: the write targets the set just allocated and the live flight buffer.
    unsafe { device.update_descriptor_sets(&writes, &[]) };

    let push_ranges = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::COMPUTE)
        .offset(0)
        .size(8)];
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts)
        .push_constant_ranges(&push_ranges);
    // SAFETY: layout creation on a live device.
    let layout = unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(vk_error)?;
    cleanup.defer(move || {
        // SAFETY: destroyed after the pipeline that uses it.
        unsafe { device.destroy_pipeline_layout(layout, None) }
    });

    let words = marker_spirv()?;
    let module_info = vk::ShaderModuleCreateInfo::default().code(&words);
    // SAFETY: the SPIR-V was generated and validated by naga just above.
    let module = unsafe { device.create_shader_module(&module_info, None) }.map_err(vk_error)?;
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(c"main");
    let pipeline_info = [vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(layout)];
    // SAFETY: a compute pipeline from a valid module and layout; no pipeline cache.
    let created =
        unsafe { device.create_compute_pipelines(vk::PipelineCache::null(), &pipeline_info, None) };
    // SAFETY: the module is only needed for the creation call that just returned.
    unsafe { device.destroy_shader_module(module, None) };
    let pipeline = created.map_err(|(_, error)| vk_error(error))?[0];
    cleanup.defer(move || {
        // SAFETY: destroyed before its layout.
        unsafe { device.destroy_pipeline(pipeline, None) }
    });

    Ok(MarkerPipeline {
        pipeline,
        layout,
        set,
    })
}

fn marker_spirv() -> Result<Vec<u32>, Error> {
    let mut frontend = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options::from(naga::ShaderStage::Compute);
    let module = frontend
        .parse(&options, MARKER_SHADER)
        .map_err(|errors| Error::Shader(errors.to_string()))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| Error::Shader(error.to_string()))?;
    let options = naga::back::spv::Options {
        lang_version: (1, 0),
        ..Default::default()
    };
    let pipeline = naga::back::spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Compute,
        entry_point: "main".into(),
    };
    naga::back::spv::write_vec(&module, &info, &options, Some(&pipeline))
        .map_err(|error| Error::Shader(error.to_string()))
}

/// Records one dispatch that writes `id` into word `id - 1`, followed by a
/// barrier so the next marker cannot run ahead of this one.
fn record_marker_dispatch(
    device: &ash::Device,
    command: vk::CommandBuffer,
    pipeline: &MarkerPipeline,
    id: u32,
) {
    let mut push = [0u8; 8];
    push[..4].copy_from_slice(&(id - 1).to_ne_bytes());
    push[4..].copy_from_slice(&id.to_ne_bytes());
    let sets = [pipeline.set];
    let barrier = [vk::MemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)];
    // SAFETY: the pipeline, layout, and set belong to this device, and the
    // command buffer is in the recording state.
    unsafe {
        device.cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, pipeline.pipeline);
        device.cmd_bind_descriptor_sets(
            command,
            vk::PipelineBindPoint::COMPUTE,
            pipeline.layout,
            0,
            &sets,
            &[],
        );
        device.cmd_push_constants(
            command,
            pipeline.layout,
            vk::ShaderStageFlags::COMPUTE,
            0,
            &push,
        );
        device.cmd_dispatch(command, 1, 1, 1);
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &barrier,
            &[],
            &[],
        );
    }
}

fn memory_type(properties: &vk::PhysicalDeviceMemoryProperties, supported: u32) -> Option<u32> {
    let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
    (0..properties.memory_type_count).find(|&index| {
        supported & (1 << index) != 0
            && properties.memory_types[index as usize]
                .property_flags
                .contains(wanted)
    })
}

fn device_name(instance: &ash::Instance, physical: vk::PhysicalDevice) -> String {
    // SAFETY: properties of an enumerated physical device.
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    String::from_utf8_lossy(&c_name(&properties.device_name)).into_owned()
}

/// Bytes of a NUL-terminated Vulkan `char` array, without the terminator.
fn c_name(raw: &[c_char]) -> Vec<u8> {
    raw.iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect()
}

fn vk_error(error: vk::Result) -> Error {
    Error::Vulkan(error.to_string())
}
