# Planned

Advertised (README/docs/CHANGELOG) but missing, stubbed, or partial. Wire or de-advertise each.

## DoM tracker gaps (2026-07-22)

Feature requests from the DCP-o-matic Mantis tracker (dom#N =
https://dcpomatic.com/bugs/view.php?id=N) that dcpdoctor lacks. Priority order.
Done items are in the dated done notes below.

The list is now empty: every DoM tracker gap is done (notes below).

## Verify encrypted DCPs with a KDM: done (2026-07-22)

dom#2971 (decrypt + verify encrypted essence) and dom#1957 (HMAC/MIC integrity),
built on postkit's KDM-unwrap API (`certificate::unwrap_kdm`, keys zeroed on drop,
Debug redacted).

- CLI: `validate --kdm <kdm.xml> --recipient-key <private.pem>`. Threaded through
  `VerifyOptions.{kdm,recipient_key}` and the REST body (`"kdm"` +
  `"recipient_key"`, same shape as `"ov"`).
- `kdm::ContentKeys` unwraps the KDM once and resolves each essence by the MXF's
  own `cryptographic_key_id` (the CPL KeyId): `EssenceKey::{Cleartext, Available,
  Missing}`. `Available` builds an `AesDecContext` + `HmacContext` (HMAC key is the
  content key derived per the label set); the readers verify the MIC per frame.
- Checks that used to skip encrypted essence now run on decrypted frames: the
  0xFFFF legacy scan + ISO 15444-1 cinema constraints (`check_picture_j2k_mxf`),
  the RDD 52 guard-bit scan (`check_guard_bits_mxf`, `--deep-j2k`), MXF-wrapped
  glyph coverage (`check_glyph_coverage_mxf`), and the sound descriptor
  (`check_sound_essence_mxf`, read via asdcplib since ffprobe can't see encrypted
  essence). A KDM that doesn't cover a KeyId keeps the skip and emits a clear note;
  no KDM keeps the silent skip as before.
- HMAC/MIC (dom#1957) is a real check, not a binding gap: the readers take the
  `HmacContext`, and a mismatched content key surfaces `mxf_hash_mismatch` on the
  offending frame (frame 0 for picture/sound) rather than a bogus finding.
- Wrong/expired/mismatched KDM produce clear errors: a wrong recipient key fails
  loud at unwrap (`kdm_required` error), expiry/not-yet-valid/CPL-mismatch reuse
  the existing `kdm::validate_kdm` window + CPL-match checks, and a mismatched
  content key fails the MIC.
- Tests (`kdm::decrypt_tests`): build an AES+HMAC-encrypted 2K picture MXF and a
  KDM for it via postkit `build_kdm` + a generated cert chain, then prove (a) no
  KDM skips, (b) the right KDM decrypts and a planted guard-bit violation fires,
  (c) a wrong recipient key fails loud, (d) a mismatched content key fails the
  MIC. Real E2E confirmed against the ClairMeta ECL29 encrypted IOP package with
  its `leaf.key`: without the KDM the picture check skips, with it every picture
  MXF decrypts to a real J2K codestream (SOC) and the MIC verifies clean; a wrong
  key fails loud. (The tests/dcps/isdcf fixture ships no KDM/key, so that package
  can't be an E2E; the ECL29 KDM+key live in the ClairMeta data set.)

## DoM tracker gaps 1-2: done (2026-07-22)

Leq(m) loudness (dom#3092) and the six smaller verifier checks. All wired into
the core verify path except the report-folder flag (a CLI option) and Leq(m)
(reported, not a note).

- Leq(m) (ISO 21727, CCIR 468-weighted, SMPTE B-chain 85 dB reference): computed
  locally in `loudness.rs` from decoded PCM (ffmpeg -> mono f32 -> CCIR 468
  weighting via rustfft -> equivalent level). Reported alongside EBU R128 in the
  `loudness` CLI command (text + JSON `leq_m_db`) and the qc-report HTML table.
  Verified against the derivable reference: a full-scale 1 kHz sine is
  -3.01 dBFS RMS, weighting is 0 dB at 1 kHz, +105 dB offset -> 101.99 dB (unit
  test). Migrate to postkit::loudness later (see dedup section).
- `reel_too_short` (dom#2723, ST 429-7): warns when a reel's picture
  duration/edit-rate is under 1 s. Fires on 0.5 s, silent at exactly 1 s.
- `sound_channel_config_invalid` (dom#1960, Bv2.1 §10.3.1 / RDD 52): the request
  said "SMPTE forbids config 4", but RDD 52 §10.3.1 is the opposite: SMPTE Bv2.1
  *requires* Static Container Channel Configuration 4 (the "open" ChannelAssignment
  UL) with ST 377-4 MCA labels. Implemented the spec-correct check: a SMPTE sound
  essence declaring a legacy static config (1/2/3/5) is warned; config 4 and MCA
  are clean. Read from the WAV descriptor's ChannelAssignment via asdcplib. Test
  writes a Cfg1 MXF (fires) and a Cfg4 MXF (silent).
- `subtitle_frame_rate_mismatch` (dom#2994, ST 428-7 §5.9): warns when a subtitle
  document's TimeCodeRate differs from the composition edit rate rounded to the
  nearest integer. Test: 25 vs 24 fires, 24 vs 24 silent.
- `non_ascii_filename` (dom#3016): warns on non-ASCII characters in the DCP folder
  name and any file/sub-folder name.
- `j2k_legacy_ffff` (dom#2740, SMPTE Cat. 862 / Legacy Compatibility Note 1):
  detects two consecutive 0xFF bytes at a byte position 254 mod 256 (realigned by
  each tile-part length), which crashes legacy Dolby decoders (DSS200 Cat. 862,
  DSP100). Codestream scanner in `j2k.rs`, wired into the `--check-mxf` picture
  path (reads frame 0 via asdcplib jp2k). The codestream header parse could later
  move to the grok library, which already parses J2K headers.
- `--report-to-folder` (dom#2990): writes the report (reusing the existing
  text/JSON/HTML writers) as `dcpdoctor-report.{txt,json,html}` into each DCP's
  own folder.

## ISO/IEC 15444-1 cinema J2K constraints: done (2026-07-22)

dom#2451/#1664, beyond what `validate_j2k_dci` already covered (wavelet,
decomposition, code-block, components, bit depth, RSIZ-vs-resolution). New
`j2k::validate_cinema_j2k` parses the picture frame's codestream and checks
against SMPTE ST 429-4 / ISO 15444-1 digital-cinema profiles:

- single tile (error), required SIZ/COD/QCD markers (error).
- tile-part organisation: 3 for 2K (one per colour component), 6 for 4K (warning).
- frame size vs profile: 2048x1080 (2K) / 4096x2160 (4K) (error).
- per-colour-component byte limit: each of the first three tile-parts (one 2K
  colour component) within the DCI 200 Mbps-equivalent budget, scaling with the
  picture edit rate (1,041,666 bytes at 24 fps) (error).

Wired into the `--check-mxf` picture path alongside the 0xFFFF check, sharing one
frame read (`j2k::check_picture_j2k_mxf`). Tests build synthetic codestreams that
fire each rule and a conformant one that stays clean; verified silent on the real
ClairMeta `dcp_ov` picture MXF. Skipped, to avoid guessing: the progression-order
(CPRL) rule was not confirmed from a primary source this pass, and the total-frame
byte limit is already covered by the existing bitrate check, so neither was added.
The guard-bit count check moved out of `validate_cinema_j2k` into the per-frame
`check_guard_bits_mxf` so it can report a timecode (dom#2984, done note below);
`validate_cinema_j2k` keeps only the QCD-present marker check.

## Subtitle glyph coverage + guard-bit timecode: done (2026-07-22)

- Subtitle glyph coverage (dom#3080, dom#838): `subtitle::check_glyph_coverage`
  parses each plain-XML subtitle asset, collects every code point used in each cue
  (tracking the active `<Font>` via a stack, or the sole LoadFont when a document
  has one), resolves each `LoadFont` to a font file, and warns per missing glyph
  with the cue's TimeIn and code point (`subtitle_glyph_missing`). Font resolution:
  Interop DCSubtitle uses the `LoadFont URI` file (relative to the subtitle, then
  the DCP root); SMPTE ST 428-7 uses the `LoadFont` element text as an asset urn
  resolved through the ASSETMAP. A font that doesn't resolve is skipped silently
  (the structural check already warns on a missing LoadFont). Wired into the core
  verify path (`validate.rs` §4b) next to `validate_subtitle`. Both the plain-XML
  and the SMPTE MXF-wrapped ST 429-5 form are now covered:
  `subtitle::check_glyph_coverage_mxf` opens the timed-text MXF, reads the document
  and its embedded OpenType fonts (ancillary resources, filtering out `Png` bitmap
  subs and `Binary`), maps each `LoadFont` urn to a resource uuid, and runs the same
  cmap coverage over the cues. Both entry points share one core (`glyph_notes`), the
  MXF path handing back font bytes straight from the essence instead of a file.
  Still skipped: encrypted timed text with no KDM, detected up front via
  `writer_info().encrypted_essence` (the document and fonts would be ciphertext), so
  the encrypted ISDCF fixture (whose `sub.mxf` is encrypted, verified) stays clean.
  Uses `skrifa` (googlefonts/fontations) for cmap lookup rather than the suggested
  `ttf-parser`: ttf-parser 0.25.1 is flagged unmaintained (RUSTSEC-2026-0192),
  while skrifa/read-fonts is the actively-maintained equivalent and its `Charmap`
  picks the broadest Unicode subtable. Tests build a synthetic sfnt (cmap format 12)
  and prove a missing glyph fires, full coverage is silent, and an unresolvable
  font is skipped; the MXF path builds an in-test timed-text MXF via asdcplib
  `open_write_with_resources` embedding that same synthetic font and proves the same
  fire / silent / no-OpenType-resource skip cases.
- Guard-bit error locations with timecode (dom#2984): `j2k::check_guard_bits_mxf`
  iterates the picture MXF's frames via asdcplib jp2k, reads each frame's QCD guard
  bits (reusing `qcd_guard_bits`), and on the first frame that violates the RDD 52
  rule (1 guard bit for 2K, 2 for 4K) emits `j2k_guard_bits` with the frame index,
  its derived SMPTE timecode (from the picture edit rate), and how many frames are
  affected. Essence that isn't a readable codestream (encrypted without a KDM, or
  non-picture) has no SOC on frame 0 and is skipped, same as `j2k_legacy_ffff`.
  Wired into `--deep-j2k` (`run_deep_j2k`). This replaces the package-level guard
  warning that had been in `validate_cinema_j2k`. Spec note: the request said "1
  guard bit for 2K/4K", but RDD 52 (referencing ISO 15444-1) requires 1 for 2K and
  2 for 4K; implemented the spec-correct rule (confirmed via RDD 52 / SMPTE
  Bv2.1). Tested on synthetic codestreams: correct counts stay silent, wrong ones
  fire, and the timecode derivation is unit-tested.

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
- loudness.rs Leq(m) (ISO 21727 / CCIR 468) is implemented locally (dom#3092);
  the weighting + level math should migrate to `postkit::loudness` alongside the
  R128 helper once a pin bump is worthwhile. Added a `rustfft` dep to dcpdoctor-core
  for the CCIR 468 weighting FFT; if it moves to postkit the dep moves with it.
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
