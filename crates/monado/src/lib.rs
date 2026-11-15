//! Safe wrapper around the OpenXR loader for runtime probing.
//!
//! vivelite's stance on tracking: the headset's vendor runtime (HTC VIVE
//! OpenXR) owns display and controller tracking — the safe layer for that
//! is the patched WinlatorXR APK. Monado is the runtime vivelite talks to
//! from inside the Linux side (proot) for SteamVR-class compositing
//! experiments. This crate therefore does three things: detect which
//! runtime a socket/loader will hand us, pack versions, and map results
//! to errors worth logging.

use std::path::PathBuf;

use monado_sys::{
    XrInstance, XrResult, XrVersion, XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY, XR_TYPE_INSTANCE_CREATE_INFO,
    XR_TYPE_SYSTEM_GET_INFO,
};

/// `XR_MAKE_VERSION(major, minor, patch)` per the Khronos header:
/// major at bit 55, minor at bit 38, patch in the low 38 bits.
/// Verify against `XR_MAKE_VERSION`/`XR_VERSION_MAJOR` in openxr.h when the
/// vendored SDK moves.
pub const fn version_parts(v: XrVersion) -> (u16, u16, u32) {
    let major = (v >> 55) as u16;
    let minor = ((v >> 38) & 0x1F_FFFF) as u16;
    let patch = (v & 0x3F_FFFF_FFFF) as u32;
    (major, minor, patch)
}

/// `XR_MAKE_VERSION`.
pub const fn make_version(major: u16, minor: u16, patch: u32) -> XrVersion {
    ((major as u64) << 55) | ((minor as u64) << 38) | (patch as u64)
}

/// Where a Monado-style runtime can be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEndpoint {
    /// IPC socket path, e.g. `/tmp/xr-runtime` (Monado's service socket).
    Socket(PathBuf),
    /// OpenXR loader library path, e.g. `libopenxr_loader.so` via ld.
    Loader(String),
}

/// Detect the endpoint the environment will hand to an OpenXR client.
///
/// Precedence: explicit `VIVELITE_XR_RUNTIME` (socket path or .so path),
/// then `XR_RUNTIME_JSON` pointing at an active runtime json, then the
/// Monado default socket.
pub fn detect_endpoint() -> RuntimeEndpoint {
    if let Ok(v) = std::env::var("VIVELITE_XR_RUNTIME") {
        if v.ends_with(".json") {
            return RuntimeEndpoint::Loader(v);
        }
        if v.contains(".so") {
            return RuntimeEndpoint::Loader(v);
        }
        return RuntimeEndpoint::Socket(PathBuf::from(v));
    }
    if std::env::var("XR_RUNTIME_JSON").is_ok() {
        return RuntimeEndpoint::Loader("libopenxr_loader.so".to_string());
    }
    RuntimeEndpoint::Socket(PathBuf::from("/tmp/xr-runtime"))
}

/// A live OpenXR instance, destroyed on drop.
#[derive(Debug)]
pub struct Instance {
    raw: XrInstance,
}

// Safety: an XrInstance is thread-safe per the OpenXR spec (calls may run
// from any thread; the loader serializes internally).
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Instance {
    /// Create an instance named `app_name` against the active runtime.
    ///
    /// # Safety
    ///
    /// Requires the OpenXR loader to be loadable in this process
    /// (`libopenxr_loader.so` on the library path) and a running runtime.
    #[cfg(feature = "link")]
    pub unsafe fn create(app_name: &str) -> Result<Instance, XrError> {
        let mut info = std::mem::zeroed::<monado_sys::XrInstanceCreateInfo>();
        info.type_ = XR_TYPE_INSTANCE_CREATE_INFO;
        info.application_info.application_version = 1;
        info.application_info.api_version = make_version(1, 0, 0);
        write_padded(&mut info.application_info.application_name, app_name);
        write_padded(&mut info.application_info.engine_name, "vivelite");
        let mut raw = 0;
        let r = monado_sys::xrCreateInstance(&info, &mut raw);
        if r < 0 {
            return Err(XrError::from_result(r));
        }
        Ok(Instance { raw })
    }

    /// Ask for a HMD system id.
    #[cfg(feature = "link")]
    pub fn system(&self) -> Result<u64, XrError> {
        let mut get = std::mem::zeroed::<monado_sys::XrSystemGetInfo>();
        get.type_ = XR_TYPE_SYSTEM_GET_INFO;
        get.form_factor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
        let mut sys = 0;
        let r = unsafe { monado_sys::xrGetSystem(self.raw, &get, &mut sys) };
        if r < 0 {
            return Err(XrError::from_result(r));
        }
        Ok(sys)
    }

    /// Raw handle, for the raw call sites the safe layer has not wrapped.
    pub fn raw(&self) -> XrInstance {
        self.raw
    }
}

#[cfg(feature = "link")]
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            monado_sys::xrDestroyInstance(self.raw);
        }
    }
}

/// Errors surfaced by the safe layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XrError {
    /// `XR_ERROR_RUNTIME_UNAVAILABLE` (value -6): no usable runtime.
    RuntimeUnavailable,
    /// `XR_ERROR_FORM_FACTOR_UNAVAILABLE` (-7): no HMD system right now.
    FormFactorUnavailable,
    /// `XR_ERROR_INSTANCE_LOST` (-13): the runtime went away.
    InstanceLost,
    /// Any other negative result.
    Other(i32),
}

impl XrError {
    /// Map a raw `XrResult` (documented values from openxr.h).
    pub fn from_result(r: XrResult) -> XrError {
        match r {
            -6 => XrError::RuntimeUnavailable,
            -7 => XrError::FormFactorUnavailable,
            -13 => XrError::InstanceLost,
            other => XrError::Other(other),
        }
    }

    /// Whether retrying after a fresh `detect_endpoint` could help.
    pub fn is_retryable(&self) -> bool {
        matches!(self, XrError::RuntimeUnavailable | XrError::InstanceLost)
    }
}

fn write_padded(dst: &mut [std::os::raw::c_char], s: &str) {
    for (i, b) in s.bytes().take(dst.len() - 1).enumerate() {
        dst[i] = b as std::os::raw::c_char;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_roundtrip() {
        let v = make_version(1, 0, 0);
        assert_eq!(version_parts(v), (1, 0, 0));
        let v = make_version(1, 1, 32);
        assert_eq!(version_parts(v), (1, 1, 32));
    }

    #[test]
    fn error_mapping_uses_documented_values() {
        // Values from the common XrResult enum in openxr.h.
        assert_eq!(XrError::from_result(-6), XrError::RuntimeUnavailable);
        assert_eq!(XrError::from_result(-7), XrError::FormFactorUnavailable);
        assert_eq!(XrError::from_result(-13), XrError::InstanceLost);
        assert_eq!(XrError::from_result(-999), XrError::Other(-999));
        assert!(XrError::RuntimeUnavailable.is_retryable());
        assert!(!XrError::FormFactorUnavailable.is_retryable());
    }

    // Env-touching tests are serialized into one test to avoid racing
    // with each other (tests in one binary share the process env).
    #[test]
    fn endpoint_detection_order() {
        std::env::remove_var("VIVELITE_XR_RUNTIME");
        std::env::remove_var("XR_RUNTIME_JSON");
        assert_eq!(detect_endpoint(), RuntimeEndpoint::Socket(PathBuf::from("/tmp/xr-runtime")));

        std::env::set_var("VIVELITE_XR_RUNTIME", "/run/my/monado.sock");
        assert_eq!(detect_endpoint(), RuntimeEndpoint::Socket(PathBuf::from("/run/my/monado.sock")));

        std::env::set_var("VIVELITE_XR_RUNTIME", "/lib/libopenxr_loader.so");
        assert_eq!(detect_endpoint(), RuntimeEndpoint::Loader("/lib/libopenxr_loader.so".into()));

        std::env::remove_var("VIVELITE_XR_RUNTIME");
        std::env::set_var("XR_RUNTIME_JSON", "/etc/openxr/1/runtime/monado.json");
        assert_eq!(detect_endpoint(), RuntimeEndpoint::Loader("libopenxr_loader.so".into()));

        std::env::remove_var("VIVELITE_XR_RUNTIME");
        std::env::remove_var("XR_RUNTIME_JSON");
    }
}
