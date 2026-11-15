//! Device execution of the HCF cases through `ash`.
//!
//! `ash` is this crate's wrapped C API (the project's `-sys` layer), so the
//! `unsafe` surface lives here and is documented per block. The module is
//! compiled only with the `vulkan` feature: the default workspace build, its
//! tests, and the coverage gate never link a Vulkan loader; the code runs
//! for real through `cargo xtask gpu-stress <case>`, deliberately outside
//! the coverage gate (see the README ideas list).
//!
//! Object lifetimes follow upstream HCF: every case object is a child of
//! the `VkDevice` and is reclaimed by `vkDestroyDevice` in [`Gpu::drop`];
//! like upstream, the never-signaled event is never explicitly destroyed.

#![allow(unsafe_code)]
// The unsafe blocks below all carry SAFETY comments; the allow only
// silences the workspace's `unsafe_code = warn` lint, matching the `-sys`
// crates' stance.

use std::fmt;
use std::time::Duration;

use ash::vk;
use ash::Entry;

use crate::case::{buffer_init_words, Case, Recipe, ENTRIES};
use crate::spirv::translate;

/// How long the crash check waits on its fence before declaring a hang.
/// Upstream: "30s should be enough waiting to detect hang/crash."
const FENCE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bytes per input/output buffer: 256 entries, 4 bytes each (upstream HCF).
const BUFFER_BYTES: vk::DeviceSize = ENTRIES as vk::DeviceSize * 4;

/// What the GPU (or its driver) did with the workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every submission completed and the device stayed alive.
    Completed,
    /// The crash-check fence did not signal in time: the workload is still
    /// wedged in the queue (the watchdog terminates the process next).
    HangDetected,
    /// The driver reported `VK_ERROR_DEVICE_LOST`.
    DeviceLost,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Completed => f.write_str("completed"),
            Outcome::HangDetected => f.write_str("hang detected (fence timed out)"),
            Outcome::DeviceLost => f.write_str("device lost (crash detected by the driver)"),
        }
    }
}

/// Why a case could not run at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No Vulkan loader library could be loaded.
    Loader(String),
    /// No physical device with a compute queue family was found.
    NoDevice,
    /// No `HOST_VISIBLE | HOST_COHERENT` memory type backs the buffers.
    NoMemoryType,
    /// The case's GLSL could not be translated to SPIR-V.
    Shader(String),
    /// The driver returned an unexpected error.
    Vulkan(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Loader(e) => write!(f, "no Vulkan loader: {e}"),
            Error::NoDevice => f.write_str("no device with a compute queue family"),
            Error::NoMemoryType => {
                f.write_str("no HOST_VISIBLE|HOST_COHERENT memory type for the buffers")
            }
            Error::Shader(e) => write!(f, "shader translation failed: {e}"),
            Error::Vulkan(e) => write!(f, "Vulkan error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The live Vulkan objects one case runs against.
struct Gpu {
    /// Kept alive: `ash` loads function pointers from the loader this owns.
    _entry: Entry,
    instance: ash::Instance,
    device: ash::Device,
    /// The compute queue family the case submits to.
    family: u32,
    queue: vk::Queue,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
}

impl Gpu {
    /// Creates an instance and picks the first physical device exposing a
    /// compute queue family — upstream's default single-device setup.
    fn new() -> Result<Self, Error> {
        // SAFETY: `Entry::load` dlopen's the system Vulkan loader, the
        // documented contract of the `loaded` feature; the `Entry` is owned
        // by this struct, so every function pointer it hands out stays
        // valid for the process lifetime.
        let entry = unsafe { Entry::load() }.map_err(|e| Error::Loader(e.to_string()))?;

        let name = c"vivelite hcf";
        let app_info = vk::ApplicationInfo::default()
            .application_name(name)
            .engine_name(name)
            .api_version(vk::API_VERSION_1_0);
        let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
        // SAFETY: no layers or extensions are requested, and the instance
        // is destroyed in `drop` while the owning entry is still alive.
        let instance = unsafe { entry.create_instance(&create_info, None) }
            .map_err(|e| Error::Vulkan(e.to_string()))?;

        // SAFETY: standard enumeration on a live instance.
        let physicals = unsafe { instance.enumerate_physical_devices() }
            .map_err(|e| Error::Vulkan(e.to_string()))?;

        let mut chosen = None;
        for physical in physicals {
            // SAFETY: querying properties of a valid physical device.
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(physical) };
            if let Some(family) = families
                .iter()
                .position(|f| f.queue_flags.contains(vk::QueueFlags::COMPUTE))
            {
                chosen = Some((physical, family as u32));
                break;
            }
        }
        let (physical_device, family) = chosen.ok_or(Error::NoDevice)?;

        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&[1.0]);
        let queue_infos = [queue_info];
        let device_info = vk::DeviceCreateInfo::default().queue_create_infos(&queue_infos);
        // SAFETY: the queue family index comes from the device's own
        // property list and the device is destroyed in `drop`.
        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|e| Error::Vulkan(e.to_string()))?;
        // SAFETY: queue index 0 of a family with at least one queue.
        let queue = unsafe { device.get_device_queue(family, 0) };
        // SAFETY: querying properties of a valid physical device.
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        eprintln!(
            "hcf: running on {} (compute queue family {family})",
            device_name(&instance, physical_device)
        );
        Ok(Self {
            _entry: entry,
            instance,
            device,
            family,
            queue,
            memory_properties,
        })
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: the device and instance are not used after this, and
        // destroying the device reclaims every case object (buffers,
        // descriptor pool, pipeline, command pool, event) exactly like
        // upstream's cleanup.
        unsafe {
            self.device.destroy_device(None);
            // SAFETY: destroyed after the device, before the entry drops.
            self.instance.destroy_instance(None);
        }
    }
}

/// The first `HOST_VISIBLE | HOST_COHERENT` memory type usable with
/// `supported` (a `memoryTypeBits` mask).
fn memory_type(props: &vk::PhysicalDeviceMemoryProperties, supported: u32) -> Option<u32> {
    let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
    (0..props.memory_type_count).find(|&i| {
        supported & (1 << i) != 0
            && props.memory_types[i as usize]
                .property_flags
                .contains(wanted)
    })
}

/// The physical device's name, for progress output.
fn device_name(instance: &ash::Instance, physical: vk::PhysicalDevice) -> String {
    // SAFETY: querying properties of a valid physical device.
    let props = unsafe { instance.get_physical_device_properties(physical) };
    let bytes = &props.device_name;
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end].iter().map(|&c| c as u8).collect::<Vec<_>>()).into_owned()
}

/// One case's SSBO pair, carved out of a single allocation exactly like
/// upstream: `bufferIn` bound at offset 0, `bufferOut` at offset
/// `BUFFER_BYTES`, one `HOST_VISIBLE|HOST_COHERENT` mapping for both.
///
/// The device owns the allocation; no handle needs to outlive setup.
struct Buffers {
    input: vk::Buffer,
    output: vk::Buffer,
}

/// Creates and initializes the SSBO pair for `case`.
fn create_buffers(gpu: &Gpu, case: &Case) -> Result<Buffers, Error> {
    let info = vk::BufferCreateInfo::default()
        .size(BUFFER_BYTES)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    // SAFETY: plain object creation on a live device.
    let input = unsafe { gpu.device.create_buffer(&info, None) }.map_err(vk_err)?;
    // SAFETY: plain object creation on a live device.
    let output = unsafe { gpu.device.create_buffer(&info, None) }.map_err(vk_err)?;

    // SAFETY: `input` is a valid buffer of this device.
    let requirements = unsafe { gpu.device.get_buffer_memory_requirements(input) };
    let ty = memory_type(&gpu.memory_properties, requirements.memory_type_bits)
        .ok_or(Error::NoMemoryType)?;
    let memory_info = vk::MemoryAllocateInfo::default()
        .allocation_size(BUFFER_BYTES * 2)
        .memory_type_index(ty);
    // SAFETY: one small allocation for the whole case, sized like upstream
    // (2 * bufferSize).
    let memory = unsafe { gpu.device.allocate_memory(&memory_info, None) }.map_err(vk_err)?;

    // SAFETY: offsets 0 and BUFFER_BYTES are within the allocation and
    // satisfy the buffer alignment requirements (both buffers have the
    // same size and usage).
    unsafe {
        gpu.device
            .bind_buffer_memory(input, memory, 0)
            .map_err(vk_err)?;
        gpu.device
            .bind_buffer_memory(output, memory, BUFFER_BYTES)
            .map_err(vk_err)?;
    }

    let (in_words, out_words) = buffer_init_words(case.init);
    // SAFETY: the memory type is host-visible and coherent, the mapping
    // covers both buffers (allocation is `BUFFER_BYTES * 2`), and the word
    // slices match the buffer sizes, so the copies stay in bounds.
    unsafe {
        let ptr = gpu
            .device
            .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            .map_err(vk_err)?;
        std::ptr::copy_nonoverlapping(
            in_words.as_ptr().cast::<u8>(),
            ptr.cast::<u8>(),
            in_words.len() * 4,
        );
        std::ptr::copy_nonoverlapping(
            out_words.as_ptr().cast::<u8>(),
            ptr.cast::<u8>().add(BUFFER_BYTES as usize),
            out_words.len() * 4,
        );
        gpu.device.unmap_memory(memory);
    }

    Ok(Buffers { input, output })
}

/// The descriptor set, pipeline layout, shader module, and compute
/// pipeline for one case. All children of the device; reclaimed by
/// `vkDestroyDevice`.
struct Program {
    set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}

/// Builds the SSBO descriptor set and the compute pipeline for `case`.
fn create_program(gpu: &Gpu, buffers: &Buffers, case: &Case) -> Result<Program, Error> {
    let bindings = [
        vk::DescriptorSetLayoutBinding {
            binding: 0,
            descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: vk::ShaderStageFlags::COMPUTE,
            ..Default::default()
        },
        vk::DescriptorSetLayoutBinding {
            binding: 1,
            descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: vk::ShaderStageFlags::COMPUTE,
            ..Default::default()
        },
    ];
    let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    // SAFETY: plain object creation on a live device.
    let layout =
        unsafe { gpu.device.create_descriptor_set_layout(&layout_info, None) }.map_err(vk_err)?;

    let pool_sizes = [vk::DescriptorPoolSize {
        ty: vk::DescriptorType::STORAGE_BUFFER,
        descriptor_count: 2,
    }];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(&pool_sizes)
        .max_sets(1);
    // SAFETY: plain object creation on a live device.
    let pool = unsafe { gpu.device.create_descriptor_pool(&pool_info, None) }.map_err(vk_err)?;

    let sets_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(std::slice::from_ref(&layout));
    // SAFETY: the layout belongs to the same device and the pool has room
    // for one set.
    let set = *unsafe { gpu.device.allocate_descriptor_sets(&sets_info) }
        .map_err(vk_err)?
        .first()
        .expect("one descriptor set was requested");

    let in_info = vk::DescriptorBufferInfo {
        buffer: buffers.input,
        offset: 0,
        range: vk::WHOLE_SIZE,
    };
    let out_info = vk::DescriptorBufferInfo {
        buffer: buffers.output,
        offset: 0,
        range: vk::WHOLE_SIZE,
    };
    let writes = [
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(std::slice::from_ref(&in_info)),
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(std::slice::from_ref(&out_info)),
    ];
    // SAFETY: the writes reference buffers bound to live memory of the
    // same device.
    unsafe { gpu.device.update_descriptor_sets(&writes, &[]) };

    let pipeline_layout_info =
        vk::PipelineLayoutCreateInfo::default().set_layouts(std::slice::from_ref(&layout));
    // SAFETY: plain object creation on a live device.
    let pipeline_layout = unsafe {
        gpu.device
            .create_pipeline_layout(&pipeline_layout_info, None)
    }
    .map_err(vk_err)?;

    let words = translate(case).map_err(|e| Error::Shader(e.to_string()))?;
    let shader_info = vk::ShaderModuleCreateInfo::default().code(&words);
    // SAFETY: the SPIR-V comes from naga's validated output for this case.
    let module = unsafe { gpu.device.create_shader_module(&shader_info, None) }.map_err(vk_err)?;

    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(c"main");
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    // SAFETY: one compute pipeline from a live module and layout; a null
    // cache is the documented no-cache case.
    let pipeline = unsafe {
        gpu.device.create_compute_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&pipeline_info),
            None,
        )
    }
    .map_err(|(_, e)| vk_err(e))?
    .pop()
    .expect("one pipeline was requested");

    Ok(Program {
        set,
        pipeline_layout,
        pipeline,
    })
}

/// The command pool and the case's command buffers.
struct Commands {
    buffers: Vec<vk::CommandBuffer>,
}

/// Records the case's command buffers. The last buffer is always the empty
/// crash-check buffer; upstream allocates it separately but records the
/// same thing.
fn create_commands(
    gpu: &Gpu,
    case: &Case,
    program: &Program,
    event: Option<vk::Event>,
) -> Result<Commands, Error> {
    let pool_info = vk::CommandPoolCreateInfo::default().queue_family_index(gpu.family);
    // SAFETY: plain object creation on a live device.
    let pool = unsafe { gpu.device.create_command_pool(&pool_info, None) }.map_err(vk_err)?;

    let count = match case.recipe {
        Recipe::InfiniteLoop => 2,
        Recipe::HostEvent => 3,
    };
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(count);
    // SAFETY: the pool was created on this device for this queue family.
    let buffers = unsafe { gpu.device.allocate_command_buffers(&alloc_info) }.map_err(vk_err)?;

    let begin =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    for (i, &cb) in buffers[..buffers.len() - 1].iter().enumerate() {
        // SAFETY: freshly allocated, not pending, recorded in order.
        unsafe {
            gpu.device
                .begin_command_buffer(cb, &begin)
                .map_err(vk_err)?;
            gpu.device
                .cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, program.pipeline);
            gpu.device.cmd_bind_descriptor_sets(
                cb,
                vk::PipelineBindPoint::COMPUTE,
                program.pipeline_layout,
                0,
                std::slice::from_ref(&program.set),
                &[],
            );
            gpu.device.cmd_dispatch(cb, 1, 1, 1);
            // Only the first recipe buffer carries the case's own hang: a
            // dispatch, then a wait on a never-signaled event, then another
            // dispatch to see if commands execute after the wait. Later
            // buffers are the plain "is anything still executing" submits
            // (upstream's `primary_cb2`).
            if i == 0 && case.recipe == Recipe::HostEvent {
                // The event is created unsignaled and never signaled;
                // upstream also never destroys it (the device reclaims it).
                let event = event.expect("host-event recipe carries its event");
                gpu.device.cmd_wait_events(
                    cb,
                    std::slice::from_ref(&event),
                    vk::PipelineStageFlags::HOST,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    &[],
                    &[],
                    &[],
                );
                // Dispatch again to see if commands execute after the wait.
                gpu.device.cmd_dispatch(cb, 1, 1, 1);
            }
            gpu.device.end_command_buffer(cb).map_err(vk_err)?;
        }
    }

    Ok(Commands { buffers })
}

/// Submits one command buffer to the case's queue, no fence (upstream
/// passes `VK_NULL_HANDLE`).
fn submit(gpu: &Gpu, cb: vk::CommandBuffer) -> Result<(), Error> {
    let info = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cb));
    // SAFETY: the queue belongs to the device and the command buffer was
    // allocated from a pool created for that queue family.
    unsafe {
        gpu.device
            .queue_submit(gpu.queue, std::slice::from_ref(&info), vk::Fence::null())
    }
    .map_err(vk_err)
}

/// `vkQueueWaitIdle`, mapping device-lost to an outcome instead of an error.
fn wait_idle(gpu: &Gpu) -> Result<Outcome, Error> {
    // SAFETY: the queue is a valid handle of this device.
    match unsafe { gpu.device.queue_wait_idle(gpu.queue) } {
        Ok(()) => Ok(Outcome::Completed),
        Err(vk::Result::ERROR_DEVICE_LOST) => Ok(Outcome::DeviceLost),
        Err(e) => Err(vk_err(e)),
    }
}

/// Runs the case's own submissions, mirroring the flow of each upstream
/// `.cc` file.
fn run_recipe(gpu: &Gpu, case: &Case, cbs: &[vk::CommandBuffer]) -> Result<Outcome, Error> {
    match case.recipe {
        Recipe::InfiniteLoop => {
            eprintln!("hcf: submit (long-running dispatch)...");
            submit(gpu, cbs[0])?;
            // Expected to block until the driver resets the GPU, or until
            // the watchdog fires.
            wait_idle(gpu)
        }
        Recipe::HostEvent => {
            eprintln!("hcf: submit 1 (dispatch, wait on never-signaled event, dispatch)...");
            submit(gpu, cbs[0])?;
            // NOTE (upstream): this can return VK_SUCCESS while the hang is
            // in progress; the second submission is what exposes it.
            let first = wait_idle(gpu)?;
            eprintln!("hcf: submit 2 (plain dispatch)...");
            submit(gpu, cbs[1])?;
            let second = wait_idle(gpu)?;
            Ok(merge(first, second))
        }
    }
}

/// Upstream's crash check: an empty submission fenced with a 30 s wait
/// detects a wedged queue or a lost device without trusting the case's own
/// waits.
fn crash_check(gpu: &Gpu, cb: vk::CommandBuffer) -> Result<Outcome, Error> {
    let begin =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    // SAFETY: the buffer is freshly allocated and not pending.
    unsafe {
        gpu.device
            .begin_command_buffer(cb, &begin)
            .map_err(vk_err)?;
        gpu.device.end_command_buffer(cb).map_err(vk_err)?;
    }

    // SAFETY: plain object creation on a live device.
    let fence = unsafe {
        gpu.device
            .create_fence(&vk::FenceCreateInfo::default(), None)
    }
    .map_err(vk_err)?;

    eprintln!("hcf: crash check (empty submit, fenced 30s wait)...");
    // SAFETY: the fence was just created unsignaled on this device.
    let waited = unsafe {
        let info = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cb));
        gpu.device
            .queue_submit(gpu.queue, std::slice::from_ref(&info), fence)
            .map_err(vk_err)?;
        gpu.device.wait_for_fences(
            std::slice::from_ref(&fence),
            true,
            FENCE_TIMEOUT.as_nanos() as u64,
        )
    };
    match waited {
        Ok(()) => wait_idle(gpu),
        Err(vk::Result::TIMEOUT) => Ok(Outcome::HangDetected),
        Err(vk::Result::ERROR_DEVICE_LOST) => Ok(Outcome::DeviceLost),
        Err(e) => Err(vk_err(e)),
    }
}

/// The worse of two outcomes: `DeviceLost` > `HangDetected` > `Completed`.
fn merge(a: Outcome, b: Outcome) -> Outcome {
    use Outcome::*;
    match (a, b) {
        (DeviceLost, _) | (_, DeviceLost) => DeviceLost,
        (HangDetected, _) | (_, HangDetected) => HangDetected,
        (Completed, Completed) => Completed,
    }
}

/// Executes one case on the first compute-capable device.
///
/// This is the whole workload: buffers and descriptors are created like
/// upstream, the recipe's submissions run, and the crash check has the
/// final say. The caller is expected to have armed a [`crate::Watchdog`]:
/// a true hang never returns from here.
///
/// # Errors
///
/// Any setup failure (no loader, no device, no memory type, translation
/// failure, driver error). Hangs and device loss are outcomes, not errors.
pub fn execute(case: &Case) -> Result<Outcome, Error> {
    let gpu = Gpu::new()?;
    let buffers = create_buffers(&gpu, case)?;
    let program = create_program(&gpu, &buffers, case)?;

    let event = match case.recipe {
        Recipe::HostEvent => Some(
            // SAFETY: plain object creation on a live device; unsignaled by
            // default.
            unsafe {
                gpu.device
                    .create_event(&vk::EventCreateInfo::default(), None)
            }
            .map_err(vk_err)?,
        ),
        Recipe::InfiniteLoop => None,
    };

    let commands = create_commands(&gpu, case, &program, event)?;
    let (recipe_cbs, check_cb) = commands.buffers.split_at(commands.buffers.len() - 1);

    let recipe = run_recipe(&gpu, case, recipe_cbs)?;
    let check = crash_check(&gpu, check_cb[0])?;
    Ok(merge(recipe, check))
}

/// Maps a Vulkan result to an [`Error`].
fn vk_err(e: vk::Result) -> Error {
    Error::Vulkan(e.to_string())
}
