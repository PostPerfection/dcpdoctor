# Changelog

## Unreleased

### Added
- `check_skipped`: a check that should have run and could not now says so, naming the check and the reason. Introduced by an audit that removed every silent skip found in the codebase: "checked and clean" and "not checked" no longer look the same in any output.

### Fixed
- Fabricated passes, each now a reported failure: `hdr-validate` read a file ffprobe could not open as SDR/BT.709 and passed it against default expectations, `check_netflix_delivery` printed PASS for a package whose CPLs were never read, `checksum-verify` reported "All checksums verified OK (0 asset(s))" on a PKL it could not parse and counted an asset with no readable Hash or Size as verified, `facility-check` said "Ready for delivery: YES" without ever verifying a hash (it now runs the PKL hash verification, and `--no-hashes` shows an explicit not-checked item), `av-sync` printed a drift of 0.0 ms when ffprobe was missing, and `auto-qc` reported a 0.0 dBFS peak for audio ffmpeg could not read and false clipping on silent channels (astats' `-inf` did not parse).
- Missing-tool skips now warn instead of thinning the run: without ffprobe an encrypted DCP read as unencrypted from `--studio`, 3D essence classified as 2D, `--hdr`/`--atmos`/`--dolby-vision`/`--prores` returned nothing at all, and the picture essence checks under `--check-mxf` silently did not run. The same for a missing or partial schema directory, a schema file absent for one document's type, and xmllint failures on the `schema-validate` and `kdm` paths.
- Unreadable input no longer passes as clean: a corrupt picture MXF skipped every codestream check, a cleartext frame that is not JPEG 2000 passed them, a corrupt subtitle or closed-caption MXF (and an unreadable loose XML asset) skipped every timed-text rule, an unreadable embedded font or one that resolves to nothing skipped glyph coverage, an unreadable CPL skipped the BV2.1 and HFR passes, and `dcp diff` called two packages identical while some assets were never compared.
- Parse failures no longer disable the checks reading the value: a PKL asset with no Id vanished from the size and hash checks, a garbage PKL `Size` or ASSETMAP `Length` read as "not declared", a garbage reel `Duration` or `EntryPoint` read as zero (an unparseable timed-text EntryPoint passed the must-be-zero rule), an ASSETMAP that stops parsing mid-file returned the ids read so far, and a truncated codestream marker walk passed the placement rules over the unwalked remainder. Each is an error now.
- `fix` no longer loses findings: a hash mismatch it could not repair, a repair helper that returned without writing, and a PKL write that failed after the fact all landed in neither "repaired" nor "skipped", so the summary read "Nothing to fix" while the defect remained. `checksum-verify` also parsed `ASSETMAP.xml` as a second PKL (it contains the string `PackingList`), turning a valid package into bogus hash failures once empty hashes stopped counting as verified.
- A KDM with no signature errors (`signature_invalid`): ST 430-1 requires KDMs to be signed and only CPL/PKL presence was checked. A KDM missing its validity window now warns instead of passing with an empty-dated INFO, a certificate with no dnQualifier fails the ST 430-2 thumbprint rule instead of skipping it, and an undecodable certificate blob in ds:KeyInfo breaks the chain instead of vanishing. An empty `ContentTitleText` reaches the ISDCF naming rules instead of gating them off.
- The peak-bitrate measurement failing (unopenable essence, zero duration) is reported instead of leaving only the container-average estimate, and the ffprobe fallback for codestream parameters says which marker-level checks its stub fields cannot support. `qc-report` renders the notes for a picture track it could not scan and a row for a loudness measurement that failed, instead of omitting the section.
- Deleted as dead: `validate_audio` (no callers), `schema_validate::validate_namespace` (no callers), the studio supplemental pair (never invoked, its trigger never set), and `facility-check`'s `--strict` flag with the never-read `expected_standard` option. The presence test matched the literal prefixes `<Signature` and `<ds:Signature`, so a package binding the signature namespace to `dsig:`, which is what DCP-o-matic and the ISDCF reference DCPs write, read as unsigned: nothing verified its signature and `unencrypted_dcp_not_signed` fired on a package that carries one. Detection resolves the namespace now, a KDM gets the by-Id verification its `ds:Reference` elements need (it had none), and postkit canonicalizes under the algorithm each signature declares (pin 8256659 -> 97c8e82), without which a document declaring the plain comment-free C14N and carrying an XML comment failed its digest. A document changed after signing errors as `signature_invalid`. That verdict means the document has not changed since it was signed, not that a trusted party signed it: the verifying key is the leaf certificate the document itself carries, and whether that chain is acceptable remains the `certificate_*` rules' verdict.

## [0.5.0] - 2026-08-22

### Added
- CPL and PKL identity (Bv2.1 §8.1): a SMPTE CPL's `AnnotationText` must be present (`missing_required_element`) and equal its `ContentTitleText` (`cpl_annotation_text_mismatch`), and a PKL packaging exactly one CPL must repeat that title as its own `AnnotationText` (`pkl_annotation_text_mismatch`). A PKL listing one asset id twice now errors as `duplicate_asset_id`, the rule the ASSETMAP already had.
- CPL metadata (Bv2.1 §8.6.1): a SMPTE CPL with no `CompositionMetadataAsset`, or one with no `VersionNumber`, now reports `missing_required_element` instead of being skipped.
- `cpl_active_area_invalid`: a `MainPictureActiveArea` with an odd width or height, or one larger than the picture essence the reel carries, errors.
- `cpl_invalid_language`: every `Language`, `MainSubtitleLanguageList`, `AdditionalSubtitleLanguageList` and `ReleaseTerritory` value in a SMPTE CPL is checked against the IANA subtag registry, so `Deutsch` where `de` belongs is caught rather than passing a grammar test.
- J2K TLM and POC checks on the native path: a codestream with no TLM marker errors as `j2k_missing_tlm`, and `j2k_poc_invalid` covers the POC marker count the profile fixes (none for 2K, exactly one for 4K), a POC in a tile-part header, and the two fixed 4K progressions' parameter values. The marker walk is shared with the browser build.
- `--deep-j2k` now runs the codestream checks over every frame instead of frame 0 only, in the same pass as the RDD 52 guard-bit scan, so a stream that goes non-conformant partway through is caught. Each finding reports once with the first offending frame and how many frames carried it.
- Codestream forensics, collected in the same per-frame pass `--deep-j2k` already runs: the first frame's parameters (dimensions, profile, decomposition levels, code-block size, transform, layers, progression, tiles, tile-parts, MCT, TLM and POC presence) become an INFO summary (`j2k_codestream_summary`) alongside the worst frame as a percentage of the DCI per-frame byte cap, and any later frame that departs from those parameters warns as `j2k_parameters_vary` with the first frame and the count. Stereoscopic essence is scanned too, both eyes of every edit unit, each eye against half an edit unit's cap. `dcpdoctor qc-report` gains a "Codestream forensics" section, one table per picture track.
- Codestream forensics on IMF packages: an IMP's AS-02 picture tracks are scanned frame by frame under `--deep-j2k` like a DCP's, reporting the same `j2k_codestream_summary` INFO and `j2k_parameters_vary` warning, and `qc-report` gains their forensics tables. The cinema-only rules do not run on IMF essence, which is not encoded to the DCI profile: no ISO 15444-1 cinema profile checks, no 0xFFFF legacy-decoder check, no RDD 52 guard-bit check, and no DCI per-frame byte cap in the summary or the report table.
- `partially_encrypted`: a composition mixing encrypted and clear essence now errors. Each asset is well formed on its own, so nothing caught it, and the result plays in part without a KDM.
- Playback compatibility warnings, which report what installed equipment will not play rather than what the specifications forbid: a frame rate of 25, 30, 48, 50 or 60 (`projector_frame_rate_support`, naming the rate to deliver instead where there is one), 4K 3D (`projector_4k_stereo_support`), and sound with other than 8 or 16 channels (`distributor_audio_channel_count`).
- `subtitle_font_too_large`: an embedded font over 640 kB warns. The Bv2.1 cap is 10 MB across every font in the asset, which a single font this size passes while still stalling playback.
- `closed_caption_interop_overlap`: Interop closed captions overlapping in time now error, since the format cannot say which of two simultaneous captions wins.
- Timed-text timing and structure (Bv2.1 §7.2.3, §8.3.2): a SMPTE document with no `<StartTime>`, or a non-zero one that shifts every cue in the reel, errors as `missing_required_element` / `subtitle_invalid_timing`; a timed-text reel asset with no `<EntryPoint>` or a non-zero one errors as `subtitle_entry_point` / `closed_caption_entry_point`; a cue running past the reel that holds it errors as `subtitle_overlaps_reel`; and across a composition, subtitles present on only some reels (`subtitle_missing_from_reel`), subtitle assets declaring different `<Language>` values (`subtitle_language_mismatch`) and reels carrying unequal numbers of closed captions (`closed_caption_count_mismatch`) all error.
- Timed-text content rules: a `<Text>` element with no content errors as `subtitle_empty_text` (whitespace counts as content, as in libdcp, so the one-space placeholder cue DCP-o-matic writes on reels without subtitles passes), a `<Font>` naming an id no `<LoadFont>` introduced errors as `subtitle_font_missing`, an Interop asset holding no cues at all errors as `missing_required_element`, more than one namespace on the document root warns as `subtitle_namespace_count`, and a SMPTE `<IssueDate>` outside `yyyy-mm-ddThh:mm:ss` warns as `subtitle_invalid_issue_date`. Within one caption cue, lines that mix vertical alignments or are listed out of their on-screen order error as `closed_caption_layout`.
- Timed-text size and identity rules (Bv2.1 §7.2, ST 429-5): a track file over 115 MB, embedded fonts totalling over 10 MB, or a closed-caption document over 256 kB report `timed_text_size_exceeded`; an MXF whose ResourceID does not repeat the document's `<Id>`, or which reuses its AssetID as either, reports `timed_text_id_mismatch`; and essence whose duration disagrees with the reel's `Duration` reports `cpl_mismatched_durations`. Closed-caption tracks are now parsed from the CPL reel and checked alongside subtitles.
- Timed-text rules run on encrypted essence when a KDM and recipient key are supplied, instead of skipping every subtitle and caption asset in an encrypted package. Where no key covers an asset the rules still skip, but the report now says so (`kdm_required`) rather than passing in silence.
- MXF-wrapped subtitles reach the structural rule set: the ST 429-5 document is unwrapped from the essence and given the same checks a loose XML asset gets, so a SMPTE package (where subtitles are always wrapped) is no longer limited to glyph coverage.
- Subtitle documents are schema-validated: SMPTE ST 428-7 timed text routes to the DCDMSubtitle XSD for the DCST namespace it declares (2007, 2010 or 2014) and Interop DCSubtitle to `DCSubtitle.xsd`, both loose and MXF-wrapped, so a schema-invalid subtitle now reports `xml_schema_violation` instead of passing.
- `schema_validation_skipped`: the XSD pass used to skip itself silently when the schema directory or `xmllint` was absent, so a machine without either reported a clean run with no XSD coverage. It now warns and names the reason.
- CPL asset hashes: every reel asset's `<Hash>` is now read and compared with the PKL's for the same asset, so a disagreement errors as `cpl_pkl_hash_mismatch` (libdcp MISMATCHED_PICTURE_HASHES / MISMATCHED_SOUND_HASHES) and an MXF-backed asset with no `<Hash>` warns as `cpl_missing_hash`. Servers that hash-check against the CPL rather than the PKL reject the first case.
- Encrypted IMF essence reads with the run's content keys: `validate --kdm --recipient-key` now reaches an IMP, so an encrypted AS-02 picture track gets the same `picture_bitrate_measured` peak/average and the same codestream forensics a DCP with a KDM gets. Where no key covers a track the report says so (`kdm_required`, INFO with no KDM, WARNING when the KDM does not carry the track's KeyId) instead of passing in silence.
- KDM digest rules the schema cannot express, all errors: a DeviceList `<CertificateThumbprint>` that does not decode to a 20-byte SHA-1 digest (`kdm_thumbprint_invalid`), the same for `<ContentAuthenticator>` when present (`kdm_content_authenticator_invalid`), and the DCI assume-trust marker (DCSS 9.4.3.5) sharing a DeviceList with a real device thumbprint (`kdm_assume_trust_conflict`). Checked against real DCP-o-matic KDMs, one per ISDCF formulation, under `tests/fixtures/kdm`.

### Fixed
- FFMC and FFEC are required under `--strict` only in a feature, which is the Bv2.1 scope, so a test or trailer composition carrying FFOC and LFOC no longer warns. LFMC, which no rule requires, is recommended rather than required.
- `kdm_required` no longer reports a missing KDM when one was supplied to the run (`--kdm`): it only looked for a file inside the package directory.
- Picture bitrate is no longer measured off encrypted frames. asdcplib returns ciphertext when it is given no content key, and an AES frame is longer than the codestream it carries, so an encrypted track without a KDM reported the wrapping's rate: on a DCP that fed the DCI limit verdict. Encrypted essence is now measured only where a key decrypts it.
- `cpl_invalid_language` no longer fires on a conformant `ReleaseTerritory`. A territory declaring the ST 429-16 UN M.49 scope carries a numeric code, which was being measured against the RFC 5646 region grammar and flagged every time. It is now checked only where libdcp checks it, outside that scope.
- The browser build rejected any POC marker as "not permitted for DCI", which is a false positive on every conformant 4K DCP: the 4K profile requires exactly one in the main header. It now applies the same per-profile rule as the native path.
- The accepted picture sizes were the 2K/4K containers only, so every standard scope DCP (2048x858, 4096x1716) drew a spurious `picture_invalid_resolution` warning under `--strict`. Both coded scope sizes are now accepted.

## [0.2.0] - 2026-08-13

### Added
- GUI keyboard shortcuts: Ctrl+O opens a DCP folder, Ctrl+Enter validates, Ctrl+1 to Ctrl+4 pick a results severity filter, Ctrl+, toggles preferences and Escape closes them. Ctrl+K opens an overlay listing them by category, where clicking a shortcut captures a new key combination (Backspace or Delete unbinds, Escape cancels, a combination already in use is refused and names its owner). Per-shortcut and global reset buttons restore the defaults, and changes persist across restarts.
- 4K frame rate (ST 429-2 §8.2 Table 1): picture essence wider than 2K must run at 24/1, 25/1 or 30/1, so a 4K track file at 48/1 errors as `picture_invalid_frame_rate` under `--check-mxf`. Monoscopic essence only, which is the scope of the requirement.
- IMF picture bitrate: an IMP's AS-02 picture track files are measured frame by frame like a DCP's, and the peak and average are reported as a `picture_bitrate_measured` info note under `--check-mxf`. No pass/fail: the 250 Mbps ceiling is DCI's and no IMF specification sets one.
- MainSoundConfiguration (ST 429-16): SMPTE CPLs are checked for the tag in CompositionMetadataAsset, its `<soundfield>/<channels>` value is parsed against the MCA/ISDCF label set (garbage like `None` errors as `main_sound_config_invalid`), and the declared channel count is cross-checked against the sound MXF's ChannelCount (`sound_invalid_channel_count`).
- FFOC/LFOC marker offsets: FFOC in the first reel must be 1 and LFOC in the last reel must be one less than that reel's duration, warned per libdcp wording (`marker_invalid`).
- First-subtitle timing (Bv2.1): warns (`subtitle_first_event_early`) when the first reel's first displayable timed-text event starts under 4s in. Only the first reel counts and empty placeholder subtitle assets are ignored, avoiding DCP-o-matic bug #2757. Reads MXF-wrapped ST 428-7 XML and handles both SMPTE tick and Interop editable-unit TimeIn forms.
- Timed-text content (Bv2.1 §7.2.5-7.2.7): every MainSubtitle and ClosedCaption asset is checked for line count (>3 lines), line length (subtitle 52 warn / 79 max, both `subtitle_line_length`; closed caption 32, `closed_caption_line_length`, error), minimum duration (`subtitle_duration`, 15 frames) and minimum gap (`subtitle_spacing`, 2 frames). Subtitle vs closed-caption limits are picked from the CPL asset type. Character counts are unicode scalar values, not bytes (DoM bug #3097). A `closed_caption_charset` INFO lists caption characters outside the ISDCF Doc 9 set (ISO 8859-1 plus U+266A). Reuses the plain-XML/MXF ST 428-7 extraction. (DoM #3149, #3151, #3153, #3158, #3097)
- Stereoscopic 3D (ST 429-10): the msp-cpl `MainStereoscopicPicture` form is now validated as a picture track (cross-refs, duration), the FrameRate = 2x EditRate relationship is checked, and the essence type is confirmed as stereoscopic J2K where the MXF is present.
- Auxiliary data (ST 429-18): an `aux_data_detected` INFO identifies each AuxData track (Dolby Atmos / IAB), enriched with the probed essence type, and a duration mismatch against the reel's picture errors as `cpl_mismatched_durations` (ST 429-2 §9.4). The aux asset's cross-refs and PKL hashes are covered by the generic asset checks.
- PKL `Size` is checked against the actual file size (`pkl_size_mismatch`), independent of `--no-hashes`.
- Leq(m) loudness (ISO 21727, CCIR 468-weighted): reported alongside EBU R128 in the `loudness` command (text + JSON `leq_m_db`) and the qc-report HTML table.
- `dcp_not_signed` (ClairMeta `check_dcp_signed`): an encrypted package (a CPL declares a `<KeyId>` or carries an `<EncryptedDocumentKey>`) whose CPL or PKL lacks a `<Signature>` now errors instead of only surfacing the milder `kdm_required`.
- `unencrypted_dcp_not_signed`: an unsigned unencrypted package warns under its own code. No SMPTE "shall" demands a signature there, so this stays a warning where ClairMeta errors.
- `assetmap_invalid_name` (ClairMeta `check_am_name`): the asset map file's name is checked against the standard its root namespace declares. ST 429-9:2014 Annex A.4 requires `ASSETMAP.xml` for SMPTE, so a wrong name errors; the Interop name `ASSETMAP` comes from an informative annex of the MPEG Interop asset map spec (§6.2) and warns.
- `assetmap_size_mismatch` (ClairMeta `check_assets_am_size`): a chunk's `Length`, when present, must equal the asset's size on disk (ST 429-9:2014 §7.4). The element is optional, so an asset map that declares none stays silent.
- `reel_edit_rate_mismatch`: a reel's MainMarkers, MainSubtitle, MainClosedCaption and AuxData EditRate is compared against the picture's. Warns rather than errors, since ST 429-2 §9.6.1 and §9.7.1 are the only per-asset-class EditRate equality rules and they name picture and sound. Sound and CompositionMetadataAsset have their own "shall" and are not covered here.
- `composition_metadata_asset_mismatch`: ST 429-16:2014 §4.4.1 binds the CompositionMetadataAsset's `EditRate` to the picture's and its `IntrinsicDuration` to the picture's `Duration`, both "shall", so a disagreement errors.
- ST 429-2 §9.6.1 also fixes the picture element name and §9.7.1 the sound `Language`, neither of which reel coherence read. Both are now compared across reels as `reel_incoherent`.
- KDMs are schema-validated against the vendored ST 430-1 / ST 430-3 XSDs, so a KDM missing a required element such as `AuthorizedDeviceInfo` now reports `xml_schema_violation`. Runs on both `validate --kdm` and the `kdm` subcommand, and skips when the schema dir or xmllint is absent.

### Fixed
- The vendored `SMPTE-430-3-2006-ETM.xsd` had its UUID pattern facet split across a line, which is not a valid regular expression, so both KDM schemas failed to compile.
- The IMF PKL `HashAlgorithm` check matched on local name, so an element bound to the xmldsig namespace instead of the PKL one passed. `parse_pkl` now resolves namespaces and the check reports a wrong binding as `missing_required_element`.
- MCA channel labeling is read from the sound MXF's ST 429-12 subdescriptors (asdcplib `pcm::mca_labels`) instead of grepping the CPL, so `sound_invalid_channel_count` no longer false-fires on correctly labeled DCPs. Falls back to the CPL markers only for XML-only validation.
- Cross-reference and reel-coherence checks now match the namespaced (`msp-cpl:`, `axd:`) reel-asset forms real DCPs emit, so their asset ids are no longer skipped.

### Changed
- The DCI picture bitrate limit is 250 Mbps at every resolution. The 500 previously applied to 4K has no source in DCI, ST 429-4 or ST 429-2: DCSS 4.3.3 caps a 4K frame at the same 1,302,083 bytes as a 24 fps 2K one. A 4K package between 250 and 500 Mbps now errors where it passed. The IMF path's separate hardcoded copy of the old split is gone, and the limit comes from `postkit::j2k::DCI_MAX_BITRATE_MBPS`.
- Removed the fixed "DCI maximum bitrate is 500 Mbps for all HFR content" INFO, which asserted an unsourced limit from the CPL edit rate with no measurement behind it. The measured check covers this.
- The SMPTE/Interop standard is derived from the ASSETMAP's root namespace instead of its filename, so a package whose asset map is named for the wrong standard is no longer validated as that standard. `AssetMap.is_smpte`, a substring test over the whole document, is removed in favour of the shared derivation.
- `cpl_invalid_edit_rate` for the ST 429-2 §8.1 composition edit rate no longer needs `--strict`. It is a plain "shall", and the wasm validator already ran it ungated.
- Removed `compliance::check_smpte_compliance` and the private helpers only it reached. It had no caller anywhere in the workspace, and its asset-map naming branch could never fire since it keyed off a `Standard` derived from the same file name it was checking. Every code it produced is still produced elsewhere.
- Schema validation (`xml_schema_violation`) is now on by default: the SMPTE/Interop XSDs are vendored under `schemas/` (from the ClairMeta set, with `catalog.xml`), so `DCPDOCTOR_SCHEMA_DIR` is no longer required. The env var still overrides, and a missing dir degrades to skip.
- Severity escalated to ERROR where SMPTE text uses "shall": `cpl_mismatched_durations` (ST 429-2 §9.4), `subtitle_font_missing` when the subtitle carries Text (ST 428-7:2014; image-only subs still warn), and a wrong subtitle DCST namespace (`smpte_namespace_wrong` / `interop_namespace_wrong`, ST 428-7).
- j2k / bitrate / frame comparison / Leq(m) now delegate to the shared `postkit` library; the DCI-validation and note layers stay app-side. The AS-DCP jp2k reader is OP-Atom only, so IMF AS-02 / OP1a picture MXFs fall back to the ffprobe-derived path.
- Bumped asdcplib pin to `6d7b8ca` (adds `pcm::mca_labels`); postkit pinned at `8b4a034`.

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
