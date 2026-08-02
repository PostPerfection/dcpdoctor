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
- XSD schema validation (always on when schemas are available): every CPL/PKL/ASSETMAP is validated against the SMPTE/Interop XSDs via xmllint, emitting `xml_schema_violation`. Schema-path driven (`schema::locate_schema_dir`): `DCPDOCTOR_SCHEMA_DIR` env override, else a bundled `schemas/` dir next to the binary or in the tree; if none is found the check is skipped. The SMPTE XSDs are copyrighted so they are referenced, not vendored; point the env var at a SMPTE/Interop XSD set with a `catalog.xml` (ClairMeta bundles one under its package `xsd/`). The root element picks the document type and the namespace bound to that root element picks the standard (Interop = digicine.com), so a SMPTE CPL that declares the digicine CC-CPL namespace on a caption track still validates against the SMPTE schema. The catalog resolves cross-schema imports offline.
- UUID format check (`invalid_uuid`): every `urn:uuid:` identifier in the DCP's XML must be a well-formed RFC 4122 UUID (standard-agnostic).
- MXF partition structure (`mxf_invalid_structure`, SMPTE 377-1: header/footer/closed-complete) under `--check-mxf`/`--strict`, alongside the other MXF-reading checks.
- Core CPL checks (always on): encrypted-content/KDM detection, CPL-to-ASSETMAP cross-reference, supplemental/OPL detection, marker validation (FFMC/LFMC required in strict, others recommended), MCA channel labeling, reel continuity, reel coherence, stereo 3D consistency, HFR compliance, ISDCF naming.
- Reel coherence (`reel_incoherent`, error, matches ClairMeta `check_cpl_reel_coherence` / SMPTE ST 429-2 8.7): within one CPL every per-reel essence parameter the CPL carries must hold a single value across all reels that have it. Checks picture edit rate / frame rate / frame size (ScreenAspectRatio) / resolution / encryption, sound edit rate / encryption / channel count / sample rate, and subtitle edit rate. Encryption is keyed off `<KeyId>` presence (what makes ECL32, one clear picture reel among encrypted ones, incoherent). Values are only collected where the essence is present, so a track missing on some reels is not a mismatch; MXF-probe keys (resolution/channels/sample rate) are read where the CPL carries them but SMPTE CPLs usually keep them in the MXF.
- OV-aware supplemental validation (IMF and DCP): `validate --imf <supp> --ov <ov>` resolves a supplemental CPL's track-file references across both packages (supplemental ASSETMAP + OV ASSETMAP). A ref found in either passes; a ref in neither is `cross_ref_broken`. Without `--ov`, any locally-unresolved ref is reported as `supplemental_ov_not_provided` (warning) rather than a hard `cross_ref_broken`: this matches ClairMeta, which classifies a CPL with any asset missing locally as a version file (VF) referencing an external OV, and a legitimate VF ref and a corrupt one are indistinguishable without the OV. Supply `--ov` to turn genuinely broken refs (present in neither package) back into `cross_ref_broken` errors. VF detection is therefore structural (missing local ref), not keyed on an OPL marker, so real-world VF packages without that marker are handled. Threaded via `VerifyOptions.ov`. The shared resolver (`resolve_track_ref`/`RefStatus`/`validate_track_refs_ov`) lives in `dcpdoctor-imf` and is reused by the IMF path, the DCP path, and the wasm binding. Surfaces: CLI, REST (`ov` field), and the wasm binding `validate_imf_supplemental` (OV asset ids passed from JS, since the browser has no filesystem).
- XML-DSig signature verification, plus embedded X.509 certificate chain linkage and expiry checks (self-contained DCI trust model: leaf to root within the DCP's own certs).
- Deep DCI / SMPTE ST 430-2 certificate-rule compliance (`cert_rules.rs`, wired into `signature::verify_signature` for signed packages): per-cert signature algorithm (sha256WithRSA), RSA 2048-bit / exponent 65537, Basic Constraints (cA matches CA-vs-leaf role), Key Usage (keyCertSign for CAs, digitalSignature for the leaf), dnQualifier == base64(SHA-1(public-key BIT STRING)), a permissive CN role-distinctness check, and Organization (O) coherence across the chain. Emits `certificate_*` codes.
- BV2.1 full application-profile checks behind `validate --bv21`.
- Subtitle timing and required-element checks, plus glyph coverage: every code point a cue uses must have a glyph in the font the document loads. Interop resolves the font from the `LoadFont URI` attribute, SMPTE ST 428-7 from the asset urn the `LoadFont` element carries (looked up in the ASSETMAP), and MXF-wrapped ST 429-5 timed text from the fonts embedded as ancillary resources.
- Bv2.1 timed-text content limits (line length, line count, cue duration and spacing, caption charset) run on `MainSubtitle` tracks and on caption tracks named either `MainClosedCaption` (what the digicine CC-CPL schema declares and real Bv2.1 packages ship) or `ClosedCaption`, with the stricter caption limits on the latter two. Both caption spellings are also in the CPL-to-ASSETMAP cross-reference scan, so a caption track's asset id resolves like any other external asset.
- Deep J2K analysis and DCI checks, frame-qc, auto-qc (blackdetect/freezedetect/astats). A picture MXF is read frame-0 first through the asdcplib OP-Atom reader, which yields the real codestream markers (RSIZ, COD, code-block, wavelet) for AS-DCP essence. AS-02 (IMF, OP1a) picture MXFs are rejected by that reader and fall back to ffprobe: dimensions, bit depth and per-frame bytes come from the MXF descriptor, the profile is guessed from the width, and the marker-derived fields stay unset. Both paths are covered by `j2k::as02_tests`, which wraps a synthetic codestream as AS-02 through postkit `mxf_wrap` and asserts the OP-Atom reader rejects it and the ffprobe result carries the right dimensions and 2K/4K profile guess. Skipped where ffprobe is not installed.
- Loudness measure/normalize, mxf-extract, diff (with `--fingerprint` perceptual picture compare), info, watch, fix with dry-run (fix suggestions via fixes.rs).
- checksum-verify with real hash + size checking (`--no-hash` still checks sizes).
- imf-compliance runs real per-platform delivery-spec checks (netflix/disney/amazon/apple/cinema2k/cinema4k/broadcast) plus generic verify.
- hdr-validate (hdr10/hlg/dolby-vision) with MaxCLL/MaxFALL checks.
- frame-compare resolves each IMP's picture asset; an ffmpeg run producing no frames is a hard error.
- KDM validation (not generation), theater profiles, timeline SVG, manifest comparison, batch summary.
- JSON/HTML reports, standalone `schema-validate` command, Photon bootstrap with IMF-only gating.
- KDM validity window: `kdm_expired` (error) and `kdm_not_yet_valid` (warning) in the `kdm` command.
- Interop subtitles: DTD-era `<DCSubtitle>` (no namespace, `<SubtitleID>` element) is recognized, so valid Interop timed text no longer trips namespace/identifier checks meant for SMPTE DCST.
- QC report: HTML/PDF with package/track summary and optional per-track loudness.
- REST server: `GET /health`, `POST /validate {"path", "ov"?}`, legacy `POST /verify {"dcp_dir", "ov"?}`.
- Flags --studio/--netflix/--hdr/--atmos/--prores/--accessibility/--imf parsed and wired.
- GUI drag & drop, filters, sidecar (flags precede the path so they parse); web WASM validator with queue/cancel/streamed hashing.

## Known caveat

A few core modules still have zero (or partial) callers: `qc`, `dci_ctp`, `kdm_advanced`, `auto_qc` (the auto-qc CLI command uses inline ffmpeg, not this module), and `bitrate`. `mxf_advanced` is now partially wired (partition-structure check in the `--check-mxf` path; its ffprobe Dolby-Vision/DTS:X helpers are still unused). None of the remaining dead modules are advertised as wired checks. See DESIGN_TODO.md.
