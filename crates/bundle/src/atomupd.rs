//! Valve's atomupd release metadata for the aarch64 VR ("deckard") SteamOS.
//!
//! Verified against the live service on 2026-11-15. The update feed is a small
//! JSON document; the latest stable entry pointed at build `20261022.6150929`
//! (SteamOS 0.4.5) with:
//!
//! - `update_path` = `vr/20261022.6150929/deckard-stable-20261022.6150929-0.4.5.raucb`
//! - `chunks_store_path` = `vr/chunks.castr`
//!
//! The `.raucb` is a *signed squashfs of ~2.2 MiB*: it carries `manifest.raucm`
//! (compatible=steamos-aarch64, one `[image.rootfs]` with a 10 GiB slot size)
//! and `rootfs.img.caibx`, a casync chunk index. The actual rootfs bytes are
//! **not** in the bundle: they are fetched from casync chunk stores (see
//! [`chunk_stores`]) and reassembled by desync — exactly how the steamac
//! project provisions its disks, and how we stage userspace under proot.
//!
//! We deliberately parse the JSON with a tiny hand-rolled scanner: the document
//! shape is fixed by Valve, and the crate keeps its dependency surface at
//! `sha2` only so it cross-compiles to Android without a serde tree.

use crate::{Error, Result};

/// Base URL of the atomupd metadata service (aarch64, VR variant).
pub const META_BASE: &str = "https://steamdeck-atomupd.steamos.cloud/meta/holo/steamos/aarch64/vr/";

/// Base URL of the image and chunk-store CDN.
pub const IMAGES_BASE: &str = "https://steamdeck-images.steamos.cloud/";

/// A release candidate from the atomupd feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Valve build id, e.g. `20261022.6150929`; also the manifest `version`.
    pub buildid: String,
    /// Human version, e.g. `0.4.5`.
    pub version: String,
    /// Bundle path relative to [`IMAGES_BASE`], e.g. `vr/<buildid>/deckard-stable-…raucb`.
    pub update_path: String,
    /// Shared chunk store path relative to [`IMAGES_BASE`], e.g. `vr/chunks.castr`.
    pub chunks_store_path: String,
}

impl Candidate {
    /// Absolute URL of the signed `.raucb` bundle.
    pub fn bundle_url(&self) -> String {
        format!("{}{}", IMAGES_BASE, self.update_path)
    }

    /// The two casync chunk stores holding the rootfs chunks, in the order
    /// steamac passes them to desync: the per-image store first (denser,
    /// contains everything of this build), the shared `chunks.castr` second.
    pub fn chunk_stores(&self) -> Vec<String> {
        let stem = self
            .update_path
            .strip_suffix(".raucb")
            .unwrap_or(&self.update_path);
        vec![
            format!("{IMAGES_BASE}{stem}.castr/"),
            format!("{}{}/", IMAGES_BASE, self.chunks_store_path),
        ]
    }

    /// File name of the bundle on disk.
    pub fn bundle_filename(&self) -> String {
        self.update_path
            .rsplit('/')
            .next()
            .unwrap_or("steamos.raucb")
            .to_string()
    }
}

/// URL of the metadata document for a branch (`stable`, `beta`, …).
pub fn meta_url(branch: &str) -> String {
    format!("{META_BASE}{branch}.json")
}

/// Parse the candidates out of an atomupd metadata document.
///
/// Only `minor.candidates` is considered: XR staging wants the latest full
/// image, never a delta.
pub fn parse_candidates(json: &str) -> Result<Vec<Candidate>> {
    let Some(cands_start) = json.find("\"candidates\"") else {
        return Err(Error::Other("atomupd meta: no candidates array".into()));
    };
    let body = &json[cands_start..];
    let Some(arr) = body.find('[') else {
        return Err(Error::Other(
            "atomupd meta: candidates is not an array".into(),
        ));
    };
    let body = &body[arr + 1..];
    let Some(end) = body.rfind(']') else {
        return Err(Error::Other(
            "atomupd meta: unterminated candidates array".into(),
        ));
    };
    let body = &body[..end];

    let mut out = Vec::new();
    for (start, obj) in objects(body) {
        let get = |key: &str| json_string_field(obj, key);
        let (Some(buildid), Some(update_path)) = (get("buildid"), get("update_path")) else {
            return Err(Error::Other(format!(
                "atomupd meta: candidate {start} lacks buildid/update_path"
            )));
        };
        out.push(Candidate {
            buildid,
            version: get("version").unwrap_or_default(),
            update_path,
            chunks_store_path: get("chunks_store_path").unwrap_or_else(|| "vr/chunks.castr".into()),
        });
    }
    if out.is_empty() {
        return Err(Error::Other(
            "atomupd meta: candidates array is empty".into(),
        ));
    }
    Ok(out)
}

/// Byte ranges of the top-level `{ … }` objects inside `s`.
fn objects(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(st) = start {
                        out.push((st, &s[st..=i]));
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    out
}

/// Value of a `"key" : "value"` string field within one JSON object.
fn json_string_field(obj: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let i = obj.find(&pat)?;
    let rest = &obj[i + pat.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(unescape(&rest[..end]))
}

/// Resolve the minimal JSON escape set we care about.
fn unescape(s: &str) -> String {
    s.replace("\\/", "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live `stable.json` as served on 2026-11-15, verbatim.
    const LIVE: &str = r#"{
    "minor": {
        "candidates": [
            {
                "chunks_store_path": "vr/chunks.castr",
                "image": {
                    "arch": "aarch64",
                    "branch": "stable",
                    "buildid": "20261022.6150929",
                    "default_update_branch": "stable",
                    "estimated_size": 0,
                    "product": "steamos",
                    "release": "holo",
                    "variant": "vr",
                    "version": "0.4.5"
                },
                "update_path": "vr/20261022.6150929/deckard-stable-20261022.6150929-0.4.5.raucb"
            }
        ],
        "release": "holo"
    }
}"#;

    #[test]
    fn parses_the_live_feed() {
        let cands = parse_candidates(LIVE).unwrap();
        assert_eq!(cands.len(), 1);
        let c = &cands[0];
        assert_eq!(c.buildid, "20261022.6150929");
        assert_eq!(c.version, "0.4.5");
        assert_eq!(c.chunks_store_path, "vr/chunks.castr");
        assert_eq!(
            c.update_path,
            "vr/20261022.6150929/deckard-stable-20261022.6150929-0.4.5.raucb"
        );
    }

    #[test]
    fn builds_the_verified_urls() {
        let c = parse_candidates(LIVE).unwrap().remove(0);
        assert_eq!(
            c.bundle_url(),
            "https://steamdeck-images.steamos.cloud/vr/20261022.6150929/deckard-stable-20261022.6150929-0.4.5.raucb"
        );
        assert_eq!(
            c.bundle_filename(),
            "deckard-stable-20261022.6150929-0.4.5.raucb"
        );
        let stores = c.chunk_stores();
        assert_eq!(
            stores[0],
            "https://steamdeck-images.steamos.cloud/vr/20261022.6150929/deckard-stable-20261022.6150929-0.4.5.castr/"
        );
        assert_eq!(
            stores[1],
            "https://steamdeck-images.steamos.cloud/vr/chunks.castr/"
        );
    }

    #[test]
    fn meta_url_shape() {
        assert_eq!(
            meta_url("stable"),
            "https://steamdeck-atomupd.steamos.cloud/meta/holo/steamos/aarch64/vr/stable.json"
        );
    }

    #[test]
    fn rejects_documents_without_candidates() {
        assert!(parse_candidates("{\"major\": {}}").is_err());
    }

    #[test]
    fn rejects_malformed_candidates_shapes() {
        assert!(parse_candidates("{\"candidates\": null}")
            .unwrap_err()
            .to_string()
            .contains("not an array"));
        assert!(parse_candidates("{\"candidates\": [")
            .unwrap_err()
            .to_string()
            .contains("unterminated"));
        assert!(parse_candidates("{\"candidates\": []}")
            .unwrap_err()
            .to_string()
            .contains("empty"));
        assert!(parse_candidates("{\"candidates\": [{}]}")
            .unwrap_err()
            .to_string()
            .contains("lacks buildid/update_path"));
    }

    #[test]
    fn objects_scanner_tolerates_stray_closing_braces() {
        let parsed =
            parse_candidates(r#"{"candidates": [} {"buildid":"b1","update_path":"b.raucb"}] }"#)
                .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].buildid, "b1");
    }

    #[test]
    fn truncated_candidate_fields_are_rejected_not_panicked() {
        let truncated = r#"{"candidates": [{"buildid": "1", "update_path"#;
        assert!(parse_candidates(truncated).is_err());
        let no_quote = r#"{"candidates": [{"buildid": 1, "update_path": "x"}]}"#;
        assert!(parse_candidates(no_quote).is_err());
    }
}
