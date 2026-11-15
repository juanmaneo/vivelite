//! Per-frame render decision, as data.
//!
//! One frame's timing and the overlay's compositor phase arrive together;
//! the runner's only decision is what to do with the frame. Keeping that
//! decision pure means every combination is testable headlessly while the
//! loop itself (feature `xr`) stays thin.

use crate::compositor::Phase;
use monado::FrameTiming;

/// What the frame loop does with one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameAction {
    /// Acquire a swapchain image, render, submit one quad layer.
    Render,
    /// Open and close the frame with no layers; the compositor keeps the
    /// last submitted image.
    Skip,
    /// Leave the frame loop entirely (session lost or exiting).
    Detach,
}

/// The decision for one frame: render while focused and the runtime wants
/// a frame, keep frame timing while stopping (end-of-frame must still be
/// called for every frame that was opened), and detach as soon as the
/// session is gone.
#[must_use]
pub fn frame_action(phase: Phase, timing: &FrameTiming) -> FrameAction {
    match phase {
        Phase::Lost => FrameAction::Detach,
        Phase::Rendering if timing.should_render => FrameAction::Render,
        Phase::Rendering | Phase::Stopping => FrameAction::Skip,
        Phase::Detached | Phase::Connecting | Phase::Ready => FrameAction::Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(should_render: bool) -> FrameTiming {
        FrameTiming {
            display_time: 1,
            should_render,
        }
    }

    #[test]
    fn renders_while_running_and_wanted() {
        assert_eq!(
            frame_action(Phase::Rendering, &timing(true)),
            FrameAction::Render
        );
    }

    #[test]
    fn skips_when_the_runtime_does_not_want_pixels() {
        assert_eq!(
            frame_action(Phase::Rendering, &timing(false)),
            FrameAction::Skip
        );
    }

    #[test]
    fn keeps_frame_timing_while_stopping() {
        // Even with should_render set, STOPPING means no new layers —
        // finish frames until the session goes IDLE.
        assert_eq!(
            frame_action(Phase::Stopping, &timing(true)),
            FrameAction::Skip
        );
        assert_eq!(
            frame_action(Phase::Stopping, &timing(false)),
            FrameAction::Skip
        );
    }

    #[test]
    fn every_pre_session_phase_skips() {
        for phase in [Phase::Detached, Phase::Connecting, Phase::Ready] {
            assert_eq!(frame_action(phase, &timing(true)), FrameAction::Skip);
            assert_eq!(frame_action(phase, &timing(false)), FrameAction::Skip);
        }
    }

    #[test]
    fn lost_detaches_regardless_of_timing() {
        assert_eq!(
            frame_action(Phase::Lost, &timing(true)),
            FrameAction::Detach
        );
        assert_eq!(
            frame_action(Phase::Lost, &timing(false)),
            FrameAction::Detach
        );
    }
}
