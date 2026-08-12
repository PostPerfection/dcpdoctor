/// XML schema validation via the system xmllint tool.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{Code, Note, Severity};

/// Pick the XSD to validate against from the document's root element and
/// standard (Interop docs bind their root element to a digicine.com namespace).
/// Mirrors the namespace->schema mapping in ClairMeta's XML catalog. `None` if
/// the file is not a CPL/PKL/ASSETMAP we schema-check.
fn schema_file_for(content: &str) -> Option<&'static str> {
    // only the root element's own namespace decides the standard: a SMPTE CPL
    // may declare the digicine closed-caption namespace on a track element.
    let interop = root_namespace(content).is_some_and(|ns| ns.contains("digicine.com"));
    // Key off the root element tag. AssetMap must be checked first: a SMPTE
    // ASSETMAP carries a <PackingList>true</PackingList> boolean per asset, so a
    // bare "PackingList" substring would mis-route it to the PKL schema.
    if content.contains("<AssetMap") {
        Some(if interop {
            "PROTO-ASDCP-AM-20040311.xsd"
        } else {
            "SMPTE-429-9-2007-AM.xsd"
        })
    } else if content.contains("KDMRequiredExtensions") {
        // must precede the CompositionPlaylist arm: a KDM carries a
        // <CompositionPlaylistId> element, which matches that arm's substring.
        // The 430-1 schema imports 430-3, so pointing xmllint here loads both
        // and the strict RequiredExtensions wildcard reaches the KDM body.
        Some("SMPTE-430-1-2006-KDM.xsd")
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

/// Namespace bound to the document's root element, empty when it declares none.
/// `None` when no element can be read at all.
pub(crate) fn root_namespace(content: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;

    let mut reader = quick_xml::NsReader::from_str(content);
    loop {
        match reader.read_resolved_event() {
            Ok((ns, Event::Start(_) | Event::Empty(_))) => {
                return Some(match ns {
                    ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.0).into_owned(),
                    _ => String::new(),
                });
            }
            Ok((_, Event::Eof)) | Err(_) => return None,
            Ok(_) => {}
        }
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

    // xsd validation shells out to xmllint; a self-contained schema keyed off the
    // SMPTE AM filename proves check_schema fires on a violation and stays clean
    // on a conformant doc (non-vacuous). skips where xmllint is not installed.
    #[test]
    fn check_schema_fires_on_violation_not_on_valid() {
        if std::process::Command::new("xmllint")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("SMPTE-429-9-2007-AM.xsd"),
            r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
    targetNamespace="http://www.smpte-ra.org/schemas/429-9/2007/AM"
    xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"
    elementFormDefault="qualified">
  <xs:element name="AssetMap">
    <xs:complexType><xs:sequence>
      <xs:element name="Id" type="xs:string"/>
    </xs:sequence></xs:complexType>
  </xs:element>
</xs:schema>"#,
        )
        .unwrap();

        let valid = dir.path().join("valid_am.xml");
        std::fs::write(
            &valid,
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Id>x</Id></AssetMap>"#,
        )
        .unwrap();
        assert!(
            check_schema(&valid, dir.path()).is_empty(),
            "conformant ASSETMAP must not fire xml_schema_violation"
        );

        let invalid = dir.path().join("invalid_am.xml");
        std::fs::write(
            &invalid,
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Bogus/></AssetMap>"#,
        )
        .unwrap();
        let notes = check_schema(&invalid, dir.path());
        assert!(
            notes.iter().any(|n| n.code == Code::XmlSchemaViolation),
            "schema-invalid ASSETMAP must fire xml_schema_violation, got: {notes:?}"
        );
    }

    const CCAP_CPL: &str = r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <cc:ClosedCaption xmlns:cc="http://www.digicine.com/PROTO-ASDCP-CC-CPL-20070926#">
    <cc:Id>urn:uuid:dddddddd-0000-0000-0000-000000000000</cc:Id>
  </cc:ClosedCaption>
</CompositionPlaylist>"#;

    const INTEROP_CPL: &str = r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
</CompositionPlaylist>"#;

    #[test]
    fn interop_cpl_routes_to_interop_schema() {
        assert_eq!(
            schema_file_for(INTEROP_CPL),
            Some("PROTO-ASDCP-CPL-20040511.xsd")
        );
    }

    // both CPL schemas are written out, so mis-routing the SMPTE CPL to the
    // Interop one surfaces as a violation instead of a silent skip.
    #[test]
    fn smpte_cpl_declaring_digicine_cc_namespace_uses_smpte_schema() {
        if std::process::Command::new("xmllint")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("SMPTE-429-16-2014-CPL-Metadata.xsd"),
            r###"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
    targetNamespace="http://www.smpte-ra.org/schemas/429-7/2006/CPL"
    xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL"
    elementFormDefault="qualified">
  <xs:element name="CompositionPlaylist">
    <xs:complexType><xs:sequence>
      <xs:element name="Id" type="xs:string"/>
      <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence></xs:complexType>
  </xs:element>
</xs:schema>"###,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("PROTO-ASDCP-CPL-20040511.xsd"),
            r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
    targetNamespace="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#"
    xmlns="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#"
    elementFormDefault="qualified">
  <xs:element name="CompositionPlaylist">
    <xs:complexType><xs:sequence>
      <xs:element name="Id" type="xs:string"/>
    </xs:sequence></xs:complexType>
  </xs:element>
</xs:schema>"#,
        )
        .unwrap();

        let cpl = dir.path().join("cpl.xml");
        std::fs::write(&cpl, CCAP_CPL).unwrap();
        let notes = check_schema(&cpl, dir.path());
        assert!(
            notes.is_empty(),
            "SMPTE CPL with a digicine CC namespace must not fire a schema violation, got: {notes:?}"
        );
        assert_eq!(
            schema_file_for(CCAP_CPL),
            Some("SMPTE-429-16-2014-CPL-Metadata.xsd")
        );
    }
}
