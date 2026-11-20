//! Runtime settings, validated.
//!
//! Modeled on the settings object frame-monado documents for the Steam
//! Frame (`~/.config/monado/config_v0.json`, read when the compositor
//! starts): passthrough scratch scale, whether passthrough keeps running
//! behind an opaque layer, a headset-view mirror with a rate cap, and the
//! eye cameras' rate. vivelite keeps the shape and the documented ranges
//! so a settings file written for one device reads sensibly on the other;
//! the on-device application of each knob stays with the runtime.

/// Validated runtime settings.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSettings {
    /// Size (0–1) of the passthrough scratch images while they only hold
    /// passthrough. The cameras resolve far fewer pixels per radian than
    /// the panel, so smaller values save memory and bandwidth at a
    /// visible sharpness cost.
    pub passthrough_scratch_scale: f32,
    /// Keep the cameras and passthrough running while an opaque
    /// projection layer hides them. Off saves power while streaming.
    pub passthrough_behind_opaque: bool,
    /// Show the headset view on a mirror device.
    pub mirror: bool,
    /// Rate cap (Hz) for the mirror view.
    pub mirror_fps: u32,
    /// Eye cameras' rate (15–90 Hz). Higher rates cost power.
    pub eye_tracking_fps: u32,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        RuntimeSettings {
            passthrough_scratch_scale: 0.5,
            passthrough_behind_opaque: false,
            mirror: true,
            mirror_fps: 60,
            eye_tracking_fps: 36,
        }
    }
}

/// A settings value that failed validation.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsError {
    /// `passthrough_scratch_scale` outside 0.0–1.0.
    ScratchScale(f32),
    /// `mirror_fps` is zero.
    MirrorFps(u32),
    /// `eye_tracking_fps` outside 15–90.
    EyeTrackingFps(u32),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::ScratchScale(v) => {
                write!(f, "passthrough_scratch_scale {v} outside 0.0-1.0")
            }
            SettingsError::MirrorFps(v) => write!(f, "mirror_fps {v} must be at least 1"),
            SettingsError::EyeTrackingFps(v) => write!(f, "eye_tracking_fps {v} outside 15-90"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl RuntimeSettings {
    /// Validate a settings value read from a file or the environment.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !(0.0..=1.0).contains(&self.passthrough_scratch_scale) {
            return Err(SettingsError::ScratchScale(self.passthrough_scratch_scale));
        }
        if self.mirror_fps == 0 {
            return Err(SettingsError::MirrorFps(self.mirror_fps));
        }
        if !(15..=90).contains(&self.eye_tracking_fps) {
            return Err(SettingsError::EyeTrackingFps(self.eye_tracking_fps));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_values() {
        let s = RuntimeSettings::default();
        assert_eq!(s.passthrough_scratch_scale, 0.5);
        assert!(!s.passthrough_behind_opaque);
        assert!(s.mirror);
        assert_eq!(s.mirror_fps, 60);
        assert_eq!(s.eye_tracking_fps, 36);
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn validation_accepts_the_documented_ranges() {
        let s = RuntimeSettings {
            passthrough_scratch_scale: 0.0,
            mirror_fps: 1,
            eye_tracking_fps: 15,
            ..RuntimeSettings::default()
        };
        assert_eq!(s.validate(), Ok(()));
        let s = RuntimeSettings {
            passthrough_scratch_scale: 1.0,
            mirror_fps: u32::MAX,
            eye_tracking_fps: 90,
            ..RuntimeSettings::default()
        };
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        let s = RuntimeSettings {
            passthrough_scratch_scale: 1.5,
            ..RuntimeSettings::default()
        };
        assert_eq!(s.validate(), Err(SettingsError::ScratchScale(1.5)));
        assert_eq!(
            s.validate().unwrap_err().to_string(),
            "passthrough_scratch_scale 1.5 outside 0.0-1.0"
        );

        let s = RuntimeSettings {
            passthrough_scratch_scale: -0.1,
            ..RuntimeSettings::default()
        };
        assert_eq!(s.validate(), Err(SettingsError::ScratchScale(-0.1)));

        let s = RuntimeSettings {
            mirror_fps: 0,
            ..RuntimeSettings::default()
        };
        assert_eq!(s.validate(), Err(SettingsError::MirrorFps(0)));
        assert_eq!(
            s.validate().unwrap_err().to_string(),
            "mirror_fps 0 must be at least 1"
        );

        let s = RuntimeSettings {
            eye_tracking_fps: 14,
            ..RuntimeSettings::default()
        };
        assert_eq!(s.validate(), Err(SettingsError::EyeTrackingFps(14)));
        assert_eq!(
            s.validate().unwrap_err().to_string(),
            "eye_tracking_fps 14 outside 15-90"
        );

        let s = RuntimeSettings {
            eye_tracking_fps: 91,
            ..RuntimeSettings::default()
        };
        assert_eq!(s.validate(), Err(SettingsError::EyeTrackingFps(91)));
    }
}
