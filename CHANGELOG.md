# Changelog

## [0.5.0] — 2025-01-20

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
