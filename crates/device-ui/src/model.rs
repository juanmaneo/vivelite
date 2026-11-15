//! Overlay state machine for the on-device UI.
//!
//! Pure data and transitions, no iced types, so the overlay behavior is
//! unit-testable headlessly. `crate::view` turns this state into iced
//! widgets; `crate::run` drives both on a real surface.

use std::path::PathBuf;

/// What the overlay is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    /// Staging status: which pipeline stage ran, with counts from the
    /// stage manifest.
    Staging,
    /// Readiness checks over the on-device layout.
    Doctor,
}

/// One line of the doctor checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Human-readable description.
    pub label: String,
    /// Whether the check passed.
    pub ok: bool,
}

/// Snapshot of the staged SteamOS components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageStatus {
    /// Directory holding the staged layout (the device layout rootfs).
    pub root: Option<PathBuf>,
    /// Files counted by the stage manifest.
    pub files: Option<u64>,
    /// Bytes counted by the stage manifest.
    pub bytes: Option<u64>,
}

/// All state the overlay renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    /// Current page.
    pub page: Page,
    /// Staged-component snapshot.
    pub stage: StageStatus,
    /// Doctor checklist lines.
    pub checks: Vec<Check>,
    /// Last transient message (errors, hints).
    pub notice: String,
}

/// Input events the runtime sends into the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Switch page.
    Select(Page),
    /// New doctor results.
    Checks(Vec<Check>),
    /// Stage manifest was (re)loaded from `root`.
    Staged {
        /// Staged layout root.
        root: PathBuf,
        /// Files per the manifest.
        files: u64,
        /// Bytes per the manifest.
        bytes: u64,
    },
    /// A stage step failed; show why.
    StageFailed(String),
    /// Tick from the runtime's frame loop (refresh doctor data).
    Tick,
}

impl Overlay {
    /// Overlay with no data yet; the runtime fills it via messages.
    pub fn new() -> Self {
        Overlay {
            page: Page::Staging,
            stage: StageStatus {
                root: None,
                files: None,
                bytes: None,
            },
            checks: Vec::new(),
            notice: "Waiting for staging status…".into(),
        }
    }

    /// Apply one message, returning whether the state changed.
    pub fn update(&mut self, message: Message) -> bool {
        match message {
            Message::Select(page) => {
                if self.page == page {
                    return false;
                }
                self.page = page;
                true
            }
            Message::Checks(checks) => {
                if self.checks == checks {
                    return false;
                }
                self.checks = checks;
                true
            }
            Message::Staged { root, files, bytes } => {
                let next = StageStatus {
                    root: Some(root),
                    files: Some(files),
                    bytes: Some(bytes),
                };
                if self.stage == next {
                    return false;
                }
                self.stage = next;
                self.notice = "SteamOS components staged.".into();
                true
            }
            Message::StageFailed(reason) => {
                let next = format!("Stage failed: {reason}");
                if self.notice == next {
                    return false;
                }
                self.notice = next;
                true
            }
            Message::Tick => false,
        }
    }

    /// Convenience constructor used by tests and the desktop preview.
    pub fn with_checks(checks: Vec<Check>) -> Self {
        let mut overlay = Self::new();
        overlay.checks = checks;
        overlay
    }
}

impl Default for Overlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(label: &str, ok: bool) -> Check {
        Check {
            label: label.into(),
            ok,
        }
    }

    #[test]
    fn starts_on_staging_with_no_data() {
        let overlay = Overlay::new();
        assert_eq!(overlay.page, Page::Staging);
        assert_eq!(
            overlay.stage,
            StageStatus {
                root: None,
                files: None,
                bytes: None
            }
        );
        assert!(overlay.checks.is_empty());
    }

    #[test]
    fn select_switches_pages_and_ignores_duplicates() {
        let mut overlay = Overlay::new();
        assert!(overlay.update(Message::Select(Page::Doctor)));
        assert_eq!(overlay.page, Page::Doctor);
        assert!(!overlay.update(Message::Select(Page::Doctor)));
    }

    #[test]
    fn staged_message_fills_the_snapshot() {
        let mut overlay = Overlay::new();
        let changed = overlay.update(Message::Staged {
            root: PathBuf::from("/data/vivelite/rootfs"),
            files: 12,
            bytes: 4096,
        });
        assert!(changed);
        assert_eq!(overlay.stage.files, Some(12));
        assert_eq!(overlay.notice, "SteamOS components staged.");
        assert!(!overlay.update(Message::Staged {
            root: PathBuf::from("/data/vivelite/rootfs"),
            files: 12,
            bytes: 4096,
        }));
    }

    #[test]
    fn stage_failure_sets_a_notice() {
        let mut overlay = Overlay::new();
        assert!(overlay.update(Message::StageFailed("desync exploded".into())));
        assert_eq!(overlay.notice, "Stage failed: desync exploded");
    }

    #[test]
    fn checks_replace_and_deduplicate() {
        let mut overlay = Overlay::new();
        assert!(overlay.update(Message::Checks(vec![check("curl", true)])));
        assert_eq!(overlay.checks.len(), 1);
        assert!(!overlay.update(Message::Checks(vec![check("curl", true)])));
        assert!(overlay.update(Message::Checks(vec![check("curl", false)])));
        assert_eq!(overlay.checks[0].ok, false);
    }

    #[test]
    fn tick_is_a_no_op() {
        let mut overlay = Overlay::new();
        assert!(!overlay.update(Message::Tick));
    }
}
