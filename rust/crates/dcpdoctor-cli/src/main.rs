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

        /// BV2.1 application profile check
        #[arg(long)]
        bv21: bool,

        /// Deep J2K codestream validation
        #[arg(long)]
        deep_j2k: bool,

        /// Generate SVG timeline to file
        #[arg(long)]
        timeline: Option<PathBuf>,

        /// Compare against reference manifest JSON
        #[arg(long)]
        manifest: Option<PathBuf>,

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

    /// Automatically fix repairable issues in a DCP
    Fix {
        /// DCP directory to repair
        dcp_dir: PathBuf,
        /// Dry run — show what would be fixed without modifying files
        #[arg(long)]
        dry_run: bool,
    },

    /// Validate Key Delivery Message (KDM)
    Kdm {
        /// KDM XML file
        kdm_file: PathBuf,
        /// DCP directory to cross-validate against
        #[arg(long)]
        dcp: Option<PathBuf>,
    },

    /// List or check theater compatibility profiles
    Profiles {
        /// Check DCP against a specific theater profile
        #[arg(long)]
        check: Option<String>,
        /// DCP directory to check
        #[arg(long)]
        dcp: Option<PathBuf>,
    },

    /// Verify all PKL asset checksums
    #[command(name = "checksum-verify")]
    ChecksumVerify {
        /// DCP or IMP directory
        dcp_dir: PathBuf,
        /// Stop on first mismatch
        #[arg(long)]
        stop_on_error: bool,
    },

    /// Measure audio loudness (EBU R128 / ATSC A/85)
    Loudness {
        /// Audio file (WAV or MXF)
        audio_file: PathBuf,
    },

    /// Analyze J2K codestream frames
    #[command(name = "frame-qc")]
    FrameQc {
        /// Directory of J2K frame files, or MXF file
        path: PathBuf,
        /// Max acceptable bitrate in Mbps
        #[arg(long, default_value = "250")]
        max_bitrate: f64,
        /// Min acceptable bitrate in Mbps
        #[arg(long, default_value = "10")]
        min_bitrate: f64,
        /// Frame rate for bitrate calculation
        #[arg(long, default_value = "24")]
        fps: f64,
    },

    /// Automated QC — detect black frames, silence, clipping
    #[command(name = "auto-qc")]
    AutoQc {
        /// Video file (MXF or image sequence directory)
        #[arg(long)]
        video: Option<PathBuf>,
        /// Audio file (MXF or WAV)
        #[arg(long)]
        audio: Option<PathBuf>,
        /// Black frame threshold (0.0-1.0)
        #[arg(long, default_value = "0.98")]
        black_threshold: f64,
        /// Silence threshold in dBFS
        #[arg(long, default_value = "-60")]
        silence_threshold: f64,
        /// Clipping threshold in dBFS
        #[arg(long, default_value = "-0.5")]
        clipping_threshold: f64,
    },

    /// Check IMF delivery compliance against platform specs
    #[command(name = "imf-compliance")]
    ImfCompliance {
        /// IMP directory
        imp_dir: PathBuf,
        /// Target platform (netflix, disney, amazon, apple, cinema4k, broadcast)
        #[arg(long)]
        target: String,
        /// Disable strict checking
        #[arg(long)]
        no_strict: bool,
    },

    /// Extract video/audio essence from MXF containers
    #[command(name = "mxf-extract")]
    MxfExtract {
        /// MXF file to extract from
        mxf_file: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
        /// Skip video extraction
        #[arg(long)]
        no_video: bool,
        /// Skip audio extraction
        #[arg(long)]
        no_audio: bool,
        /// Start frame
        #[arg(long)]
        start_frame: Option<u32>,
        /// End frame
        #[arg(long)]
        end_frame: Option<u32>,
    },

    /// Validate XML against SMPTE XSD schemas
    #[command(name = "schema-validate")]
    SchemaValidate {
        /// DCP or IMP directory
        dcp_dir: PathBuf,
        /// Path to XSD schema directory
        #[arg(long)]
        schema_dir: Option<PathBuf>,
    },

    /// Generate detailed QC report
    #[command(name = "qc-report")]
    QcReport {
        /// DCP or IMP directory
        dcp_dir: PathBuf,
        /// Output file
        #[arg(short, long)]
        output: PathBuf,
        /// Report title
        #[arg(long)]
        title: Option<String>,
        /// Client name
        #[arg(long)]
        client: Option<String>,
    },

    /// Detect audio/video sync drift
    #[command(name = "av-sync")]
    AvSync {
        /// Video file (MXF or image sequence directory)
        #[arg(short = 'v', long)]
        video: PathBuf,
        /// Audio file (MXF or WAV)
        #[arg(short = 'a', long)]
        audio: PathBuf,
        /// Frame rate numerator
        #[arg(long, default_value = "24")]
        fps_num: u32,
        /// Frame rate denominator
        #[arg(long, default_value = "1")]
        fps_den: u32,
    },

    /// Validate HDR metadata
    #[command(name = "hdr-validate")]
    HdrValidate {
        /// Video file (MXF)
        video_file: PathBuf,
        /// HDR standard (hdr10, hlg, dolby-vision)
        #[arg(short = 's', long)]
        standard: Option<String>,
        /// Expected MaxCLL
        #[arg(long)]
        max_cll: Option<u32>,
        /// Expected MaxFALL
        #[arg(long)]
        max_fall: Option<u32>,
    },

    /// Compare frames between two files or IMPs
    #[command(name = "frame-compare")]
    FrameCompare {
        /// First IMP directory
        #[arg(long)]
        imp_a: Option<PathBuf>,
        /// Second IMP directory
        #[arg(long)]
        imp_b: Option<PathBuf>,
        /// First file
        #[arg(long)]
        file_a: Option<PathBuf>,
        /// Second file
        #[arg(long)]
        file_b: Option<PathBuf>,
        /// Include VMAF metrics
        #[arg(long)]
        vmaf: bool,
        /// Generate HTML report
        #[arg(long)]
        html: bool,
        /// Extract diff images
        #[arg(long)]
        extract_diffs: bool,
        /// Output directory
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Display IMP package info
    #[command(name = "imp-info")]
    ImpInfo {
        /// IMP directory
        imp_dir: PathBuf,
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
            bv21,
            deep_j2k,
            timeline,
            manifest,
            output,
        }) => {
            run_validate(
                &dcp_dirs,
                no_hashes,
                no_signatures,
                check_mxf,
                strict || bv21,
                deep_j2k,
                timeline,
                manifest,
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
        Some(Commands::Fix { dcp_dir, dry_run }) => {
            if dry_run {
                let opts = dcpdoctor_core::VerifyOptions {
                    check_hashes: true,
                    check_signatures: false,
                    check_picture_details: false,
                    strict_smpte: true,
                };
                let verify_result = dcpdoctor_core::verify(&dcp_dir, &opts);
                let fixable: Vec<_> = verify_result
                    .notes
                    .iter()
                    .filter(|n| is_fixable(n.code))
                    .collect();
                if fixable.is_empty() {
                    println!("Nothing to fix.");
                } else {
                    println!("Would fix {} issue(s):", fixable.len());
                    for note in fixable {
                        println!("  [{}] {}", note.code.as_str(), note.message);
                    }
                }
            } else {
                let fix_result = dcpdoctor_core::fix::fix_dcp(&dcp_dir);
                if fix_result.repairs.is_empty() {
                    println!("Nothing to fix — DCP is clean.");
                } else {
                    println!("Fixed {} issue(s):", fix_result.repair_count());
                    for repair in &fix_result.repairs {
                        println!("  [{}] {}", repair.code.as_str(), repair.description);
                    }
                }
                if !fix_result.skipped.is_empty() {
                    let unfixable: Vec<_> = fix_result
                        .skipped
                        .iter()
                        .filter(|n| n.severity == dcpdoctor_core::Severity::Error)
                        .collect();
                    if !unfixable.is_empty() {
                        eprintln!(
                            "\n{} error(s) remain that cannot be auto-fixed:",
                            unfixable.len()
                        );
                        for note in unfixable {
                            eprintln!("  [{}] {}", note.code.as_str(), note.message);
                        }
                        std::process::exit(1);
                    }
                }
            }
        }
        Some(Commands::Kdm { kdm_file, dcp }) => {
            let notes = dcpdoctor_core::kdm::validate_kdm(&kdm_file, dcp.as_deref());
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&notes).unwrap());
            } else {
                for note in &notes {
                    println!("{note}");
                }
                let errors = notes
                    .iter()
                    .filter(|n| n.severity == dcpdoctor_core::Severity::Error)
                    .count();
                if errors > 0 {
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Profiles { check, dcp }) => {
            if let Some(profile_name) = check {
                let profile = match dcpdoctor_core::profiles::find_profile(&profile_name) {
                    Some(p) => p,
                    None => {
                        eprintln!("Unknown profile: {profile_name}");
                        eprintln!("Available profiles:");
                        for p in dcpdoctor_core::profiles::all_profiles() {
                            eprintln!("  {} ({})", p.name, p.vendor);
                        }
                        std::process::exit(1);
                    }
                };

                let dcp_dir = match dcp {
                    Some(d) => d,
                    None => {
                        eprintln!("--dcp is required when using --check");
                        std::process::exit(1);
                    }
                };

                let info = match dcpdoctor_core::info::get_dcp_info(&dcp_dir) {
                    Some(i) => i,
                    None => {
                        eprintln!("Failed to read DCP at {}", dcp_dir.display());
                        std::process::exit(1);
                    }
                };

                let issues = dcpdoctor_core::profiles::check_compatibility(
                    &profile,
                    (info.picture_width, info.picture_height),
                    info.frame_rate,
                    info.audio_channels,
                    info.has_atmos,
                    info.is_stereo3d,
                );

                if cli.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "profile": profile.name,
                            "compatible": issues.is_empty(),
                            "issues": issues,
                        }))
                        .unwrap()
                    );
                } else if issues.is_empty() {
                    println!("PASS: DCP is compatible with {}", profile.name);
                } else {
                    println!("FAIL: {} issue(s) for {}:", issues.len(), profile.name);
                    for issue in &issues {
                        println!("  - {issue}");
                    }
                    std::process::exit(1);
                }
            } else {
                let profiles = dcpdoctor_core::profiles::all_profiles();
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&profiles).unwrap());
                } else {
                    println!("Built-in theater compatibility profiles:\n");
                    for p in &profiles {
                        println!(
                            "  {:30} {:12} {}x{} @ {}fps, {} ch{}{}",
                            p.name,
                            p.vendor,
                            p.max_resolution.0,
                            p.max_resolution.1,
                            p.max_frame_rate,
                            p.max_channels,
                            if p.supports_atmos { ", Atmos" } else { "" },
                            if p.supports_4k { ", 4K" } else { "" },
                        );
                    }
                }
            }
        }
        Some(Commands::ChecksumVerify {
            dcp_dir,
            stop_on_error,
        }) => {
            let opts = dcpdoctor_core::VerifyOptions {
                check_hashes: true,
                check_signatures: false,
                check_picture_details: false,
                strict_smpte: false,
            };
            let result = dcpdoctor_core::verify(&dcp_dir, &opts);
            let hash_notes: Vec<_> = result
                .notes
                .iter()
                .filter(|n| {
                    matches!(
                        n.code,
                        dcpdoctor_core::Code::PklHashMismatch | dcpdoctor_core::Code::AssetNotFound
                    )
                })
                .collect();

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&hash_notes).unwrap());
            } else if hash_notes.is_empty() {
                println!("All checksums verified OK");
            } else {
                println!("{} checksum issue(s):", hash_notes.len());
                for note in &hash_notes {
                    println!("  [{}] {}", note.code.as_str(), note.message);
                    if stop_on_error {
                        std::process::exit(1);
                    }
                }
                std::process::exit(1);
            }
        }
        Some(Commands::Loudness { audio_file }) => {
            match dcpdoctor_core::audio::measure_loudness(&audio_file) {
                Ok(result) => {
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    } else {
                        println!("Integrated loudness: {:.1} LUFS", result.integrated_lufs);
                        println!("True peak:           {:.1} dBTP", result.true_peak_dbtp);
                        println!("Loudness range:      {:.1} LU", result.loudness_range_lu);
                    }
                }
                Err(e) => {
                    eprintln!("Loudness measurement failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::FrameQc {
            path,
            max_bitrate,
            min_bitrate,
            fps,
        }) => {
            if path.is_file() {
                match dcpdoctor_core::j2k::analyze_j2k(&path) {
                    Ok(info) => {
                        if cli.json {
                            println!("{}", serde_json::to_string_pretty(&info).unwrap());
                        } else {
                            println!("J2K Codestream Analysis:");
                            println!("  Profile:       {}", info.profile);
                            println!("  Resolution:    {}x{}", info.width, info.height);
                            println!("  Components:    {}", info.components);
                            println!("  Bit depth:     {}", info.bit_depth);
                            println!("  Decomp levels: {}", info.decomposition_levels);
                            if info.codeblock_width_exp > 0 {
                                let cb_w = 1u32 << (info.codeblock_width_exp + 2);
                                let cb_h = 1u32 << (info.codeblock_height_exp + 2);
                                println!("  Code-block:    {cb_w}x{cb_h}");
                            }
                            println!(
                                "  Wavelet:       {}",
                                if info.irreversible_transform {
                                    "9-7 (irreversible)"
                                } else {
                                    "5-3 (reversible)"
                                }
                            );
                            println!("  Progression:   {}", info.progression_order);
                            println!("  Layers:        {}", info.layers);
                            if info.frame_bytes > 0 {
                                let bitrate = info.frame_bytes as f64 * fps * 8.0 / 1_000_000.0;
                                println!(
                                    "  Frame size:    {} bytes ({:.1} Mbps @ {fps} fps)",
                                    info.frame_bytes, bitrate
                                );
                            }

                            let dci_notes = dcpdoctor_core::j2k::validate_j2k_dci(&info);
                            if !dci_notes.is_empty() {
                                println!("\nDCI Compliance Issues:");
                                for note in &dci_notes {
                                    println!("  [{}] {}", note.severity, note.message);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("J2K analysis failed: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                match dcpdoctor_core::j2k::analyze_frame_bitrates(&path, fps) {
                    Ok(stats) => {
                        if cli.json {
                            println!("{}", serde_json::to_string_pretty(&stats).unwrap());
                        } else {
                            println!(
                                "Frame Bitrate Analysis ({} frames @ {fps} fps):",
                                stats.frame_count
                            );
                            println!("  Avg: {:.1} Mbps", stats.avg_bitrate_mbps);
                            println!("  Min: {:.1} Mbps", stats.min_bitrate_mbps);
                            println!("  Max: {:.1} Mbps", stats.max_bitrate_mbps);

                            let mut issues = 0;
                            if stats.max_bitrate_mbps > max_bitrate {
                                println!(
                                    "  WARNING: Max bitrate {:.1} exceeds limit {:.1} Mbps",
                                    stats.max_bitrate_mbps, max_bitrate
                                );
                                issues += 1;
                            }
                            if stats.min_bitrate_mbps < min_bitrate {
                                println!(
                                    "  WARNING: Min bitrate {:.1} below threshold {:.1} Mbps",
                                    stats.min_bitrate_mbps, min_bitrate
                                );
                                issues += 1;
                            }
                            if issues > 0 {
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Frame analysis failed: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        Some(Commands::AutoQc {
            video,
            audio,
            black_threshold,
            silence_threshold,
            clipping_threshold,
        }) => {
            let mut findings = Vec::new();

            if let Some(ref video_path) = video {
                let output = std::process::Command::new("ffmpeg")
                    .arg("-i")
                    .arg(video_path)
                    .arg("-vf")
                    .arg(format!("blackdetect=d=0.04:pix_th={black_threshold}"))
                    .arg("-f")
                    .arg("null")
                    .arg("-")
                    .output();

                if let Ok(o) = output {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    let black_count = stderr.matches("black_start:").count();
                    if black_count > 0 {
                        findings.push(format!("Black frames detected: {black_count} segment(s)"));
                    }

                    let freeze_output = std::process::Command::new("ffmpeg")
                        .arg("-i")
                        .arg(video_path)
                        .arg("-vf")
                        .arg("freezedetect=n=-60dB:d=0.5")
                        .arg("-f")
                        .arg("null")
                        .arg("-")
                        .output();

                    if let Ok(fo) = freeze_output {
                        let stderr = String::from_utf8_lossy(&fo.stderr);
                        let freeze_count = stderr.matches("freeze_start:").count();
                        if freeze_count > 0 {
                            findings
                                .push(format!("Freeze frames detected: {freeze_count} segment(s)"));
                        }
                    }
                }
            }

            if let Some(ref audio_path) = audio {
                match dcpdoctor_core::audio::analyze_audio(audio_path) {
                    Ok(analysis) => {
                        for ch in &analysis.channels {
                            if ch.peak_dbfs >= clipping_threshold {
                                findings.push(format!(
                                    "Audio clipping: channel {} peak {:.1} dBFS",
                                    ch.channel + 1,
                                    ch.peak_dbfs
                                ));
                            }
                            if ch.rms_dbfs < silence_threshold {
                                findings.push(format!(
                                    "Audio silence: channel {} RMS {:.1} dBFS",
                                    ch.channel + 1,
                                    ch.rms_dbfs
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        findings.push(format!("Audio analysis failed: {e}"));
                    }
                }
            }

            if video.is_none() && audio.is_none() {
                eprintln!("At least one of --video or --audio is required");
                std::process::exit(1);
            }

            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "pass": findings.is_empty(),
                        "findings": findings,
                    }))
                    .unwrap()
                );
            } else if findings.is_empty() {
                println!("Auto-QC PASS: no issues detected");
            } else {
                println!("Auto-QC findings ({}):", findings.len());
                for f in &findings {
                    println!("  - {f}");
                }
                std::process::exit(1);
            }
        }
        Some(Commands::ImfCompliance {
            imp_dir,
            target,
            no_strict,
        }) => {
            let opts = dcpdoctor_core::VerifyOptions {
                check_hashes: true,
                check_signatures: true,
                check_picture_details: true,
                strict_smpte: !no_strict,
            };
            let result = dcpdoctor_core::verify(&imp_dir, &opts);

            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "target": target,
                        "pass": result.ok(),
                        "errors": result.error_count,
                        "warnings": result.warning_count,
                        "notes": result.notes,
                    }))
                    .unwrap()
                );
            } else {
                println!("IMF Compliance Check: {target}");
                println!("---");
                for note in &result.notes {
                    println!("{note}");
                }
                if result.ok() {
                    println!("\nPASS: Compliant with {target} requirements");
                } else {
                    println!(
                        "\nFAIL: {} error(s), {} warning(s)",
                        result.error_count, result.warning_count
                    );
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::MxfExtract {
            mxf_file,
            output,
            no_video,
            no_audio,
            start_frame,
            end_frame,
        }) => {
            let opts = dcpdoctor_core::mxf_extract::MxfExtractOptions {
                input: mxf_file,
                output_dir: output,
                extract_video: !no_video,
                extract_audio: !no_audio,
                start_frame: start_frame.unwrap_or(0),
                end_frame: end_frame.unwrap_or(0),
            };
            let result = dcpdoctor_core::mxf_extract::extract_mxf(&opts);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else if result.success {
                println!("Extracted {} frame(s)", result.frames_extracted);
                for f in &result.extracted_files {
                    println!("  {}", f.display());
                }
            } else {
                eprintln!("Extraction failed: {}", result.error);
                std::process::exit(1);
            }
        }
        Some(Commands::SchemaValidate {
            dcp_dir,
            schema_dir,
        }) => {
            let notes = if let Some(ref sd) = schema_dir {
                let mut n = dcpdoctor_core::schema_validate::check_namespace_consistency(&dcp_dir);
                let schema_result = dcpdoctor_core::schema::validate_schema(&dcp_dir, sd);
                for err in &schema_result.errors {
                    n.push(dcpdoctor_core::Note::error(
                        dcpdoctor_core::Code::XmlParseError,
                        format!("{}:{}: {}", err.line, err.column, err.message),
                    ));
                }
                n
            } else {
                dcpdoctor_core::schema_validate::check_namespace_consistency(&dcp_dir)
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&notes).unwrap());
            } else if notes.is_empty() {
                println!("Schema validation passed");
            } else {
                println!("{} schema issue(s):", notes.len());
                for note in &notes {
                    println!("{note}");
                }
                let errors = notes
                    .iter()
                    .filter(|n| n.severity == dcpdoctor_core::Severity::Error)
                    .count();
                if errors > 0 {
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::QcReport {
            dcp_dir,
            output,
            title,
            client,
        }) => {
            let opts = dcpdoctor_core::qc_report::DetailedQcOptions {
                imp_dir: dcp_dir,
                output_file: output.clone(),
                title: title.unwrap_or_else(|| "QC Report".to_string()),
                client: client.unwrap_or_default(),
                include_loudness: true,
            };
            let result = dcpdoctor_core::qc_report::generate_detailed_qc(&opts);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else if result.success {
                println!("QC report written to {}", output.display());
            } else {
                eprintln!("QC report generation failed: {}", result.error);
                std::process::exit(1);
            }
        }
        Some(Commands::AvSync {
            video,
            audio,
            fps_num,
            fps_den,
        }) => {
            let opts = dcpdoctor_core::av_sync::AvSyncOptions {
                video_file: video,
                audio_file: audio,
                fps_num,
                fps_den,
                sample_rate: 48000,
            };
            let result = dcpdoctor_core::av_sync::detect_av_sync(&opts);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!("A/V Sync Analysis:");
                println!(
                    "  Drift:        {:.1} ms ({:.2} frames)",
                    result.drift_ms, result.drift_frames
                );
                println!("  Drift samples: {}", result.drift_samples);
                if result.in_sync {
                    println!("  Status:       IN SYNC");
                } else {
                    println!("  Status:       OUT OF SYNC");
                    if !result.recommendation.is_empty() {
                        println!("  Recommendation: {}", result.recommendation);
                    }
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::HdrValidate {
            video_file,
            standard,
            max_cll,
            max_fall,
        }) => {
            let expected_transfer = match standard.as_deref() {
                Some("hdr10") | Some("pq") => dcpdoctor_core::hdr_validate::TransferFunction::Pq,
                Some("hlg") => dcpdoctor_core::hdr_validate::TransferFunction::Hlg,
                _ => dcpdoctor_core::hdr_validate::TransferFunction::default(),
            };
            let opts = dcpdoctor_core::hdr_validate::HdrValidateOptions {
                video_path: video_file,
                expected_transfer,
                expected_colorimetry: dcpdoctor_core::hdr_validate::Colorimetry::default(),
                expected_bit_depth: 0,
                expected_max_cll: max_cll.unwrap_or(0) as u16,
                expected_max_luminance: max_fall.unwrap_or(0),
            };
            let result = dcpdoctor_core::hdr_validate::validate_hdr_metadata(&opts);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!("HDR Metadata Validation:");
                println!("  Transfer:    {:?}", result.detected.transfer);
                println!("  Colorimetry: {:?}", result.detected.colorimetry);
                println!("  Bit depth:   {}", result.detected.bit_depth);
                if let Some(ref cll) = result.detected.content_light {
                    println!("  MaxCLL:      {}", cll.max_cll);
                    println!("  MaxFALL:     {}", cll.max_fall);
                }
                if result.issues.is_empty() {
                    println!("  Status: PASS");
                } else {
                    println!("  Issues:");
                    for issue in &result.issues {
                        println!("    - {}", issue.description);
                    }
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::FrameCompare {
            imp_a,
            imp_b,
            file_a,
            file_b,
            vmaf,
            html: generate_html,
            extract_diffs,
            output,
        }) => {
            let (a, b) = if let (Some(fa), Some(fb)) = (file_a, file_b) {
                (fa, fb)
            } else if let (Some(ia), Some(ib)) = (imp_a, imp_b) {
                (ia, ib)
            } else {
                eprintln!("Provide either --file-a/--file-b or --imp-a/--imp-b");
                std::process::exit(1);
            };

            let opts = dcpdoctor_core::frame_compare::CompareOptions {
                start_frame: 0,
                end_frame: 0,
                sample_interval: 1,
                threshold_psnr: 30.0,
                threshold_ssim: 0.95,
                compute_ssim: true,
                compute_vmaf: vmaf,
                extract_diff_frames: extract_diffs,
                generate_report: true,
                generate_html,
                output_dir: output.unwrap_or_else(|| PathBuf::from(".")),
                vmaf_model: PathBuf::new(),
            };
            let result = dcpdoctor_core::frame_compare::compare_files(&a, &b, &opts);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!("Frame Comparison:");
                println!("  Frames compared: {}", result.frames_compared);
                println!("  PSNR avg:        {:.2} dB", result.avg_psnr);
                println!("  SSIM avg:        {:.4}", result.avg_ssim);
                if vmaf {
                    println!("  VMAF:            {:.2}", result.vmaf_score);
                }
                if result.identical {
                    println!("  Result:          IDENTICAL");
                } else {
                    println!("  Diff frames:     {}", result.frames_different);
                }
            }
        }
        Some(Commands::ImpInfo { imp_dir }) => {
            let opts = dcpdoctor_core::VerifyOptions {
                check_hashes: false,
                check_signatures: false,
                check_picture_details: false,
                strict_smpte: false,
            };
            let result = dcpdoctor_core::verify(&imp_dir, &opts);
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "directory": imp_dir,
                        "errors": result.error_count,
                        "warnings": result.warning_count,
                        "notes": result.notes,
                    }))
                    .unwrap()
                );
            } else if let Some(info) = dcpdoctor_core::info::get_dcp_info(&imp_dir) {
                println!("IMP Info: {}", imp_dir.display());
                println!("  Title:    {}", info.title);
                println!("  Standard: {}", info.standard);
                println!("  Assets:   {}", info.asset_count);
                println!("  CPLs:     {}", info.cpl_count);
                println!("  PKLs:     {}", info.pkl_count);
                println!("  Duration: {} frames", info.total_duration_frames);
            } else {
                eprintln!("Failed to read IMP at {}", imp_dir.display());
                std::process::exit(1);
            }
        }
        None => {
            if cli.dcp_dirs.is_empty() {
                eprintln!("No DCP directories specified. Use --help for usage.");
                std::process::exit(1);
            }
            run_validate(
                &cli.dcp_dirs,
                false,
                false,
                false,
                false,
                false,
                None,
                None,
                format,
                None,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_validate(
    dcp_dirs: &[PathBuf],
    no_hashes: bool,
    no_signatures: bool,
    check_mxf: bool,
    strict: bool,
    deep_j2k: bool,
    timeline: Option<PathBuf>,
    manifest: Option<PathBuf>,
    format: ReportFormat,
    output: Option<PathBuf>,
) {
    let opts = dcpdoctor_core::VerifyOptions {
        check_hashes: !no_hashes,
        check_signatures: !no_signatures,
        check_picture_details: check_mxf || deep_j2k,
        strict_smpte: strict,
    };

    let mut any_failed = false;
    let mut batch_results = Vec::new();

    for dir in dcp_dirs {
        let mut result = dcpdoctor_core::verify(dir, &opts);

        // Deep J2K validation
        if deep_j2k {
            let j2k_notes = run_deep_j2k(dir);
            for note in j2k_notes {
                result.add(note);
            }
        }

        // Manifest comparison
        if let Some(ref manifest_path) = manifest {
            let manifest_notes = check_manifest(dir, manifest_path);
            for note in manifest_notes {
                result.add(note);
            }
        }

        if !result.ok() {
            any_failed = true;
        }

        // Timeline SVG generation
        if let Some(ref timeline_path) = timeline
            && let Ok(dcp) = dcpdoctor_core::dcp::open_dcp(dir)
        {
            for (_cpl_path, cpl) in &dcp.cpls {
                if let Ok(mut file) = std::fs::File::create(timeline_path) {
                    if let Err(e) = dcpdoctor_core::timeline::write_timeline_svg(cpl, &mut file) {
                        eprintln!("Failed to write timeline SVG: {e}");
                    } else {
                        eprintln!("Timeline SVG written to {}", timeline_path.display());
                    }
                    break;
                }
            }
        }

        batch_results.push((
            dir.clone(),
            result.error_count,
            result.warning_count,
            result.ok(),
        ));

        if let Some(ref output_path) = output {
            let mut file = std::fs::File::create(output_path).unwrap();
            dcpdoctor_core::report::write_report(&result, dir, &mut file, format).unwrap();
        } else {
            let mut stdout = std::io::stdout().lock();
            dcpdoctor_core::report::write_report(&result, dir, &mut stdout, format).unwrap();
        }
    }

    // Batch summary table for multiple DCPs
    if dcp_dirs.len() > 1 && format == ReportFormat::Text {
        println!("\n--- Batch Summary ---");
        println!(
            "{:<50} {:>6} {:>8} {:>6}",
            "DCP", "Errors", "Warnings", "Status"
        );
        println!("{}", "-".repeat(76));
        for (path, errors, warnings, ok) in &batch_results {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            println!(
                "{:<50} {:>6} {:>8} {:>6}",
                name,
                errors,
                warnings,
                if *ok { "PASS" } else { "FAIL" }
            );
        }
        let total_pass = batch_results.iter().filter(|(_, _, _, ok)| *ok).count();
        println!("\n{}/{} passed", total_pass, batch_results.len());
    }

    if any_failed {
        std::process::exit(1);
    }
}

/// Run deep J2K validation on MXF files in a DCP.
fn run_deep_j2k(dcp_dir: &std::path::Path) -> Vec<dcpdoctor_core::Note> {
    let mut notes = Vec::new();

    let entries = match std::fs::read_dir(dcp_dir) {
        Ok(e) => e,
        Err(_) => return notes,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("mxf") {
            match dcpdoctor_core::j2k::analyze_j2k(&path) {
                Ok(info) => {
                    let j2k_notes = dcpdoctor_core::j2k::validate_j2k_dci(&info);
                    for mut note in j2k_notes {
                        note.file = Some(path.clone());
                        notes.push(note);
                    }
                }
                Err(_) => {
                    // Not a picture MXF or ffprobe unavailable; skip
                }
            }
        }
    }

    notes
}

/// Check DCP against a reference manifest JSON.
fn check_manifest(
    dcp_dir: &std::path::Path,
    manifest_path: &std::path::Path,
) -> Vec<dcpdoctor_core::Note> {
    let mut notes = Vec::new();

    let manifest_str = match std::fs::read_to_string(manifest_path) {
        Ok(s) => s,
        Err(e) => {
            notes.push(dcpdoctor_core::Note::error(
                dcpdoctor_core::Code::AssetNotFound,
                format!("Failed to read manifest: {e}"),
            ));
            return notes;
        }
    };

    let manifest: serde_json::Value = match serde_json::from_str(&manifest_str) {
        Ok(v) => v,
        Err(e) => {
            notes.push(dcpdoctor_core::Note::error(
                dcpdoctor_core::Code::XmlParseError,
                format!("Failed to parse manifest JSON: {e}"),
            ));
            return notes;
        }
    };

    if let Some(assets) = manifest["assets"].as_array() {
        for asset in assets {
            let filename = asset["filename"].as_str().unwrap_or("");
            let expected_size = asset["size"].as_u64();

            if filename.is_empty() {
                continue;
            }

            let full_path = dcp_dir.join(filename);
            if !full_path.exists() {
                notes.push(
                    dcpdoctor_core::Note::error(
                        dcpdoctor_core::Code::AssetNotFound,
                        format!("Manifest asset missing: {filename}"),
                    )
                    .with_file(&full_path),
                );
                continue;
            }

            if let Some(expected) = expected_size {
                let actual = std::fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
                if actual != expected {
                    notes.push(
                        dcpdoctor_core::Note::error(
                            dcpdoctor_core::Code::MxfHashMismatch,
                            format!(
                                "Size mismatch for {filename}: expected {expected}, got {actual}"
                            ),
                        )
                        .with_file(&full_path),
                    );
                }
            }
        }
    }

    notes
}

fn is_fixable(code: dcpdoctor_core::Code) -> bool {
    matches!(
        code,
        dcpdoctor_core::Code::PklHashMismatch
            | dcpdoctor_core::Code::SmpteNamespaceWrong
            | dcpdoctor_core::Code::InteropNamespaceWrong
            | dcpdoctor_core::Code::CplInvalidContentKind
    )
}
