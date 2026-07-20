use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ValidationResult {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub file: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResponse {
    pub results: Vec<ValidationResult>,
    pub summary: String,
    pub exit_code: i32,
}

fn find_dcpdoctor_binary() -> String {
    // Look for the binary in common locations
    let candidates = vec![
        // Sidecar (bundled with Tauri — next to the executable)
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("dcpdoctor")))
            .unwrap_or_default(),
        // Build directory relative to project root (development)
        std::path::PathBuf::from("../../build/dcpdoctor"),
        std::path::PathBuf::from("../build/dcpdoctor"),
        std::path::PathBuf::from("build/dcpdoctor"),
        // Common system PATH
        std::path::PathBuf::from("dcpdoctor"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return candidate.canonicalize()
                .unwrap_or_else(|_| candidate.clone())
                .to_string_lossy()
                .to_string();
        }
    }

    // Fallback to PATH
    "dcpdoctor".to_string()
}

fn parse_output(output: &str) -> Vec<ValidationResult> {
    let mut results = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse lines like: [ERROR] code - message (file)
        // or:                [WARNING] code - message (file)
        // or:                [INFO] code - message (file)
        let severity = if line.starts_with("[ERROR]") {
            "error"
        } else if line.starts_with("[WARN]") || line.starts_with("[WARNING]") {
            "warning"
        } else if line.starts_with("[INFO]") {
            "info"
        } else {
            continue;
        };

        let rest = line
            .trim_start_matches("[ERROR]")
            .trim_start_matches("[WARNING]")
            .trim_start_matches("[WARN]")
            .trim_start_matches("[INFO]")
            .trim();

        // Format: "code - message (file)"
        let (code, rest) = if let Some(dash_pos) = rest.find(" - ") {
            (&rest[..dash_pos], &rest[dash_pos + 3..])
        } else {
            ("unknown", rest)
        };

        // Extract file from trailing parentheses: "message (file)"
        let (message, file) = if let Some(paren_pos) = rest.rfind(" (") {
            if rest.ends_with(')') {
                (&rest[..paren_pos], &rest[paren_pos + 2..rest.len() - 1])
            } else {
                (rest, "")
            }
        } else {
            (rest, "")
        };

        results.push(ValidationResult {
            severity: severity.to_string(),
            code: code.to_string(),
            message: message.to_string(),
            file: file.to_string(),
        });
    }
    results
}

#[tauri::command]
fn validate_dcp(path: String, flags: Vec<String>) -> Result<ValidationResponse, String> {
    let binary = find_dcpdoctor_binary();
    eprintln!("[dcpdoctor-gui] binary: {}", binary);
    eprintln!("[dcpdoctor-gui] cwd: {:?}", std::env::current_dir());
    eprintln!("[dcpdoctor-gui] validating: {}", path);

    // Flags must precede the path: the shorthand positional is trailing_var_arg,
    // so anything after the path is captured as a value, not parsed as a flag.
    let mut cmd = Command::new(&binary);
    for flag in &flags {
        cmd.arg(flag);
    }
    cmd.arg(&path);

    let output = cmd.output().map_err(|e| {
        format!(
            "Failed to run dcpdoctor binary at '{}': {}. \
             Make sure dcpdoctor is built (cd build && make).",
            binary, e
        )
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}\n{}", stdout, stderr);
    eprintln!("[dcpdoctor-gui] exit: {:?}, stdout: {}", output.status.code(), stdout.trim());

    let results = parse_output(&combined);
    let exit_code = output.status.code().unwrap_or(-1);

    let errors = results.iter().filter(|r| r.severity == "error").count();
    let warnings = results.iter().filter(|r| r.severity == "warning").count();

    let summary = if errors == 0 && warnings == 0 && exit_code == 0 {
        "DCP is valid — no issues found.".to_string()
    } else if results.is_empty() && exit_code != 0 {
        // Binary ran but we couldn't parse output — show raw output
        format!("Validation failed (exit {}): {}", exit_code, combined.trim())
    } else {
        format!(
            "{} error(s), {} warning(s) found.",
            errors, warnings
        )
    };

    Ok(ValidationResponse {
        results,
        summary,
        exit_code,
    })
}

#[tauri::command]
fn get_version() -> Result<String, String> {
    let binary = find_dcpdoctor_binary();
    let output = Command::new(&binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("Failed to run dcpdoctor at '{}': {} (cwd: {:?})", binary, e, std::env::current_dir()))?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![validate_dcp, get_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
