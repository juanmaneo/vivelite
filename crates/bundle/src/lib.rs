//! Fetch, verify and extract Valve's signed ARM64 SteamOS RAUC bundle.
//!
//! Trust model (same as the steamac project): the bundle is downloaded from
//! Valve's servers, its sha256 is checked and its CMS signature is verified.
//! The root filesystem inside is treated as read-only inputs; our own layers
//! are applied on top at stage time, never by editing Valve's files.
//!
//! The exact bundle URL rotates; `xtask fetch` takes it as an argument and
//! reads it from the `VIVELITE_BUNDLE_URL` environment variable first.

pub mod atomupd;
pub mod desync;
pub mod extract;
pub mod fetch;
pub mod manifest;
pub mod verify;

pub use atomupd::{meta_url, parse_candidates, Candidate, IMAGES_BASE, META_BASE};
pub use desync::{DesyncReconstructor, ReconstructInfo, RootfsReconstructor};
pub use extract::{steamos_components, Component, Ext4Extractor, Extractor, SquashfsExtractor};
pub use fetch::{fetch_to, fetch_to_quiet, Range};
pub use manifest::{parse_manifest, ImageSection, Manifest};
pub use verify::{
    sha256_file, sha256_hex, verify_file_sha256, OpensslVerifier, RaucVerifier, SignatureVerifier,
};

/// Errors shared across the bundle pipeline.
#[derive(Debug)]
pub enum Error {
    /// I/O failure.
    Io(std::io::Error),
    /// Manifest syntax is invalid.
    BadManifest(String),
    /// A checksum did not match.
    ChecksumMismatch { expected: String, actual: String },
    /// An external tool failed (openssl, unsquashfs, rauc).
    Tool {
        tool: &'static str,
        status: std::process::ExitStatus,
        stderr: String,
    },
    /// Anything else, with context.
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::BadManifest(m) => write!(f, "bad manifest: {m}"),
            Error::ChecksumMismatch { expected, actual } => {
                write!(f, "sha256 mismatch: expected {expected}, got {actual}")
            }
            Error::Tool {
                tool,
                status,
                stderr,
            } => {
                write!(f, "{tool} failed ({status}): {stderr}")
            }
            Error::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Result alias for the pipeline.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_is_helpful() {
        let e = Error::ChecksumMismatch {
            expected: "aa".into(),
            actual: "bb".into(),
        };
        assert!(e.to_string().contains("expected aa, got bb"));
        let io = Error::Io(std::io::Error::other("disk"));
        assert!(io.to_string().contains("io error: disk"));
        let bad = Error::BadManifest("syntax".into());
        assert_eq!(bad.to_string(), "bad manifest: syntax");
        let other = Error::Other("context".into());
        assert_eq!(other.to_string(), "context");
        let tool = Error::Tool {
            tool: "rauc",
            status: std::process::Command::new("true").status().unwrap(),
            stderr: "bad signature".into(),
        };
        assert!(tool.to_string().contains("rauc failed ("));
        assert!(tool.to_string().contains("bad signature"));
    }
}
