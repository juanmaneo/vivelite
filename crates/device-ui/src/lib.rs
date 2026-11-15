//! vivelite on-device overlay UI (iced).
//!
//! The headset's vendor runtime owns display and controller tracking; the
//! WinlatorXR-derived APK composites this overlay into the OpenXR
//! swapchain. The UI itself is a plain iced program ([`model`] + [`view`])
//! that runs today in a desktop preview window and later against the
//! compositor's swapchain images once the Monado wrapper grows
//! session/swapchain APIs.

pub mod model;
pub mod view;

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

    /// Creating a window needs a display. Headless hosts (the owner's CI-less
    /// box) take the failure path; hosts with a display skip this test so it
    /// never opens a real window.
    #[test]
    fn run_without_a_display_fails_instead_of_hanging() {
        if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return;
        }
        let _ = std::panic::catch_unwind(crate::run);
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
