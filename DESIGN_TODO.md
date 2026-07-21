# Planned

Advertised (README/docs/CHANGELOG) but missing, stubbed, or partial. Wire or de-advertise each.

## MCA / 3D / Atmos essence awareness: done

Native asdcplib probing (pin bumped to `5fe4d61` for `pcm::mca_labels`, aligned in
vendored postkit) wired into the core verify path:

- MCA labeling: `check_audio_channels` reads the sound MXF's ST 429-12
  subdescriptors and clears `sound_invalid_channel_count` when present; falls back
  to the CPL markers only when no MXF is readable (XML-only). Verified: a 5.1
  dcpwizard DCP clears the INFO, an unlabeled one keeps it, ECL09/ECL07 clean.
- Stereo (ST 429-10): `check_stereo` handles the `msp-cpl:` prefixed form,
  validates FrameRate = 2x EditRate, and confirms Jpeg2000Stereo essence. Verified:
  a dcpwizard `--right-eye` DCP passes, a FrameRate-not-doubled variant fails, real
  ECL07 (TST-3D-48) passes clean.
- Atmos (ST 429-18): `check_aux_data` surfaces `aux_data_detected` for each AuxData
  track (essence-enriched) and warns `cpl_mismatched_durations` when the aux
  duration differs from the reel's picture (ClairMeta
  `check_cpl_reel_duration_picture_aux`); cross-refs/PKL hashes for the aux asset
  are covered by the generic checks (proven by a hash-corruption test). The
  cross-ref and reel-coherence regexes now match the `msp-cpl:`/`axd:` namespaced
  forms.

## Dead modules: done

Deleted (zero callers, no cheap value): `qc.rs`, `auto_qc.rs` (inline CLI auto-qc
supersedes it), `kdm_advanced.rs`, `dci_ctp.rs`, `advanced::compare_manifest`/
`BatchResult`/`write_batch_summary`, `validators::check_color_space`,
`fixes::apply_fixes` + its `fix_*` helpers (duplicate `fix.rs`), and
`mxf_advanced::detect_dolby_vision` (redundant with `premium::parse_dolby_vision`,
wired into `--dolby-vision`).

Wired: `mxf_advanced::detect_dtsx`/`check_dtsx_compliance` (immersive-audio sound
check, no core equivalent) into `--studio --deep`.

The four unwired `hfr_stereo` helpers were deleted, not wired, because each is
unsound or redundant: `analyze_stereo3d` splits every stereo Duration in half so
its eye-offset is always 0 (the alignment finding can never fire); `analyze_multi_cpl`
has no note companion and cross-CPL frame-rate variance is legitimate; `trace_cpl_chain`/
`check_cpl_chain` overlap `check_supplemental` and key off a non-standard
`OriginalPackageList` element. `check_hfr_compliance` stays (already wired).

## Reel coherence: done

`validators::check_reel_coherence` (`reel_incoherent`) closes the last differential
gap vs ClairMeta `check_cpl_reel_coherence`. Wired into the core verify path. The
diff harness now shows CLAIRMETA_ONLY_FAIL 0 (ECL32 moved to BOTH_FAIL). MXF-probe
coherence keys (resolution/channels/sample rate) are read only where the CPL carries
them; deeper essence-level coherence would need MXF probing, which ClairMeta itself
skips in the harness env (no asdcp-info).

## OV-aware supplemental validation: done

All surfaces are wired. The shared resolver (`resolve_track_ref`/`RefStatus`/`validate_track_refs_ov`) lives in `dcpdoctor-imf` and is reused by the IMF path, the DCP path, and wasm.

- CLI: `--ov <ov_dir>` (IMF or DCP) resolves cross-package refs.
- REST: `POST /validate`/`/verify` accept an optional `"ov"` (alias `"ov_dir"`), threaded into `VerifyOptions.ov`.
- DCP path: `validators::check_cross_references` is OV-aware for SMPTE supplemental DCPs (429-7), sharing the IMF resolver over id sets.
- wasm: `validate_imf_supplemental(cpl_xml, assetmap_xml, ov_asset_ids_json, cpl_path)` exposes the OV capability at the binding.
- web/ UI: an optional "+ OV" folder picker (`web/app.js`) extracts the OV ASSETMAP's urn:uuid ids and threads them to the worker, which calls `validate_imf_supplemental` for each IMF CPL and replaces the non-OV `cross_ref_broken`/`supplemental_ov_not_provided` notes with the OV-aware result.

## Differential vs ClairMeta gaps: done

Closes the correctness gaps recorded in `dci-ctp/DESIGN_TODO.md`. All wired into
the core `verify_dcp` path and covered by tests.

- XSD schema validation: `schema::check_schema` runs on ASSETMAP/CPL/PKL in
  `verify_dcp`, emitting `xml_schema_violation` via xmllint against the SMPTE/
  Interop XSDs. Now on by default: the XSD set (plus `catalog.xml`) is vendored
  under `schemas/` from the ClairMeta set, found via the compile-time source path;
  `DCPDOCTOR_SCHEMA_DIR` still overrides and a missing dir degrades to skip (so
  XML-only/wasm contexts keep working). Non-vacuous: the ClairMeta ECL packages
  (Interop + SMPTE, incl. 3D/Atmos) pass the schema clean while schema-invalid docs
  fire. dcpwizard packages emit schema-valid XML. wasm is unaffected (the wasm
  crate builds from dcpdoctor-imf/-parse, not dcpdoctor-core).
- `kdm_not_yet_valid` / `kdm_expired`: the KDM validator now distinguishes the
  not-yet-valid window (warning) from the expired window (error) instead of
  `kdm_required` for both.
- `invalid_uuid`: `compliance::check_uuids` is wired into `verify_dcp`; malformed
  `urn:uuid:` tokens surface from `validate`.
- VF/supplemental: unresolved external refs are `supplemental_ov_not_provided`
  (warning) without `--ov` and `cross_ref_broken` (error) with `--ov`. The ECL VF
  packages (ECL02 IOP, ECL10 SMPTE) now match the dcpwizard OPL-marker behavior.
- Interop scoping: subtitle validation recognizes the Interop DCSubtitle format
  (`<DCSubtitle>` root, `SubtitleID`), so Interop encrypted CPLs (ECL29/ECL33) no
  longer get SMPTE-only `missing_required_element`.

## ClairMeta coverage: done

- Deep certificate checks live in `cert_rules.rs` (sig algorithm, RSA 2048/e=65537,
  BasicConstraints, KeyUsage, role distinctness, dnQualifier thumbprint, Organization
  consistency) and run from `signature.rs`; expiry/validity window is `signature.rs`.
- Sound essence: `mxf::check_sound_descriptor` flags non-24-bit PCM
  (`sound_invalid_quantization`) and, when the prober reports it, a wrong block
  align (`sound_invalid_block_align`). Wired into the `--check-mxf` path.
- CPL metadata: `validators::check_cpl_metadata` requires ContentTitleText/IssueDate
  and (SMPTE-only, so Interop is not false-flagged) ContentVersion.
- Package hygiene: `validators::check_package_files` flags unreferenced
  (`foreign_file_in_package`) and zero-byte (`empty_file_in_package`) files.
- PKL `Size` vs actual file size: `pkl_size_mismatch` (ClairMeta
  `check_assets_pkl_size`), checked in `verify_dcp` regardless of `--no-hashes`.
  The last ClairMeta ERROR check with no equivalent is `check_dcp_signed`
  (encrypted-but-unsigned; those packages already fail via `kdm_required`).
- Verified clean on ECL09 (SMPTE OV) and ECL01 (Interop OV): 0 errors, no new notes.

## GUI: done

- Preferences panel (`gui/index.html`) is trimmed to the two prefs that map to a
  real `validate` flag without changing the text output the sidecar parses:
  Verify Hashes (`--no-hashes`) and Inspect MXF essence (`--check-mxf`). Applied to
  every run via `getPrefFlags()`. The other seven prefs (standard, schemas,
  loudness, max bitrate, report format, output dir, schema dir) had no such flag
  and were dropped.

## --studio depth overlaps core: done

- `studio.rs` keeps its deeper ffprobe checks; the CLI (`suppress_core_studio_overlap`)
  now drops the lighter core note when a deep studio note covers the same finding
  (mixed-encryption supersedes reel-coherence encryption; ffprobe stereo eye checks
  supersede structural `stereo_mismatch`), so each finding shows once.

## Dedup into postkit

- hash.rs now adapts `sha1_base64`/`sha1_hex` onto `postkit::hash::hash_file`. Done.
- loudness.rs now uses `postkit::loudness::LoudnessResult` (the local copy's unused
  `compliant_*` flags were dropped). Done.
- Still open (needs postkit API work, left as-is): j2k.rs in dcpdoctor-core and
  dcpdoctor-wasm re-implement postkit::j2k; bitrate.rs redoes postkit's analyse_bitrate;
  frame_compare.rs duplicated with imfwizard-core.

## Keep in sync with the wizards (deliberately duplicated, no clean shared home)

Final dedup pass (2026-07-20): these CI workflows are ~0.94 copies across
dcpdoctor, dcpwizard, imfwizard, differing by binary/artifact names and per-app
build deps. Separate git repos, so there is no shared reusable-workflow without a
central repo. Keep aligned by hand:

- .github/workflows/release.yml, gui-release.yml

Grok CI 2026-07-21: dcpdoctor does not need grok. Its postkit dep enables no
grok-ffi feature and no dcpdoctor source uses grok; `--all-features` only reaches
workspace-member features, not the transitive postkit grok-ffi. Workflows left
unchanged (no grok step, no openjpeg deps to drop).
