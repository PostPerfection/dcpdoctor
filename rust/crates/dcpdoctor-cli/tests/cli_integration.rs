use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

#[test]
fn version_flag() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("dcpdoctor"));
}

#[test]
fn help_flag() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("validate"))
        .stdout(predicate::str::contains("diff"))
        .stdout(predicate::str::contains("info"))
        .stdout(predicate::str::contains("kdm"))
        .stdout(predicate::str::contains("mxf-extract"))
        .stdout(predicate::str::contains("qc-report"))
        .stdout(predicate::str::contains("imp-info"));
}

#[test]
fn validate_missing_directory() {
    cmd()
        .args(["validate", "/nonexistent/path"])
        .assert()
        .failure();
}

#[test]
fn validate_empty_directory() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["validate", dir.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn validate_with_json_output() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["validate", dir.path().to_str().unwrap(), "--json"])
        .assert()
        .failure();
}

#[test]
fn validate_help() {
    cmd()
        .args(["validate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-hashes"))
        .stdout(predicate::str::contains("--no-signatures"))
        .stdout(predicate::str::contains("--check-mxf"))
        .stdout(predicate::str::contains("--strict"));
}

#[test]
fn info_missing_directory() {
    cmd().args(["info", "/nonexistent/path"]).assert().failure();
}

#[test]
fn shorthand_positional_arg() {
    let dir = TempDir::new().unwrap();
    // Positional arg without subcommand should act like `validate`
    cmd().arg(dir.path().to_str().unwrap()).assert().failure();
}

#[test]
fn rust_command_surface_help_smoke() {
    let subcommands = [
        "watch",
        "serve",
        "fix",
        "kdm",
        "profiles",
        "checksum-verify",
        "loudness",
        "frame-qc",
        "auto-qc",
        "imf-compliance",
        "mxf-extract",
        "schema-validate",
        "qc-report",
        "av-sync",
        "hdr-validate",
        "frame-compare",
        "imp-info",
    ];

    for subcommand in subcommands {
        cmd().args([subcommand, "--help"]).assert().success();
    }
}

#[test]
fn kdm_missing_file_fails() {
    cmd()
        .args(["kdm", "/nonexistent/file.kdm.xml"])
        .assert()
        .failure();
}

#[test]
fn checksum_verify_missing_directory_fails() {
    cmd()
        .args(["checksum-verify", "/nonexistent/path"])
        .assert()
        .failure();
}

#[test]
fn imf_compliance_rejects_unknown_target() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args([
            "imf-compliance",
            dir.path().to_str().unwrap(),
            "--target",
            "bogus",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown target"));
}

#[test]
fn validate_accepts_imf_flag() {
    // --imf is a real global flag now; a plain path with --imf must parse (not exit 2)
    let dir = TempDir::new().unwrap();
    cmd()
        .args([dir.path().to_str().unwrap(), "--imf"])
        .assert()
        .code(1);
}

#[test]
fn diff_help_lists_fingerprint_flag() {
    cmd()
        .args(["diff", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--fingerprint"));
}

#[test]
fn imf_compliance_help_lists_target_flag() {
    cmd()
        .args(["imf-compliance", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--target"));
}

#[test]
fn schema_validate_rejects_malformed_package_xml_without_schemas() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("CPL_broken.xml"), "<CompositionPlaylist>").unwrap();

    cmd()
        .args(["schema-validate", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicate::str::contains("XML parse error"));
}

#[test]
fn schema_validate_uses_supplied_xsd_for_each_package_xml() {
    let package_dir = TempDir::new().unwrap();
    let schema_dir = TempDir::new().unwrap();
    std::fs::write(
        package_dir.path().join("CPL_test.xml"),
        r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL"><Id>test</Id></CompositionPlaylist>"#,
    )
    .unwrap();
    std::fs::write(
        // SMPTE CPLs map to the 429-16 metadata schema (ClairMeta's convention)
        schema_dir.path().join("SMPTE-429-16-2014-CPL-Metadata.xsd"),
        r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           targetNamespace="http://www.smpte-ra.org/schemas/429-7/2006/CPL"
           xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL"
           elementFormDefault="qualified">
  <xs:element name="CompositionPlaylist">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="Required" type="xs:string"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#,
    )
    .unwrap();

    cmd()
        .args([
            "schema-validate",
            package_dir.path().to_str().unwrap(),
            "--schema-dir",
            schema_dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Required"));
}

/// An IMP whose ASSETMAP, CPL and PKL all list a picture track file that is not
/// on disk: what a delivery looks like when the essence was left behind.
fn write_imp_missing_its_picture(dir: &std::path::Path) {
    const CPL_ID: &str = "394080ca-5471-40e9-9827-e6e577753400";
    const PKL_ID: &str = "d74e5590-b9fd-4482-8fd3-ddb7fe496e64";
    const PICTURE_ID: &str = "c7d75d7b-7cec-4974-a665-b91536bec4cd";

    std::fs::write(
        dir.join("CPL.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016" xmlns:cc="http://www.smpte-ra.org/ns/2067-2/2020">
  <Id>urn:uuid:{CPL_ID}</Id>
  <ContentTitle>Missing picture</ContentTitle>
  <EditRate>24 1</EditRate>
  <SegmentList><Segment>
    <Id>urn:uuid:a3dfa541-9c0a-4b4b-9a43-59de39b5f0d2</Id>
    <SequenceList><cc:MainImageSequence>
      <Id>urn:uuid:186bf940-2770-4046-a7dd-b5b90bd3c85e</Id>
      <ResourceList><Resource>
        <Id>urn:uuid:d8c4801a-5bdb-47f0-8867-102b88af8815</Id>
        <EditRate>24 1</EditRate>
        <IntrinsicDuration>2</IntrinsicDuration>
        <SourceDuration>2</SourceDuration>
        <TrackFileId>urn:uuid:{PICTURE_ID}</TrackFileId>
      </Resource></ResourceList>
    </cc:MainImageSequence></SequenceList>
  </Segment></SegmentList>
</CompositionPlaylist>"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.join("PKL.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">
  <Id>urn:uuid:{PKL_ID}</Id>
  <AssetList>
    <Asset>
      <Id>urn:uuid:{CPL_ID}</Id>
      <Type>text/xml</Type>
      <HashAlgorithm Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>
    </Asset>
    <Asset>
      <Id>urn:uuid:{PICTURE_ID}</Id>
      <Type>application/mxf</Type>
      <HashAlgorithm Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>
    </Asset>
  </AssetList>
</PackingList>"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.join("ASSETMAP.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:2fd93ab2-dab7-481d-bb43-5779ba62384d</Id>
  <AssetList>
    <Asset>
      <Id>urn:uuid:{PKL_ID}</Id>
      <PackingList>true</PackingList>
      <ChunkList><Chunk><Path>PKL.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{CPL_ID}</Id>
      <ChunkList><Chunk><Path>CPL.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{PICTURE_ID}</Id>
      <ChunkList><Chunk><Path>VIDEO.mxf</Path></Chunk></ChunkList>
    </Asset>
  </AssetList>
</AssetMap>"#
        ),
    )
    .unwrap();
}

#[test]
fn validate_fails_an_imp_whose_track_file_is_missing() {
    let dir = TempDir::new().unwrap();
    write_imp_missing_its_picture(dir.path());

    cmd()
        .args(["validate", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicate::str::contains("asset_not_found"))
        .stdout(predicate::str::contains("VIDEO.mxf"));
}
