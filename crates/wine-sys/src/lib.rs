//! Raw FFI bindings to Wine's embedded C API.
//!
//! These mirror the declarations in Wine's `include/wine/` headers
//! (`wine_get_version`, `wine_get_build_id`, `wine_init`, and the ARM64EC
//! FEX hooks exposed through `__wine_unixlib`). Only the surface vivelite
//! drives is declared; extend as needed rather than generating everything.
//!
//! # Safety contract
//!
//! Everything in this crate is `unsafe` and intentionally undocumented at the
//! raw level; use the safe `wine` crate instead. The pointers passed to
//! `wine_init` must outlive the initialized loader.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use std::os::raw::{c_char, c_int};

/// Opaque environment-adjacent block passed to `wine_init` (argv layout).
///
/// In Wine's `wine_init`, the loader receives argc/argv in this form; the
/// strings must remain valid for the lifetime of the process once Wine's
/// threads are started.
#[repr(C)]
pub struct WineInitArgs {
    /// Argument count, including argv[0].
    pub argc: c_int,
    /// Argument vector, null-terminated.
    pub argv: *mut *mut c_char,
}

#[cfg(feature = "link")]
extern "C" {
    /// Returns the running Wine version, e.g. `"wine-10.0-arm64ec"`.
    ///
    /// Requires Wine to already be initialized (in-process embedding).
    pub fn wine_get_version() -> *const c_char;

    /// Returns the git build id of the running Wine, e.g. `"wine-10.0 (Valve …)"`.
    pub fn wine_get_build_id() -> *const c_char;

    /// Initializes the Wine loader in-process: starts wineserver and the
    /// loader threads. Must be called exactly once, from the thread that will
    /// drive the main Wine thread.
    pub fn wine_init(
        argc: c_int,
        argv: *mut *mut c_char,
        error: c_int,
        error_length: c_int,
    ) -> *mut c_char;

    /// Returns a file descriptor for the wineserver pipe on success, -1 on error.
    pub fn wine_server_send_fd(fd: c_int) -> c_int;

    /// Wine's debug channel printf; routed to the ETSY_LOG configured sink.
    pub fn wine_dbg_printf(format: *const c_char, ...) -> c_int;
}

/// C string → Rust, borrowed: the buffer is owned by Wine and lives as long
/// as the loader; safe wrapper calls this only right after the getter.
///
/// # Safety
///
/// `ptr` must be a valid null-terminated C string.
pub unsafe fn cstr(ptr: *const c_char) -> &'static str {
    if ptr.is_null() {
        return "";
    }
    std::ffi::CStr::from_ptr(ptr).to_str().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_args_layout_is_c_compatible() {
        assert_eq!(
            std::mem::size_of::<WineInitArgs>(),
            std::mem::size_of::<usize>() * 2
        );
    }

    #[test]
    fn cstr_handles_null_valid_and_invalid_utf8() {
        let valid = std::ffi::CString::new("wine-10.0").unwrap();
        let invalid = std::ffi::CString::new(vec![0xff]).unwrap();
        unsafe {
            assert_eq!(cstr(std::ptr::null()), "");
            assert_eq!(cstr(valid.as_ptr()), "wine-10.0");
            assert_eq!(cstr(invalid.as_ptr()), "");
        }
    }
}
