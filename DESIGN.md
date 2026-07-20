# Design

DCP/IMF validation tool. Rust core with CLI, Tauri GUI, and a browser WASM validator.

## Layout

- `rust/crates/dcpdoctor-core`: validation engine and media tooling. Shares code via postkit (path dep) and asdcplib-rs (git).
- `rust/crates/dcpdoctor-cli`: clap CLI, all subcommands.
- `rust/crates/dcpdoctor-parse`, `-imf`: no-std-io XML parsers shared with wasm.
- `rust/crates/dcpdoctor-wasm`: browser validator; deliberately avoids postkit, uses -parse/-imf.
- `gui/`: Tauri app (drag & drop, filters, sidecar CLI).
- `web/`: embedded WASM validator page, deployed to Pages.
- `schemas/`: XSDs for schema-validate (xmllint).
- dci-ctp (sibling repo) holds the DCI CTP test suite that runs against this tool.

## What is implemented and wired

- Structure validation: ASSETMAP/PKL/CPL parse, duplicate IDs, asset existence, PKL hash verify, PKL to ASSETMAP cross-reference.
- Core CPL checks (always on): encrypted-content/KDM detection, CPL-to-ASSETMAP cross-reference, supplemental/OPL detection, marker validation (FFMC/LFMC required in strict, others recommended), MCA channel labeling, reel continuity, stereo 3D consistency, HFR compliance, ISDCF naming.
- XML-DSig signature verification, plus embedded X.509 certificate chain linkage and expiry checks (self-contained DCI trust model: leaf to root within the DCP's own certs).
- BV2.1 full application-profile checks behind `validate --bv21`.
- Subtitle timing and required-element checks.
- Deep J2K analysis and DCI checks, frame-qc, auto-qc (blackdetect/freezedetect/astats).
- Loudness measure/normalize, mxf-extract, diff (with `--fingerprint` perceptual picture compare), info, watch, fix with dry-run (fix suggestions via fixes.rs).
- checksum-verify with real hash + size checking (`--no-hash` still checks sizes).
- imf-compliance runs real per-platform delivery-spec checks (netflix/disney/amazon/apple/cinema2k/cinema4k/broadcast) plus generic verify.
- hdr-validate (hdr10/hlg/dolby-vision) with MaxCLL/MaxFALL checks.
- frame-compare resolves each IMP's picture asset; an ffmpeg run producing no frames is a hard error.
- KDM validation (not generation), theater profiles, timeline SVG, manifest comparison, batch summary.
- JSON/HTML reports, schema-validate, Photon bootstrap with IMF-only gating.
- QC report: HTML/PDF with package/track summary and optional per-track loudness.
- REST server: `GET /health`, `POST /validate {"path"}`, legacy `POST /verify {"dcp_dir"}`.
- Flags --studio/--netflix/--hdr/--atmos/--prores/--accessibility/--imf parsed and wired.
- GUI drag & drop, filters, sidecar (flags precede the path so they parse); web WASM validator with queue/cancel/streamed hashing.

## Known caveat

A few core modules still have zero callers: `qc`, `dci_ctp`, `kdm_advanced`, `mxf_advanced`, `auto_qc` (the auto-qc CLI command uses inline ffmpeg, not this module), and `bitrate`. None are advertised as wired checks. See DESIGN_TODO.md.
