//! GLSL → SPIR-V translation for the ported cases, at runtime, in pure Rust.
//!
//! Upstream ships prebuilt `.spv` files next to the binaries; vivelite keeps
//! the GLSL in-tree (auditable, diffable) and translates with `naga`, so no
//! glslang/glslc toolchain is needed on the host or the device.

use std::fmt;

use naga::back::spv::{self, Options as SpvOptions, PipelineOptions};
use naga::front::glsl::{Frontend, Options};
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use naga::{Module, ShaderStage};

use crate::case::Case;

/// Why a case could not be turned into SPIR-V.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslateError {
    /// The GLSL source does not parse.
    Parse {
        /// The case that failed.
        case: &'static str,
        /// The parser errors, rendered.
        source: String,
    },
    /// The parsed module is not valid IR.
    Validate {
        /// The case that failed.
        case: &'static str,
        /// The validation errors, rendered.
        source: String,
    },
    /// SPIR-V generation failed for the module.
    Write {
        /// The case that failed.
        case: &'static str,
        /// The writer error, rendered.
        source: String,
    },
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranslateError::Parse { case, source } => {
                write!(f, "GLSL parse failed for {case}: {source}")
            }
            TranslateError::Validate { case, source } => {
                write!(f, "IR validation failed for {case}: {source}")
            }
            TranslateError::Write { case, source } => {
                write!(f, "SPIR-V generation failed for {case}: {source}")
            }
        }
    }
}

impl std::error::Error for TranslateError {}

/// Translates one case's GLSL into SPIR-V words, ready for
/// `VkShaderModuleCreateInfo::code`.
///
/// # Errors
///
/// Any of parse, validation, or SPIR-V generation failure; see
/// [`TranslateError`].
pub fn translate(case: &Case) -> Result<Vec<u32>, TranslateError> {
    let module = parse(case)?;
    let info = validate(case, &module)?;
    write(case, &module, &info, "main")
}

/// Parses the case's GLSL into a `naga` module for the compute stage.
fn parse(case: &Case) -> Result<Module, TranslateError> {
    let options = Options::from(ShaderStage::Compute);
    Frontend::default()
        .parse(&options, case.shader)
        .map_err(|e| TranslateError::Parse {
            case: case.name,
            source: e.to_string(),
        })
}

/// Validates the module.
///
/// The allowed capabilities exclude f64 and friends: the HCF shaders are
/// plain uint/float compute with SSBOs, so anything beyond that set is a
/// case-authoring bug worth failing loudly on.
fn validate(case: &Case, module: &Module) -> Result<ModuleInfo, TranslateError> {
    let allowed = Capabilities::all() - Capabilities::FLOAT64;
    Validator::new(ValidationFlags::all(), allowed)
        .validate(module)
        .map_err(|e| TranslateError::Validate {
            case: case.name,
            source: e.to_string(),
        })
}

/// Generates SPIR-V 1.0 for the module's `entry_point`.
fn write(
    case: &Case,
    module: &Module,
    info: &ModuleInfo,
    entry_point: &str,
) -> Result<Vec<u32>, TranslateError> {
    let options = SpvOptions {
        lang_version: (1, 0),
        ..Default::default()
    };
    let pipeline = PipelineOptions {
        shader_stage: ShaderStage::Compute,
        entry_point: entry_point.to_string(),
    };
    spv::write_vec(module, info, &options, Some(&pipeline)).map_err(|e| TranslateError::Write {
        case: case.name,
        source: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPIR-V magic number, little-endian words: 0x0723_0203.
    const MAGIC: u32 = 0x0723_0203;

    #[test]
    fn every_case_translates_to_valid_spirv_1_0() {
        for case in crate::case::cases() {
            let words = translate(case).expect(case.name);
            assert_eq!(words[0], MAGIC, "case {}", case.name);
            assert_eq!(words[1] >> 16, 1, "case {}: version", case.name);
            assert!(words.len() > 20, "case {}: suspiciously small", case.name);
        }
    }

    #[test]
    fn broken_glsl_reports_a_parse_error() {
        let case = Case {
            name: "broken",
            summary: "test fixture",
            shader: "#version 450\nvoid main() {",
            init: crate::case::BufferInit::Default,
            recipe: crate::case::Recipe::InfiniteLoop,
        };
        let err = translate(&case).expect_err("broken GLSL must not translate");
        assert!(
            err.to_string().starts_with("GLSL parse failed for broken"),
            "{err}"
        );
    }

    #[test]
    fn double_precision_reports_a_validation_error() {
        // `double` parses fine but the IR needs the FLOAT64 capability,
        // which the validator deliberately does not allow.
        let case = Case {
            name: "doubled",
            summary: "test fixture",
            shader: "\
#version 450
layout(local_size_x = 1) in;
void main() { double d = double(1.0); d += 1.0; }
",
            init: crate::case::BufferInit::Default,
            recipe: crate::case::Recipe::InfiniteLoop,
        };
        let err = translate(&case).expect_err("double precision must not translate");
        assert!(
            err.to_string()
                .starts_with("IR validation failed for doubled"),
            "{err}"
        );
    }

    #[test]
    fn a_missing_entry_point_reports_a_write_error() {
        let case = crate::case::case("hang_infinite_loop").unwrap();
        let module = parse(case).expect("fixture parses");
        let info = validate(case, &module).expect("fixture validates");
        let err = write(case, &module, &info, "definitely-not-main")
            .expect_err("a missing entry point must not write");
        assert!(
            err.to_string()
                .starts_with("SPIR-V generation failed for hang_infinite_loop"),
            "{err}"
        );
    }

    #[test]
    fn all_error_variants_render_their_case_name() {
        let variants = [
            TranslateError::Parse {
                case: "c",
                source: "s".into(),
            },
            TranslateError::Validate {
                case: "c",
                source: "s".into(),
            },
            TranslateError::Write {
                case: "c",
                source: "s".into(),
            },
        ];
        assert_eq!(variants.len(), 3, "all variants rendered");
        for v in &variants {
            let text = v.to_string();
            assert!(text.contains("c"), "{text}");
        }
    }
}
