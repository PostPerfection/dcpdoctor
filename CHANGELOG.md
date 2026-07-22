# Changelog

## [Unreleased]

### Added
- MainSoundConfiguration (ST 429-16): SMPTE CPLs are checked for the tag in CompositionMetadataAsset, its `<soundfield>/<channels>` value is parsed against the MCA/ISDCF label set (garbage like `None` errors as `main_sound_config_invalid`), and the declared channel count is cross-checked against the sound MXF's ChannelCount (`sound_invalid_channel_count`).
- FFOC/LFOC marker offsets: FFOC in the first reel must be 1 and LFOC in the last reel must be one less than that reel's duration, warned per libdcp wording (`marker_invalid`).
- First-subtitle timing (Bv2.1): warns (`subtitle_first_event_early`) when the first reel's first displayable timed-text event starts under 4s in. Only the first reel counts and empty placeholder subtitle assets are ignored, avoiding DCP-o-matic bug #2757. Reads MXF-wrapped ST 428-7 XML and handles both SMPTE tick and Interop editable-unit TimeIn forms.
- Stereoscopic 3D (ST 429-10): the msp-cpl `MainStereoscopicPicture` form is now validated as a picture track (cross-refs, duration), the FrameRate = 2x EditRate relationship is checked, and the essence type is confirmed as stereoscopic J2K where the MXF is present.
- Auxiliary data (ST 429-18): an `aux_data_detected` INFO identifies each AuxData track (Dolby Atmos / IAB), enriched with the probed essence type, and a duration mismatch against the reel's picture warns as `cpl_mismatched_durations`. The aux asset's cross-refs and PKL hashes are covered by the generic asset checks.
- PKL `Size` is checked against the actual file size (`pkl_size_mismatch`), independent of `--no-hashes`.

### Fixed
- MCA channel labeling is read from the sound MXF's ST 429-12 subdescriptors (asdcplib `pcm::mca_labels`) instead of grepping the CPL, so `sound_invalid_channel_count` no longer false-fires on correctly labeled DCPs. Falls back to the CPL markers only for XML-only validation.
- Cross-reference and reel-coherence checks now match the namespaced (`msp-cpl:`, `axd:`) reel-asset forms real DCPs emit, so their asset ids are no longer skipped.

### Changed
- Schema validation (`xml_schema_violation`) is now on by default: the SMPTE/Interop XSDs are vendored under `schemas/` (from the ClairMeta set, with `catalog.xml`), so `DCPDOCTOR_SCHEMA_DIR` is no longer required. The env var still overrides, and a missing dir degrades to skip.
- Bumped asdcplib pin to `5fe4d61` (adds `pcm::mca_labels`), aligned across the vendored postkit.

## [0.1.1] - 2026-07-19

### Added
- Sound essence checks: 24-bit PCM quantization and block-align (`--check-mxf`).
- CPL metadata checks (ContentTitleText/IssueDate, SMPTE ContentVersion).
- Package hygiene: unreferenced (`foreign_file_in_package`) and zero-byte (`empty_file_in_package`) files.
- DTS:X immersive-audio detection under `--studio --deep`.
- Web validator: optional "+ OV" folder picker resolves a supplemental package's cross-package references in the browser.

### Fixed
- IMF and Photon validation now run only for packages with an ST 2067-3 CPL.
- `schema-validate` now parses every package XML file and applies supplied XSDs per file.

### Changed
- GUI preferences trimmed to the two backed by a real validate flag (verify hashes, inspect MXF essence) and applied to every run.
- `--studio` no longer double-reports encryption/stereo findings the core path already covers.
- Removed dead modules (qc, auto_qc, kdm_advanced, dci_ctp, and stray helpers); hash and loudness now delegate to postkit.
- CI now checks the Rust workspace on all three platforms and the Tauri GUI on Linux.
- GUI dependency management now uses pnpm.

## [1.1.0] — 2026-05-28

### Added
- **Browser-based DCP validator** — WASM-powered validator hosted on GitHub Pages
- **Online validator embedded in landing page** — Unified documentation + validation experience
- **Cancel button** — Abort validation in progress
- **Preferences panel** — Gear icon toggles settings (default standard, hash/schema/bitrate/loudness checks, max bitrate, report format, output dir, schema dir); saves to localStorage

### Fixed
- **Large DCP performance** — Skip binary file content to prevent page lockup
- **XML-only metadata reading** — Only parse XML files, skip MXF/J2C binaries
- **Directory picker fallback** — Fallback for Brave/Firefox/Safari browsers

## [1.0.0] — 2025-01-20

### Added
- **CLI: Validate subcommand** — Full DCP/IMF validation
  - `--no-hashes` to skip hash verification
  - `--no-signatures` to skip signature verification
  - `--check-mxf` for MXF essence metadata inspection
  - `--strict` for SMPTE-strict mode
  - `--output` for writing reports to file
- **CLI: Diff subcommand** — Compare two DCPs side-by-side
- **CLI: Info subcommand** — Display DCP metadata
- **CLI: Watch subcommand** — Monitor directory for new DCPs
- **CLI: Serve subcommand** — REST API server for validation
- **JSON output** — `--json` flag for machine-readable reports
- **HTML output** — `--html` flag for browser-viewable reports
- **Shorthand positional arguments** — `dcpdoctor /path/to/dcp` validates directly
- **Panic hook** — User-friendly crash messages with issue tracker link
- **CLI integration tests** — 8 end-to-end tests using assert_cmd
- **Release CI** — GitHub Actions workflow for building release binaries on tag push
- **GUI Release CI** — Tauri build workflow producing .deb, .AppImage, .dmg, .msi

### Changed
- Version unified to 0.5.0 across all workspace crates
- Git dependencies pinned to v0.5.0 tags (asdcplib-rs, postkit)

### Fixed
- Clippy warnings cleaned up across entire workspace
