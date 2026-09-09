use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};

use dcpdoctor_core::server::{ServerConfig, bind_server};

const API_KEY: &str = "correct-horse-battery-staple";
const MAX_BODY_BYTES: usize = 1024 * 1024;

fn fixture_package() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte")
}

fn copy_package(target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(fixture_package()).unwrap().flatten() {
        std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
    }
}

fn serve(api_key: Option<&str>) -> SocketAddr {
    let config = ServerConfig {
        bind: "127.0.0.1".to_string(),
        port: 0,
        api_key: api_key.map(str::to_string),
    };
    let (server, listener) = bind_server(&config).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || server.serve_forever(listener).unwrap());
    address
}

fn send(address: SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn post(address: SocketAddr, path: &str, body: &str, headers: &str) -> String {
    send(
        address,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

fn get(address: SocketAddr, path: &str, headers: &str) -> String {
    send(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{headers}Connection: close\r\n\r\n"),
    )
}

fn status_line(response: &str) -> &str {
    response.lines().next().unwrap_or("")
}

fn body_of(response: &str) -> &str {
    response.split("\r\n\r\n").nth(1).unwrap_or("")
}

#[test]
fn health_answers_without_a_key_and_the_rest_does_not() {
    let address = serve(Some(API_KEY));

    assert!(
        get(address, "/health", "").contains(r#""status":"ok""#),
        "health must answer without a key"
    );
    assert_eq!(
        status_line(&post(address, "/validate", r#"{"path":"/tmp"}"#, "")),
        "HTTP/1.1 401 Unauthorized"
    );
}

#[test]
fn either_key_header_authorizes_validate() {
    let address = serve(Some(API_KEY));
    let package = tempfile::TempDir::new().unwrap();
    copy_package(package.path());
    let body = serde_json::json!({ "path": package.path() }).to_string();

    for header in [
        format!("X-Api-Key: {API_KEY}\r\n"),
        format!("Authorization: Bearer {API_KEY}\r\n"),
    ] {
        let response = post(address, "/validate", &body, &header);
        assert_eq!(
            status_line(&response),
            "HTTP/1.1 200 OK",
            "{header} was refused: {response}"
        );
    }

    let wrong = post(
        address,
        "/validate",
        &body,
        &format!("X-Api-Key: {API_KEY}x\r\n"),
    );
    assert_eq!(status_line(&wrong), "HTTP/1.1 401 Unauthorized");
}

#[test]
fn a_valid_package_passes_and_a_mutated_copy_fails() {
    let address = serve(None);

    let good = tempfile::TempDir::new().unwrap();
    copy_package(good.path());
    let response = post(
        address,
        "/validate",
        &serde_json::json!({ "path": good.path() }).to_string(),
        "",
    );
    assert_eq!(status_line(&response), "HTTP/1.1 200 OK");
    let result: serde_json::Value = serde_json::from_str(body_of(&response)).unwrap();
    assert_eq!(
        result["error_count"], 0,
        "the committed package must pass: {result}"
    );

    // one flipped byte in the sound essence, so its PKL hash no longer matches
    let broken = tempfile::TempDir::new().unwrap();
    copy_package(broken.path());
    let sound = broken.path().join("sound.mxf");
    let mut bytes = std::fs::read(&sound).unwrap();
    bytes[40] ^= 0xff;
    std::fs::write(&sound, bytes).unwrap();

    let response = post(
        address,
        "/verify",
        &serde_json::json!({ "dcp_dir": broken.path() }).to_string(),
        "",
    );
    assert_eq!(status_line(&response), "HTTP/1.1 200 OK");
    let result: serde_json::Value = serde_json::from_str(body_of(&response)).unwrap();
    assert!(
        result["error_count"].as_u64().unwrap() > 0,
        "the mutated package must fail: {result}"
    );
    assert!(
        result["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["code"] == "PklHashMismatch"),
        "the flipped byte must show up as a hash mismatch: {result}"
    );
}

const OV_PICTURE_ID: &str = "aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa";
const SUPPLEMENTAL_SOUND_ID: &str = "bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb";

/// A SMPTE DCP holding `asset_ids` whose CPL references `picture_id` and
/// `sound_id`. `supplemental` gives the CPL the OPL marker that makes it a
/// version file.
fn write_cross_package_dcp(
    dir: &Path,
    cpl_id: &str,
    asset_ids: &[&str],
    picture_id: &str,
    sound_id: &str,
    supplemental: bool,
) {
    let mut assets = format!(
        r#"<Asset><Id>urn:uuid:{cpl_id}</Id><ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>"#
    );
    for id in asset_ids {
        assets.push_str(&format!(
            r#"<Asset><Id>urn:uuid:{id}</Id><ChunkList><Chunk><Path>{id}.mxf</Path></Chunk></ChunkList></Asset>"#
        ));
    }
    std::fs::write(
        dir.join("ASSETMAP.xml"),
        format!(
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <AssetList>{assets}</AssetList>
</AssetMap>"#
        ),
    )
    .unwrap();

    let opl = if supplemental {
        "<OriginalPackagingList>ov</OriginalPackagingList>"
    } else {
        ""
    };
    std::fs::write(
        dir.join("cpl.xml"),
        format!(
            r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{cpl_id}</Id>
  <ContentTitleText>t</ContentTitleText>
  {opl}
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:{picture_id}</Id><Duration>48</Duration></MainPicture>
      <MainSound><Id>urn:uuid:{sound_id}</Id><Duration>48</Duration></MainSound>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#
        ),
    )
    .unwrap();
}

fn note_codes(response: &str) -> Vec<String> {
    let result: serde_json::Value = serde_json::from_str(body_of(response)).unwrap();
    result["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("no notes in {result}"))
        .iter()
        .map(|note| note["code"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn ov_in_the_body_resolves_a_supplemental_packages_cross_package_refs() {
    let address = serve(None);

    let ov = tempfile::TempDir::new().unwrap();
    write_cross_package_dcp(
        ov.path(),
        "0f0f0f0f-0000-0000-0000-000000000000",
        &[OV_PICTURE_ID],
        OV_PICTURE_ID,
        OV_PICTURE_ID,
        false,
    );
    // the supplemental holds only the sound; its picture ref lives in the OV
    let supplemental = tempfile::TempDir::new().unwrap();
    write_cross_package_dcp(
        supplemental.path(),
        "1a1a1a1a-0000-0000-0000-000000000000",
        &[SUPPLEMENTAL_SOUND_ID],
        OV_PICTURE_ID,
        SUPPLEMENTAL_SOUND_ID,
        true,
    );

    let alone = post(
        address,
        "/validate",
        &serde_json::json!({ "path": supplemental.path() }).to_string(),
        "",
    );
    assert_eq!(status_line(&alone), "HTTP/1.1 200 OK", "{alone}");
    assert!(
        note_codes(&alone).contains(&"SupplementalOvNotProvided".to_string()),
        "without an OV the picture ref is unresolved: {alone}"
    );

    let with_ov = post(
        address,
        "/validate",
        &serde_json::json!({ "path": supplemental.path(), "ov": ov.path() }).to_string(),
        "",
    );
    assert_eq!(status_line(&with_ov), "HTTP/1.1 200 OK", "{with_ov}");
    let codes = note_codes(&with_ov);
    assert!(
        !codes.contains(&"SupplementalOvNotProvided".to_string()),
        "the OV must satisfy the picture ref: {with_ov}"
    );
    assert!(
        !codes.contains(&"CrossRefBroken".to_string()),
        "the OV must not turn the ref into a break: {with_ov}"
    );
}

#[test]
fn a_path_that_does_not_exist_is_a_404_naming_it() {
    let address = serve(None);
    let response = post(address, "/validate", r#"{"path":"/no/such/package"}"#, "");
    assert_eq!(status_line(&response), "HTTP/1.1 404 Not Found");
    assert!(
        body_of(&response).contains("/no/such/package"),
        "the 404 must name the path: {response}"
    );
}

#[test]
fn a_body_that_is_not_a_request_is_a_400_naming_the_problem() {
    let address = serve(None);

    let not_json = post(address, "/validate", "this is not json", "");
    assert_eq!(status_line(&not_json), "HTTP/1.1 400 Bad Request");
    assert!(
        body_of(&not_json).contains("not a valid request"),
        "{not_json}"
    );

    let no_path = post(address, "/validate", r#"{"ov":"/ov"}"#, "");
    assert_eq!(status_line(&no_path), "HTTP/1.1 400 Bad Request");
    assert!(body_of(&no_path).contains("missing path"), "{no_path}");

    let legacy = post(address, "/verify", r#"{"ov":"/ov"}"#, "");
    assert!(body_of(&legacy).contains("missing dcp_dir"), "{legacy}");
}

#[test]
fn a_file_is_refused_rather_than_read_back() {
    let address = serve(None);
    let secret = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(secret.path(), "the contents of an arbitrary file").unwrap();

    let response = post(
        address,
        "/validate",
        &serde_json::json!({ "path": secret.path() }).to_string(),
        "",
    );

    assert_eq!(status_line(&response), "HTTP/1.1 400 Bad Request");
    assert!(
        !response.contains("the contents of an arbitrary file"),
        "the server must never hand back what a file holds: {response}"
    );
}

#[test]
fn a_body_over_the_cap_is_refused_before_it_is_read() {
    let address = serve(None);
    let response = send(
        address,
        &format!(
            "POST /validate HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        ),
    );
    assert_eq!(status_line(&response), "HTTP/1.1 413 Payload Too Large");
}

#[test]
fn an_unknown_path_is_a_404() {
    let address = serve(None);
    assert_eq!(
        status_line(&get(address, "/nope", "")),
        "HTTP/1.1 404 Not Found"
    );
}
