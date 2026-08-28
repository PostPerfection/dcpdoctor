# Planned

Open items and standing decisions: deliberate policy plus the deliberate
omissions below.

## Policy: no silent skips

A check that cannot run must say so. `schema_validation_skipped` is the
template, `check_skipped` is the generic code for it, and `kdm_required`'s split
(INFO when nothing was supplied, WARNING when what was supplied does not cover
the asset) is the severity model. "Checked and clean" and "not checked" must
never look the same in the output: a missing tool, an unreadable input, a parse
failure coerced to a default that disables a downstream guard, and a fallback
that checks less all get a note. The 2026-08-22 audit that enforced this found
roughly sixty such paths, several hiding real defects (an encrypted DCP read as
unencrypted when ffprobe was absent, a corrupt subtitle MXF passing a default
verify, a garbage PKL Size disabling the size check). New checks follow the
policy from the start.

## P-HFR gets no bitrate limit of its own

Peak bitrate is read frame by frame for every picture essence dcpdoctor can
open, AS-DCP mono and 3D and AS-02 (see DESIGN.md). What is left out is
P-HFR, deliberately. ISDCF's P-HFR paper (v005, 2012) sets 500 Mb/s for the total
codestream of 2K stereoscopic HFR, keyed on a P-HFR-2K essence label, and calls
itself a proposal for experimental use. asdcplib applies 400 to that same label
citing no source, stricter than the document defining it. Neither is normative,
and either would need the essence coding UL, which dcpdoctor does not read. Same
note in postkit's DESIGN_TODO.

## App 2E picture is not decoded

The four descriptor checks (Rsiz is an IMF profile, ColorPrimaries and
TransferCharacteristic present, coding label matches the Rsiz, pixel layout
matches the codestream) read the AS-02 header and are in `app2e_picture`. The
fifth check, a decoded saturated patch coming back RGB rather than X'Y'Z', needs
a decoder and dcpdoctor does not link grok, so it stays out until there is
somewhere to decode a frame. A codestream whose Rsiz and label say IMF but
whose samples are X'Y'Z' would still pass.

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
the four codes at ERROR (cpl_mismatched_durations, subtitle_font_missing,
smpte_namespace_wrong, interop_namespace_wrong), each backed by the spec citation
in its code comment.
