/// XML schema validation via the system xmllint tool.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{Code, Note, Severity};

/// The tool the XSD pass shells out to.
const XMLLINT: &str = "xmllint";

/// Env override naming the directory of XSDs to validate against.
const SCHEMA_DIR_ENV: &str = "DCPDOCTOR_SCHEMA_DIR";

/// Root element of a SMPTE ST 428-7 timed-text document.
const SMPTE_SUBTITLE_ROOT: &str = "SubtitleReel";

/// The ST 428-7 namespaces, each pinned to the XSD published alongside it. A
/// document declaring some other DCST namespace picks no schema rather than
/// being validated against the wrong version of it.
const SMPTE_SUBTITLE_SCHEMAS: &[(&str, &str)] = &[
    (
        "http://www.smpte-ra.org/schemas/428-7/2007/DCST",
        "DCDMSubtitle-2007.xsd",
    ),
    (
        "http://www.smpte-ra.org/schemas/428-7/2010/DCST",
        "DCDMSubtitle-2010.xsd",
    ),
    (
        "http://www.smpte-ra.org/schemas/428-7/2014/DCST",
        "DCDMSubtitle-2014.xsd",
    ),
];

/// The ST 428-7 timed-text namespaces, newest publication last. One list, so the
/// schema router and the structural namespace check cannot disagree about which
/// versions of the format exist.
pub(crate) fn smpte_subtitle_namespaces() -> impl Iterator<Item = &'static str> {
    SMPTE_SUBTITLE_SCHEMAS
        .iter()
        .map(|(namespace, _)| *namespace)
}

/// Interop subtitles are a DCSubtitle document in no namespace at all, so the
/// root element name is the only signal and the vendored XSD carries no
/// targetNamespace to match against.
const INTEROP_SUBTITLE_ROOT: &str = "DCSubtitle";
const INTEROP_SUBTITLE_SCHEMA: &str = "DCSubtitle.xsd";

/// The XSD for a subtitle document, `None` for anything else. Keyed off the root
/// element rather than a substring: a CPL also mentions subtitles.
fn subtitle_schema_file(content: &str) -> Option<&'static str> {
    let (root, namespace) = root_element(content)?;
    if root == INTEROP_SUBTITLE_ROOT {
        return Some(INTEROP_SUBTITLE_SCHEMA);
    }
    if root != SMPTE_SUBTITLE_ROOT {
        return None;
    }
    SMPTE_SUBTITLE_SCHEMAS
        .iter()
        .find(|(ns, _)| *ns == namespace)
        .map(|(_, schema)| *schema)
}

/// Pick the XSD to validate against from the document's root element and
/// standard (Interop docs bind their root element to a digicine.com namespace).
/// Mirrors the namespace->schema mapping in ClairMeta's XML catalog. `None` if
/// the file is not a CPL/PKL/ASSETMAP/subtitle we schema-check.
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
        subtitle_schema_file(content)
    }
}

/// Local name and namespace of the document's root element. The namespace is
/// empty when the root declares none. `None` when no element can be read at all.
pub(crate) fn root_element(content: &str) -> Option<(String, String)> {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;

    let mut reader = quick_xml::NsReader::from_str(content);
    loop {
        match reader.read_resolved_event() {
            Ok((ns, Event::Start(e) | Event::Empty(e))) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let namespace = match ns {
                    ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.0).into_owned(),
                    _ => String::new(),
                };
                return Some((name, namespace));
            }
            Ok((_, Event::Eof)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// Namespace bound to the document's root element, empty when it declares none.
/// `None` when no element can be read at all.
pub(crate) fn root_namespace(content: &str) -> Option<String> {
    root_element(content).map(|(_, namespace)| namespace)
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

    if let Ok(dir) = std::env::var(SCHEMA_DIR_ENV) {
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

/// True when the `xmllint` the XSD pass shells out to is installed.
pub fn xmllint_available() -> bool {
    std::process::Command::new(XMLLINT)
        .arg("--version")
        .output()
        .is_ok()
}

/// Why the XSD pass cannot run against `schema_dir`, or `None` when it can. Both
/// causes are environmental, so a run keeps going without XSD coverage; saying so
/// is what stops a machine with no xmllint from reporting a clean pass it never
/// performed.
pub fn schema_pass_unavailable(schema_dir: Option<&Path>) -> Option<String> {
    if schema_dir.is_none() {
        return Some(format!(
            "XSD validation did not run: no schema directory found (set {SCHEMA_DIR_ENV})"
        ));
    }
    (!xmllint_available())
        .then(|| format!("XSD validation did not run: {XMLLINT} is not installed"))
}

/// Schema-validate a single CPL/PKL/ASSETMAP/subtitle against the XSDs in
/// `schema_dir`, emitting [`Code::XmlSchemaViolation`] for each violation.
/// Returns empty when the file is not schema-checkable, its schema is absent, or
/// xmllint is not installed (schema validation is best-effort and never a hard
/// dependency; [`schema_pass_unavailable`] is how a run reports that).
pub fn check_schema(xml_file: &Path, schema_dir: &Path) -> Vec<Note> {
    let Ok(content) = std::fs::read_to_string(xml_file) else {
        return Vec::new();
    };
    schema_notes(&content, xml_file, xml_file, schema_dir)
}

/// Same check for a document that exists only in memory: the ST 429-5 timed text
/// unwrapped from a subtitle MXF. `source` is the path findings name.
pub fn check_schema_xml(xml: &str, source: &Path, schema_dir: &Path) -> Vec<Note> {
    use std::io::Write;

    let Ok(mut file) = tempfile::NamedTempFile::new() else {
        return Vec::new();
    };
    if file.write_all(xml.as_bytes()).is_err() || file.flush().is_err() {
        return Vec::new();
    }
    schema_notes(xml, source, file.path(), schema_dir)
}

/// `content` picks the schema and `linted` is the file xmllint reads; the two
/// differ only for XML unwrapped from an MXF, where `source` is the MXF findings
/// point at.
fn schema_notes(content: &str, source: &Path, linted: &Path, schema_dir: &Path) -> Vec<Note> {
    let Some(schema_file) = schema_file_for(content) else {
        return Vec::new();
    };
    if !schema_dir.join(schema_file).exists() {
        return Vec::new();
    }

    let result = validate_schema(linted, schema_dir);
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
            file: Some(source.to_path_buf()),
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
    let mut cmd = std::process::Command::new(XMLLINT);
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
                if elements.is_empty() && text.iter().any(|byte| !byte.is_ascii_whitespace()) =>
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
        if !xmllint_available() {
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
        if !xmllint_available() {
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

    // ─── subtitle documents ───────────────────────────────────────────────

    /// The conformant ST 428-7 document the structural rules also test against,
    /// so a change that satisfies one of the two passes cannot quietly break the
    /// other.
    use crate::subtitle::tests::smpte_subtitle_doc as smpte_subtitle;

    const INTEROP_SUBTITLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<DCSubtitle Version="1.0">
  <SubtitleID>11111111-2222-3333-4444-555555555555</SubtitleID>
  <MovieTitle>Test</MovieTitle>
  <ReelNumber>1</ReelNumber>
  <Language>English</Language>
  <Font Id="f" Color="FFFFFFFF" Effect="border" EffectColor="FF000000" Italic="no"
        Script="normal" Size="42" Underlined="no" Weight="normal">
    <Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000"
              FadeUpTime="0" FadeDownTime="0">
      <Text HAlign="center" HPosition="0.0" VAlign="bottom" VPosition="8.0">Hi</Text>
    </Subtitle>
  </Font>
</DCSubtitle>"#;

    #[test]
    fn each_smpte_subtitle_namespace_picks_its_own_schema() {
        for (namespace, schema) in SMPTE_SUBTITLE_SCHEMAS {
            assert_eq!(
                schema_file_for(&smpte_subtitle(namespace)),
                Some(*schema),
                "{namespace} must route to {schema}"
            );
        }
    }

    #[test]
    fn interop_subtitle_routes_to_the_dcsubtitle_schema() {
        assert_eq!(
            schema_file_for(INTEROP_SUBTITLE),
            Some(INTEROP_SUBTITLE_SCHEMA)
        );
    }

    #[test]
    fn a_subtitle_reel_in_an_unknown_namespace_picks_no_schema() {
        // validating a 2014 document against the 2010 XSD would be worse than
        // not validating it, so an unrecognised DCST namespace is skipped
        assert_eq!(
            schema_file_for(&smpte_subtitle("http://example.invalid/DCST")),
            None
        );
    }

    /// Run `check_schema` over `xml` against the repository's real XSDs.
    fn notes_against_vendored_schemas(xml: &str) -> Vec<Note> {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub.xml");
        std::fs::write(&file, xml).unwrap();
        check_schema(
            &file,
            &locate_schema_dir().expect("the repository vendors its XSDs"),
        )
    }

    // against the real vendored DCDMSubtitle/DCSubtitle XSDs, not a stub, so a
    // routing mistake shows up as a violation on a conformant document.
    #[test]
    fn subtitle_schema_violations_fire_and_conformant_documents_stay_silent() {
        if !xmllint_available() {
            return;
        }
        for (namespace, _) in SMPTE_SUBTITLE_SCHEMAS {
            let good = smpte_subtitle(namespace);
            assert!(
                notes_against_vendored_schemas(&good).is_empty(),
                "a conformant {namespace} document must stay silent"
            );
            // Id is pattern-restricted to a urn:uuid in every DCST version
            let bad = good.replace(
                "urn:uuid:22222222-2222-3333-4444-555555555555",
                "not-a-uuid",
            );
            assert!(
                notes_against_vendored_schemas(&bad)
                    .iter()
                    .any(|n| n.code == Code::XmlSchemaViolation),
                "a malformed Id in {namespace} must fire xml_schema_violation"
            );
        }

        assert!(
            notes_against_vendored_schemas(INTEROP_SUBTITLE).is_empty(),
            "a conformant DCSubtitle document must stay silent"
        );
        let bad_interop =
            INTEROP_SUBTITLE.replace("TimeIn=\"00:00:05:000\"", "TimeIn=\"nonsense\"");
        assert!(
            notes_against_vendored_schemas(&bad_interop)
                .iter()
                .any(|n| n.code == Code::XmlSchemaViolation),
            "a malformed TimeIn in DCSubtitle must fire xml_schema_violation"
        );
    }

    #[test]
    fn check_schema_xml_validates_a_document_that_is_not_on_disk() {
        if !xmllint_available() {
            return;
        }
        let schema_dir = locate_schema_dir().expect("the repository vendors its XSDs");
        let source = Path::new("sub.mxf");
        let (namespace, _) = SMPTE_SUBTITLE_SCHEMAS[1];

        assert!(
            check_schema_xml(&smpte_subtitle(namespace), source, &schema_dir).is_empty(),
            "a conformant unwrapped document must stay silent"
        );
        let bad = smpte_subtitle(namespace).replace(
            "urn:uuid:22222222-2222-3333-4444-555555555555",
            "not-a-uuid",
        );
        let notes = check_schema_xml(&bad, source, &schema_dir);
        assert!(
            notes.iter().any(|n| n.code == Code::XmlSchemaViolation),
            "a malformed unwrapped document must fire, got: {notes:?}"
        );
        assert_eq!(
            notes[0].file.as_deref(),
            Some(source),
            "findings must name the MXF, not the temporary file xmllint read"
        );
    }

    // ─── the pass reporting that it did not run ───────────────────────────

    #[test]
    fn a_missing_schema_directory_is_reported_not_swallowed() {
        let reason =
            schema_pass_unavailable(None).expect("no schema dir means the pass cannot run");
        assert!(
            reason.contains(SCHEMA_DIR_ENV),
            "the reason must name the override that fixes it, got: {reason}"
        );
    }

    #[test]
    fn a_complete_environment_reports_nothing() {
        if !xmllint_available() {
            return;
        }
        let schema_dir = locate_schema_dir().expect("the repository vendors its XSDs");
        assert_eq!(schema_pass_unavailable(Some(&schema_dir)), None);
    }
}
