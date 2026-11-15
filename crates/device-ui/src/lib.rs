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

use model::Overlay;

/// Preview the overlay in a normal window (development host).
///
/// # Errors
///
/// Returns any error iced reports while creating the window or event loop.
pub fn run() -> Result<(), iced::Error> {
    let mut overlay = Overlay::new();
    let root = vivelite_core::device_layout(std::path::Path::new(".")).rootfs;
    overlay.notice = format!("Preview; staged root would be {}", root.display());
    iced::application("vivelite overlay", view::update, view::view)
        .run_with(move || (overlay, iced::Task::none()))
}

#[cfg(test)]
mod tests {
    use crate::model::{Check, Message, Overlay, Page};

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
