//! Per-axis eye-gaze calibration.
//!
//! The model frame-shell documents: `corrected = (measured - offset) /
//! gain` per axis in degrees, fitted from pointer clicks (the wearer
//! looks at what they click). A fresh calibration starts as the identity
//! (offset 0, gain 1), which is also the fallback when no fit exists.

/// One raw eye-gaze reading, in degrees per axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyeSample {
    /// Horizontal angle in degrees; positive is to the wearer's right.
    pub yaw_deg: f32,
    /// Vertical angle in degrees; positive is up.
    pub pitch_deg: f32,
}

/// Per-axis linear calibration: `corrected = (measured - offset) / gain`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    /// Horizontal offset in degrees.
    pub yaw_offset: f32,
    /// Horizontal gain (dimensionless, must be non-zero).
    pub yaw_gain: f32,
    /// Vertical offset in degrees.
    pub pitch_offset: f32,
    /// Vertical gain (dimensionless, must be non-zero).
    pub pitch_gain: f32,
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            yaw_offset: 0.0,
            yaw_gain: 1.0,
            pitch_offset: 0.0,
            pitch_gain: 1.0,
        }
    }
}

impl Calibration {
    /// Apply the calibration to a raw sample.
    ///
    /// A non-finite or zero gain axis falls back to the identity for that
    /// axis rather than producing garbage or dividing by zero.
    pub fn correct(&self, sample: EyeSample) -> EyeSample {
        EyeSample {
            yaw_deg: correct_axis(sample.yaw_deg, self.yaw_offset, self.yaw_gain),
            pitch_deg: correct_axis(sample.pitch_deg, self.pitch_offset, self.pitch_gain),
        }
    }
}

fn correct_axis(measured: f32, offset: f32, gain: f32) -> f32 {
    if gain.is_finite() && gain != 0.0 {
        (measured - offset) / gain
    } else {
        measured
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(yaw: f32, pitch: f32) -> EyeSample {
        EyeSample {
            yaw_deg: yaw,
            pitch_deg: pitch,
        }
    }

    #[test]
    fn identity_calibration_leaves_samples_unchanged() {
        let c = Calibration::default();
        assert_eq!(c.correct(sample(3.0, -7.5)), sample(3.0, -7.5));
    }

    #[test]
    fn offset_and_gain_apply_per_axis() {
        // The generic model frame-shell documents: measured about 7° too
        // high and 1.2× too steep vertically.
        let c = Calibration {
            pitch_offset: 7.0,
            pitch_gain: 1.2,
            ..Calibration::default()
        };
        let out = c.correct(sample(0.0, 13.4));
        assert_eq!(out.yaw_deg, 0.0);
        assert!((out.pitch_deg - (13.4 - 7.0) / 1.2).abs() < 1e-5);
    }

    #[test]
    fn zero_or_non_finite_gain_falls_back_to_identity() {
        let c = Calibration {
            yaw_offset: 5.0,
            yaw_gain: 0.0,
            pitch_offset: 5.0,
            pitch_gain: f32::NAN,
        };
        assert_eq!(c.correct(sample(3.0, -2.0)), sample(3.0, -2.0));
    }
}
