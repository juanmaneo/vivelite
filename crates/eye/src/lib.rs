//! Eye-gaze plumbing for the vivelite overlay: calibration, smoothing,
//! and VRChat OSC encoding.
//!
//! Modeled on the behavior frame-shell documents for the Steam Frame:
//! a per-axis linear eye calibration fitted from pointer clicks
//! (`corrected = (measured - offset) / gain`), a One Euro filter between
//! the gaze and the network, gaze capped at ±45°, and the VRChat native
//! tracking OSC messages (`/tracking/eye/LeftRightPitchYaw`,
//! `/tracking/eye/EyesClosedAmount`, and a `/tracking/face/v1` blob with
//! the `EyeLook*` keys). The implementation here is written from that
//! documented behavior, not from frame-shell's GPL-3.0 code.
//!
//! Everything in this crate is pure data: a sender would live with the
//! process that owns the socket.

pub mod calibration;
pub mod filter;
pub mod osc;

pub use calibration::{Calibration, EyeSample};
pub use filter::OneEuro;
pub use osc::{encode_face_v1, FaceExpressions};
