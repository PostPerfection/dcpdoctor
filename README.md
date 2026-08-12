# DcpDoctor

[![CI](https://github.com/PostPerfection/dcpdoctor/actions/workflows/ci.yml/badge.svg)](https://github.com/PostPerfection/dcpdoctor/actions/workflows/ci.yml)

[Documentation](https://postperfection.github.io/dcpdoctor/)

Current release: `v0.1.1`.

A comprehensive, professional-grade DCP (Digital Cinema Package) validator, analyzer, and diagnostic tool. Written in Rust.

DcpDoctor validates DCPs against SMPTE ST 429/ST 2067, Interop, and BV2.1 standards with the depth and precision required for theatrical distribution.

## Features

### Core Validation
- **Structure validation**: ASSETMAP, PKL, CPL parsing with full cross-referencing
- **Hash verification**: SHA-1 integrity checking for all assets
- **XML digital signatures**: enveloped signature verification plus embedded X.509 chain linkage and expiry checks; an encrypted package with an unsigned CPL or PKL errors (`dcp_not_signed`), an unsigned unencrypted one warns (`unencrypted_dcp_not_signed`)
- **Schema validation:** Well-formedness checks for every package XML file, with full XSD validation when schemas are supplied
- **Duplicate detection**: Identifies duplicate asset IDs across packages

### Standards Compliance
- **SMPTE ST 429**: Complete SMPTE DCP standard validation
- **Interop**: Legacy Interop DCP support
- **BV2.1 (Bv2.1)**: SMPTE Best Practices for theatrical distribution:
  - ASSETMAP.xml naming enforcement
  - PKL .xml extension check
  - ContentVersion element requirement
  - ExtensionMetadata presence
  - MainMarkers in first reel
  - Approved EditRate validation (24/25/30/48/60 fps)
- **ISDCF Naming**: Content title naming convention validation

### Picture Validation
- **J2K bitrate analysis**: Per-frame bitrate statistics (min/max/avg)
- **DCI bitrate limits**: 250 Mbps (2K) / 500 Mbps (4K) enforcement
- **Deep J2K codestream**: Profile (RSIZ), decomposition levels, code-block sizes, wavelet type, component validation
- **4K/2K detection**: Resolution and aspect ratio verification

### Sound Validation
- **Audio level analysis**: Per-channel peak and RMS in dBFS
- **Clipping detection**: Flags audio near 0 dBFS
- **Silence detection**: Warns on channels below -80 dBFS
- **Channel count**: Validates channel configuration
- **MainSoundConfiguration (ST 429-16)**: Presence, `<soundfield>/<channels>` syntax with MCA/ISDCF labels, and channel count matched against the sound MXF (flags garbage like `None`)
- **Quantization / block align**: 24-bit PCM and block-align check (`--check-mxf`)
- **MCA labeling**: Multi-Channel Audio label presence check
- **Audio sync drift**: Detects picture/sound duration mismatches per reel
- **DTS:X**: Immersive-audio detection under `--studio --deep`

### Subtitle & Caption Validation
- **SMPTE ST 429-5** timed text support
- **Timing validation**: TimeIn/TimeOut ordering and overlap detection
- **First-event timing (Bv2.1)**: Warns when the first reel's first subtitle starts under 4s in; ignores empty placeholder assets (avoids DCP-o-matic bug #2757)
- **Line count & length (Bv2.1)**: More than 3 lines at once, and subtitle lines over 52 (recommended) / 79 (max) characters, warn; closed-caption lines over 32 characters error. Character counts are unicode scalar values, not bytes
- **Duration & spacing (Bv2.1)**: Warns on timed-text events shorter than 15 frames or gaps under 2 frames
- **Closed-caption character set**: Info note lists characters outside the ISDCF Doc 9 set (ISO 8859-1 plus U+266A)
- **Required element checks**: ReelNumber, Language, LoadFont
- **SubtitleID presence**: Unique identifier validation

### Encryption & Security
- **Encrypted content detection**: Identifies encrypted MXF assets
- **KDM validation**: Parse and validate Key Delivery Messages:
  - Validity period checking (expired / not-yet-valid)
  - CPL reference cross-validation against DCP
  - Content title extraction

### Dolby Atmos
- **IAB detection**: Identifies Immersive Audio Bitstream essence via ffprobe (with an estimated object count)

### Reel & Structure Analysis
- **Reel continuity**: Validates sequential entry points across reels
- **Stereo 3D**: Checks left/right eye consistency
- **Marker validation**: FFOC, LFOC, FFMC, LFMC presence (strict mode), plus FFOC=1 / LFOC=(reel duration - 1) offset checks per reel
- **Cross-reference integrity**: All PKL/CPL asset references resolve
- **Supplemental DCP**: Original Package List validation
- **CPL metadata**: ContentTitleText/IssueDate, and SMPTE ContentVersion
- **Package hygiene**: Flags unreferenced and zero-byte files in the package dir

### Advanced Tools
- **DCP comparison/diff**: Side-by-side structural comparison of two DCPs
- **Checksum verification**: Verify all PKL asset hashes and sizes (DCP or IMF)
- **MXF essence extraction**: Extract video/audio tracks from MXF containers
- **Automated QC**: Detect black frames, freeze frames, audio silence, and audio clipping
- **IMP validation:** Route ST 2067 IMF packages to native checks and Netflix Photon without running IMF tools on DCPs
- **Schema validation**: XML schema validation against SMPTE ST 2067 XSDs
- **IMF compliance**: Platform-specific compliance checks (Netflix, Disney, Amazon, Apple, Cinema, Broadcast)
- **Frame-level QC**: Per-frame J2K bitrate analysis with over/under-budget detection
- **QC reports**: HTML/PDF QC reports with package and track summary, plus per-track EBU R128 loudness
- **Loudness measurement**: EBU R128 / ATSC A/85 and ISO 21727 Leq(m) measurement, plus normalization
- **AV sync detection**: Audio/video sync drift detection and measurement
- **HDR validation**: HDR10, HLG, Dolby Vision metadata validation
- **Frame comparison**: Frame-by-frame PSNR/SSIM/VMAF comparison between IMPs or files
- **IMP info**: Display IMP package structure, tracks, and metadata
- **Theater compatibility profiles**: Pre-built profiles for major server vendors:
  - Dolby IMS3000, IMS2000, Cinema (Premium)
  - Barco SP4K, SP2K
  - Christie CP4440-RGB, CP2230
  - GDC SX-4000, SR-1000
  - IMAX Digital
- **Automated fix suggestions**: Actionable remediation advice for common issues
- **SVG timeline visualization**: Visual reel structure diagram with timecodes
- **Manifest comparison**: Validate DCP against a reference manifest JSON
- **Content fingerprinting**: Perceptual picture hash to compare two packages (`diff --fingerprint`)
- **Batch processing**: Multi-DCP validation with summary table

### Output & Integration
- **Text/JSON/HTML reports**: Multiple output formats
- **REST API**: HTTP server mode (POST /validate, GET /health)
- **Directory watch**: Auto-validates new DCPs as they appear
- **Exit codes**: Machine-parseable pass/fail status

## Installation

### Pre-built binaries (recommended)

Download from the [GitHub Releases](https://github.com/PostPerfection/dcpdoctor/releases/latest) page:

| Platform | CLI | Desktop GUI |
|----------|-----|-------------|
| **Linux** (x86_64) | `dcpdoctor-linux-x86_64.tar.gz` | `.deb`, `.AppImage` |
| **macOS** (Apple Silicon) | `dcpdoctor-macos-aarch64.tar.gz` | `.dmg` |
| **Windows** (x86_64) | `dcpdoctor-windows-x86_64.zip` | `.msi` |

The CLI binary is fully self-contained (all dependencies statically linked). Extract and run.

### Install from source

The build itself needs only a Rust toolchain (1.85+). The following tools are runtime dependencies, invoked when the relevant checks run:

- `ffmpeg` / `ffprobe`: media analysis (auto-qc, loudness, HDR, Atmos, frame-compare, mxf-extract)
- `xmllint`: XSD schema validation (`schema-validate --schema-dir`)

#### Linux (Ubuntu/Debian)

```bash
sudo apt-get install -y ffmpeg libxml2-utils
# For GUI: also install libwebkit2gtk-4.1-dev librsvg2-dev libgtk-3-dev libsoup-3.0-dev

cd rust
cargo build --release
# Binary at rust/target/release/dcpdoctor
```

#### macOS

```bash
brew install ffmpeg libxml2

cd rust
cargo build --release
```

#### Windows

```powershell
# ffmpeg (includes ffprobe) and xmllint via your package manager, e.g.:
winget install Gyan.FFmpeg

cd rust
cargo build --release
```

## Usage

### Basic Validation

```bash
# Validate a DCP
dcpdoctor /path/to/dcp

# Verbose output (shows INFO notes)
dcpdoctor -v /path/to/dcp

# Quiet mode (errors only)
dcpdoctor -q /path/to/dcp

# Multiple DCPs (shows batch summary)
dcpdoctor /dcp1 /dcp2 /dcp3
```

### Standards & Compliance

```bash
# BV2.1 application profile check
dcpdoctor validate --bv21 /path/to/dcp

# Strict SMPTE compliance
dcpdoctor validate --strict /path/to/dcp

# Deep J2K codestream validation
dcpdoctor validate --deep-j2k /path/to/dcp

# MXF essence inspection (bitrate, audio levels)
dcpdoctor validate --check-mxf /path/to/dcp
```

### Reports & Output

```bash
# JSON report
dcpdoctor --json /path/to/dcp

# HTML report
dcpdoctor validate --html -o report.html /path/to/dcp

# SVG timeline visualization
dcpdoctor validate --timeline timeline.svg /path/to/dcp

# Auto-fix repairable issues
dcpdoctor fix /path/to/dcp

# Dry-run (show what would be fixed without modifying)
dcpdoctor fix --dry-run /path/to/dcp
```

### Auto-Fix

The `fix` subcommand automatically repairs common issues:

| Issue | What it fixes |
|-------|---------------|
| PKL hash mismatch | Recomputes SHA-1 and rewrites PKL |
| Wrong namespace | Swaps Interop↔SMPTE namespace URIs |
| Invalid ContentKind | Normalizes to canonical SMPTE value |

After fixing XML files, PKL hashes are automatically recalculated to keep everything consistent.

```bash
# Fix and then re-validate
dcpdoctor fix /path/to/dcp && dcpdoctor validate --strict /path/to/dcp
```

### Photon Integration (IMF)

dcpdoctor integrates [Netflix Photon](https://github.com/Netflix/photon) for deep IMF Application 2/2E conformance checks. It runs only when an ST 2067-3 Composition Playlist identifies an IMF package. SMPTE and Interop DCPs skip IMF and Photon validation.

Photon has to be fetched first. dcpdoctor does not build it: Netflix pins Gradle 8.5, which cannot read Java 25 class files. Use imfwizard's `scripts/fetch_photon.sh`, which pulls the jars from Maven Central into `$PHOTON_DIR`. Without jars, validation runs everything else and reports the skipped Photon pass as an INFO note.

**Requirements:** Java 11+ and Photon jars.

```bash
export PHOTON_DIR=~/.cache/photon
dcpdoctor validate /path/to/imp
```

Photon findings are merged into dcpdoctor's report with `[Photon]` prefix. Discovery order:
1. `PHOTON_DIR` environment variable (a jar, a directory of jars, or one with `build/libs/*.jar`)
2. `/usr/local/share/photon/libs/`
3. `/usr/share/photon/libs/`
4. `/opt/photon/build/libs/`
5. `~/.cache/dcpdoctor/photon/` and `~/.cache/dcpdoctor/photon/build/libs/`

### DCP Comparison

```bash
# Compare two DCPs
dcpdoctor diff /path/to/dcp_v1 /path/to/dcp_v2

# Include content hash comparison (slower)
dcpdoctor diff --hashes /path/to/dcp_v1 /path/to/dcp_v2

# Compare picture content by perceptual fingerprint
dcpdoctor diff --fingerprint /path/to/dcp_v1 /path/to/dcp_v2
```

### KDM Validation

```bash
# Validate KDM file
dcpdoctor kdm /path/to/kdm.xml

# Validate KDM against specific DCP
dcpdoctor kdm /path/to/kdm.xml --dcp /path/to/dcp

# Verify an encrypted DCP: decrypt essence with a KDM and run the full checks
dcpdoctor validate /path/to/dcp --kdm /path/to/kdm.xml --recipient-key /path/to/private.pem
```

### Theater Profiles

```bash
# List all built-in theater profiles
dcpdoctor profiles

# Check DCP against specific theater
dcpdoctor profiles --check "dolby ims3000" --dcp /path/to/dcp
dcpdoctor profiles --check "imax" --dcp /path/to/dcp
```

### Manifest Comparison

```bash
# Compare DCP against reference manifest
dcpdoctor validate --manifest manifest.json /path/to/dcp
```

Manifest JSON format:
```json
{
  "assets": [
    {"filename": "picture.mxf", "size": 1234567890},
    {"filename": "sound.mxf", "size": 987654321}
  ]
}
```

### Server & Watch Modes

```bash
# REST API server
dcpdoctor serve --port 8080

# Auto-validate new DCPs in a directory
dcpdoctor watch /ingest/incoming --interval 5000
```

REST API endpoints:
- `GET /health`: Returns `{"status": "ok"}`
- `POST /validate`: Body: `{"path": "/path/to/dcp"}`, returns validation result. Add an optional `"ov": "/path/to/ov"` to resolve a supplemental package's cross-package references against the OV.

### Performance Options

```bash
# Skip hash verification (fast structural check only)
dcpdoctor validate --no-hashes /path/to/dcp

# Skip signature verification
dcpdoctor validate --no-signatures /path/to/dcp
```

### Checksum Verification

```bash
# Verify all PKL checksums in a DCP or IMP
dcpdoctor checksum-verify /path/to/dcp

# JSON output
dcpdoctor checksum-verify --json /path/to/dcp

# Skip hash computation (just check sizes)
dcpdoctor checksum-verify --no-hash /path/to/dcp

# Stop on first mismatch
dcpdoctor checksum-verify --stop-on-error /path/to/dcp
```

### MXF Extraction

```bash
# Extract all essence from an MXF file
dcpdoctor mxf-extract /path/to/picture.mxf -o /output/dir

# Extract only audio
dcpdoctor mxf-extract /path/to/sound.mxf -o /output/dir --no-video

# Extract specific frame range
dcpdoctor mxf-extract /path/to/picture.mxf -o /output/dir --start-frame 100 --end-frame 200
```

### Automated QC

```bash
# Run full QC on a video file
dcpdoctor auto-qc --video /path/to/content.mxf

# QC with JSON output
dcpdoctor auto-qc --video /path/to/video.mxf --audio /path/to/audio.wav --json

# Custom thresholds
dcpdoctor auto-qc --video /path/to/content.mxf \
  --black-threshold 0.95 \
  --freeze-threshold 0.005 \
  --silence-threshold -50 \
  --clipping-threshold -1.0
```

### IMP Validation

```bash
# Validate an IMF package via Netflix Photon
dcpdoctor validate-imp /path/to/IMP/

# Check every package XML file for well-formedness
dcpdoctor schema-validate /path/to/IMP/

# Also validate recognized package XML files against supplied XSDs
dcpdoctor schema-validate /path/to/IMP/ --schema-dir /path/to/xsd/
```

### IMF Compliance

```bash
# Check Netflix delivery compliance
dcpdoctor imf-compliance /path/to/IMP/ --target netflix

# Check Cinema 4K compliance (non-strict)
dcpdoctor imf-compliance /path/to/IMP/ --target cinema4k --no-strict
```

### Frame-Level QC

```bash
# Analyze J2K bitrate compliance
dcpdoctor frame-qc /path/to/j2k/frames/ --max-bitrate 300 --min-bitrate 50
```

### QC Report

```bash
# Generate detailed HTML report
dcpdoctor qc-report /path/to/IMP/ -o report.html --title "Feature Film QC"

# PDF report with client name
dcpdoctor qc-report /path/to/IMP/ -o report.pdf --client "Studio A"
```

### Loudness

```bash
# Measure EBU R128 loudness
dcpdoctor loudness /path/to/audio.wav

# Normalize to -23 LUFS
dcpdoctor loudness /path/to/audio.wav -o normalized.wav --normalize --target -23
```

### AV Sync

```bash
# Check sync between video and audio
dcpdoctor av-sync -v /path/to/video.mxf -a /path/to/audio.wav --fps-num 24
```

### HDR Validation

```bash
# Validate HDR10 metadata
dcpdoctor hdr-validate /path/to/video.mxf -s hdr10

# Validate with expected values
dcpdoctor hdr-validate /path/to/video.mxf -s hdr10 --max-cll 1000 --max-fall 400
```

### Frame Comparison

```bash
# Compare two IMPs (resolves each IMP's picture asset)
dcpdoctor frame-compare --imp-a /path/to/IMP_v1/ --imp-b /path/to/IMP_v2/

# Compare two files with VMAF
dcpdoctor frame-compare --file-a ref.mxf --file-b test.mxf --vmaf
```

### IMP Info

```bash
# Display IMP package details
dcpdoctor imp-info /path/to/IMP/
```

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | All DCPs passed validation |
| `1` | One or more DCPs failed |
| `2` | Usage/configuration error |

## Environment Variables

| Variable | Effect |
|---|---|
| `PHOTON_DIR` | Path to an existing Netflix Photon install (skips auto-bootstrap) |
| `RUST_BACKTRACE` | Set to `1` for a detailed backtrace on crash |

## Running Tests

```bash
cd rust
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

GitHub Actions runs these Rust checks on Linux, macOS, and Windows. It also builds the frontend and checks the Tauri Rust crate on Linux.

### Studio Validation

```bash
# Studio-level checks (loudness, color, resolution, encryption, subtitles)
dcpdoctor --studio /path/to/dcp

# Deep per-MXF analysis (color space, bit depth, resolution per file)
dcpdoctor --studio --deep /path/to/dcp

# Netflix IMF delivery spec check
dcpdoctor --netflix /path/to/imf

# HDR metadata detection
dcpdoctor --hdr /path/to/dcp

# Dolby Atmos IAB deep inspection
dcpdoctor --atmos /path/to/dcp

# Accessibility track validation (AD/HI/CC)
dcpdoctor --accessibility /path/to/dcp

# Dolby Vision metadata detection and compliance
dcpdoctor --dolby-vision /path/to/dcp

# ProRes essence detection (non-DCI)
dcpdoctor --prores /path/to/dcp
```

### Facility Check

```bash
# Pre-delivery readiness check for theater ingest
dcpdoctor facility-check /path/to/dcp

# Strict, skip hashing and naming checks
dcpdoctor facility-check /path/to/dcp --strict --no-hashes --no-naming
```

### DCI Conformance

```bash
# Run the DCI conformance test suite
dcpdoctor conformance /path/to/dcp

# Skip picture-profile or security tests
dcpdoctor conformance /path/to/dcp --no-picture --no-security
```

## Desktop GUI (Tauri)

DcpDoctor includes an optional desktop GUI built with [Tauri](https://tauri.app), providing a modern native interface for DCP validation.

### GUI Features

- **Drag & drop**: Drop a DCP folder to validate
- **Visual results**: Color-coded severity badges (error/warning/info)
- **Filterable table**: Filter results by severity
- **Option chips**: Toggle Studio, Deep, Netflix, HDR, Atmos, IMF, Accessibility checks
- **Cross-platform**: Builds for Linux (.deb, .rpm, AppImage), macOS (.dmg), Windows (.msi)
- **Sidecar architecture**: Bundles the `dcpdoctor` CLI binary, no separate install needed

### GUI Prerequisites

| Dependency | Platform | Install |
|---|---|---|
| Rust 1.85+ | All | [rustup.rs](https://rustup.rs) |
| Node.js 18+ | All | [nodejs.org](https://nodejs.org) |
| webkit2gtk-4.1 | Linux | `sudo dnf install webkit2gtk4.1-devel` (Fedora) / `sudo apt install libwebkit2gtk-4.1-dev` (Debian) |
| librsvg2 | Linux | `sudo dnf install librsvg2-devel` (Fedora) / `sudo apt install librsvg2-dev` (Debian) |
| gtk3 | Linux | `sudo dnf install gtk3-devel` (Fedora) / `sudo apt install libgtk-3-dev` (Debian) |
| libsoup3 | Linux | `sudo dnf install libsoup3-devel` (Fedora) / `sudo apt install libsoup-3.0-dev` (Debian) |

### Build the GUI

```bash
# Build the CLI (needed as sidecar)
cd rust && cargo build --release && cd ..

# Copy CLI binary as sidecar
cp rust/target/release/dcpdoctor gui/src-tauri/dcpdoctor-$(rustc -vV | grep host | cut -d' ' -f2)

# Install frontend dependencies
cd gui && pnpm install

# Build the desktop app
pnpm tauri build
```

Built packages are in `gui/src-tauri/target/release/bundle/`:
- **Linux:** `.deb`, `.rpm`, `.AppImage`
- **macOS:** `.dmg`
- **Windows:** `.msi`

### Development Mode

```bash
cd gui
pnpm tauri dev
```

This starts a hot-reloading dev server: edit `gui/src/` files and see changes live.

## Architecture

```
dcpdoctor/
├── rust/                 # Rust workspace
│   ├── crates/
│   │   ├── dcpdoctor-core/   # Core validation library
│   │   └── dcpdoctor-cli/    # CLI binary
│   └── Cargo.toml
├── gui/                  # Tauri desktop GUI
│   ├── src/              # Web frontend (HTML/CSS/JS)
│   ├── src-tauri/        # Rust backend (IPC commands)
│   └── package.json      # Node.js dependencies
└── docs/                 # GitHub Pages website
```

## License

AGPL-3.0-or-later. Copyright (C) 2026 Grok Image Compression Inc. See [LICENSE](LICENSE).
