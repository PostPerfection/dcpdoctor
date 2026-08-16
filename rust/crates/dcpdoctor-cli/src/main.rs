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

    /// Run studio-specific checks (loudness, color, resolution, encryption, subtitles)
    #[arg(long, global = true)]
    studio: bool,

    /// Deep per-MXF studio analysis (color space, bit depth, resolution per file)
    #[arg(long, global = true)]
    deep: bool,

    /// Netflix IMF delivery spec check
    #[arg(long, global = true)]
    netflix: bool,

    /// HDR metadata detection and compliance
    #[arg(long, global = true)]
    hdr: bool,

    /// Dolby Atmos IAB deep inspection
    #[arg(long, global = true)]
    atmos: bool,

    /// Dolby Vision metadata detection and compliance
    #[arg(long = "dolby-vision", global = true)]
    dolby_vision: bool,

    /// ProRes essence detection (non-DCI)
    #[arg(long, global = true)]
    prores: bool,

    /// Accessibility track validation (AD/HI/CC)
    #[arg(long, global = true)]
    accessibility: bool,

    /// Expect an IMF (IMP) package; warn if the target is a plain DCP
    #[arg(long, global = true)]
    imf: bool,

    /// OV (Original Version) IMP directory, to resolve a supplemental's
    /// cross-package references against the OV's assets
    #[arg(long, global = true, value_name = "OV_DIR")]
    ov: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate DCP or IMF (IMP) directories
    #[command(visible_alias = "validate-imp")]
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

        /// Also write the report into each DCP's own folder
        #[arg(long)]
        report_to_folder: bool,

        /// KDM XML to decrypt an encrypted DCP (with --recipient-key)
        #[arg(long)]
        kdm: Option<PathBuf>,

        /// Recipient RSA private key (PEM) matching the KDM
        #[arg(long)]
        recipient_key: Option<PathBuf>,
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
        /// Compare picture content by perceptual fingerprint
        #[arg(long)]
        fingerprint: bool,
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
        /// Skip hash computation (just check asset presence)
        #[arg(long)]
        no_hash: bool,
    },

    /// Measure or normalize audio loudness (EBU R128 / ATSC A/85)
    Loudness {
        /// Audio file (WAV or MXF)
        audio_file: PathBuf,
        /// Normalize to target loudness instead of only measuring
        #[arg(long)]
        normalize: bool,
        /// Output file for normalized audio
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Target integrated loudness in LUFS
        #[arg(long, default_value = "-23")]
        target: f64,
        /// True-peak limit in dBTP
        #[arg(long, default_value = "-1")]
        true_peak: f64,
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
        /// Freeze-frame noise tolerance for freezedetect (ratio 0-1, ~0.001 = -60dB)
        #[arg(long, default_value = "0.001")]
        freeze_threshold: f64,
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
        #[arg(long)]
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
    },

    /// Display IMP package info
    #[command(name = "imp-info")]
    ImpInfo {
        /// IMP directory
        imp_dir: PathBuf,
    },

    /// Pre-delivery facility check (theater ingest readiness)
    #[command(name = "facility-check")]
    FacilityCheck {
        /// DCP directory
        dcp_dir: PathBuf,
        /// Strict SMPTE compliance
        #[arg(long)]
        strict: bool,
        /// Skip hash verification
        #[arg(long)]
        no_hashes: bool,
        /// Skip ISDCF naming checks
        #[arg(long)]
        no_naming: bool,
    },

    /// Run the DCI conformance test suite
    Conformance {
        /// DCP directory
        dcp_dir: PathBuf,
        /// Skip J2K picture profile tests
        #[arg(long)]
        no_picture: bool,
        /// Skip encryption/security tests
        #[arg(long)]
        no_security: bool,
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
            report_to_folder,
            kdm,
            recipient_key,
        }) => {
            let flags = ValidateFlags {
                no_hashes,
                no_signatures,
                check_mxf,
                strict: strict || bv21,
                bv21,
                deep_j2k,
                studio: cli.studio,
                deep: cli.deep,
                netflix: cli.netflix,
                hdr: cli.hdr,
                atmos: cli.atmos,
                dolby_vision: cli.dolby_vision,
                prores: cli.prores,
                accessibility: cli.accessibility,
                imf: cli.imf,
                ov: cli.ov.clone(),
                timeline,
                manifest,
                output,
                report_to_folder,
                kdm,
                recipient_key,
            };
            run_validate(&dcp_dirs, flags, format);
        }
        Some(Commands::Diff {
            dcp_a,
            dcp_b,
            hashes,
            fingerprint,
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

            if fingerprint {
                match (resolve_imp_video(&dcp_a), resolve_imp_video(&dcp_b)) {
                    (Some(va), Some(vb)) => {
                        let fa = dcpdoctor_core::premium::generate_fingerprint(&va);
                        let fb = dcpdoctor_core::premium::generate_fingerprint(&vb);
                        if fa.hash.is_empty() || fb.hash.is_empty() {
                            eprintln!("Fingerprint unavailable (ffmpeg could not sample a frame)");
                            std::process::exit(1);
                        }
                        let distance = dcpdoctor_core::premium::compare_fingerprints(&fa, &fb);
                        let similarity = (1.0 - distance) * 100.0;
                        println!(
                            "\nPicture fingerprint: {} vs {} ({:.0}% similar)",
                            fa.hash, fb.hash, similarity
                        );
                    }
                    _ => {
                        eprintln!("Fingerprint: no picture asset found in one or both packages");
                        std::process::exit(1);
                    }
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
                    ..Default::default()
                };
                let verify_result = dcpdoctor_core::verify(&dcp_dir, &opts);
                let suggestions = dcpdoctor_core::fixes::suggest_fixes(&verify_result.notes);
                if suggestions.is_empty() {
                    println!("Nothing to fix.");
                } else {
                    println!("{} fix suggestion(s):", suggestions.len());
                    for s in &suggestions {
                        let tag = if s.auto_fixable { "auto" } else { "manual" };
                        println!("  [{tag}] {}", s.description);
                        if !s.command.is_empty() {
                            println!("        $ {}", s.command);
                        }
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
            no_hash,
        }) => {
            if !dcp_dir.exists() {
                eprintln!("Path not found: {}", dcp_dir.display());
                std::process::exit(1);
            }
            if !dcp_dir.is_dir() {
                eprintln!("Not a directory: {}", dcp_dir.display());
                std::process::exit(1);
            }

            // --no-hash still verifies sizes and presence, per docs
            let opts = dcpdoctor_core::checksum_verify::ChecksumVerifyOptions {
                package_dir: dcp_dir,
                verify_hashes: !no_hash,
                verify_sizes: true,
                stop_on_first_error: stop_on_error,
            };
            let result = dcpdoctor_core::checksum_verify::verify_package_checksums(&opts);

            if !result.error.is_empty() {
                eprintln!("{}", result.error);
                std::process::exit(1);
            }

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else if result.all_valid {
                println!(
                    "All checksums verified OK ({} asset(s))",
                    result.total_assets
                );
            } else {
                println!(
                    "{} of {} asset(s) failed: {} hash, {} size, {} missing",
                    result.total_assets - result.verified_ok,
                    result.total_assets,
                    result.hash_mismatches,
                    result.size_mismatches,
                    result.missing_files
                );
                for entry in &result.entries {
                    if !entry.file_exists {
                        println!("  [missing] {} ({})", entry.filename, entry.asset_id);
                    } else if !entry.hash_match {
                        println!("  [hash] {}", entry.filename);
                    } else if !entry.size_match {
                        println!(
                            "  [size] {} (expected {}, got {})",
                            entry.filename, entry.expected_size, entry.actual_size
                        );
                    }
                }
            }
            if !result.all_valid {
                std::process::exit(1);
            }
        }
        Some(Commands::Loudness {
            audio_file,
            normalize,
            output,
            target,
            true_peak,
        }) => {
            if normalize {
                let out = match output {
                    Some(o) => o,
                    None => {
                        eprintln!("--output is required with --normalize");
                        std::process::exit(1);
                    }
                };
                let opts = dcpdoctor_core::loudness::NormalizeOptions {
                    input_file: audio_file,
                    output_file: out,
                    target_lufs: target,
                    true_peak_limit: true_peak,
                };
                let result = dcpdoctor_core::loudness::normalize_loudness(&opts);
                if !result.success {
                    eprintln!("Loudness normalization failed: {}", result.error);
                    std::process::exit(1);
                }
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                } else {
                    println!(
                        "Normalized audio written to {}",
                        result.output_file.display()
                    );
                    println!(
                        "Measured: {:.1} LUFS, {:.1} dBTP",
                        result.measured.integrated_lufs, result.measured.true_peak_dbtp
                    );
                }
            } else {
                match dcpdoctor_core::audio::measure_loudness(&audio_file) {
                    Ok(result) => {
                        // Leq(m) (ISO 21727) reported alongside the EBU R128 result
                        let leq = dcpdoctor_core::loudness::measure_leq_m(&audio_file);
                        if cli.json {
                            let mut val = serde_json::to_value(&result).unwrap();
                            if let Some(obj) = val.as_object_mut() {
                                obj.insert(
                                    "leq_m_db".into(),
                                    serde_json::json!(if leq.success {
                                        Some(leq.leq_m_db)
                                    } else {
                                        None
                                    }),
                                );
                            }
                            println!("{}", serde_json::to_string_pretty(&val).unwrap());
                        } else {
                            println!("Integrated loudness: {:.1} LUFS", result.integrated_lufs);
                            println!("True peak:           {:.1} dBTP", result.true_peak_dbtp);
                            println!("Loudness range:      {:.1} LU", result.loudness_range_lu);
                            if leq.success {
                                println!("Leq(m) (ISO 21727):  {:.1} dB", leq.leq_m_db);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Loudness measurement failed: {e}");
                        std::process::exit(1);
                    }
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
            freeze_threshold,
        }) => {
            let mut findings = Vec::new();

            if let Some(ref video_path) = video {
                match run_qc_ffmpeg(
                    video_path,
                    &format!("blackdetect=d=0.04:pix_th={black_threshold}"),
                ) {
                    Ok(stderr) => {
                        let black_count = stderr.matches("black_start:").count();
                        if black_count > 0 {
                            findings
                                .push(format!("Black frames detected: {black_count} segment(s)"));
                        }
                    }
                    Err(e) => findings.push(format!("Black-frame analysis failed: {e}")),
                }

                match run_qc_ffmpeg(
                    video_path,
                    &format!("freezedetect=n={freeze_threshold}:d=0.5"),
                ) {
                    Ok(stderr) => {
                        let freeze_count = stderr.matches("freeze_start:").count();
                        if freeze_count > 0 {
                            findings
                                .push(format!("Freeze frames detected: {freeze_count} segment(s)"));
                        }
                    }
                    Err(e) => findings.push(format!("Freeze-frame analysis failed: {e}")),
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
            use dcpdoctor_core::imf_compliance::ImfComplianceTarget as T;
            let compliance_target = match target.to_lowercase().as_str() {
                "netflix" => T::Netflix,
                "disney" | "disney+" => T::Disney,
                "amazon" => T::Amazon,
                "apple" => T::Apple,
                "cinema2k" => T::Cinema2K,
                "cinema4k" => T::Cinema4K,
                "broadcast" | "broadcasthd" => T::BroadcastHd,
                "broadcastuhd" => T::BroadcastUhd,
                other => {
                    eprintln!(
                        "Unknown target '{other}' (netflix, disney, amazon, apple, cinema2k, cinema4k, broadcast, broadcastuhd)"
                    );
                    std::process::exit(1);
                }
            };

            // Generic structural validation first
            let opts = dcpdoctor_core::VerifyOptions {
                check_hashes: true,
                check_signatures: true,
                check_picture_details: true,
                strict_smpte: !no_strict,
                ov: cli.ov.clone(),
                ..Default::default()
            };
            let result = dcpdoctor_core::verify(&imp_dir, &opts);

            // Real per-platform delivery-spec checks
            let compliance = dcpdoctor_core::imf_compliance::check_imf_compliance(
                &dcpdoctor_core::imf_compliance::ImfComplianceOptions {
                    imp_dir: imp_dir.clone(),
                    target: compliance_target,
                },
            );
            let pass = result.ok() && compliance.compliant;

            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "target": target,
                        "pass": pass,
                        "errors": result.error_count,
                        "warnings": result.warning_count,
                        "notes": result.notes,
                        "platform_checks": compliance.checks,
                    }))
                    .unwrap()
                );
            } else {
                println!("IMF Compliance Check: {target}");
                println!("---");
                for note in &result.notes {
                    println!("{note}");
                }
                for c in &compliance.checks {
                    println!(
                        "  [{}] {}: expected {}, got {}",
                        if c.passed { "PASS" } else { "FAIL" },
                        c.rule,
                        c.expected_value,
                        if c.actual_value.is_empty() {
                            "unknown"
                        } else {
                            &c.actual_value
                        }
                    );
                }
                if pass {
                    println!("\nPASS: Compliant with {target} requirements");
                } else {
                    println!(
                        "\nFAIL: {} error(s), {} warning(s), {} platform check(s) failed",
                        result.error_count, result.warning_count, compliance.failed
                    );
                }
            }
            if !pass {
                std::process::exit(1);
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
            let mut notes = dcpdoctor_core::schema_validate::check_namespace_consistency(&dcp_dir);
            let xml_files = match std::fs::read_dir(&dcp_dir) {
                Ok(entries) => entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        if !path.is_file() {
                            return false;
                        }

                        let file_name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default();
                        path.extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
                            || file_name.eq_ignore_ascii_case("ASSETMAP")
                            || file_name.eq_ignore_ascii_case("VOLINDEX")
                    })
                    .collect::<Vec<_>>(),
                Err(error) => {
                    notes.push(dcpdoctor_core::Note {
                        severity: dcpdoctor_core::Severity::Error,
                        code: dcpdoctor_core::Code::XmlParseError,
                        message: format!("Failed to read package directory: {error}"),
                        file: Some(dcp_dir.clone()),
                        line: 0,
                    });
                    Vec::new()
                }
            };

            for xml_file in xml_files {
                let schema_result = match schema_dir.as_deref() {
                    Some(schema_dir) => {
                        dcpdoctor_core::schema::validate_schema(&xml_file, schema_dir)
                    }
                    None => dcpdoctor_core::schema::validate_wellformed_file(&xml_file),
                };
                for error in schema_result.errors {
                    notes.push(dcpdoctor_core::Note {
                        severity: dcpdoctor_core::Severity::Error,
                        code: if schema_dir.is_some() {
                            dcpdoctor_core::Code::XmlSchemaViolation
                        } else {
                            dcpdoctor_core::Code::XmlParseError
                        },
                        message: format!("{}:{}: {}", error.line, error.column, error.message),
                        file: Some(xml_file.clone()),
                        line: error.line,
                    });
                }
            }

            let has_errors = notes
                .iter()
                .any(|note| note.severity == dcpdoctor_core::Severity::Error);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&notes).unwrap());
            } else if notes.is_empty() {
                println!("Schema validation passed");
            } else {
                println!("{} schema issue(s):", notes.len());
                for note in &notes {
                    println!("{note}");
                }
            }
            if has_errors {
                std::process::exit(1);
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
                include_codestream_forensics: true,
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
            use dcpdoctor_core::hdr_validate::{Colorimetry, TransferFunction};
            // Dolby Vision is carried over a PQ (ST 2084) base layer
            let (expected_transfer, expected_colorimetry) = match standard.as_deref() {
                Some("hdr10") | Some("pq") => (TransferFunction::Pq, Colorimetry::Bt2020),
                Some("hlg") => (TransferFunction::Hlg, Colorimetry::Bt2020),
                Some("dolby-vision") | Some("dovi") => (TransferFunction::Pq, Colorimetry::Bt2020),
                _ => (TransferFunction::default(), Colorimetry::default()),
            };
            let opts = dcpdoctor_core::hdr_validate::HdrValidateOptions {
                video_path: video_file,
                expected_transfer,
                expected_colorimetry,
                expected_bit_depth: 0,
                expected_max_cll: max_cll.unwrap_or(0) as u16,
                expected_max_fall: max_fall.unwrap_or(0) as u16,
                expected_max_luminance: 0,
            };
            let result = dcpdoctor_core::hdr_validate::validate_hdr_metadata(&opts);
            if !result.success {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                } else {
                    eprintln!("HDR validation failed: {}", result.error);
                }
                std::process::exit(1);
            }
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
        }) => {
            let (a, b) = if let (Some(fa), Some(fb)) = (file_a, file_b) {
                (fa, fb)
            } else if let (Some(ia), Some(ib)) = (imp_a, imp_b) {
                let resolve = |imp: &std::path::Path| match resolve_imp_video(imp) {
                    Some(v) => v,
                    None => {
                        eprintln!("No picture asset found in IMP: {}", imp.display());
                        std::process::exit(1);
                    }
                };
                (resolve(&ia), resolve(&ib))
            } else {
                eprintln!("Provide either --file-a/--file-b or --imp-a/--imp-b");
                std::process::exit(1);
            };

            let opts = dcpdoctor_core::frame_compare::CompareOptions {
                threshold_psnr: 30.0,
                compute_ssim: true,
                compute_vmaf: vmaf,
            };
            let result = dcpdoctor_core::frame_compare::compare_files(&a, &b, &opts);
            if !result.success {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                } else {
                    eprintln!("Frame comparison failed: {}", result.error);
                }
                std::process::exit(1);
            }
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
                ..Default::default()
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
        Some(Commands::FacilityCheck {
            dcp_dir,
            strict,
            no_hashes,
            no_naming,
        }) => {
            let opts = dcpdoctor_core::facility_check::FacilityCheckOptions {
                expected_standard: dcpdoctor_core::dcp::detect_standard(&dcp_dir),
                dcp_dir,
                strict,
                check_naming: !no_naming,
                check_hashes: !no_hashes,
            };
            let result = dcpdoctor_core::facility_check::run_facility_check(&opts);
            if !result.error.is_empty() {
                eprintln!("{}", result.error);
                std::process::exit(1);
            }
            if cli.json {
                println!(
                    "{}",
                    dcpdoctor_core::facility_check::facility_check_to_json(&result)
                );
            } else {
                println!("Facility Check: {}", result.summary);
                println!(
                    "  {}/{} checks passed ({} errors, {} warnings)",
                    result.checks_passed, result.checks_total, result.errors, result.warnings
                );
                for item in &result.items {
                    if !item.passed {
                        println!(
                            "  [{}] {} / {}: {}",
                            item.severity, item.category, item.check_name, item.detail
                        );
                    }
                }
                println!(
                    "  Ready for delivery: {}",
                    if result.ready { "YES" } else { "NO" }
                );
            }
            if !result.ready {
                std::process::exit(1);
            }
        }
        Some(Commands::Conformance {
            dcp_dir,
            no_picture,
            no_security,
        }) => {
            let opts = dcpdoctor_core::conformance::ConformanceOptions {
                dcp_dir,
                check_picture_profile: !no_picture,
                check_security: !no_security,
            };
            let report = dcpdoctor_core::conformance::run_conformance_tests(&opts);
            if !report.error.is_empty() {
                eprintln!("{}", report.error);
                std::process::exit(1);
            }
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            } else {
                println!("DCI Conformance: {}", report.content_title);
                println!(
                    "  {}/{} tests passed ({} failed)",
                    report.tests_passed, report.total_tests, report.tests_failed
                );
                let all = report
                    .structure_tests
                    .iter()
                    .chain(&report.cpl_tests)
                    .chain(&report.picture_tests)
                    .chain(&report.audio_tests)
                    .chain(&report.security_tests);
                for t in all {
                    if !t.passed {
                        println!(
                            "  FAIL [{}] {}: {}",
                            t.spec_reference, t.description, t.detail
                        );
                    }
                }
                println!(
                    "  Conformant: {}",
                    if report.conformant { "YES" } else { "NO" }
                );
            }
            if !report.conformant {
                std::process::exit(1);
            }
        }
        None => {
            if cli.dcp_dirs.is_empty() {
                eprintln!("No DCP directories specified. Use --help for usage.");
                std::process::exit(1);
            }
            let flags = ValidateFlags {
                studio: cli.studio,
                deep: cli.deep,
                netflix: cli.netflix,
                hdr: cli.hdr,
                atmos: cli.atmos,
                dolby_vision: cli.dolby_vision,
                prores: cli.prores,
                accessibility: cli.accessibility,
                imf: cli.imf,
                ov: cli.ov.clone(),
                ..Default::default()
            };
            run_validate(&cli.dcp_dirs, flags, format);
        }
    }
}

#[derive(Default)]
struct ValidateFlags {
    no_hashes: bool,
    no_signatures: bool,
    check_mxf: bool,
    strict: bool,
    bv21: bool,
    deep_j2k: bool,
    studio: bool,
    deep: bool,
    netflix: bool,
    hdr: bool,
    atmos: bool,
    dolby_vision: bool,
    prores: bool,
    accessibility: bool,
    imf: bool,
    ov: Option<PathBuf>,
    timeline: Option<PathBuf>,
    manifest: Option<PathBuf>,
    output: Option<PathBuf>,
    report_to_folder: bool,
    kdm: Option<PathBuf>,
    recipient_key: Option<PathBuf>,
}

/// The --studio ffprobe checks re-cover a couple of findings the core path
/// already reports from CPL declarations. When a deep studio note is present,
/// drop the superseded core note so the finding shows once, then recount.
fn suppress_core_studio_overlap(
    result: &mut dcpdoctor_core::VerifyResult,
    studio: &[dcpdoctor_core::Note],
) {
    use dcpdoctor_core::{Code, Severity};
    let studio_has = |needle: &str| studio.iter().any(|n| n.message.contains(needle));

    // studio "Mixed encryption" reads actual MXF essence state; it supersedes the
    // core reel-coherence note derived from CPL KeyId presence.
    let drop_enc = studio_has("Mixed encryption");
    // studio stereo eye checks (ffprobe frame counts) supersede the core
    // StereoMismatch structural note.
    let drop_stereo = studio_has("Stereoscopic eye")
        || studio_has("missing left eye")
        || studio_has("missing right eye");

    if !drop_enc && !drop_stereo {
        return;
    }

    result.notes.retain(|n| {
        let enc_dup =
            drop_enc && n.code == Code::ReelIncoherent && n.message.contains("encryption");
        let stereo_dup = drop_stereo && n.code == Code::StereoMismatch;
        !enc_dup && !stereo_dup
    });

    result.error_count = result
        .notes
        .iter()
        .filter(|n| n.severity == Severity::Error)
        .count() as u32;
    result.warning_count = result
        .notes
        .iter()
        .filter(|n| n.severity == Severity::Warning)
        .count() as u32;
}

fn run_validate(dcp_dirs: &[PathBuf], flags: ValidateFlags, format: ReportFormat) {
    let opts = dcpdoctor_core::VerifyOptions {
        check_hashes: !flags.no_hashes,
        check_signatures: !flags.no_signatures,
        check_picture_details: flags.check_mxf || flags.deep_j2k,
        // --deep-j2k is what pays for reading past frame 0
        scan_every_frame: flags.deep_j2k,
        strict_smpte: flags.strict,
        ov: flags.ov.clone(),
        kdm: flags.kdm.clone(),
        recipient_key: flags.recipient_key.clone(),
    };

    let mut any_failed = false;
    let mut batch_results = Vec::new();

    for dir in dcp_dirs {
        let mut result = dcpdoctor_core::verify(dir, &opts);

        // --imf: caller asserts this is an IMF package
        if flags.imf && !dcpdoctor_core::imf::is_imf_package(dir) {
            result.add(dcpdoctor_core::Note::warning(
                dcpdoctor_core::Code::MissingRequiredElement,
                "--imf given but target is not an IMF (IMP) package",
            ));
        }

        // --ov must match the main package type: an IMP for IMF, a DCP for a DCP.
        if let Some(ref ov_dir) = flags.ov {
            let main_is_imf = dcpdoctor_core::imf::is_imf_package(dir);
            let ov_is_imf = dcpdoctor_core::imf::is_imf_package(ov_dir);
            let ov_is_package =
                ov_dir.join("ASSETMAP").exists() || ov_dir.join("ASSETMAP.xml").exists();
            let mismatch = if main_is_imf {
                !ov_is_imf
            } else {
                ov_is_imf || !ov_is_package
            };
            if mismatch {
                result.add(dcpdoctor_core::Note::warning(
                    dcpdoctor_core::Code::MissingRequiredElement,
                    "--ov given but the OV target does not match the main package type",
                ));
            }
        }

        // Full BV2.1 application-profile checks
        if flags.bv21 {
            let standard = dcpdoctor_core::dcp::detect_standard(dir);
            for note in dcpdoctor_core::advanced::check_bv21_compliance(dir, standard) {
                result.add(note);
            }
        }

        // Deep J2K validation
        if flags.deep_j2k {
            let j2k_notes = run_deep_j2k(dir);
            for note in j2k_notes {
                result.add(note);
            }
        }

        // Manifest comparison
        if let Some(ref manifest_path) = flags.manifest {
            let manifest_notes = check_manifest(dir, manifest_path);
            for note in manifest_notes {
                result.add(note);
            }
        }

        // Studio checks. The ffprobe essence-level checks supersede a couple of
        // the lighter CPL-declaration notes from the core path, so drop those
        // superseded core notes before appending to avoid a duplicate per finding.
        if flags.studio {
            let studio_notes = dcpdoctor_core::studio::run_studio_checks(dir, flags.deep);
            suppress_core_studio_overlap(&mut result, &studio_notes);
            for note in studio_notes {
                result.add(note);
            }
        }

        // Premium checks (Netflix, HDR, Atmos, Dolby Vision, ProRes, accessibility)
        for note in run_premium_checks(dir, &flags) {
            result.add(note);
        }

        if !result.ok() {
            any_failed = true;
        }

        // Timeline SVG generation
        if let Some(ref timeline_path) = flags.timeline
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

        // Write the report into the DCP's own folder, reusing the report writers.
        if flags.report_to_folder && dir.is_dir() {
            let name = match format {
                ReportFormat::Json => "dcpdoctor-report.json",
                ReportFormat::Html => "dcpdoctor-report.html",
                ReportFormat::Text => "dcpdoctor-report.txt",
            };
            let report_path = dir.join(name);
            match std::fs::File::create(&report_path) {
                Ok(mut file) => {
                    dcpdoctor_core::report::write_report(&result, dir, &mut file, format).unwrap();
                    eprintln!("Report written to {}", report_path.display());
                }
                Err(e) => eprintln!("Failed to write report to {}: {e}", report_path.display()),
            }
        }

        if let Some(ref output_path) = flags.output {
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

/// Run the descriptor-level J2K profile checks on the MXF files in a DCP. The
/// per-frame codestream scan --deep-j2k also turns on runs inside verify_dcp,
/// which already holds the KDM keys and reads the essence once.
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

/// Run an ffmpeg detect filter and return its stderr, or an error on failure.
fn run_qc_ffmpeg(video: &std::path::Path, filter: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(video)
        .arg("-vf")
        .arg(filter)
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("failed to run ffmpeg: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.trim().lines().last().unwrap_or("unknown error");
        return Err(format!("ffmpeg exited with error: {tail}"));
    }
    Ok(String::from_utf8_lossy(&output.stderr).into_owned())
}

/// Resolve the picture-track MXF inside a DCP/IMP directory.
///
/// Prefers the MainPicture asset referenced by a CPL; falls back to the largest
/// MXF (the picture track is far larger than audio) so we never hand ffmpeg a
/// directory.
fn resolve_imp_video(imp_dir: &std::path::Path) -> Option<PathBuf> {
    if let Ok(dcp) = dcpdoctor_core::dcp::open_dcp(imp_dir) {
        let id_to_path: std::collections::HashMap<&str, &str> = dcp
            .assetmap
            .assets
            .iter()
            .map(|a| (a.id.as_str(), a.path.as_str()))
            .collect();
        for (_p, cpl) in &dcp.cpls {
            for reel in &cpl.reels {
                if reel.picture.id.is_empty() {
                    continue;
                }
                if let Some(&path) = id_to_path.get(reel.picture.id.as_str()) {
                    let full = imp_dir.join(path);
                    if full.exists() {
                        return Some(full);
                    }
                }
            }
        }
    }

    // Fallback: largest MXF in the directory.
    mxf_files(imp_dir)
        .into_iter()
        .max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
}

/// List top-level MXF files in a directory.
fn mxf_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("mxf"))
        .collect()
}

/// Run premium delivery checks selected by validate flags.
fn run_premium_checks(dir: &std::path::Path, flags: &ValidateFlags) -> Vec<dcpdoctor_core::Note> {
    use dcpdoctor_core::premium;
    let mut notes = Vec::new();

    if flags.netflix {
        let r = premium::check_netflix_delivery(dir);
        notes.extend(premium::netflix_to_notes(&r, dir));
    }
    if flags.accessibility {
        notes.extend(premium::check_accessibility(dir));
    }

    if flags.hdr || flags.atmos || flags.dolby_vision || flags.prores {
        for mxf in mxf_files(dir) {
            if flags.hdr {
                let h = premium::detect_hdr_metadata(&mxf);
                notes.extend(premium::check_hdr_compliance(&h, &mxf));
            }
            if flags.atmos {
                let a = premium::parse_atmos_iab(&mxf);
                notes.extend(premium::check_atmos_compliance(&a, &mxf));
            }
            if flags.dolby_vision {
                let dv = premium::parse_dolby_vision(&mxf);
                notes.extend(premium::check_dolby_vision_compliance(&dv, &mxf));
            }
            if flags.prores {
                let p = premium::detect_prores(&mxf);
                if p.detected {
                    notes.push(dcpdoctor_core::Note {
                        severity: dcpdoctor_core::Severity::Warning,
                        code: dcpdoctor_core::Code::MxfInvalidStructure,
                        message: format!(
                            "ProRes essence detected ({}, {}x{}); DCI requires JPEG 2000",
                            p.codec_variant, p.width, p.height
                        ),
                        file: Some(mxf.clone()),
                        line: 0,
                    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use dcpdoctor_core::{Code, Note, VerifyResult};

    #[test]
    fn studio_mixed_encryption_suppresses_core_encryption_incoherence() {
        let mut result = VerifyResult::default();
        result.add(Note::error(
            Code::ReelIncoherent,
            "Reel 2 picture encryption 'clear' is not coherent with earlier reels ('encrypted')",
        ));
        result.add(Note::error(Code::CplMissingReel, "unrelated"));
        let studio = vec![Note::error(
            Code::MxfInvalidStructure,
            "Mixed encryption: 2 encrypted + 1 unencrypted assets",
        )];
        suppress_core_studio_overlap(&mut result, &studio);
        assert!(
            !result.notes.iter().any(|n| n.code == Code::ReelIncoherent),
            "core encryption-coherence note should be suppressed by the deep studio note"
        );
        assert!(result.notes.iter().any(|n| n.code == Code::CplMissingReel));
        assert_eq!(result.error_count, 1);
    }

    #[test]
    fn no_studio_deep_note_keeps_core_notes() {
        let mut result = VerifyResult::default();
        result.add(Note::error(
            Code::ReelIncoherent,
            "Reel 2 picture encryption 'clear' is not coherent",
        ));
        suppress_core_studio_overlap(&mut result, &[]);
        assert!(result.notes.iter().any(|n| n.code == Code::ReelIncoherent));
        assert_eq!(result.error_count, 1);
    }
}
