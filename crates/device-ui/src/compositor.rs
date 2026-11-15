//! Compositor attachment: how OpenXR session events move the overlay.
//!
//! The runner (the future compositor loop) owns the monado session: it
//! polls events, maps them to a [`Phase`] with [`Phase::on_event`], and
//! feeds [`crate::model::Message::Compositor`] into the overlay. The
//! overlay itself never links against OpenXR; this module is the pure
//! translation layer, so every lifecycle path is testable headlessly.

use monado::{Event, SessionState};

/// The overlay's compositor attach phase, shown on the staging page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No session yet: desktop preview, or before attach.
    Detached,
    /// A session exists, waiting for READY.
    Connecting,
    /// READY: the runner may begin the session and create swapchains.
    Ready,
    /// RUNNING (Synchronized/Visible/Focused): render every frame.
    Rendering,
    /// STOPPING or LOSS_PENDING: finish the frame in flight, then end.
    Stopping,
    /// EXITING, or the instance went away: detach.
    Lost,
}

impl Phase {
    /// One-line label for the overlay's staging page.
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Detached => "compositor: detached (desktop preview)",
            Phase::Connecting => "compositor: connecting…",
            Phase::Ready => "compositor: session ready",
            Phase::Rendering => "compositor: rendering into the swapchain",
            Phase::Stopping => "compositor: stopping…",
            Phase::Lost => "compositor: session lost — re-attach to retry",
        }
    }

    /// The next phase for one polled runtime event.
    pub fn on_event(&self, event: &Event) -> Phase {
        match event {
            Event::None => *self,
            Event::InstanceLossPending => Phase::Lost,
            // Events this wrapper does not model do not move the phase.
            Event::Other(_) => *self,
            Event::SessionStateChanged { state, .. } => self.on_state(*state),
        }
    }

    /// The next phase for a session-state change.
    pub fn on_state(&self, state: SessionState) -> Phase {
        match state {
            SessionState::Unknown | SessionState::Idle => Phase::Connecting,
            SessionState::Ready => Phase::Ready,
            SessionState::Synchronized | SessionState::Visible | SessionState::Focused => {
                Phase::Rendering
            }
            SessionState::Stopping | SessionState::LossPending => Phase::Stopping,
            SessionState::Exiting => Phase::Lost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session-state change event; the handle and time do not matter to
    /// the phase machine.
    fn changed(state: SessionState) -> Event {
        Event::SessionStateChanged {
            state,
            session: 1,
            time: 0,
        }
    }

    #[test]
    fn every_state_maps_to_the_expected_phase() {
        let detached = Phase::Detached;
        assert_eq!(detached.on_state(SessionState::Unknown), Phase::Connecting);
        assert_eq!(detached.on_state(SessionState::Idle), Phase::Connecting);
        assert_eq!(detached.on_state(SessionState::Ready), Phase::Ready);
        assert_eq!(
            detached.on_state(SessionState::Synchronized),
            Phase::Rendering
        );
        assert_eq!(detached.on_state(SessionState::Visible), Phase::Rendering);
        assert_eq!(detached.on_state(SessionState::Focused), Phase::Rendering);
        assert_eq!(detached.on_state(SessionState::Stopping), Phase::Stopping);
        assert_eq!(
            detached.on_state(SessionState::LossPending),
            Phase::Stopping
        );
        assert_eq!(detached.on_state(SessionState::Exiting), Phase::Lost);
    }

    #[test]
    fn events_advance_the_phase_from_anywhere() {
        // The READY → begin → Focused happy path, from detached.
        let mut phase = Phase::Detached;
        phase = phase.on_event(&changed(SessionState::Idle));
        assert_eq!(phase, Phase::Connecting);
        phase = phase.on_event(&changed(SessionState::Ready));
        assert_eq!(phase, Phase::Ready);
        phase = phase.on_event(&changed(SessionState::Focused));
        assert_eq!(phase, Phase::Rendering);
        // A clean stop: STOPPING → end → (next session) Idle.
        phase = phase.on_event(&changed(SessionState::Stopping));
        assert_eq!(phase, Phase::Stopping);
        phase = phase.on_event(&changed(SessionState::Exiting));
        assert_eq!(phase, Phase::Lost);
    }

    #[test]
    fn no_events_and_unmodeled_events_hold_the_phase() {
        let rendering = Phase::Rendering;
        assert_eq!(rendering.on_event(&Event::None), Phase::Rendering);
        assert_eq!(rendering.on_event(&Event::Other(99)), Phase::Rendering);
    }

    #[test]
    fn instance_loss_detaches_immediately() {
        assert_eq!(
            Phase::Rendering.on_event(&Event::InstanceLossPending),
            Phase::Lost
        );
        assert_eq!(
            Phase::Ready.on_event(&Event::InstanceLossPending),
            Phase::Lost
        );
    }

    #[test]
    fn labels_are_one_line_and_stable() {
        for phase in [
            Phase::Detached,
            Phase::Connecting,
            Phase::Ready,
            Phase::Rendering,
            Phase::Stopping,
            Phase::Lost,
        ] {
            let label = phase.label();
            assert!(!label.is_empty());
            assert!(!label.contains('\n'), "{label}");
        }
    }
}
