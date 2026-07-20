# Planned

Advertised (README/docs/CHANGELOG) but missing, stubbed, or partial. Wire or de-advertise each.

## Remaining dead modules (not advertised, not yet wired)

These have zero callers. They are not advertised as wired checks, so they are low priority; wire into a command or delete.

- `qc.rs`: QC helpers with no caller (the `auto-qc` command uses inline ffmpeg + `audio::analyze_audio`).
- `auto_qc.rs`: superseded by the inline `auto-qc` implementation in the CLI; delete or route the command through it.
- `kdm_advanced.rs`, `mxf_advanced.rs`, `dci_ctp.rs`: advanced KDM/MXF/DCI-CTP helpers with no caller.
- Leftover dead functions inside otherwise-wired modules: `advanced::compare_manifest`/`BatchResult`/`write_batch_summary`, `validators::check_color_space`, `fixes::apply_fixes` (duplicates `fix.rs`), and the unused `hfr_stereo` helpers (`analyze_multi_cpl`, `analyze_stereo3d`, `trace_cpl_chain`, `check_cpl_chain`).

## OV-aware supplemental validation: remaining surfaces

- Core CLI is wired: `--ov <ov_dir>` resolves a supplemental's cross-package refs against the OV. wasm and REST server are not.
- `dcpdoctor-wasm/src/imf.rs` calls `dcpdoctor_imf::validate_track_refs` directly against a single package's asset ids. No OV surface in the browser (second-directory access model differs). Supplementals validated there still report the OV track files as broken.
- REST server (`POST /validate {"path"}`) has no OV field; `VerifyOptions.ov` is plumbed but the server never sets it. Add an optional `ov` to the request if supplemental validation over HTTP is needed.

## GUI

- Preferences panel (`gui/index.html`) saves to localStorage but nothing is applied to validation runs; web/ has no preferences UI at all. Either apply the saved prefs to the sidecar invocation or remove the panel and the CHANGELOG claim.

## Deferred: --studio depth overlaps core

- `studio.rs` keeps its own ffprobe-based reel-continuity / stereo / encryption checks, now layered on top of the lightweight core `validators.rs` versions. Intentional (core = structural baseline, --studio = deep), but produces some overlapping notes under `--studio`. Consider consolidating messages.

## Dedup into postkit

- j2k.rs in dcpdoctor-core and dcpdoctor-wasm re-implement postkit::j2k (pure bytes, wasm-usable); bitrate.rs redoes postkit's analyse_bitrate.
- hash.rs reimplements sha1_base64/sha1_hex over postkit::hash::hash_file.
- loudness.rs delegates to postkit but redefines LoudnessResult locally.
- frame_compare.rs duplicated with imfwizard-core; candidate postkit module.

## Keep in sync with the wizards (deliberately duplicated, no clean shared home)

Final dedup pass (2026-07-20): these CI workflows are ~0.94 copies across
dcpdoctor, dcpwizard, imfwizard, differing by binary/artifact names and per-app
build deps. Separate git repos, so there is no shared reusable-workflow without a
central repo. Keep aligned by hand:

- .github/workflows/release.yml, gui-release.yml
