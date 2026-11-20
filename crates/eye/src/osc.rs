//! Minimal OSC encoding for VRChat's documented native tracking messages.
//!
//! OSC is encoded here directly so eye-gaze data can be tested without a
//! socket or third-party crate. The OSC packet layout is the standard
//! padded address string, padded type-tag string, then big-endian values.

use crate::EyeSample;

/// VRChat's gaze range in degrees.
pub const GAZE_LIMIT_DEG: f32 = 45.0;

/// Normalized eye expressions for one face-tracking packet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceExpressions {
    /// Left eyelid closure (0 open, 1 closed).
    pub left_closed: f32,
    /// Right eyelid closure (0 open, 1 closed).
    pub right_closed: f32,
    /// Left eye widen (0 neutral, 1 maximally wide).
    pub left_wide: f32,
    /// Right eye widen (0 neutral, 1 maximally wide).
    pub right_wide: f32,
    /// Left pupil dilation (0 neutral, 1 maximally dilated).
    pub left_dilation: f32,
    /// Right pupil dilation (0 neutral, 1 maximally dilated).
    pub right_dilation: f32,
}

impl Default for FaceExpressions {
    fn default() -> Self {
        FaceExpressions {
            left_closed: 0.0,
            right_closed: 0.0,
            left_wide: 0.0,
            right_wide: 0.0,
            left_dilation: 0.0,
            right_dilation: 0.0,
        }
    }
}

/// Clamp degrees and convert to VRChat's normalized [-1, 1] range.
pub fn normalized_gaze(sample: EyeSample) -> (f32, f32) {
    (
        sample.yaw_deg.clamp(-GAZE_LIMIT_DEG, GAZE_LIMIT_DEG) / GAZE_LIMIT_DEG,
        sample.pitch_deg.clamp(-GAZE_LIMIT_DEG, GAZE_LIMIT_DEG) / GAZE_LIMIT_DEG,
    )
}

/// Build an OSC message containing the VRChat `EyeLook*`, `EyeClosed*`,
/// `EyeWide*`, and pupil-dilation keys inside `/tracking/face/v1`.
///
/// Eye-look values use the binocular mean and are clamped to ±45 degrees
/// before normalization. The caller supplies normalized expressions;
/// values outside [0, 1] are clamped.
pub fn encode_face_v1(
    left_gaze: EyeSample,
    right_gaze: EyeSample,
    expressions: FaceExpressions,
) -> Vec<u8> {
    let (left_yaw, left_pitch) = normalized_gaze(left_gaze);
    let (right_yaw, right_pitch) = normalized_gaze(right_gaze);
    let yaw = (left_yaw + right_yaw) / 2.0;
    let pitch = (left_pitch + right_pitch) / 2.0;
    let address = "/tracking/face/v1";
    let names = [
        "EyeLookLeft",
        "EyeLookRight",
        "EyeLookUp",
        "EyeLookDown",
        "EyeClosedLeft",
        "EyeClosedRight",
        "EyeWideLeft",
        "EyeWideRight",
        "EyeDilationLeft",
        "EyeDilationRight",
        "EyeConstrictLeft",
        "EyeConstrictRight",
    ];
    let mut packet = Vec::with_capacity(256);
    append_string(&mut packet, address);
    let type_tags = format!(",{}{}", "s".repeat(names.len()), "f".repeat(names.len()));
    append_string(&mut packet, &type_tags);
    for name in names {
        append_string(&mut packet, name);
    }
    let floats = [
        (-yaw).max(0.0),
        yaw.max(0.0),
        pitch.max(0.0),
        (-pitch).max(0.0),
        unit(expressions.left_closed),
        unit(expressions.right_closed),
        unit(expressions.left_wide),
        unit(expressions.right_wide),
        unit(expressions.left_dilation),
        unit(expressions.right_dilation),
        1.0 - unit(expressions.left_dilation),
        1.0 - unit(expressions.right_dilation),
    ];
    for value in floats {
        packet.extend_from_slice(&value.to_bits().to_be_bytes());
    }
    packet
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn append_string(packet: &mut Vec<u8>, value: &str) {
    packet.extend_from_slice(value.as_bytes());
    packet.push(0);
    while !packet.len().is_multiple_of(4) {
        packet.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_string(packet: &[u8], cursor: &mut usize) -> String {
        let start = *cursor;
        let end = packet[start..]
            .iter()
            .position(|b| *b == 0)
            .map(|n| start + n)
            .unwrap();
        let value = std::str::from_utf8(&packet[start..end])
            .unwrap()
            .to_string();
        *cursor = (end + 4) & !3;
        value
    }

    #[test]
    fn default_face_expressions_are_neutral() {
        assert_eq!(
            FaceExpressions::default(),
            FaceExpressions {
                left_closed: 0.0,
                right_closed: 0.0,
                left_wide: 0.0,
                right_wide: 0.0,
                left_dilation: 0.0,
                right_dilation: 0.0,
            }
        );
    }

    #[test]
    fn gaze_is_clamped_and_normalized() {
        assert_eq!(
            normalized_gaze(EyeSample {
                yaw_deg: 90.0,
                pitch_deg: -22.5
            }),
            (1.0, -0.5)
        );
        assert_eq!(
            normalized_gaze(EyeSample {
                yaw_deg: -45.0,
                pitch_deg: 45.0
            }),
            (-1.0, 1.0)
        );
    }

    #[test]
    fn osc_face_packet_has_standard_string_layout_and_values() {
        let packet = encode_face_v1(
            EyeSample {
                yaw_deg: -22.5,
                pitch_deg: 45.0,
            },
            EyeSample {
                yaw_deg: 90.0,
                pitch_deg: -90.0,
            },
            FaceExpressions {
                left_closed: 0.25,
                right_closed: 0.5,
                left_wide: 0.75,
                right_wide: 1.0,
                left_dilation: 0.4,
                right_dilation: 0.7,
            },
        );
        let mut cursor = 0;
        assert_eq!(read_string(&packet, &mut cursor), "/tracking/face/v1");
        assert_eq!(
            read_string(&packet, &mut cursor),
            ",ssssssssssssffffffffffff"
        );
        for name in [
            "EyeLookLeft",
            "EyeLookRight",
            "EyeLookUp",
            "EyeLookDown",
            "EyeClosedLeft",
            "EyeClosedRight",
            "EyeWideLeft",
            "EyeWideRight",
            "EyeDilationLeft",
            "EyeDilationRight",
            "EyeConstrictLeft",
            "EyeConstrictRight",
        ] {
            assert_eq!(read_string(&packet, &mut cursor), name);
        }
        let mut values = Vec::new();
        let (chunks, remainder) = packet[cursor..].as_chunks::<4>();
        assert!(remainder.is_empty());
        for bytes in chunks {
            values.push(f32::from_bits(u32::from_be_bytes(*bytes)));
        }
        assert_eq!(values.len(), 12);
        assert_eq!(values[0], 0.0);
        assert_eq!(values[1], 0.25); // binocular mean yaw is +11.25°
        assert_eq!(values[2], 0.0); // binocular mean pitch is 0°
        assert_eq!(values[3], 0.0);
        assert_eq!(values[4], 0.25);
        assert_eq!(values[5], 0.5);
        assert_eq!(values[6], 0.75);
        assert_eq!(values[7], 1.0);
        assert_eq!(values[8], 0.4);
        assert_eq!(values[9], 0.7);
        assert!((values[10] - 0.6).abs() < 1e-6);
        assert!((values[11] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn non_finite_and_out_of_range_expression_inputs_are_safe() {
        let packet = encode_face_v1(
            EyeSample {
                yaw_deg: f32::NAN,
                pitch_deg: f32::INFINITY,
            },
            EyeSample {
                yaw_deg: f32::NEG_INFINITY,
                pitch_deg: 0.0,
            },
            FaceExpressions {
                left_closed: f32::NAN,
                right_closed: -1.0,
                left_wide: 2.0,
                right_wide: f32::INFINITY,
                left_dilation: -1.0,
                right_dilation: f32::NAN,
            },
        );
        // Strings occupy aligned bytes, followed by 12 big-endian floats.
        let float_start = packet.len() - 48;
        let (chunks, remainder) = packet[float_start..].as_chunks::<4>();
        assert!(remainder.is_empty());
        let values: Vec<f32> = chunks
            .iter()
            .map(|bytes| f32::from_bits(u32::from_be_bytes(*bytes)))
            .collect();
        assert_eq!(values[4], 0.0);
        assert_eq!(values[5], 0.0);
        assert_eq!(values[6], 1.0);
        assert_eq!(values[7], 0.0);
        assert_eq!(values[8], 0.0);
        assert_eq!(values[9], 0.0);
    }
}
