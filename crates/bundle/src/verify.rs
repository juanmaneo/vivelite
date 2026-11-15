//! Checksum and signature verification.
//!
//! sha256 is computed in Rust (`sha2`). The CMS signature on a RAUC bundle is
//! verified by shelling out to `rauc` (preferred) or `openssl cms` — both are
//! C tools, wrapped here behind a trait, consistent with the project rule:
//! crypto primitives stay C, orchestration is Rust.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Lowercase hex encoding (std still lacks a stable one for slices).
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

/// Compute the sha256 of a file, streaming.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// In-memory sha256 helper, used by tests and small files.
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// Verify a file against an expected lowercase-hex sha256.
pub fn verify_file_sha256(path: &Path, expected: &str) -> Result<()> {
    let expected_norm = expected.trim().to_lowercase();
    let actual = sha256_file(path)?;
    if actual == expected_norm {
        Ok(())
    } else {
        Err(Error::ChecksumMismatch {
            expected: expected_norm,
            actual,
        })
    }
}

/// How to verify the bundle's CMS signature.
pub trait SignatureVerifier {
    /// Verify the RAUC bundle at `bundle`. Returns `Ok(())` when the
    /// signature is valid and chains to a trusted root.
    fn verify(&self, bundle: &Path) -> Result<()>;
}

/// Shells out to `rauc` — the reference implementation, C.
#[derive(Debug, Clone, Default)]
pub struct RaucVerifier {
    /// Path to the trusted RAUC keyring. Verification fails closed when unset.
    pub keyring: Option<String>,
    /// Path to the `rauc` binary; defaults to `rauc` on PATH.
    pub rauc_bin: Option<String>,
}

impl RaucVerifier {
    fn cmd(&self) -> Command {
        Command::new(self.rauc_bin.as_deref().unwrap_or("rauc"))
    }
}

impl SignatureVerifier for RaucVerifier {
    fn verify(&self, bundle: &Path) -> Result<()> {
        let keyring = self.keyring.as_deref().ok_or_else(|| {
            Error::Other("RAUC verification requires an explicit trusted keyring".into())
        })?;
        let out = self
            .cmd()
            .arg("info")
            .arg("--output-format=json")
            .arg("--keyring")
            .arg(keyring)
            .arg(bundle)
            .output()
            .map_err(|e| Error::Tool {
                tool: "rauc",
                status: None,
                stderr: format!("could not run rauc: {e}"),
            })?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Error::Tool {
                tool: "rauc",
                status: Some(out.status),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        }
    }
}

/// Fallback when `rauc` is not available: `openssl cms -verify`.
#[derive(Debug, Clone, Default)]
pub struct OpensslVerifier {
    /// Path to the trusted CA bundle (Valve's public certificate).
    pub ca_file: Option<String>,
    /// Path to `openssl`; defaults to `openssl` on PATH.
    pub openssl_bin: Option<String>,
}

impl SignatureVerifier for OpensslVerifier {
    fn verify(&self, bundle: &Path) -> Result<()> {
        let mut cmd = Command::new(self.openssl_bin.as_deref().unwrap_or("openssl"));
        cmd.arg("cms")
            .arg("-verify")
            .arg("-binary")
            .arg("-in")
            .arg(bundle);
        let ca_file = self.ca_file.as_deref().ok_or_else(|| {
            Error::Other("OpenSSL verification requires an explicit trusted CA file".into())
        })?;
        cmd.arg("-CAfile").arg(ca_file);
        let out = cmd.output().map_err(|e| Error::Tool {
            tool: "openssl",
            status: None,
            stderr: format!("could not run openssl: {e}"),
        })?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Error::Tool {
                tool: "openssl",
                status: Some(out.status),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn known_sha256_vector() {
        // Well-known NIST vector for "abc".
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hex_is_lowercase_pairs() {
        assert_eq!(hex(&[0x00, 0xff, 0x0a]), "00ff0a");
    }

    #[test]
    fn verify_roundtrip_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        std::fs::write(&p, b"steamos").unwrap();
        let sum = sha256_file(&p).unwrap();
        assert!(verify_file_sha256(&p, &sum).is_ok());
        assert!(
            verify_file_sha256(&p, &sum.to_uppercase()).is_ok(),
            "case-insensitive"
        );
        assert!(verify_file_sha256(&p, "00").is_err());
        assert!(sha256_file(&dir.path().join("missing")).is_err());
    }

    fn fake_verifier_tool(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho \"$@\" >> '{}.log'\n{body}", path.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn rauc_verifier_success_failure_and_spawn_error() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle.raucb");
        std::fs::write(&bundle, b"signed bundle").unwrap();
        let success = fake_verifier_tool(dir.path(), "rauc-ok", "exit 0");
        RaucVerifier {
            keyring: Some("/etc/rauc/valve-ca.pem".into()),
            rauc_bin: Some(success.to_string_lossy().into_owned()),
        }
        .verify(&bundle)
        .unwrap();
        let calls = std::fs::read_to_string(dir.path().join("rauc-ok.log")).unwrap();
        assert!(
            calls.contains("info --output-format=json --keyring /etc/rauc/valve-ca.pem"),
            "{calls}"
        );
        assert!(calls.contains("bundle.raucb"), "{calls}");

        let failure = fake_verifier_tool(dir.path(), "rauc-fail", "echo invalid >&2; exit 2");
        let err = RaucVerifier {
            keyring: Some("trusted.pem".into()),
            rauc_bin: Some(failure.to_string_lossy().into_owned()),
        }
        .verify(&bundle)
        .unwrap_err();
        assert!(err.to_string().contains("invalid"), "{err}");

        let err = RaucVerifier {
            keyring: Some("trusted.pem".into()),
            rauc_bin: Some("/nonexistent/rauc".into()),
        }
        .verify(&bundle)
        .unwrap_err();
        assert!(err.to_string().contains("could not run rauc"), "{err}");

        let err = RaucVerifier::default().verify(&bundle).unwrap_err();
        assert!(
            err.to_string().contains("explicit trusted keyring"),
            "{err}"
        );
    }

    #[test]
    fn openssl_verifier_success_ca_file_failure_and_spawn_error() {
        let _guard = crate::spawn_lock();
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle.raucb");
        std::fs::write(&bundle, b"signed bundle").unwrap();

        let missing_ca = OpensslVerifier {
            ca_file: None,
            openssl_bin: Some("openssl".into()),
        }
        .verify(&bundle)
        .unwrap_err();
        assert!(
            missing_ca.to_string().contains("explicit trusted CA"),
            "{missing_ca}"
        );

        let cafile = fake_verifier_tool(dir.path(), "openssl-cafile", "exit 0");
        OpensslVerifier {
            ca_file: Some("/etc/ssl/valve-ca.pem".into()),
            openssl_bin: Some(cafile.to_string_lossy().into_owned()),
        }
        .verify(&bundle)
        .unwrap();
        let calls = std::fs::read_to_string(dir.path().join("openssl-cafile.log")).unwrap();
        assert!(calls.contains("-CAfile /etc/ssl/valve-ca.pem"), "{calls}");

        let failure = fake_verifier_tool(dir.path(), "openssl-fail", "echo bad-cms >&2; exit 2");
        let err = OpensslVerifier {
            ca_file: Some("trusted.pem".into()),
            openssl_bin: Some(failure.to_string_lossy().into_owned()),
        }
        .verify(&bundle)
        .unwrap_err();
        assert!(err.to_string().contains("bad-cms"), "{err}");

        let err = OpensslVerifier {
            ca_file: Some("trusted.pem".into()),
            openssl_bin: Some("/nonexistent/openssl".into()),
        }
        .verify(&bundle)
        .unwrap_err();
        assert!(err.to_string().contains("could not run openssl"), "{err}");
    }

    #[test]
    fn sha256_file_reports_read_errors() {
        let dir = tempfile::tempdir().unwrap();
        // Reading a directory fails after `open` succeeds on Linux.
        assert!(sha256_file(dir.path()).is_err());
        assert!(verify_file_sha256(dir.path(), "00").is_err());
    }
}
