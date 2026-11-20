//! Versioned persistence for eye-gaze calibration.

use std::fs;
use std::io;
use std::path::Path;

use crate::Calibration;

const HEADER: &str = "vivelite-eye-calibration-v1";

/// Save calibration as a small, versioned UTF-8 text file.
///
/// Offsets and gains must be finite, and gains must be non-zero. The file is
/// written directly to `path`; callers that need crash-safe replacement
/// should save to an application-managed temporary path and rename it.
pub fn save(path: &Path, calibration: Calibration) -> io::Result<()> {
    validate(calibration)?;
    let contents = format!(
        "{HEADER}\nyaw_offset={}\nyaw_gain={}\npitch_offset={}\npitch_gain={}\n",
        calibration.yaw_offset,
        calibration.yaw_gain,
        calibration.pitch_offset,
        calibration.pitch_gain
    );
    fs::write(path, contents)
}

/// Load calibration from the versioned format written by [`save`].
pub fn load(path: &Path) -> io::Result<Calibration> {
    let contents = fs::read_to_string(path)?;
    let mut lines = contents.lines();
    if lines.next() != Some(HEADER) {
        return Err(invalid_data("unsupported eye calibration format"));
    }

    let calibration = Calibration {
        yaw_offset: field(&mut lines, "yaw_offset")?,
        yaw_gain: field(&mut lines, "yaw_gain")?,
        pitch_offset: field(&mut lines, "pitch_offset")?,
        pitch_gain: field(&mut lines, "pitch_gain")?,
    };
    if lines.next().is_some() {
        return Err(invalid_data("unexpected trailing calibration data"));
    }
    validate(calibration)?;
    Ok(calibration)
}

fn field(lines: &mut std::str::Lines<'_>, expected: &str) -> io::Result<f32> {
    let line = lines
        .next()
        .ok_or_else(|| invalid_data("missing eye calibration field"))?;
    let (key, value) = line
        .split_once('=')
        .ok_or_else(|| invalid_data("malformed eye calibration field"))?;
    if key != expected {
        return Err(invalid_data("unexpected eye calibration field"));
    }
    value
        .parse::<f32>()
        .map_err(|_| invalid_data("invalid eye calibration number"))
}

fn validate(calibration: Calibration) -> io::Result<()> {
    let values = [
        calibration.yaw_offset,
        calibration.yaw_gain,
        calibration.pitch_offset,
        calibration.pitch_gain,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(invalid_data("eye calibration values must be finite"));
    }
    if calibration.yaw_gain == 0.0 || calibration.pitch_gain == 0.0 {
        return Err(invalid_data("eye calibration gains must be non-zero"));
    }
    Ok(())
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_calibration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("eye-calibration.conf");
        let calibration = Calibration {
            yaw_offset: 2.5,
            yaw_gain: 1.25,
            pitch_offset: -4.0,
            pitch_gain: 0.75,
        };
        save(&path, calibration).unwrap();
        assert_eq!(load(&path).unwrap(), calibration);
    }

    #[test]
    fn rejects_invalid_calibration_when_saving() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("eye-calibration.conf");
        for invalid in [
            Calibration {
                yaw_offset: f32::NAN,
                ..Calibration::default()
            },
            Calibration {
                pitch_gain: f32::INFINITY,
                ..Calibration::default()
            },
            Calibration {
                yaw_gain: 0.0,
                ..Calibration::default()
            },
            Calibration {
                pitch_gain: -0.0,
                ..Calibration::default()
            },
        ] {
            assert_eq!(
                save(&path, invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn rejects_bad_headers_fields_numbers_values_and_trailing_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("eye-calibration.conf");
        for invalid in [
            "wrong-version\n",
            "vivelite-eye-calibration-v1\nyaw_offset=1\n",
            "vivelite-eye-calibration-v1\nyaw-offset=1\nyaw_gain=1\npitch_offset=0\npitch_gain=1\n",
            "vivelite-eye-calibration-v1\nyaw_offset\nyaw_gain=1\npitch_offset=0\npitch_gain=1\n",
            "vivelite-eye-calibration-v1\nyaw_offset=nope\nyaw_gain=1\npitch_offset=0\npitch_gain=1\n",
            "vivelite-eye-calibration-v1\nyaw_offset=0\nyaw_gain=1\npitch_offset=nope\npitch_gain=1\n",
            "vivelite-eye-calibration-v1\nyaw_offset=0\nyaw_gain=1\npitch_offset=0\npitch_gain=nope\n",
            "vivelite-eye-calibration-v1\nyaw_offset=NaN\nyaw_gain=1\npitch_offset=0\npitch_gain=1\n",
            "vivelite-eye-calibration-v1\nyaw_offset=0\nyaw_gain=0\npitch_offset=0\npitch_gain=1\n",
            "vivelite-eye-calibration-v1\nyaw_offset=0\nyaw_gain=1\npitch_offset=0\npitch_gain=1\nextra\n",
        ] {
            fs::write(&path, invalid).unwrap();
            assert_eq!(load(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn propagates_missing_file_errors() {
        let directory = tempfile::tempdir().unwrap();
        let error = load(&directory.path().join("missing.conf")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
