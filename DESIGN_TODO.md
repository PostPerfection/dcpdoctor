# Planned

Genuinely open items and standing decisions. Everything advertised in
README/docs/CHANGELOG is wired (done notes below); every DoM tracker gap (dom#N =
https://dcpomatic.com/bugs/view.php?id=N) is done. What remains is deliberate
policy plus the measurement gaps listed below.

## 4K frame rate is not checked against ST 429-2 Table 1

§8.2 Table 1 limits the 4K formats to a frame rate of 24/1, 25/1 or 30/1, where
the 2K formats allow all six, and the same section says "Monoscopic picture
essence tracks shall have matching frame rate and edit rate". So a monoscopic 4K
composition at 48/1 violates a "shall", and nothing here reads it. Applying it
needs the pixel array, which means the picture descriptor rather than the CPL, so
it belongs with the checks that open the essence. Note this is §8.2, not §8.3,
which is Sound Essence Encoding and says nothing about frame rates.

## Picture bitrate: what is still not measured

Peak bitrate is now read frame by frame for AS-DCP picture essence, mono and 3D
(see Done, 2026-08-12). Two gaps remain:

- AS-02 / IMF picture essence gets no measurement. Both asdcplib JP2K readers are
  OP-Atom, so an IMP picture track file falls through to no note at all. ffprobe
  is the only reader that opens it and it reports container size, not codestream
  length, so there is nothing honest to derive a frame rate from yet.
- P-HFR gets no special limit, deliberately. ISDCF's P-HFR paper (v005, 2012)
  sets 500 Mb/s for the total codestream of 2K stereoscopic HFR, keyed on a
  P-HFR-2K essence label, and calls itself a proposal for experimental use.
  asdcplib applies 400 to that same label citing no source, stricter than the
  document defining it. Neither is normative, and either would need the essence
  coding UL, which dcpdoctor does not read. Same note in postkit's DESIGN_TODO.

## Photon has to be fetched, not built

`bootstrap_photon` is gone (see Done, 2026-08-12). dcpdoctor now runs Photon only
when jars already exist, so a machine without them gets an INFO note saying the
deep IMF pass was skipped. There is no fetch script in this repo: imfwizard's
`scripts/fetch_photon.sh` pulls the jars from Maven Central and reads the same
`PHOTON_DIR`, so pointing both at one directory works. Vendoring a fetch script
here would mean two copies of the same pinned coordinates and checksums.

## Severity policy: four checks stay WARNING

`reel_discontinuity`, `pkl_missing_asset_reference`, `bv21_pkl_no_xml_ext`, and
`unencrypted_dcp_not_signed` stay WARNING: no SMPTE "shall" demands rejection, so
they stay CLAIRMETA_ONLY_FAIL in the dci-ctp differential (per-code justification
in dci-ctp/DESIGN_TODO.md). Escalate only if a spec citation forces it, as with
the four codes already moved to ERROR (see "Severity escalations to spec" under
Done).

# Done

## One DCI bitrate limit at every resolution (2026-08-12)

postkit pin d8d97cf -> 8b4a034, which the bitrate change needs: it replaces
`j2k::dci_max_bitrate_mbps(width)` with `j2k::DCI_MAX_BITRATE_MBPS = 250.0`. No
new code, and no code loses its producer.

The 500 for 4K that this used to apply has no source in DCI, ST 429-4 or
ST 429-2, and asdcplib has no 4K case at all. DCSS 4.3.3 states the cap as bytes
per frame rather than a rate, and every figure works out to the same number:
1,302,083 bytes at 24 fps for 2K and for 4K, 651,041 at 48 fps. So there is no
resolution branch to make.

- `bitrate::check_bitrate_compliance` reads the constant. This is a real verdict
  change: a 4K package between 250 and 500 Mb/s now fails where it passed. Its
  4K test flipped with it, from asserting the higher limit to asserting the same
  limit, and gained an under-the-limit 4K case so the path is not just failing
  everything.
- The IMF path in validate.rs had its own `if pic.width > 2048 { 500.0 } else
  { 250.0 }`, attributed to ST 429-4, which does not carry it. Deleted in favour
  of the constant, so the number has one source.
- `hfr_stereo.rs` emitted `j2k_bitrate_exceeded` as a fixed INFO reading "DCI
  maximum bitrate is 500 Mbps for all HFR content" whenever the CPL edit rate
  exceeded 30 fps. Deleted: it asserted a limit with no measurement behind it and
  no source for the number, and the measured check covers the ground honestly.
  The frame-rate findings in that file are a separate question and are untouched.
- `fixes.rs` told the user to re-encode against "250 Mbps 2K / 500 Mbps 4K",
  which would have contradicted the check that raised the note.

## Standard from the namespace, and three EditRate rules at ERROR (2026-08-12)

One new code, `composition_metadata_asset_mismatch`, so dci-ctp's `ALL_CODES`
denominator goes 85 -> 86. All three ST 429-2 and ST 429-16 clauses below were
read out of the published PDFs before being quoted in a comment.

- `dcp::detect_standard` derives from the asset map's root namespace via
  `schema::root_namespace`, not the file name. The file name is the thing
  `assetmap_invalid_name` exists to catch, so keying the standard off it meant a
  package under the wrong name was validated as the wrong standard throughout,
  silently skipping three SMPTE-only checks and rescaling every subtitle time.
  Matches on the namespace authority rather than one exact URI, the same test
  `schema_file_for` uses, so the IMF asset map namespace still reads as SMPTE and
  IMP behaviour is unchanged. An unrecognised namespace is Unknown.
- `check_assetmap_name` now takes that shared `Standard` instead of deriving its
  own, so the two cannot drift. That left `AssetMap.is_smpte` with no consumer and
  it is deleted: it was a substring test over the whole document rather than a
  root binding, which is exactly the weak signal this change moves away from.
- ST 429-2 §9.6.1 and §9.7.1 turned out to be already enforced at ERROR by
  `check_reel_coherence` as `reel_incoherent`, for EditRate, FrameRate,
  ScreenAspectRatio and sound EditRate. Neither clause says a sound EditRate must
  equal the picture's: both are intra-class rules, "All picture assets ... shall
  have identical values" and "All sound assets ... shall have identical values".
  So the two real gaps were the two list items nothing read: the picture element
  name, which §9.6.1 names explicitly and which `essence_block` hides by
  resolving both spellings to one block, and the sound `Language` element. Both
  are now rows in the coherence table. No new code.
- `composition_metadata_asset_mismatch` (ERROR): ST 429-16:2014 §4.4.1 binds the
  CompositionMetadataAsset to the picture of its own reel in two "shall"
  sentences, one for EditRate and one for IntrinsicDuration against the picture's
  Duration. `check_composition_metadata_asset` covers both.
- ST 429-2 §8.1, "The composition shall have an Edit Rate of 24/1, 25/1, 30/1,
  48/1, 50/1 or 60/1", was already implemented at ERROR with exactly that list,
  but gated behind `--strict`. A plain "shall" does not need a flag, so the gate
  is gone. Read off the picture EditRate, which §9.6.1 makes identical across
  every picture asset, so the picture rate is the composition Edit Rate. Not
  applied to marker, subtitle or metadata assets: §8.1 speaks of the
  composition's Edit Rate, not every asset's. The wasm validator already ran this
  ungated, so this brings the native path in line rather than diverging.

## Reel EditRate, KDM schema validation, dead compliance module (2026-08-12)

One new code, `reel_edit_rate_mismatch`, so dci-ctp's `ALL_CODES` denominator
goes 84 -> 85.

- `reel_edit_rate_mismatch` (WARNING): every reel asset carries its own EditRate
  and only the picture's was ever read, so the DCP-o-matic package whose
  MainMarkers asset runs at 13 1 against a 24 1 picture was reported clean at
  every severity. `check_reel_edit_rates` compares MainMarkers, MainSubtitle,
  MainClosedCaption and AuxData against the picture. WARNING because no "shall"
  reaches those classes: ST 429-2 §9.4 constrains Duration and scopes itself to
  assets referring to an external track file, which excludes MainMarkers, and
  §9.6.1 and §9.7.1 are the only per-asset-class EditRate equality rules and they
  name picture and sound. ST 429-16 §4.4.1 giving CompositionMetadataAsset an
  explicit rule is the argument that the omission for MainMarkers is deliberate.
  ST 429-2 §9.9 is why it matters anyway: a marker Offset is compared against the
  reel duration and that comparison only means anything if the rates agree.
  Sound and CompositionMetadataAsset are excluded on purpose, and both are
  covered at ERROR by their own rules (see the 2026-08-12 note above).
- KDMs are schema-validated. `schema::check_schema` ran on ASSETMAP, CPL and PKL
  only, so postkit shipping KDMs with no `AuthorizedDeviceInfo` element, which ST
  430-1 Annex B makes required, went unremarked here. This is the same shape as
  the sound-descriptor finding: the only thing checking the output was the family
  of code that produced it. `schema_file_for` gained a KDM arm keyed on
  `KDMRequiredExtensions` and placed ahead of the CompositionPlaylist arm, since
  a KDM carries a `<CompositionPlaylistId>` element that matches that arm's
  substring. Pointing xmllint at the 430-1 schema loads 430-3 through its import
  and the strict `RequiredExtensions` wildcard reaches the KDM body. Wired into
  `kdm::validate_kdm`, which both `validate --kdm` and the `kdm` subcommand route
  through, so one seam covers both. Reports the existing `xml_schema_violation`.
- The vendored `SMPTE-430-3-2006-ETM.xsd` had its UUID pattern facet split across
  a line at `[0-9a-fA-\nF]`, which is not a valid regular expression, so both KDM
  schemas failed to compile and no KDM could have been validated against them.
  Joining the line was the whole fix. `SMPTE-429-10-2008-Main-Stereo-Picture-CPL.xsd`
  still fails to compile, for an unrelated reason (it references
  `cpl:PictureTrackFileAssetType`, which does not resolve), but nothing routes to
  it so it is inert.
- `compliance::check_smpte_compliance` is deleted, with the whole private subtree
  that only it reached: `check_assetmap_compliance`, `check_pkl_compliance`,
  `check_cpl_compliance`, `check_reel_compliance`, `check_namespace`,
  `check_uuid`, `extract_tag` and seven constants. It had no caller anywhere in
  the workspace, and its asset-map naming branch could never have fired even if
  called, because it tested `standard == Smpte` against a `Standard` derived from
  that same file name. Every code it produced is still produced elsewhere, so
  nothing lost its only producer. 466 lines down to 50; `check_uuids` stays.

## Standard derivation: blast radius of moving off the filename (2026-08-12)

The investigation that led to the fix above. `dcp::detect_standard` returned Smpte when
`ASSETMAP.xml` exists, Interop when `ASSETMAP` does, else Unknown. Three live
call sites: `dcp::open_dcp`, and the CLI's facility-check and `--bv21` paths.

Six checks change behaviour if the value changes, all reached from plain
`validate` through `dcp.standard`:

- `subtitle::validate_subtitle` picks the expected DCST namespace per standard
  and reports a mismatch at ERROR, so a wrong standard inverts which of
  `smpte_namespace_wrong` / `interop_namespace_wrong` fires.
- `check_cpl_metadata` requires ContentVersion only for SMPTE.
- `check_main_sound_configuration`, `check_sound_channel_configuration` and
  `advanced::check_bv21_compliance` return empty unless SMPTE, so an Interop
  verdict silently skips them entirely.
- `check_first_subtitle_timing` reads Interop subtitle times as editable units
  and SMPTE ones as ticks, via `subtitle_time_seconds`, so a wrong standard
  scales every timing by the frame rate.

`is_smpte` is not a sufficient replacement as it stands. It is
`xml.contains(SMPTE_AM_NS)`, a substring test over the whole document rather than
the root element's binding, so it is weaker than `schema::root_namespace`, which
already resolves this properly and is what a fix should use. It is also binary
and cannot express Unknown, which `validate_subtitle` and `validate_namespace`
both branch on.

No test depends on the filename derivation. Every fixture under `tests/fixtures`
uses `ASSETMAP.xml`, and every standard-dependent unit test passes `Standard`
directly rather than going through detection. The one package in the tree pairing
a SMPTE-namespace asset map with the Interop name is
`validate::tests::assetmap_name_and_chunk_length_reach_the_pipeline`, and its
assertions are namespace-driven, so it is unaffected either way.

Two dead things found while tracing this, left alone: `schema_validate::validate_namespace`
has no caller, and `facility_check::FacilityCheckOptions::expected_standard` is
written by the CLI and never read.

## The last three ClairMeta gaps, and the HashAlgorithm namespace (2026-08-12)

The 2026-08-12 differential over 155 packages (dci-ctp `diff/report.md`) left
three ClairMeta ERROR checks with no equivalent here. All three are closed, and
so is the namespace hole in the IMF PKL check. Three new codes, so dci-ctp's
`ALL_CODES` denominator goes 81 -> 84.

- `assetmap_invalid_name` (ClairMeta `check_am_name`): the asset map file's own
  name was never checked. ST 429-9:2014 Annex A.4 (normative) says "Each Asset
  Map document shall be a file named "ASSETMAP.xml"", so a SMPTE package under
  any other name is an ERROR. The Interop name `ASSETMAP` comes from the MPEG
  Interop asset map spec v3.4 §6.2, an informative annex and not SMPTE text, so
  the Interop side is a WARNING. `check_assetmap_name` reads the standard from the
  root namespace, never from the file name: `dcp::detect_standard` derives the
  standard from the name, so a check built on it could never fire.
- `assetmap_size_mismatch` (ClairMeta `check_assets_am_size`): nothing here read
  the chunk `Length`. ST 429-9:2014 §7.4: "It shall be absent, or equal to the
  length in bytes of the asset." Absent is legal and dcpwizard writes none at
  all, so `check_assetmap_chunk_size` is silent unless a Length is present and
  disagrees with the file, and silent on a missing file, which is already
  `asset_not_found`. `dcpdoctor_parse::Asset` gained `length: Option<u64>` so
  "not declared" stays distinct from zero.
- `unencrypted_dcp_not_signed`: `dcp_not_signed` keeps its deliberate reading of
  "encrypted packages shall be signed" and still fires at ERROR for encrypted
  packages only. An unsigned unencrypted package now gets this separate code at
  WARNING, a documented divergence from ClairMeta, which errors on it (see
  "Severity policy" above). One `check_dcp_signed` picks severity and code from
  whether the package is encrypted, so the two can never be confused.
- IMF PKL `HashAlgorithm` namespace: the check matched on local name, so an
  element bound to xmldsig instead of the PKL namespace passed. `ds:DigestMethodType`
  is what invites that binding. `parse_pkl` now reads through quick-xml's
  `NsReader` and records the root namespace plus the namespace `HashAlgorithm`
  itself resolved to; `validate_pkl_hash_algorithm` reports a mismatch as
  `missing_required_element`, the same code the absent element already used, so
  no new code. Confined to the PKL parser: the rest of `dcpdoctor-parse` still
  matches by local name.

Not touched, but found while wiring this: `compliance::check_smpte_compliance`
has no caller anywhere in the workspace, and its ASSETMAP naming branch is
unreachable even if called, because it tests `standard == Smpte` against a
`Standard` derived from that same file name.

## Sound essence read from the descriptor, not ffprobe (2026-08-12)

`read_mxf_info` built its SoundDescriptor from ffprobe, which reports only some
WaveAudioDescriptor fields for an MXF. Anything it dropped arrived as 0 and the
check for that field skipped itself instead of running, which is how the
block-align check went unexercised on every cleartext DCP until BlockAlign got a
one-field fallback. `wave_sound_descriptor` now reads the whole descriptor
through asdcplib and ffprobe is the fallback, so the class is gone rather than
patched per field. Verified by hiding ffprobe from PATH: the block-align fixture
still fires and the 5.1 baseline stays silent, so the descriptor path carries the
sound checks on its own. Returns None for anything that is not a cleartext PCM
OP-Atom file (picture, aux data, IMF OP1a, encrypted), each of which falls back
to ffprobe as before. Encrypted essence stays with `check_sound_essence_mxf` so
neither reports twice.

## postkit resolves once across the workspaces (2026-08-12)

`postkit` was a path dep here on `extern/postkit`, and dcpwizard and imfwizard
pull `dcpdoctor-core` by git, which carries its own path dep on the postkit
inside that checkout. Cargo saw two source ids and compiled postkit twice in both
apps, which cost build time and left a latent hazard: `dcpdoctor_core::loudness`
re-exports postkit types, so the day an app used that re-export alongside its own
`postkit::loudness` the two would not unify.

postkit is a git dep here now, pinned at the same rev as the `extern/postkit`
gitlink, with a `[patch]` in `rust/Cargo.toml` redirecting it back at the
submodule. Local builds still use the submodule, so the edit-and-rebuild loop is
unchanged, and consumers add the same patch pointing at their own checkout, which
collapses both references to one crate. `cargo tree -d` reports no postkit
duplicate in dcpwizard and its 267 tests pass against the single copy.

A dependency's own `[patch]` is ignored when it is consumed, so only the
top-level workspace's redirect applies, and nothing local ever fetches the pinned
rev. That means the pin and the gitlink could drift unnoticed, so CI asserts they
match before building.

## Asset identity, IMF PKL digest, Photon bootstrap (2026-08-12)

Three findings from diffing against ClairMeta and Photon, each a defect real
packages carried past dcpdoctor unremarked.

- `mxf_asset_id_mismatch` (new code): a CPL's Id for a track file has to be that
  file's own AssetUUID. `check_asset_id_matches_essence` in validators.rs reads
  the header id through asdcplib (one reader per essence family, picked from
  `essence_type`) and compares it to every CPL id that resolves to an MXF. Runs on
  plain `validate`, no flag: it is header metadata, not essence. ClairMeta rejects
  this in `check_assets_cpl_metadata`. dcpdoctor passed it silently, which is how a
  writer can satisfy every ASSETMAP and PKL reference and still ship assets that
  disagree about what they are. AS-02 PCM aside, every DCP and IMF essence type is
  covered. New code means dci-ctp's `ALL_CODES` denominator goes 80 -> 81.
- IMF PKL `HashAlgorithm`: ST 2067-2:2016 makes it the last element of AssetType,
  and ST 429-8 has no such element, so `validate_pkl_hash_algorithm` runs on the
  IMF path only. The element is empty and carries its value in the `Algorithm`
  attribute, so `dcpdoctor-parse` was reading text that is never there and
  `PklAsset.hash_algorithm` was always empty. It now reads the attribute. Reported
  as `missing_required_element`, one note per offending asset. Photon reports two
  errors per PKL for the same defect.
- Photon bootstrap: plain `validate` on an IMP used to git-clone Netflix/photon
  and gradle-build it. Netflix pins Gradle 8.5, which cannot read Java 25 class
  files, so on a current JDK it failed and the whole gradle stack trace went into
  one `warning:` line. `bootstrap_photon` is deleted. `find_photon` now also
  accepts `PHOTON_DIR` pointing at a single jar and looks in the cache directory
  itself, which is where imfwizard's fetch script drops jars. Nothing to fetch is
  an INFO note, not a warning: an absent optional tool is not a package defect.
  Any failure keeps only its first line, so a Note stays one line.

## Picture bitrate is measured, not estimated (2026-08-12)

`j2k_bitrate_exceeded` never came from a measurement. `validate_j2k_dci` guessed
it from one frame size at a hardcoded 24 fps, and for essence the OP-Atom reader
cannot open (every 3D track file) the ffprobe fallback divided the whole file by
an `nb_frames` that defaults to 1. dci-ctp `valid/dcp_3d` therefore reported
12318.9 Mbps for a package asdcp-info measures at 264 Mb/s. The real analysis in
`bitrate.rs` had no caller anywhere in the workspace.

- `analyze_picture_bitrate` + `check_bitrate_compliance` now run under
  `check_picture_details` (`--check-mxf` or `--deep-j2k`), beside the other checks
  that open the essence, because measuring the peak means reading every frame.
- Stereoscopic essence: the mono reader rejects it, so `bitrate.rs` falls back to
  asdcplib's 3D reader and counts the left plus right codestream of one edit unit
  as one frame. Measurements match asdcp-info: dcp_3d 264.4 against 264.38, ECL42
  593.5 against 593.55, ECL25 358.2 against 358.25.
- The limit comes from `postkit::j2k::dci_max_bitrate_mbps` (250 for 2K, 500 above
  2048 stored width), not a second copy of the numbers.
- The frame-size estimate is deleted, and the ffprobe fallback no longer fills
  `frame_bytes`: it sees the container size, not a codestream length.
- Tests write picture MXFs of known frame size (2K under the limit, 2K over, 4K
  over the 2K limit but under the 4K one, and stereoscopic) and assert the
  measured peak. asdcp-info reports the same rate for each of those files.
- Fallout: dci-ctp's `valid/dcp_3d` baseline genuinely runs at 264 Mb/s and now
  fails `--strict --check-mxf`. That correction belongs in dci-ctp.

## Severity escalations to spec (2026-07-23)

Four codes moved WARNING -> ERROR where SMPTE text uses "shall" (citations in the
code comments); the dci-ctp differential moved these from CLAIRMETA_ONLY_FAIL to
BOTH_FAIL:

- `cpl_mismatched_durations` (sound/picture in validate.rs, aux-data in
  validators.rs): ST 429-2 §9.4, all non-timed-text reel Durations shall be equal.
- `subtitle_font_missing`: ST 428-7:2014, LoadFont shall be present when Text
  elements are; escalated only when the subtitle carries Text (image-only subs
  still warn), so `Scan` now tracks `has_text`.
- `smpte_namespace_wrong` / `interop_namespace_wrong` on the subtitle path:
  ST 428-7 fixes the DCST namespace string, so a wrong namespace is unparseable.

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

Native asdcplib probing (asdcplib pinned 6d7b8ca; `pcm::mca_labels` landed at
5fe4d61) wired into the core verify path:

- MCA labeling: `check_audio_channels` reads the sound MXF's ST 429-12
  subdescriptors and clears `sound_invalid_channel_count` when present; falls back
  to the CPL markers only when no MXF is readable (XML-only). Verified: a 5.1
  dcpwizard DCP clears the INFO, an unlabeled one keeps it, ECL09/ECL07 clean.
- Stereo (ST 429-10): `check_stereo` handles the `msp-cpl:` prefixed form,
  validates FrameRate = 2x EditRate, and confirms Jpeg2000Stereo essence. Verified:
  a dcpwizard `--right-eye` DCP passes, a FrameRate-not-doubled variant fails, real
  ECL07 (TST-3D-48) passes clean.
- Atmos (ST 429-18): `check_aux_data` surfaces `aux_data_detected` for each AuxData
  track (essence-enriched) and errors `cpl_mismatched_durations` when the aux
  duration differs from the reel's picture (ClairMeta
  `check_cpl_reel_duration_picture_aux`; escalation rationale under "Severity
  escalations to spec"); cross-refs/PKL hashes for the aux asset are covered by the
  generic checks (proven by a hash-corruption test). The cross-ref and
  reel-coherence regexes now match the `msp-cpl:`/`axd:` namespaced forms.

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
- `dcp_not_signed` (ClairMeta `check_dcp_signed`, done 2026-07-23): an encrypted
  package (a CPL declares a `<KeyId>` or carries an `<EncryptedDocumentKey`) whose
  CPL or PKL lacks a `<Signature>` errors. Closes the recorded gap where such a
  package only surfaced as the milder `kdm_required`. `validators::check_dcp_signed`
  runs in `verify_dcp` gated on `check_signatures` (so `info`/ImpInfo skip it),
  silent on unencrypted packages. Tests: encrypted-unsigned fires (CPL + PKL),
  encrypted-signed silent, unencrypted-unsigned silent.
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
- App switch DONE 2026-07-23 (extern/postkit synced to canonical HEAD be89fe0).
  j2k / bitrate / frame_compare / Leq(m) now delegate to postkit; the workspace
  compiled clean against the newer postkit with no aba7c12->be89fe0 call-site
  fallout. Per item:
  - j2k.rs: dropped local `parse_cod_extras`; `J2kCodestreamInfo` is now built from
    `postkit::j2k::parse_j2k_header` (it carries code-block exponents +
    irreversible_transform). `analyze_j2k_from_mxf` now reads
    `postkit::j2k::read_mxf_j2k_frame` + `parse_j2k_header` first, so a DCP picture
    MXF reports its real RSIZ/components/transform instead of ffprobe guesses (used by
    `--deep-j2k` / `frame-qc`). The AS-02 / OP1a ffprobe fallback is described in
    DESIGN.md and covered by `j2k::as02_tests`.
    The DCI validation + Note layers (`validate_j2k_dci`, `validate_cinema_j2k`,
    `detect_legacy_ffff`, `check_picture_j2k_mxf`, `check_guard_bits_mxf`) stay
    app-side (they need per-tile-part sizes + encryption-aware reads postkit doesn't
    expose). dcpdoctor-wasm stays pure-bytes (deliberately avoids postkit).
  - bitrate.rs: `FrameBitrateStats` is now a type alias for
    `postkit::j2k::MxfBitrateStats` and `analyze_picture_bitrate` delegates to
    `postkit::j2k::analyse_mxf_bitrate`; the local reader is gone. Note-producing
    `check_bitrate_compliance` stays app-side.
  - frame_compare.rs: local ffmpeg PSNR/SSIM/VMAF core deleted; `compare_files` is a
    thin wrapper over `postkit::frame_compare::compare_frames` (+ `compute_vmaf`) that
    keeps dcpdoctor's per-frame PSNR-threshold scoring. Unused `QualityMetrics`/
    `QualityOptions`/`compute_quality` dropped. `CompareOptions` lost `start_frame`/
    `end_frame` (postkit has no range support; the CLI always passed 0/0).
  - loudness.rs Leq(m): local CCIR 468 weighting + level math (and the `rustfft` dep)
    removed; re-exports `postkit::loudness::{leq_m_from_samples, measure_leq_m,
    LeqMResult}`. The 101.99 dB full-scale-sine assertion is kept as an app-level
    integration test over the re-exported function.

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
