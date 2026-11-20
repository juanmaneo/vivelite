//! The HCF case model: a shader, a buffer initialization policy, and a
//! command-buffer recipe, as data.
//!
//! Keeping a case as data (instead of a function full of Vulkan calls) means
//! the registry, the shaders, and the buffer contents are testable on a
//! machine with no GPU at all; only [`crate::vulkan`] interprets the data on
//! a device.

/// How many `f32`/`u32` entries each input/output buffer holds. Upstream HCF
/// uses 256 entries per buffer ([`ENTRIES`] * 4 bytes = 1024 bytes each).
pub const ENTRIES: usize = 256;

/// How the input/output buffer pair is initialized before a case runs,
/// mirroring upstream's `BufferInitialization`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferInit {
    /// Input holds `f32(2 + i * 2)` (2, 4, 6, …), output is zeroed.
    /// Upstream: `BufferInitialization::Default`.
    Default,
    /// Input holds `65535u32` in every slot — the loop bound that makes
    /// `hang_infinite_loop` a not-quite-infinite loop (65535³ iterations).
    /// Output is zeroed. Upstream: `BufferInitialization::_64K`.
    K64,
}

/// The command-buffer recipe a case records and submits, mirroring the
/// bodies of upstream's `hang_infinite_loop.cc` / `hang_host_event.cc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipe {
    /// One dispatch of the long-running compute shader, submitted to the
    /// queue; the shader's runtime is the hang. Upstream then expects the
    /// driver to time out, reset, or wedge.
    InfiniteLoop,
    /// Dispatch, then `vkCmdWaitEvents` on a host event that is never
    /// signaled (src `HOST` → dst `TOP_OF_PIPE`), then dispatch again;
    /// submit, wait idle, submit a second plain-dispatch buffer, wait idle.
    HostEvent,
    /// Submit the dispatch repeatedly, each time waiting on a binary
    /// semaphore that is never signaled, matching upstream `hang_semaphore`.
    BinarySemaphoreWait,
    /// Wait on a timeline semaphore in the queue, host-signal that semaphore,
    /// then block the host forever on a second timeline semaphore (Vulkan 1.1
    /// with `VK_KHR_timeline_semaphore` or Vulkan 1.2+).
    HostTimelineSemaphore,
}

/// One HCF hang case, as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Case {
    /// Command-line name, e.g. `hang_infinite_loop`.
    pub name: &'static str,
    /// One-line human summary, printed by the runner.
    pub summary: &'static str,
    /// GLSL (`#version 450`) compute source, translated at runtime.
    pub shader: &'static str,
    /// How the SSBO pair is initialized before the run.
    pub init: BufferInit,
    /// What the case records and submits.
    pub recipe: Recipe,
}

/// `hang_infinite_loop`: a compute shader with a long running time (a
/// not-quite infinite loop), ported from upstream `infinite_loop.comp`.
///
/// The triple-nested loop runs `cnt³` iterations where `cnt` is read from
/// the input buffer; with the 64K initialization that is 65535³ ≈ 2.8e14
/// iterations, far beyond any driver's hang threshold.
pub const HANG_INFINITE_LOOP: Case = Case {
    name: "hang_infinite_loop",
    summary: "compute shader with a not-quite-infinite loop (upstream infinite_loop.comp)",
    init: BufferInit::K64,
    recipe: Recipe::InfiniteLoop,
    shader: "\
#version 450
layout(local_size_x = 4, local_size_y = 1, local_size_z = 1) in;

layout(std430, binding = 0) buffer buf_in { uint in_data[]; };
layout(std430, binding = 1) buffer buf_out { float out_data[]; };

void main() {
    uint idx = gl_GlobalInvocationID.x;
    uint cnt = in_data[0];

    float sum = 0.0;
    for (uint i = 0; i < cnt; ++i) {
        for (uint j = 0; j < cnt; ++j) {
            for (uint k = 0; k < cnt; ++k) {
                // uint sum first, then to float: identical to upstream's
                // implicit conversion in `sum += i + j + k`.
                sum += float(i + j + k);
            }
        }
    }

    out_data[idx] = sum;
}
",
};

/// `hang_host_event`: work that waits on a host-signaled event that is
/// never signaled, ported from upstream `hang_host_event.cc` +
/// `read_write.comp`.
pub const HANG_HOST_EVENT: Case = Case {
    name: "hang_host_event",
    summary:
        "submitted work waits on an event that is never signaled (upstream hang_host_event.cc)",
    init: BufferInit::Default,
    recipe: Recipe::HostEvent,
    shader: "\
#version 450
layout(local_size_x = 4, local_size_y = 1, local_size_z = 1) in;

layout(std430, binding = 0) buffer buf_in { float in_data[]; };
layout(std430, binding = 1) buffer buf_out { float out_data[]; };

void main() {
    uint idx = gl_GlobalInvocationID.x;
    out_data[idx] = in_data[idx] * 4.0;
}
",
};

/// `hang_semaphore`: queue submissions wait on a binary semaphore that is
/// never signaled, ported from upstream `hang_semaphore.cc`.
pub const HANG_SEMAPHORE: Case = Case {
    name: "hang_semaphore",
    summary: "queue repeatedly waits on a never-signaled binary semaphore",
    shader: HANG_HOST_EVENT.shader,
    init: BufferInit::Default,
    recipe: Recipe::BinarySemaphoreWait,
};

/// `hang_timeline_semaphore_host`: queue waits on a host-signaled timeline
/// semaphore, then the host waits forever on an un-signaled timeline
/// semaphore, ported from upstream `hang_timeline_semaphore_host.cc`.
pub const HANG_TIMELINE_SEMAPHORE_HOST: Case = Case {
    name: "hang_timeline_semaphore_host",
    summary: "host blocks forever waiting on an unsignaled timeline semaphore",
    shader: HANG_HOST_EVENT.shader,
    init: BufferInit::Default,
    recipe: Recipe::HostTimelineSemaphore,
};

/// Every ported case, in a stable order.
pub const CASES: &[Case] = &[
    HANG_INFINITE_LOOP,
    HANG_HOST_EVENT,
    HANG_SEMAPHORE,
    HANG_TIMELINE_SEMAPHORE_HOST,
];

/// All ported cases.
pub fn cases() -> &'static [Case] {
    CASES
}

/// Looks a case up by command-line name.
pub fn case(name: &str) -> Option<&'static Case> {
    CASES.iter().find(|c| c.name == name)
}

/// The `(input, output)` initial buffer contents for one policy, as words.
///
/// Upstream writes the input words at offset 0 of one mapped allocation and
/// zeroes the output words at offset `ENTRIES * 4`; the shape here matches.
pub fn buffer_init_words(init: BufferInit) -> (Vec<u32>, Vec<u32>) {
    let mut input = vec![0u32; ENTRIES];
    match init {
        BufferInit::Default => {
            for (i, word) in input.iter_mut().enumerate() {
                // Upstream: `*pBufferData++ = 2 + i * 2.0f`.
                *word = (2.0 + i as f32 * 2.0).to_bits();
            }
        }
        BufferInit::K64 => {
            // Upstream: `*pBufferData++ = 65535` for every entry.
            input.fill(65535);
        }
    }
    (input, vec![0u32; ENTRIES])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_the_ported_cases_in_order() {
        assert_eq!(
            cases().iter().map(|c| c.name).collect::<Vec<_>>(),
            vec![
                "hang_infinite_loop",
                "hang_host_event",
                "hang_semaphore",
                "hang_timeline_semaphore_host",
            ]
        );
    }

    #[test]
    fn lookup_finds_known_cases_and_rejects_unknown_ones() {
        assert_eq!(
            case("hang_host_event").map(|c| c.name),
            Some("hang_host_event")
        );
        assert!(case("hang_infinite_loop").is_some());
        assert!(case("hang_semaphore").is_some());
        assert!(case("hang_timeline_semaphore_host").is_some());
        assert_eq!(case("crash_shader"), None);
    }

    #[test]
    fn default_init_matches_upstream_contents() {
        let (input, output) = buffer_init_words(BufferInit::Default);
        assert_eq!(input.len(), ENTRIES);
        assert_eq!(output, vec![0u32; ENTRIES]);
        assert_eq!(f32::from_bits(input[0]), 2.0);
        assert_eq!(f32::from_bits(input[1]), 4.0);
        assert_eq!(f32::from_bits(input[255]), 512.0);
    }

    #[test]
    fn k64_init_sets_the_loop_bound_in_every_word() {
        let (input, output) = buffer_init_words(BufferInit::K64);
        assert_eq!(input, vec![65535u32; ENTRIES]);
        assert_eq!(output, vec![0u32; ENTRIES]);
    }

    #[test]
    fn cases_pin_the_upstream_policies_and_recipes() {
        assert_eq!(case("hang_infinite_loop").unwrap().init, BufferInit::K64);
        assert_eq!(
            case("hang_infinite_loop").unwrap().recipe,
            Recipe::InfiniteLoop
        );
        assert_eq!(case("hang_host_event").unwrap().init, BufferInit::Default);
        assert_eq!(case("hang_host_event").unwrap().recipe, Recipe::HostEvent);
        assert_eq!(
            case("hang_semaphore").unwrap().recipe,
            Recipe::BinarySemaphoreWait
        );
        assert_eq!(
            case("hang_timeline_semaphore_host").unwrap().recipe,
            Recipe::HostTimelineSemaphore
        );
    }
}
