/// XML schema validation via the system xmllint tool.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{Code, Note, Severity};

/// Pick the XSD to validate against from the document's root element and
/// standard (Interop docs carry the digicine.com namespace). Mirrors the
/// namespace->schema mapping in ClairMeta's XML catalog. `None` if the file is
/// not a CPL/PKL/ASSETMAP we schema-check.
fn schema_file_for(content: &str) -> Option<&'static str> {
    let interop = content.contains("digicine.com");
    // Key off the root element tag. AssetMap must be checked first: a SMPTE
    // ASSETMAP carries a <PackingList>true</PackingList> boolean per asset, so a
    // bare "PackingList" substring would mis-route it to the PKL schema.
    if content.contains("<AssetMap") {
        Some(if interop {
            "PROTO-ASDCP-AM-20040311.xsd"
        } else {
            "SMPTE-429-9-2007-AM.xsd"
        })
    } else if content.contains("<CompositionPlaylist") {
        // The 429-16 metadata schema extends 429-7 and is what ClairMeta uses.
        Some(if interop {
            "PROTO-ASDCP-CPL-20040511.xsd"
        } else {
            "SMPTE-429-16-2014-CPL-Metadata.xsd"
        })
    } else if content.contains("<PackingList") {
        Some(if interop {
            "PROTO-ASDCP-PKL-20040311.xsd"
        } else {
            "SMPTE-429-8-2006-PKL.xsd"
        })
    } else {
        None
    }
}

/// Locate a directory of SMPTE/Interop XSDs. Schema-path driven: the
/// `DCPDOCTOR_SCHEMA_DIR` env override wins, else a bundled `schemas/` dir next
/// to the executable or in the source tree. `None` means schema checks are
/// skipped (the schemas are not vendored, since SMPTE XSDs are copyrighted).
pub fn locate_schema_dir() -> Option<PathBuf> {
    let has_xsd = |dir: &Path| {
        std::fs::read_dir(dir).ok().is_some_and(|mut entries| {
            entries.any(|e| {
                e.ok().is_some_and(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("xsd"))
                })
            })
        })
    };

    if let Ok(dir) = std::env::var("DCPDOCTOR_SCHEMA_DIR") {
        let p = PathBuf::from(dir);
        if has_xsd(&p) {
            return Some(p);
        }
    }

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("schemas"));
        candidates.push(exe_dir.join("../schemas"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../schemas"));

    candidates.into_iter().find(|p| has_xsd(p))
}

/// Schema-validate a single CPL/PKL/ASSETMAP against the XSDs in `schema_dir`,
/// emitting [`Code::XmlSchemaViolation`] for each violation. Returns empty when
/// the file is not schema-checkable, its schema is absent, or xmllint is not
/// installed (schema validation is best-effort and never a hard dependency).
pub fn check_schema(xml_file: &Path, schema_dir: &Path) -> Vec<Note> {
    let Ok(content) = std::fs::read_to_string(xml_file) else {
        return Vec::new();
    };
    let Some(schema_file) = schema_file_for(&content) else {
        return Vec::new();
    };
    if !schema_dir.join(schema_file).exists() {
        return Vec::new();
    }

    let result = validate_schema(xml_file, schema_dir);
    if result.valid {
        return Vec::new();
    }
    result
        .errors
        .into_iter()
        .take(20)
        .map(|e| Note {
            severity: Severity::Error,
            code: Code::XmlSchemaViolation,
            message: format!("Schema violation ({schema_file}): {}", e.message),
            file: Some(xml_file.to_path_buf()),
            line: e.line,
        })
        .collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaError {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaValidationResult {
    pub valid: bool,
    pub errors: Vec<SchemaError>,
}

/// Validate XML against SMPTE XSD schemas using xmllint.
///
/// Delegates to the system `xmllint` tool for full XSD validation.
/// Falls back to basic well-formedness checking if xmllint is unavailable.
pub fn validate_schema(xml_file: &Path, schema_dir: &Path) -> SchemaValidationResult {
    // Determine which XSD to use based on the XML content
    let content = match std::fs::read_to_string(xml_file) {
        Ok(c) => c,
        Err(e) => {
            return SchemaValidationResult {
                valid: false,
                errors: vec![SchemaError {
                    line: 0,
                    column: 0,
                    message: format!("Failed to read XML file: {e}"),
                }],
            };
        }
    };

    // Detect schema type from root element + standard
    let Some(schema_file) = schema_file_for(&content) else {
        // Can't determine schema — do well-formedness check only
        return validate_wellformed(&content);
    };

    let schema_path = schema_dir.join(schema_file);
    if !schema_path.exists() {
        // Schema file not found — fall back to well-formedness
        tracing::warn!(
            "Schema file {} not found, falling back to well-formedness check",
            schema_path.display()
        );
        return validate_wellformed(&content);
    }

    // Use xmllint for full XSD validation. The schemas import each other via
    // http URLs; the catalog maps those to the local files so it runs offline.
    let mut cmd = std::process::Command::new("xmllint");
    cmd.arg("--nonet")
        .arg("--schema")
        .arg(&schema_path)
        .arg("--noout");
    let catalog = schema_dir.join("catalog.xml");
    if catalog.exists() {
        cmd.arg("--catalogs").env("XML_CATALOG_FILES", &catalog);
    }
    let output = cmd.arg(xml_file).output();

    match output {
        Ok(o) => {
            if o.status.success() {
                SchemaValidationResult {
                    valid: true,
                    errors: Vec::new(),
                }
            } else {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let mut errors: Vec<SchemaError> = stderr
                    .lines()
                    .filter(|l| l.contains("error") || l.contains("Error"))
                    .map(|line| {
                        // xmllint format: "file:line: element error : message"
                        let parts: Vec<&str> = line.splitn(3, ':').collect();
                        let line_num = parts
                            .get(1)
                            .and_then(|s| s.trim().parse().ok())
                            .unwrap_or(0);
                        SchemaError {
                            line: line_num,
                            column: 0,
                            message: parts.last().unwrap_or(&line).trim().to_string(),
                        }
                    })
                    .collect();
                if errors.is_empty() {
                    errors.push(SchemaError {
                        line: 0,
                        column: 0,
                        message: stderr.trim().to_string(),
                    });
                }

                SchemaValidationResult {
                    valid: false,
                    errors,
                }
            }
        }
        Err(_) => {
            // xmllint not available — fall back to well-formedness
            tracing::warn!("xmllint not found, falling back to well-formedness check");
            validate_wellformed(&content)
        }
    }
}

/// Basic XML well-formedness check using quick-xml.
fn validate_wellformed(content: &str) -> SchemaValidationResult {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(content);
    let mut errors = Vec::new();
    let mut elements = Vec::new();
    let mut root_count = 0u32;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                if elements.is_empty() {
                    root_count += 1;
                }
                elements.push(element.name().as_ref().to_vec());
            }
            Ok(Event::Empty(_)) => {
                if elements.is_empty() {
                    root_count += 1;
                }
            }
            Ok(Event::End(element)) => match elements.pop() {
                Some(start) if start == element.name().as_ref() => {}
                Some(start) => {
                    errors.push(SchemaError {
                        line: 0,
                        column: reader.error_position() as u32,
                        message: format!(
                            "XML parse error: expected </{}>, found </{}>",
                            String::from_utf8_lossy(&start),
                            String::from_utf8_lossy(element.name().as_ref())
                        ),
                    });
                    break;
                }
                None => {
                    errors.push(SchemaError {
                        line: 0,
                        column: reader.error_position() as u32,
                        message: "XML parse error: unexpected closing element".to_string(),
                    });
                    break;
                }
            },
            Ok(Event::Text(text))
                if elements.is_empty()
                    && text.as_ref().iter().any(|byte| !byte.is_ascii_whitespace()) =>
            {
                errors.push(SchemaError {
                    line: 0,
                    column: reader.error_position() as u32,
                    message: "XML parse error: text outside root element".to_string(),
                });
                break;
            }
            Ok(Event::CData(_)) if elements.is_empty() => {
                errors.push(SchemaError {
                    line: 0,
                    column: reader.error_position() as u32,
                    message: "XML parse error: CDATA outside root element".to_string(),
                });
                break;
            }
            Ok(Event::Eof) => {
                if let Some(element) = elements.last() {
                    errors.push(SchemaError {
                        line: 0,
                        column: reader.error_position() as u32,
                        message: format!(
                            "XML parse error: unexpected end of file inside <{}>",
                            String::from_utf8_lossy(element)
                        ),
                    });
                } else if root_count != 1 {
                    errors.push(SchemaError {
                        line: 0,
                        column: reader.error_position() as u32,
                        message: format!(
                            "XML parse error: expected one root element, found {root_count}"
                        ),
                    });
                }
                break;
            }
            Ok(_) => {}
            Err(e) => {
                errors.push(SchemaError {
                    line: 0,
                    column: reader.error_position() as u32,
                    message: format!("XML parse error: {e}"),
                });
                break;
            }
        }
    }

    SchemaValidationResult {
        valid: errors.is_empty(),
        errors,
    }
}

/// Validate that an XML file is well-formed.
pub fn validate_wellformed_file(xml_file: &Path) -> SchemaValidationResult {
    let content = match std::fs::read_to_string(xml_file) {
        Ok(content) => content,
        Err(e) => {
            return SchemaValidationResult {
                valid: false,
                errors: vec![SchemaError {
                    line: 0,
                    column: 0,
                    message: format!("Failed to read XML file: {e}"),
                }],
            };
        }
    };

    validate_wellformed(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wellformed_validation_checks_element_nesting() {
        assert!(validate_wellformed("<root><child/></root>").valid);
        assert!(!validate_wellformed("<root><child></root>").valid);
        assert!(!validate_wellformed("<root/></other>").valid);
        assert!(!validate_wellformed("<root/>text").valid);
    }
}
