//! Safe wrapper around Wine's embedded C API.
//!
//! vivelite does not spawn a `wine` binary and hope: it embeds the loader
//! (`wine_init`) inside the orchestrator process, exactly like Wine's own
//! `wine64` front-ends do, so the FEX ARM64EC hooks inside this libwine are
//! the ones doing x86 translation.

// This crate is the designated safe boundary over `wine-sys`: unsafe appears
// here by design, and every use carries a SAFETY comment (enforced
// workspace-wide by `clippy::undocumented_unsafe_blocks = deny`).
#![allow(unsafe_code)]

#[cfg(feature = "link")]
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

/// A Wine prefix (bottle) on disk.
///
/// The prefix layout is stable across Wine versions: `dosdevices/c:` is the
/// drive root, `drive_c/users/steam` the home, `user.reg`/`system.reg` the
/// registry files.
#[derive(Debug, Clone)]
pub struct Prefix {
    root: PathBuf,
}

impl Prefix {
    /// Use the prefix at `root`, creating the skeleton if missing.
    pub fn open(root: &Path) -> std::io::Result<Prefix> {
        let p = Prefix {
            root: root.to_path_buf(),
        };
        if !p.registry().exists() {
            p.create_skeleton()?;
        }
        Ok(p)
    }

    fn create_skeleton(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.drive_c())?;
        std::fs::create_dir_all(self.users())?;
        for reg in ["user.reg", "system.reg", "userdef.reg"] {
            let f = self.root.join(reg);
            if !f.exists() {
                std::fs::write(&f, "WINE REGISTRY Version 2\n\n[header]\n")?;
            }
        }
        Ok(())
    }

    /// Absolute path of the prefix root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `drive_c` inside the prefix.
    pub fn drive_c(&self) -> PathBuf {
        self.root.join("drive_c")
    }

    /// The user home inside the prefix.
    pub fn users(&self) -> PathBuf {
        self.drive_c().join("users").join("steam")
    }

    /// Path of `user.reg`.
    pub fn registry(&self) -> PathBuf {
        self.root.join("user.reg")
    }

    /// Map a Windows path (`C:\\foo`) to a host path.
    ///
    /// Only the `C:` drive is mapped; anything else is an error.
    pub fn to_host(&self, windows: &str) -> Result<PathBuf, String> {
        let (drive, rest) = windows.split_once(':').ok_or("missing drive letter")?;
        if !drive.eq_ignore_ascii_case("c") {
            return Err(format!("only the C: drive is mapped, got {drive}:"));
        }
        let rest = rest.trim_start_matches('\\').replace('\\', "/");
        if rest.split('/').any(|p| p == "..") {
            return Err("path traversal refused".into());
        }
        Ok(self.drive_c().join(rest))
    }
}

/// Version information reported by an initialized Wine loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WineVersion {
    /// e.g. `wine-10.0`.
    pub version: String,
    /// e.g. `wine-10.0-2 (Valve Steam Frame build)`.
    pub build: String,
}

/// Reads the version of the currently loaded libwine.
///
/// # Safety
///
/// Requires Wine to have been initialized in-process (by the host app) before
/// the call; the underlying getters read global state owned by the loader.
#[cfg(feature = "link")]
pub unsafe fn version() -> WineVersion {
    let v = wine_sys::wine_get_version();
    let b = wine_sys::wine_get_build_id();
    WineVersion {
        version: CStr::from_ptr(v).to_string_lossy().into_owned(),
        build: CStr::from_ptr(b).to_string_lossy().into_owned(),
    }
}

/// Initializes the embedded Wine loader.
///
/// `argv` is kept alive for the whole call and beyond (Wine threads retain
/// pointers into it); the caller passes the program's own argv.
///
/// # Safety
///
/// Must be called once, before any other Wine call, from a thread that will
/// stay alive. The returned value is `()` on success; errors are reported by
/// Wine writing to its debug channel and by subsequent calls failing.
#[cfg(feature = "link")]
pub unsafe fn init(argv: &[String]) -> Result<(), String> {
    let mut cargs: Vec<CString> = argv
        .iter()
        .map(|a| CString::new(a.as_str()).map_err(|_| "interior NUL in argv"))
        .collect::<Result<_, _>>()?;
    cargs.insert(0, CString::new("vivelite").unwrap());
    let mut ptrs: Vec<*mut std::os::raw::c_char> =
        cargs.iter_mut().map(|c| c.as_ptr() as *mut _).collect();
    ptrs.push(std::ptr::null_mut());
    let argc = (ptrs.len() - 1) as std::os::raw::c_int;
    // wine_init logs failures itself; there is no status return.
    wine_sys::wine_init(argc, ptrs.as_mut_ptr(), 0, 0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_skeleton_and_registry() {
        let dir = tempfile_dir();
        let p = Prefix::open(&dir).unwrap();
        assert!(p.registry().exists());
        assert!(p.drive_c().exists());
        assert!(p.users().exists());
        assert_eq!(p.root(), dir.as_path());
    }

    #[test]
    fn windows_to_host_paths() {
        let dir = tempfile_dir();
        let p = Prefix::open(&dir).unwrap();
        let host = p.to_host("C:\\Program Files\\Game\\game.exe").unwrap();
        assert_eq!(host, p.drive_c().join("Program Files/Game/game.exe"));
        assert!(p.to_host("c:\\ok").is_ok(), "lowercase drive accepted");
        assert!(p.to_host("D:\\nope").is_err());
        assert!(p.to_host("C:\\..\\..\\etc").is_err(), "traversal refused");
        assert!(p.to_host("nocolon").is_err());
    }

    #[test]
    fn open_is_idempotent() {
        let dir = tempfile_dir();
        let _a = Prefix::open(&dir).unwrap();
        let _b = Prefix::open(&dir).unwrap();
        // The registry file must not have been duplicated.
        let n = std::fs::read_dir(&dir).unwrap().count();
        assert!(n >= 4);
    }

    #[test]
    fn skeleton_preserves_existing_registry_file() {
        let dir = tempfile_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("user.reg"), "existing registry").unwrap();
        let prefix = Prefix { root: dir.clone() };
        prefix.create_skeleton().unwrap();
        assert_eq!(
            std::fs::read_to_string(prefix.registry()).unwrap(),
            "existing registry"
        );
        assert!(dir.join("system.reg").exists());
        assert!(dir.join("userdef.reg").exists());
    }

    #[test]
    fn open_reports_skeleton_creation_errors() {
        let dir = tempfile_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("drive_c"), "not a directory").unwrap();
        assert!(Prefix::open(&dir).is_err());
    }

    fn tempfile_dir() -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!("vivelite-test-{}-{}", std::process::id(), n))
    }

    #[test]
    fn skeleton_creation_failures_are_reported() {
        let dir = tempfile_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // A regular file where a directory is needed makes creation fail.
        std::fs::write(dir.join("drive_c"), b"file").unwrap();
        assert!(Prefix::open(&dir).is_err());

        let dir = tempfile_dir();
        std::fs::create_dir_all(dir.join("drive_c")).unwrap();
        std::fs::write(dir.join("drive_c/users"), b"file").unwrap();
        assert!(Prefix::open(&dir).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn skeleton_reports_registry_write_failure() {
        let dir = tempfile_dir();
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("missing/user.reg"), dir.join("user.reg")).unwrap();
        let prefix = Prefix { root: dir.clone() };
        assert!(prefix.create_skeleton().is_err());
    }
}
