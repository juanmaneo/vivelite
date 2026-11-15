//! `VkFormat` ↔ wgpu texture format, as data.
//!
//! The runtime's `xrEnumerateSwapchainFormats` is authoritative: the overlay
//! picks the first supported format it can also render through wgpu. The
//! mapping below is the overlay's whole negotiation surface — the runtime
//! offers dozens of formats; the overlay needs one 8-bit-per-channel
//! sRGB-or-linear one that iced can clear and render into.

use wgpu::TextureFormat;

/// `VK_FORMAT_R8G8B8A8_UNORM`, per the Vulkan `VkFormat` enum.
pub const RGBA8_UNORM: i64 = 37;
/// `VK_FORMAT_R8G8B8A8_SRGB`.
pub const RGBA8_SRGB: i64 = 43;
/// `VK_FORMAT_B8G8R8A8_UNORM`.
pub const BGRA8_UNORM: i64 = 44;
/// `VK_FORMAT_B8G8R8A8_SRGB`.
pub const BGRA8_SRGB: i64 = 50;

/// The overlay's format preferences, best first: sRGB variants (the iced
/// program emits sRGB-encoded color; the compositor reads the swapchain as
/// the format says), RGBA before BGRA.
pub const PREFERRED: [i64; 4] = [RGBA8_SRGB, BGRA8_SRGB, RGBA8_UNORM, BGRA8_UNORM];

/// The wgpu texture format for a `VkFormat` the overlay can render, or
/// `None` for everything else.
///
/// The returned format is what the `VkImage` must be wrapped with on the
/// wgpu side (`wgpu::TextureDescriptor::format`); the swapchain is created
/// with the `VkFormat` value itself.
#[must_use]
pub fn texture_format(vk_format: i64) -> Option<TextureFormat> {
    match vk_format {
        RGBA8_UNORM => Some(TextureFormat::Rgba8Unorm),
        RGBA8_SRGB => Some(TextureFormat::Rgba8UnormSrgb),
        BGRA8_UNORM => Some(TextureFormat::Bgra8Unorm),
        BGRA8_SRGB => Some(TextureFormat::Bgra8UnormSrgb),
        _ => None,
    }
}

/// The best `VkFormat` for the overlay among the session's supported
/// formats, or `None` when the runtime offers nothing the overlay can
/// render.
#[must_use]
pub fn pick_format(supported: &[i64]) -> Option<i64> {
    PREFERRED
        .iter()
        .copied()
        .find(|preferred| supported.contains(preferred))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_supported_vk_format() {
        assert_eq!(texture_format(RGBA8_UNORM), Some(TextureFormat::Rgba8Unorm));
        assert_eq!(
            texture_format(RGBA8_SRGB),
            Some(TextureFormat::Rgba8UnormSrgb)
        );
        assert_eq!(texture_format(BGRA8_UNORM), Some(TextureFormat::Bgra8Unorm));
        assert_eq!(
            texture_format(BGRA8_SRGB),
            Some(TextureFormat::Bgra8UnormSrgb)
        );
    }

    #[test]
    fn rejects_unsupported_vk_formats() {
        assert_eq!(texture_format(0), None); // VK_FORMAT_UNDEFINED
        assert_eq!(texture_format(4), None); // VK_FORMAT_R4G4_UNORM_PACK8
        assert_eq!(texture_format(38), None); // R8G8B8A8_SNORM
        assert_eq!(texture_format(-1), None);
    }

    #[test]
    fn pick_prefers_srgb_then_rejects() {
        // sRGB wins over an unorm format that also appears.
        let all = [BGRA8_SRGB, RGBA8_UNORM, 37 + 1, BGRA8_UNORM, RGBA8_SRGB];
        assert_eq!(pick_format(&all), Some(RGBA8_SRGB));

        // No sRGB: the first unorm preference present is used.
        let unorm = [38, BGRA8_UNORM, 4];
        assert_eq!(pick_format(&unorm), Some(BGRA8_UNORM));

        // Nothing renderable: refuse rather than guess.
        assert_eq!(pick_format(&[0, 38, 64]), None);
        assert_eq!(pick_format(&[]), None);
    }
}
