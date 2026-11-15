//! RAUC bundle manifest parsing.
//!
//! A RAUC bundle is a squashfs containing a `manifest.raucm` (INI-like) plus
//! slot images. We parse the manifest to learn what images exist and their
//! checksums, without needing the full librauc.

use crate::{Error, Result};

/// One `[image.<slot>]` entry from a RAUC manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSection {
    /// Slot name, e.g. `core` for `[image.core]`.
    pub slot: String,
    /// File name inside the bundle.
    pub filename: String,
    /// Expected sha256, if the manifest carries one.
    pub sha256: Option<String>,
    /// Expected size in bytes, if present.
    pub size: Option<u64>,
}

/// A parsed RAUC manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// `[update]` compatible string.
    pub compatible: Option<String>,
    /// All `[image.*]` sections.
    pub images: Vec<ImageSection>,
    /// Every other `key = value` line, grouped by section, in file order.
    pub other: Vec<(String, String, String)>,
}

/// Parse a `manifest.raucm` document.
///
/// Grammar accepted (a pragmatic subset of RAUC's):
/// sections `[name]`, entries `key = value`, `#`/`;` comments, blank lines.
pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let mut m = Manifest::default();
    let mut section = String::new();

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            let end = line
                .find(']')
                .ok_or_else(|| Error::BadManifest(format!("line {}: unterminated section", lineno + 1)))?;
            section = line[1..end].trim().to_string();
            if section.is_empty() {
                return Err(Error::BadManifest(format!("line {}: empty section name", lineno + 1)));
            }
            continue;
        }
        let eq = line
            .find('=')
            .ok_or_else(|| Error::BadManifest(format!("line {}: expected key = value", lineno + 1)))?;
        let key = line[..eq].trim();
        let value = line[eq + 1..].trim();
        if key.is_empty() {
            return Err(Error::BadManifest(format!("line {}: empty key", lineno + 1)));
        }
        if section == "update" && key == "compatible" {
            m.compatible = Some(value.to_string());
        } else if let Some(slot) = section.strip_prefix("image.") {
            // Fall through to per-image handling below.
            let slot = slot.to_string();
            match key {
                "filename" => {
                    let img = m.images.iter_mut().find(|i| i.slot == slot);
                    match img {
                        Some(i) => i.filename = value.to_string(),
                        None => m.images.push(ImageSection {
                            slot,
                            filename: value.to_string(),
                            sha256: None,
                            size: None,
                        }),
                    }
                }
                "sha256" => {
                    let img = m.images.iter_mut().find(|i| i.slot == slot);
                    match img {
                        Some(i) => i.sha256 = Some(value.to_string()),
                        None => m.images.push(ImageSection {
                            slot,
                            filename: String::new(),
                            sha256: Some(value.to_string()),
                            size: None,
                        }),
                    }
                }
                "size" => {
                    let size: u64 = value
                        .parse()
                        .map_err(|_| Error::BadManifest(format!("line {}: bad size", lineno + 1)))?;
                    let img = m.images.iter_mut().find(|i| i.slot == slot);
                    match img {
                        Some(i) => i.size = Some(size),
                        None => m.images.push(ImageSection {
                            slot,
                            filename: String::new(),
                            sha256: None,
                            size: Some(size),
                        }),
                    }
                }
                _ => m.other.push((section.clone(), key.to_string(), value.to_string())),
            }
        } else {
            m.other.push((section.clone(), key.to_string(), value.to_string()));
        }
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[update]
compatible=steamos-frame
version=3.8.11

[image.core]
filename=steamos-core.img
sha256=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef
size=1073741824

# comment
[image.boot]
filename=steamos-boot.img
size=104857600
"#;

    #[test]
    fn parses_sample_manifest() {
        let m = parse_manifest(SAMPLE).unwrap();
        assert_eq!(m.compatible.as_deref(), Some("steamos-frame"));
        assert_eq!(m.images.len(), 2);

        let core = m.images.iter().find(|i| i.slot == "core").unwrap();
        assert_eq!(core.filename, "steamos-core.img");
        assert_eq!(core.size, Some(1073741824));
        assert!(core.sha256.as_deref().unwrap().starts_with("deadbeef"));

        let boot = m.images.iter().find(|i| i.slot == "boot").unwrap();
        assert_eq!(boot.sha256, None);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_manifest("no equals sign here").is_err());
        assert!(parse_manifest("[unterminated").is_err());
        assert!(parse_manifest("[update]\ncompatible\n").is_err());
    }

    #[test]
    fn keeps_unknown_keys() {
        let m = parse_manifest("[hooks]\nfilename=post-install\n").unwrap();
        assert_eq!(m.other, vec![("hooks".into(), "filename".into(), "post-install".into())]);
    }
}
