use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use tauri_plugin_shell::ShellExt;

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

const CLI_SIDECAR_NAME: &str = "dcpdoctor";

fn cli_command(app: &tauri::AppHandle) -> Result<Command, String> {
    let sidecar = app
        .shell()
        .sidecar(CLI_SIDECAR_NAME)
        .map_err(|e| format!("Cannot locate the bundled dcpdoctor CLI: {e}"))?;
    Ok(sidecar.into())
}

fn run_cli(command: &mut Command) -> Result<std::process::Output, String> {
    command.output().map_err(|e| {
        format!(
            "Failed to run the bundled dcpdoctor CLI at '{}': {e}. The app build copies it there from {CLI_SIDECAR_NAME}-{}.",
            Path::new(command.get_program()).display(),
            env!("TAURI_ENV_TARGET_TRIPLE")
        )
    })
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

/// Flags must precede the path: the shorthand positional is trailing_var_arg, so
/// anything after the path is captured as a value, not parsed as a flag.
fn validation_args(path: &str, flags: &[String]) -> Vec<String> {
    let mut args: Vec<String> = flags.to_vec();
    args.push(path.to_string());
    args
}

/// The one line the frontend shows above the findings table.
fn summarize(results: &[ValidationResult], exit_code: i32, combined: &str) -> String {
    let errors = results.iter().filter(|r| r.severity == "error").count();
    let warnings = results.iter().filter(|r| r.severity == "warning").count();

    if errors == 0 && warnings == 0 && exit_code == 0 {
        "DCP is valid — no issues found.".to_string()
    } else if results.is_empty() && exit_code != 0 {
        // Binary ran but we couldn't parse output — show raw output
        format!(
            "Validation failed (exit {}): {}",
            exit_code,
            combined.trim()
        )
    } else {
        format!("{} error(s), {} warning(s) found.", errors, warnings)
    }
}

#[tauri::command]
fn validate_dcp(
    app: tauri::AppHandle,
    path: String,
    flags: Vec<String>,
) -> Result<ValidationResponse, String> {
    let mut command = cli_command(&app)?;
    command.args(validation_args(&path, &flags));
    eprintln!(
        "[dcpdoctor-gui] binary: {}",
        Path::new(command.get_program()).display()
    );
    eprintln!("[dcpdoctor-gui] validating: {}", path);

    let output = run_cli(&mut command)?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}\n{}", stdout, stderr);
    eprintln!(
        "[dcpdoctor-gui] exit: {:?}, stdout: {}",
        output.status.code(),
        stdout.trim()
    );

    let results = parse_output(&combined);
    let exit_code = output.status.code().unwrap_or(-1);
    let summary = summarize(&results, exit_code, &combined);

    Ok(ValidationResponse {
        results,
        summary,
        exit_code,
    })
}

#[tauri::command]
fn get_version(app: tauri::AppHandle) -> Result<String, String> {
    let mut command = cli_command(&app)?;
    command.arg("--version");
    let output = run_cli(&mut command)?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    guikit_startup::prefer_shared_memory_webkit_frames_on_nvidia();
    #[cfg(unix)]
    guikit_startup::fork_terminal_guard();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![validate_dcp, get_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(severity: &str, code: &str, message: &str, file: &str) -> ValidationResult {
        ValidationResult {
            severity: severity.to_string(),
            code: code.to_string(),
            message: message.to_string(),
            file: file.to_string(),
        }
    }

    #[test]
    fn every_severity_the_cli_prints_is_recognized() {
        let output = "[ERROR] cross_ref_broken - asset is missing (CPL_test.xml)\n\
                      [WARNING] cpl_annotation_text_mismatch - titles differ (CPL_test.xml)\n\
                      [WARN] subtitle_spacing - cues are close (sub.xml)\n\
                      [INFO] j2k_codestream_summary - 2048x1080, 6 levels (pic.mxf)";
        let results = parse_output(output);
        let severities: Vec<&str> = results.iter().map(|r| r.severity.as_str()).collect();
        assert_eq!(severities, ["error", "warning", "warning", "info"]);
        assert_eq!(results[0].code, "cross_ref_broken");
        assert_eq!(results[0].message, "asset is missing");
        assert_eq!(results[0].file, "CPL_test.xml");
    }

    #[test]
    fn a_line_the_cli_did_not_tag_is_dropped() {
        let output = "Validating /tmp/dcp\n\n[ERROR] bad_xml - unparseable (PKL.xml)\nDone.";
        let results = parse_output(output);
        assert_eq!(results.len(), 1, "got: {results:?}");
        assert_eq!(results[0].code, "bad_xml");
    }

    #[test]
    fn a_message_with_no_file_or_no_code_still_reaches_the_frontend() {
        let results = parse_output("[ERROR] something went wrong");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].code, "unknown");
        assert_eq!(results[0].message, "something went wrong");
        assert_eq!(results[0].file, "");

        let results = parse_output("[WARNING] pkl_size_mismatch - declared 12, actual 13");
        assert_eq!(results[0].file, "", "no trailing parenthesis means no file");
        assert_eq!(results[0].message, "declared 12, actual 13");
    }

    #[test]
    fn a_message_holding_parentheses_keeps_the_last_one_as_the_file() {
        let results =
            parse_output("[ERROR] cpl_invalid_language - 'Deutsch' (not a subtag) (CPL.xml)");
        assert_eq!(results[0].message, "'Deutsch' (not a subtag)");
        assert_eq!(results[0].file, "CPL.xml");
    }

    #[test]
    fn flags_are_passed_before_the_path() {
        let flags = vec!["--imf".to_string(), "--bv21".to_string()];
        assert_eq!(
            validation_args("/tmp/my dcp", &flags),
            ["--imf", "--bv21", "/tmp/my dcp"],
            "the trailing positional swallows anything after it"
        );
        assert_eq!(validation_args("/tmp/dcp", &[]), ["/tmp/dcp"]);
    }

    #[test]
    fn a_clean_run_summarizes_as_valid() {
        assert_eq!(summarize(&[], 0, ""), "DCP is valid — no issues found.");
    }

    #[test]
    fn findings_are_counted_by_severity() {
        let results = [
            line("error", "a", "m", "f"),
            line("warning", "b", "m", "f"),
            line("warning", "c", "m", "f"),
            line("info", "d", "m", "f"),
        ];
        assert_eq!(
            summarize(&results, 1, ""),
            "1 error(s), 2 warning(s) found."
        );
    }

    // an INFO carries no count of its own, so a run that only produced INFOs
    // reads as valid
    #[test]
    fn an_info_only_run_summarizes_as_valid() {
        let results = [line(
            "info",
            "j2k_codestream_summary",
            "2048x1080, 6 levels",
            "pic.mxf",
        )];
        assert_eq!(
            summarize(&results, 0, ""),
            "DCP is valid — no issues found."
        );
    }

    #[test]
    fn a_failure_with_no_parseable_output_shows_the_raw_output() {
        let summary = summarize(&[], 2, "  error: not a DCP directory\n");
        assert_eq!(
            summary,
            "Validation failed (exit 2): error: not a DCP directory"
        );
    }
}
