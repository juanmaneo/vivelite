//! Performance modes and battery warnings.
//!
//! Modeled on the behavior frame-shell documents for the Steam Frame:
//! two named performance profiles applied through the platform's
//! power manager, and battery warnings at fixed charge levels that
//! each fire once as the charge falls through them. vivelite keeps the
//! model but not the SteamOS specifics: the host applies a mode through
//! whatever the device exposes, and this module owns the state machine.

/// A named performance profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceMode {
    /// Reduced clocks, tuned for streaming instead of local rendering.
    Streaming,
    /// Full clocks, for native rendering.
    Standard,
}

impl PerformanceMode {
    /// Stable machine-readable name, also used as the on-disk value.
    pub fn name(self) -> &'static str {
        match self {
            PerformanceMode::Streaming => "streaming",
            PerformanceMode::Standard => "standard",
        }
    }

    /// Parse a stored name; unknown names fall back to [`PerformanceMode::Standard`].
    pub fn from_name(name: &str) -> PerformanceMode {
        match name {
            "streaming" => PerformanceMode::Streaming,
            _ => PerformanceMode::Standard,
        }
    }
}

/// Battery charge levels, in whole percent, that warn as the charge
/// falls through them: 10, then 5, then 1.
pub const BATTERY_WARNING_LEVELS: [u8; 3] = [10, 5, 1];

/// Tracks which battery warnings have already fired.
///
/// A warning fires when the charge is observed at or below a level that
/// has not fired yet; the most severe newly crossed level wins, so a
/// jump from 12% straight to 0% reports the 1% warning, not three.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatteryWarnings {
    fired: [bool; BATTERY_WARNING_LEVELS.len()],
}

impl BatteryWarnings {
    /// Observe a charge level; returns the most severe newly crossed
    /// warning, if any. Rising charge never fires anything.
    pub fn observe(&mut self, charge_percent: u8) -> Option<u8> {
        let mut worst = None;
        for (i, &level) in BATTERY_WARNING_LEVELS.iter().enumerate() {
            if charge_percent <= level && !self.fired[i] {
                self.fired[i] = true;
                worst = Some(level);
            }
        }
        worst
    }

    /// Whether the warning for `level` has fired.
    pub fn fired(&self, level: u8) -> bool {
        BATTERY_WARNING_LEVELS
            .iter()
            .zip(&self.fired)
            .find(|(&l, _)| l == level)
            .map(|(_, &f)| f)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_names_roundtrip() {
        assert_eq!(PerformanceMode::Streaming.name(), "streaming");
        assert_eq!(PerformanceMode::Standard.name(), "standard");
        assert_eq!(
            PerformanceMode::from_name("streaming"),
            PerformanceMode::Streaming
        );
        assert_eq!(
            PerformanceMode::from_name("standard"),
            PerformanceMode::Standard
        );
        // Unknown and empty names fall back to Standard.
        assert_eq!(
            PerformanceMode::from_name("turbo"),
            PerformanceMode::Standard
        );
        assert_eq!(PerformanceMode::from_name(""), PerformanceMode::Standard);
    }

    #[test]
    fn battery_levels_are_the_documented_thresholds() {
        assert_eq!(BATTERY_WARNING_LEVELS, [10, 5, 1]);
    }

    #[test]
    fn warnings_fire_once_as_charge_falls() {
        let mut w = BatteryWarnings::default();
        assert_eq!(w.observe(50), None);
        assert_eq!(w.observe(11), None);
        assert_eq!(w.observe(10), Some(10));
        assert_eq!(w.observe(9), None);
        assert_eq!(w.observe(5), Some(5));
        assert_eq!(w.observe(3), None);
        assert_eq!(w.observe(1), Some(1));
        assert_eq!(w.observe(0), None);
        assert!(w.fired(10));
        assert!(w.fired(5));
        assert!(w.fired(1));
    }

    #[test]
    fn a_jump_reports_only_the_most_severe_new_level() {
        let mut w = BatteryWarnings::default();
        assert_eq!(w.observe(12), None);
        assert_eq!(w.observe(0), Some(1));
        assert!(w.fired(10) && w.fired(5) && w.fired(1));
    }

    #[test]
    fn rising_charge_never_fires() {
        let mut w = BatteryWarnings::default();
        assert_eq!(w.observe(100), None);
        assert_eq!(w.observe(99), None);
        assert!(!w.fired(10));
    }

    #[test]
    fn fired_is_false_for_unknown_levels() {
        let w = BatteryWarnings::default();
        assert!(!w.fired(7));
    }

    #[test]
    fn fired_is_true_for_observed_levels() {
        let mut w = BatteryWarnings::default();
        w.observe(10);
        assert!(w.fired(10));
        w.observe(5);
        assert!(w.fired(5));
        w.observe(1);
        assert!(w.fired(1));
    }
}
