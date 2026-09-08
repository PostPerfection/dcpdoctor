//! REST API server for remote validation.

use std::net::TcpListener;
use std::path::{Path, PathBuf};

use postkit::rest_api::{Request, RestServer};
use serde::Deserialize;

pub const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0";
pub const DEFAULT_PORT: u16 = 8080;

// the one path an API key is not required on
const HEALTH_PATH: &str = "/health";

// /validate names the package "path", the legacy /verify names it "dcp_dir"
const PACKAGE_KEYS: [(&str, &str); 2] = [("/validate", "path"), ("/verify", "dcp_dir")];

pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub api_key: Option<String>,
}

/// What a validation body carries. Anything else in the JSON object is a 400
/// naming the field, so a misspelt key cannot become a silently empty path.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateRequest {
    #[serde(default)]
    path: String,
    #[serde(default)]
    dcp_dir: String,
    #[serde(default)]
    ov: String,
    #[serde(default)]
    ov_dir: String,
    #[serde(default)]
    kdm: String,
    #[serde(default)]
    recipient_key: String,
}

impl ValidateRequest {
    fn package(&self, key: &str) -> &str {
        match key {
            "path" => &self.path,
            _ => &self.dcp_dir,
        }
    }

    fn options(&self) -> crate::VerifyOptions {
        crate::VerifyOptions {
            ov: optional_path(&self.ov).or_else(|| optional_path(&self.ov_dir)),
            kdm: optional_path(&self.kdm),
            recipient_key: optional_path(&self.recipient_key),
            ..crate::VerifyOptions::standard()
        }
    }
}

fn optional_path(value: &str) -> Option<PathBuf> {
    (!value.is_empty()).then(|| PathBuf::from(value))
}

fn error_json(message: String) -> String {
    serde_json::json!({ "error": message }).to_string()
}

// only the VerifyResult ever leaves this handler, never anything read off disk
fn validate_route(request: &Request, package_key: &str) -> (u16, String) {
    let parsed: ValidateRequest = match serde_json::from_str(&request.body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return (
                400,
                error_json(format!("body is not a valid request: {error}")),
            );
        }
    };

    let package = parsed.package(package_key);
    if package.is_empty() {
        return (400, error_json(format!("missing {package_key}")));
    }

    let package = Path::new(package);
    if !package.exists() {
        return (
            404,
            error_json(format!("no such path: {}", package.display())),
        );
    }
    if !package.is_dir() {
        return (
            400,
            error_json(format!("not a directory: {}", package.display())),
        );
    }

    let result = crate::verify(package, &parsed.options());
    match serde_json::to_string(&result) {
        Ok(json) => (200, json),
        Err(error) => (
            500,
            error_json(format!("could not serialize the result: {error}")),
        ),
    }
}

fn build_server(config: &ServerConfig) -> RestServer {
    let mut server = RestServer::new(&format!("{}:{}", config.bind, config.port));
    match &config.api_key {
        Some(key) => server.require_api_key(key, &[HEALTH_PATH]),
        None => tracing::warn!(
            "no API key set: any caller that reaches this port can validate any path this process can read"
        ),
    }

    server.route(
        "GET",
        HEALTH_PATH,
        Box::new(|_request| (200, r#"{"status":"ok"}"#.to_string())),
    );

    for (path, package_key) in PACKAGE_KEYS {
        server.route(
            "POST",
            path,
            Box::new(move |request| validate_route(request, package_key)),
        );
    }

    server
}

/// Bind the API without serving it, so a caller can read the address it got
/// when the port was 0.
pub fn bind_server(config: &ServerConfig) -> std::io::Result<(RestServer, TcpListener)> {
    let server = build_server(config);
    let listener = server.bind()?;
    Ok((server, listener))
}

/// Start the REST API server (blocking).
///
/// Endpoints:
/// - `GET /health` returns `{"status":"ok"}`, and needs no API key.
/// - `POST /validate` with `{"path": "/path/to/dcp"}` returns the `VerifyResult`.
///   An optional `"ov": "/path/to/ov"` resolves a supplemental package's
///   cross-package references against the OV. Optional `"kdm"` +
///   `"recipient_key"` decrypt an encrypted DCP so the essence checks run.
/// - `POST /verify` with `{"dcp_dir": "/path/to/dcp"}` (legacy alias) does the
///   same, also honoring `"ov"`, `"kdm"`, and `"recipient_key"`.
pub fn start_server(config: &ServerConfig) -> std::io::Result<()> {
    let (server, listener) = bind_server(config)?;
    tracing::info!(
        "DCP Doctor REST API listening on {}",
        listener
            .local_addr()
            .map(|address| address.to_string())
            .unwrap_or_else(|_| server.bind_address.clone())
    );
    server.serve_forever(listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> ValidateRequest {
        serde_json::from_str(body).unwrap()
    }

    #[test]
    fn ov_kdm_and_recipient_key_reach_verify_options() {
        let options =
            parse(r#"{"path":"/dcp","ov":"/ov","kdm":"/k.xml","recipient_key":"/r.pem"}"#)
                .options();
        assert_eq!(options.ov, Some(PathBuf::from("/ov")));
        assert_eq!(options.kdm, Some(PathBuf::from("/k.xml")));
        assert_eq!(options.recipient_key, Some(PathBuf::from("/r.pem")));
    }

    #[test]
    fn ov_dir_alias_is_accepted_and_empty_ignored() {
        let aliased = parse(r#"{"dcp_dir":"/dcp","ov_dir":"/ov"}"#);
        assert_eq!(aliased.package("dcp_dir"), "/dcp");
        assert_eq!(aliased.options().ov, Some(PathBuf::from("/ov")));
        assert_eq!(parse(r#"{"path":"/dcp","ov":""}"#).options().ov, None);
    }

    #[test]
    fn a_misspelt_field_is_refused_rather_than_ignored() {
        let parsed: Result<ValidateRequest, _> = serde_json::from_str(r#"{"pat":"/dcp"}"#);
        assert!(parsed.is_err(), "an unknown key must not parse");
    }
}
