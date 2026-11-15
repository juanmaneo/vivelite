//! The on-device overlay runner: the iced program against the OpenXR
//! compositor (feature `xr`).
//!
//! Launched by `cargo xtask device-ui --xr`, which builds this binary
//! with the feature on; the default workspace build never compiles it
//! (required-features).

fn main() {
    if let Err(error) = device_ui::xr::overlay::run() {
        eprintln!("xr overlay: {error}");
        std::process::exit(1);
    }
}
