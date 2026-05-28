use clap::{Parser, Subcommand};
use std::path::PathBuf;

use dcpdoctor_core::report::ReportFormat;

#[derive(Parser)]
#[command(name = "dcpdoctor", version, about = "DCP/IMF validator and verifier")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// DCP directories to validate (shorthand for `validate`)
    #[arg(trailing_var_arg = true)]
    dcp_dirs: Vec<PathBuf>,

    /// Show info-level notes
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Only show errors
    #[arg(short, long, global = true)]
    quiet: bool,

    /// JSON output
    #[arg(long, global = true)]
    json: bool,

    /// HTML report output
    #[arg(long, global = true)]
    html: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate DCP directories
    Validate {
        /// DCP directories to validate
        dcp_dirs: Vec<PathBuf>,

        /// Skip hash verification
        #[arg(long)]
        no_hashes: bool,

        /// Skip signature verification
        #[arg(long)]
        no_signatures: bool,

        /// Inspect MXF essence metadata
        #[arg(long)]
        check_mxf: bool,

        /// Strict SMPTE compliance
        #[arg(long)]
        strict: bool,

        /// Write report to file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Compare two DCPs
    Diff {
        /// First DCP directory
        dcp_a: PathBuf,
        /// Second DCP directory
        dcp_b: PathBuf,
        /// Compare content hashes
        #[arg(long)]
        hashes: bool,
    },

    /// Display DCP information
    Info {
        /// DCP directory
        dcp_dir: PathBuf,
    },

    /// Watch directory for new DCPs
    Watch {
        /// Directory to watch
        directory: PathBuf,
        /// Poll interval in ms
        #[arg(long, default_value = "5000")]
        interval: u32,
    },

    /// Start REST API server
    Serve {
        /// Bind address
        #[arg(long, default_value = "0.0.0.0")]
        bind: String,
        /// Port
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unexpected error".to_string()
        };
        let location = info
            .location()
            .map(|l| format!(" ({}:{})", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!("\nerror: dcpdoctor crashed: {payload}{location}");
        eprintln!(
            "This is a bug. Please report it at https://github.com/PostPerfection/dcpdoctor/issues"
        );
        if std::env::var("RUST_BACKTRACE").is_ok() {
            eprintln!(
                "\nBacktrace:\n{:?}",
                std::backtrace::Backtrace::force_capture()
            );
        } else {
            eprintln!("Set RUST_BACKTRACE=1 for a detailed backtrace.");
        }
    }));

    let cli = Cli::parse();

    let level = if cli.verbose {
        tracing::Level::DEBUG
    } else if cli.quiet {
        tracing::Level::ERROR
    } else {
        tracing::Level::WARN
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .init();

    let format = if cli.json {
        ReportFormat::Json
    } else if cli.html {
        ReportFormat::Html
    } else {
        ReportFormat::Text
    };

    match cli.command {
        Some(Commands::Validate {
            dcp_dirs,
            no_hashes,
            no_signatures,
            check_mxf,
            strict,
            output,
        }) => {
            run_validate(
                &dcp_dirs,
                no_hashes,
                no_signatures,
                check_mxf,
                strict,
                format,
                output,
            );
        }
        Some(Commands::Diff {
            dcp_a,
            dcp_b,
            hashes,
        }) => {
            let result = dcpdoctor_core::diff::diff_dcps(&dcp_a, &dcp_b, hashes);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else if result.identical {
                println!("DCPs are identical");
            } else {
                println!("Found {} differences:", result.differences.len());
                for diff in &result.differences {
                    println!(
                        "  [{}] {}: {} vs {}",
                        diff.category, diff.description, diff.value_a, diff.value_b
                    );
                }
            }
        }
        Some(Commands::Info { dcp_dir }) => match dcpdoctor_core::info::get_dcp_info(&dcp_dir) {
            Some(info) => {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&info).unwrap());
                } else {
                    println!("Title: {}", info.title);
                    println!("Standard: {}", info.standard);
                    println!("Content kind: {}", info.content_kind);
                    println!(
                        "Assets: {}, CPLs: {}, PKLs: {}, Reels: {}",
                        info.asset_count, info.cpl_count, info.pkl_count, info.reel_count
                    );
                    println!("Total duration: {} frames", info.total_duration_frames);
                }
            }
            None => {
                eprintln!("Failed to read DCP at {}", dcp_dir.display());
                std::process::exit(1);
            }
        },
        Some(Commands::Watch {
            directory,
            interval,
        }) => {
            let opts = dcpdoctor_core::VerifyOptions::standard();
            dcpdoctor_core::server::watch_directory(
                &directory,
                &opts,
                |path, result| {
                    let status = if result.ok() { "PASS" } else { "FAIL" };
                    println!(
                        "{}: {} ({} errors, {} warnings)",
                        path.display(),
                        status,
                        result.error_count,
                        result.warning_count
                    );
                },
                interval,
            );
        }
        Some(Commands::Serve { bind, port }) => {
            dcpdoctor_core::server::start_server(&bind, port);
        }
        None => {
            // Default: validate the dcp_dirs passed as positional args
            if cli.dcp_dirs.is_empty() {
                eprintln!("No DCP directories specified. Use --help for usage.");
                std::process::exit(1);
            }
            run_validate(&cli.dcp_dirs, false, false, false, false, format, None);
        }
    }
}

fn run_validate(
    dcp_dirs: &[PathBuf],
    no_hashes: bool,
    no_signatures: bool,
    check_mxf: bool,
    strict: bool,
    format: ReportFormat,
    output: Option<PathBuf>,
) {
    let opts = dcpdoctor_core::VerifyOptions {
        check_hashes: !no_hashes,
        check_signatures: !no_signatures,
        check_picture_details: check_mxf,
        strict_smpte: strict,
    };

    let mut any_failed = false;
    for dir in dcp_dirs {
        let result = dcpdoctor_core::verify(dir, &opts);
        if !result.ok() {
            any_failed = true;
        }

        if let Some(ref output_path) = output {
            let mut file = std::fs::File::create(output_path).unwrap();
            dcpdoctor_core::report::write_report(&result, dir, &mut file, format).unwrap();
        } else {
            let mut stdout = std::io::stdout().lock();
            dcpdoctor_core::report::write_report(&result, dir, &mut stdout, format).unwrap();
        }
    }

    if any_failed {
        std::process::exit(1);
    }
}
