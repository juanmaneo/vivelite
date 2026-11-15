//! Marker backend selection; portable markers require only core Vulkan.

use std::fmt;
use std::str::FromStr;

/// Available ways to write command-stream progress markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Use `VK_AMD_buffer_marker`; only select when the extension is present.
    Amd,
    /// Use a small compute dispatch to write each marker (Adreno-compatible).
    Portable,
}

/// User selection, including automatic extension-based selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRequest {
    /// Prefer AMD markers when exposed; otherwise use portable Vulkan.
    Auto,
    /// Require `VK_AMD_buffer_marker`.
    Amd,
    /// Force the portable compute-marker implementation.
    Portable,
}

/// Invalid backend string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseBackendError(pub String);

impl fmt::Display for ParseBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown marker backend {:?}; expected auto, amd, or portable",
            self.0
        )
    }
}

impl std::error::Error for ParseBackendError {}

impl FromStr for BackendRequest {
    type Err = ParseBackendError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "amd" => Ok(Self::Amd),
            "portable" => Ok(Self::Portable),
            other => Err(ParseBackendError(other.to_string())),
        }
    }
}

/// Resolves a user request against the device's advertised AMD extension.
pub fn select(request: BackendRequest, amd_extension: bool) -> Result<Backend, String> {
    match request {
        BackendRequest::Auto if amd_extension => Ok(Backend::Amd),
        BackendRequest::Auto => Ok(Backend::Portable),
        BackendRequest::Amd if amd_extension => Ok(Backend::Amd),
        BackendRequest::Amd => Err("VK_AMD_buffer_marker is not available".into()),
        BackendRequest::Portable => Ok(Backend::Portable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_backend_name_and_rejects_unknown() {
        assert_eq!("auto".parse(), Ok(BackendRequest::Auto));
        assert_eq!("amd".parse(), Ok(BackendRequest::Amd));
        assert_eq!("portable".parse(), Ok(BackendRequest::Portable));
        assert_eq!(
            "AMD".parse::<BackendRequest>(),
            Err(ParseBackendError("AMD".into()))
        );
        assert!("bogus"
            .parse::<BackendRequest>()
            .unwrap_err()
            .to_string()
            .contains("bogus"));
    }

    #[test]
    fn selection_resolves_supported_and_unsupported_devices() {
        assert_eq!(select(BackendRequest::Auto, true), Ok(Backend::Amd));
        assert_eq!(select(BackendRequest::Auto, false), Ok(Backend::Portable));
        assert_eq!(select(BackendRequest::Amd, true), Ok(Backend::Amd));
        assert!(select(BackendRequest::Amd, false)
            .unwrap_err()
            .contains("not available"));
        assert_eq!(
            select(BackendRequest::Portable, true),
            Ok(Backend::Portable)
        );
        assert_eq!(
            select(BackendRequest::Portable, false),
            Ok(Backend::Portable)
        );
    }

    #[test]
    fn backend_parse_error_is_standard_error() {
        let error: &dyn std::error::Error = &ParseBackendError("bad".into());
        assert!(error.to_string().contains("bad"));
    }
}
