//! iced rendering of the overlay model.
//!
//! The overlay draws the same two pages on any surface: the desktop preview
//! window (`crate::run`) today, and the OpenXR compositor swapchain on
//! device once the Monado wrapper grows session/swapchain APIs. The iced
//! program is identical in both cases; only the wgpu target differs.

use iced::widget::{button, column, container, progress_bar, text};
use iced::{Color, Element, Length, Task};

use crate::model::{Check, Message, Overlay, Page};

/// Background for the in-world overlay: mostly transparent black so the
/// compositor's passthrough/VR scene stays visible behind it.
const OVERLAY_BACKGROUND: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.55);

/// Applies one message to the model. Stage/doctor refresh work is driven by
/// the runtime, so no follow-up task is needed here.
pub fn update(overlay: &mut Overlay, message: Message) -> Task<Message> {
    overlay.update(message);
    Task::none()
}

/// Renders the current page of the overlay.
pub fn view(overlay: &Overlay) -> Element<'_, Message> {
    let content: Element<'_, Message> = match overlay.page {
        Page::Staging => staging_page(overlay),
        Page::Doctor => doctor_page(overlay),
    };
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(24)
        .style(|_theme| container::Style {
            background: Some(OVERLAY_BACKGROUND.into()),
            ..Default::default()
        })
        .into()
}

fn staging_page(overlay: &Overlay) -> Element<'_, Message> {
    let summary = match overlay.stage.files {
        Some(files) => format!(
            "{files} files, {} bytes staged",
            overlay.stage.bytes.unwrap_or(0)
        ),
        None => "No staged SteamOS components yet.".to_string(),
    };
    let progress = if overlay.stage.files.is_some() {
        100.0
    } else {
        0.0
    };
    column![
        text("vivelite — staging").size(28),
        text(summary),
        progress_bar(0.0..=100.0, progress),
        text(overlay.notice.clone()),
        button(text("Readiness checks").size(20)).on_press(Message::Select(Page::Doctor)),
    ]
    .spacing(12)
    .into()
}

fn doctor_page(overlay: &Overlay) -> Element<'_, Message> {
    let mut page = column![text("vivelite — readiness").size(28)].spacing(12);
    if overlay.checks.is_empty() {
        page = page.push(text("No checks yet."));
    }
    for check in &overlay.checks {
        page = page.push(check_row(check));
    }
    page.push(button(text("Back to staging").size(20)).on_press(Message::Select(Page::Staging)))
        .into()
}

fn check_row(check: &Check) -> Element<'_, Message> {
    let mark = if check.ok { "OK" } else { "FAIL" };
    text(format!("{mark}  {}", check.label)).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StageStatus;

    #[test]
    fn view_builds_widgets_for_both_pages() {
        let mut overlay = Overlay::new();
        let _ = view(&overlay);
        overlay.stage = StageStatus {
            root: Some("/data".into()),
            files: Some(3),
            bytes: Some(9),
        };
        let _ = view(&overlay);
        overlay.page = Page::Doctor;
        overlay.checks = vec![
            Check {
                label: "curl".into(),
                ok: true,
            },
            Check {
                label: "rauc".into(),
                ok: false,
            },
        ];
        let _ = view(&overlay);
    }

    #[test]
    fn update_routes_messages_into_the_model() {
        let mut overlay = Overlay::new();
        let _ = update(
            &mut overlay,
            Message::Staged {
                root: "/data".into(),
                files: 1,
                bytes: 2,
            },
        );
        assert_eq!(overlay.stage.files, Some(1));
    }
}
