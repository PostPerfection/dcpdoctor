/// REST API server and directory watching for remote validation.
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

/// Extract the optional OV package path from a parsed request body.
/// Accepts either `"ov"` or the `"ov_dir"` alias; empty strings are ignored.
fn request_ov(parsed: &serde_json::Value) -> Option<std::path::PathBuf> {
    parsed["ov"]
        .as_str()
        .or_else(|| parsed["ov_dir"].as_str())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
}

/// Extract an optional path field from a parsed request body; empty ignored.
fn request_path(parsed: &serde_json::Value, key: &str) -> Option<std::path::PathBuf> {
    parsed[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
}

/// Start the REST API server.
///
/// Endpoints:
/// - `GET /health` returns `{"status":"ok"}`.
/// - `POST /validate` with `{"path": "/path/to/dcp"}` returns the `VerifyResult`.
///   An optional `"ov": "/path/to/ov"` resolves a supplemental package's
///   cross-package references against the OV. Optional `"kdm"` +
///   `"recipient_key"` decrypt an encrypted DCP so the essence checks run.
/// - `POST /verify` with `{"dcp_dir": "/path/to/dcp"}` (legacy alias) does the
///   same, also honoring `"ov"`, `"kdm"`, and `"recipient_key"`.
pub fn start_server(bind: &str, port: u16) {
    let addr = format!("{bind}:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind to {addr}: {e}");
            return;
        }
    };

    tracing::info!("DCP Doctor REST API listening on {addr}");

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to accept connection: {e}");
                continue;
            }
        };

        let mut buf = [0u8; 8192];
        let n = match stream.read(&mut buf) {
            Ok(n) => n,
            Err(_) => continue,
        };

        let request = String::from_utf8_lossy(&buf[..n]);

        // /validate takes {"path"}, /verify (legacy) takes {"dcp_dir"}
        let route = if request.starts_with("POST /validate") {
            Some("path")
        } else if request.starts_with("POST /verify") {
            Some("dcp_dir")
        } else {
            None
        };

        if request.starts_with("GET /health") {
            let json = "{\"status\":\"ok\"}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                json.len(),
                json
            );
            let _ = stream.write_all(response.as_bytes());
        } else if let Some(key) = route {
            // Extract JSON body (after blank line)
            let body = request
                .split("\r\n\r\n")
                .nth(1)
                .or_else(|| request.split("\n\n").nth(1))
                .unwrap_or("");

            let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();

            let dcp_dir = parsed[key].as_str().unwrap_or("");

            if dcp_dir.is_empty() {
                let json = format!("{{\"error\":\"missing {key}\"}}");
                let response = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(response.as_bytes());
                continue;
            }

            // Optional OV package for supplemental cross-package validation, and
            // an optional KDM + recipient key to decrypt an encrypted DCP.
            let opts = crate::VerifyOptions {
                ov: request_ov(&parsed),
                kdm: request_path(&parsed, "kdm"),
                recipient_key: request_path(&parsed, "recipient_key"),
                ..crate::VerifyOptions::standard()
            };
            let result = crate::verify(Path::new(dcp_dir), &opts);
            let json = serde_json::to_string(&result).unwrap_or_default();

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                json.len(),
                json
            );
            let _ = stream.write_all(response.as_bytes());
        } else {
            let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
        }
    }
}

/// Watch a directory for new DCPs and auto-validate.
///
/// Polls the directory at the given interval and validates any new or modified
/// DCP subdirectories, invoking the callback with results.
pub fn watch_directory(
    dir: &Path,
    opts: &crate::VerifyOptions,
    on_result: impl Fn(&Path, &crate::VerifyResult),
    poll_interval_ms: u32,
) {
    let interval = std::time::Duration::from_millis(poll_interval_ms as u64);
    let mut known: HashSet<std::path::PathBuf> = HashSet::new();

    tracing::info!(
        "Watching {} for new DCPs (poll {}ms)",
        dir.display(),
        poll_interval_ms
    );

    loop {
        let entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();

        for entry in &entries {
            // Check if this looks like a DCP (has ASSETMAP or ASSETMAP.xml)
            let is_dcp = entry.join("ASSETMAP").exists() || entry.join("ASSETMAP.xml").exists();
            if !is_dcp {
                continue;
            }

            if known.insert(entry.clone()) {
                tracing::info!("New DCP detected: {}", entry.display());
                let result = crate::verify(entry, opts);
                on_result(entry, &result);
            }
        }

        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ov_field_reaches_verify_options() {
        let body: serde_json::Value =
            serde_json::from_str(r#"{"path":"/dcp","ov":"/ov"}"#).unwrap();
        let opts = crate::VerifyOptions {
            ov: request_ov(&body),
            ..crate::VerifyOptions::standard()
        };
        assert_eq!(opts.ov, Some(std::path::PathBuf::from("/ov")));
    }

    #[test]
    fn kdm_and_recipient_key_reach_verify_options() {
        let body: serde_json::Value =
            serde_json::from_str(r#"{"path":"/dcp","kdm":"/k.xml","recipient_key":"/r.pem"}"#)
                .unwrap();
        let opts = crate::VerifyOptions {
            kdm: request_path(&body, "kdm"),
            recipient_key: request_path(&body, "recipient_key"),
            ..crate::VerifyOptions::standard()
        };
        assert_eq!(opts.kdm, Some(std::path::PathBuf::from("/k.xml")));
        assert_eq!(opts.recipient_key, Some(std::path::PathBuf::from("/r.pem")));
    }

    #[test]
    fn ov_dir_alias_is_accepted_and_empty_ignored() {
        let aliased: serde_json::Value =
            serde_json::from_str(r#"{"dcp_dir":"/dcp","ov_dir":"/ov"}"#).unwrap();
        assert_eq!(request_ov(&aliased), Some(std::path::PathBuf::from("/ov")));

        let none: serde_json::Value = serde_json::from_str(r#"{"path":"/dcp","ov":""}"#).unwrap();
        assert_eq!(request_ov(&none), None);
    }
}
