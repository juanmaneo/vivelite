//! vivelite on-device overlay UI (iced).
//!
//! The headset's vendor runtime owns display and controller tracking; the
//! WinlatorXR-derived APK composites this overlay into the OpenXR
//! swapchain. The UI itself is a plain iced program ([`model`] + [`view`])
//! that runs in a desktop preview window and against the compositor's
//! swapchain images behind the `xr` feature — [`compositor`] maps runtime
//! events onto the overlay's phase, and [`xr`] renders each phase into the
//! session's swapchain via wgpu.

pub mod compositor;
pub mod model;
pub mod view;
pub mod xr;

pub use iced::Error;

/// Preview the overlay in a normal window (development host).
///
/// # Errors
///
/// Returns any error iced reports while creating the window or event loop.
pub fn run() -> Result<(), Error> {
    iced::application("vivelite overlay", view::update, view::view).run()
}

#[cfg(test)]
mod tests {
    use crate::model::{Check, Message, Overlay, Page};

    /// Always run the native launch in a child with display variables removed,
    /// so this test exercises the failure path without ever opening a real window.
    #[test]
    fn run_without_a_display_fails_instead_of_hanging() {
        const CHILD: &str = "VIVELITE_DEVICE_UI_HEADLESS_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let _ = std::panic::catch_unwind(crate::run);
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::run_without_a_display_fails_instead_of_hanging",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn overlay_flow_end_to_end() {
        let mut overlay = Overlay::new();
        overlay.update(Message::Select(Page::Doctor));
        overlay.update(Message::Checks(vec![Check {
            label: "wine staged".into(),
            ok: true,
        }]));
        assert_eq!(overlay.page, Page::Doctor);
        assert_eq!(overlay.checks.len(), 1);
    }
}
