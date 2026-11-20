//! One Euro filter, for smoothing eye gaze before it leaves the device.
//!
//! The standard One Euro filter (Casiez et al. 2012): a low-pass filter
//! whose cutoff rises with signal speed, so slow motion is smooth and
//! fast motion is responsive. One filter per axis.

/// One Euro filter state for one axis.
#[derive(Debug, Clone)]
pub struct OneEuro {
    min_cutoff: f32,
    beta: f32,
    d_cutoff: f32,
    prev_value: Option<f32>,
    prev_derivative: f32,
}

impl OneEuro {
    /// Create a filter: `min_cutoff` (Hz) sets the slow-motion cutoff,
    /// `beta` scales how much speed raises the cutoff, `d_cutoff` (Hz)
    /// smooths the speed estimate itself.
    pub fn new(min_cutoff: f32, beta: f32, d_cutoff: f32) -> OneEuro {
        OneEuro {
            min_cutoff,
            beta,
            d_cutoff,
            prev_value: None,
            prev_derivative: 0.0,
        }
    }

    /// Filter one sample taken `dt` seconds after the previous one.
    ///
    /// The first sample passes through unchanged (it only seeds the
    /// state); a non-positive `dt` returns the previous output.
    pub fn filter(&mut self, value: f32, dt: f32) -> f32 {
        let Some(prev) = self.prev_value else {
            self.prev_value = Some(value);
            return value;
        };
        if dt <= 0.0 || !dt.is_finite() {
            return prev;
        }
        let derivative = (value - prev) / dt;
        let d_alpha = alpha(self.d_cutoff, dt);
        let derivative = self.prev_derivative + d_alpha * (derivative - self.prev_derivative);
        self.prev_derivative = derivative;
        let cutoff = self.min_cutoff + self.beta * derivative.abs();
        let out = prev + alpha(cutoff, dt) * (value - prev);
        self.prev_value = Some(out);
        out
    }
}

/// The filter's alpha for a cutoff `cutoff` (Hz) and interval `dt` (s).
fn alpha(cutoff: f32, dt: f32) -> f32 {
    let tau = 1.0 / (2.0 * std::f32::consts::PI * cutoff);
    1.0 / (1.0 + tau / dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_passes_through() {
        let mut f = OneEuro::new(1.0, 0.0, 1.0);
        assert_eq!(f.filter(5.0, 0.01), 5.0);
    }

    #[test]
    fn slow_constant_signal_converges() {
        let mut f = OneEuro::new(1.0, 0.0, 1.0);
        f.filter(0.0, 0.01);
        let mut last = 0.0;
        for _ in 0..200 {
            last = f.filter(10.0, 0.01);
        }
        assert!((last - 10.0).abs() < 0.01);
    }

    #[test]
    fn non_positive_dt_returns_the_previous_output() {
        let mut f = OneEuro::new(1.0, 0.0, 1.0);
        let first = f.filter(5.0, 0.01);
        assert_eq!(f.filter(100.0, 0.0), first);
        assert_eq!(f.filter(100.0, -0.01), first);
        assert_eq!(f.filter(100.0, f32::NAN), first);
    }

    #[test]
    fn alpha_is_in_unit_interval_and_rises_with_dt() {
        assert!(alpha(1.0, 0.01) > 0.0 && alpha(1.0, 0.01) < 1.0);
        assert!(alpha(1.0, 0.05) > alpha(1.0, 0.01));
        // A higher cutoff trusts the new sample more.
        assert!(alpha(10.0, 0.01) > alpha(1.0, 0.01));
    }

    #[test]
    fn beta_makes_fast_motion_more_responsive() {
        // With beta = 0 the filter lags a ramp; with a large beta it
        // tracks it closely.
        let mut slow = OneEuro::new(1.0, 0.0, 1.0);
        let mut fast = OneEuro::new(1.0, 10.0, 1.0);
        slow.filter(0.0, 0.01);
        fast.filter(0.0, 0.01);
        let mut slow_last = 0.0;
        let mut fast_last = 0.0;
        for i in 1..=50 {
            let v = i as f32;
            slow_last = slow.filter(v, 0.01);
            fast_last = fast.filter(v, 0.01);
        }
        assert!(fast_last > slow_last, "fast {fast_last} slow {slow_last}");
    }
}
