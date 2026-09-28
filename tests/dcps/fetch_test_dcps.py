#!/usr/bin/env python3
"""Download and create test DCP content for dcpdoctor validation.

Usage:
    python fetch_test_dcps.py [--all|--isdcf|--framing|--hfr|--stereo3d|--synthetic]

Default (no args): creates synthetic test DCPs only (no network needed).
"""

import argparse
import os
import shutil
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent

GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
RED = "\033[0;31m"
NC = "\033[0m"

# Disable colours on Windows unless running in a modern terminal
if sys.platform == "win32" and "WT_SESSION" not in os.environ:
    GREEN = YELLOW = RED = NC = ""


def info(msg: str) -> None:
    print(f"{GREEN}[INFO]{NC} {msg}")


def warn(msg: str) -> None:
    print(f"{YELLOW}[WARN]{NC} {msg}")


def fail(msg: str) -> None:
    print(f"{RED}[FAIL]{NC} {msg}", file=sys.stderr)
    sys.exit(1)


def download(url: str, dest: Path) -> bool:
    if dest.exists():
        info(f"Already downloaded: {dest}")
        return True
    info(f"Downloading: {url}")
    try:
        urllib.request.urlretrieve(url, str(dest))
        return True
    except Exception as e:
        warn(f"Failed to download {url}: {e}")
        return False


# ── ISDCF Bv2.1 SMPTE Test Content ─────────────────────────────────────────
def fetch_isdcf() -> None:
    info("=== ISDCF Bv2.1 SMPTE Test Content ===")
    d = SCRIPT_DIR / "isdcf"
    d.mkdir(parents=True, exist_ok=True)

    zip_path = d / "SMPTE_TST-1-Bv21_51-71.zip"
    download(
        "http://files.isdcf.com/SMPTE-DCP-Content/"
        "SMPTE_TST-1-Bv21_51-71_20170110_SMPTE_Folders.zip",
        zip_path,
    )

    extracted = d / "SMPTE_TST-1-Bv21_51-71_20170110_SMPTE_Folders"
    if zip_path.exists() and not extracted.exists():
        info("Extracting ISDCF test content...")
        try:
            with zipfile.ZipFile(zip_path) as zf:
                zf.extractall(d)
        except Exception as e:
            warn(f"Extraction failed: {e}")

    download(
        "http://files.isdcf.com/SMPTE-DCP-Content/"
        "ISDCF_SMPTE_DCP_TEST_INSTRUCTIONS_180912.pdf",
        d / "test_instructions.pdf",
    )
    download(
        "http://files.isdcf.com/SMPTE-DCP-Content/SMPTE-DCP-checklist.pdf",
        d / "checklist.pdf",
    )
    info(f"ISDCF content ready in {d.relative_to(SCRIPT_DIR)}/")
    info(
        "NOTE: Test content is encrypted. "
        "Get keys from https://one.bydeluxe.com/self-registration"
    )


# ── ISDCF Framing Charts ───────────────────────────────────────────────────
def fetch_framing() -> None:
    info("=== ISDCF Framing Charts ===")
    d = SCRIPT_DIR / "framing"
    d.mkdir(parents=True, exist_ok=True)

    readme = d / "README.md"
    readme.write_text(
        "# ISDCF Framing Charts\n\n"
        "Download framing chart DCPs from:\n"
        "https://www.isdcf.com/smpte-dcp-tests/framing-chart/\n\n"
        "Available charts:\n"
        "- 2K Flat (1998x1080)\n"
        "- 2K Scope (2048x858)\n"
        "- 4K Flat (3996x2160)\n"
        "- 4K Scope (4096x1716)\n\n"
        "These are unencrypted test patterns for projector alignment "
        "and framing verification.\n"
    )
    info(f"Framing info saved to {readme.relative_to(SCRIPT_DIR)}")


# ── Fox HFR Test Content ───────────────────────────────────────────────────
def fetch_hfr() -> None:
    info("=== Fox HFR/Frame Rate Test Content ===")
    d = SCRIPT_DIR / "hfr"
    d.mkdir(parents=True, exist_ok=True)

    tar_path = d / "fox-format-tests-hfr.tar"
    download(
        "https://dci.foxpico.com/basic/plugfest/"
        "fox-format-tests-hfr-20160510.tar",
        tar_path,
    )

    if tar_path.exists() and not (d / "fox-format-tests").exists():
        info("Extracting HFR test content...")
        try:
            with tarfile.open(tar_path) as tf:
                tf.extractall(d)
        except Exception as e:
            warn(f"Extraction failed: {e}")

    info(f"HFR content ready in {d.relative_to(SCRIPT_DIR)}/")


# ── 3D Stereo Subtitle Test ────────────────────────────────────────────────
def fetch_stereo3d() -> None:
    info("=== 3D Stereo Subtitle Comparison Test ===")
    d = SCRIPT_DIR / "stereo3d"
    d.mkdir(parents=True, exist_ok=True)

    readme = d / "README.md"
    readme.write_text(
        "# ISDCF 3D Stereo Subtitle Comparison Test\n\n"
        "Download from:\n"
        "http://files.isdcf.com/SMPTE-DCP-Content/"
        "3d_stereo-comparison-test_smpte_nov-2015\n\n"
        "Contains stereoscopic 3D subtitle test content "
        "for SMPTE DCP validation.\n"
    )
    info(f"3D stereo info saved to {readme.relative_to(SCRIPT_DIR)}")


# ── Synthetic Test DCPs ────────────────────────────────────────────────────
def _write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def create_synthetic() -> None:
    info("=== Creating Synthetic Test DCPs ===")
    base = SCRIPT_DIR / "synthetic"

    # ── Minimal SMPTE 2K ──────────────────────────────────────────────────────
    vd = base / "minimal" / "minimal_smpte_2k"
    if not vd.exists():
        info("Creating minimal SMPTE 2K DCP...")
        _write(
            vd / "ASSETMAP.xml",
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">\n'
            "  <Id>urn:uuid:a1b2c3d4-e5f6-7890-abcd-ef1234567890</Id>\n"
            "  <AnnotationText>Test DCP</AnnotationText>\n"
            "  <VolumeCount>1</VolumeCount>\n"
            "  <IssueDate>2024-01-01T00:00:00+00:00</IssueDate>\n"
            "  <Issuer>DcpDoctor Test Suite</Issuer>\n"
            "  <Creator>DcpDoctor</Creator>\n"
            "  <AssetList>\n"
            "    <Asset>\n"
            "      <Id>urn:uuid:b2c3d4e5-f6a7-890b-cdef-234567890abc</Id>\n"
            "      <PackingList>true</PackingList>\n"
            "      <ChunkList>\n"
            "        <Chunk><Path>PKL_test.xml</Path></Chunk>\n"
            "      </ChunkList>\n"
            "    </Asset>\n"
            "    <Asset>\n"
            "      <Id>urn:uuid:c3d4e5f6-a7b8-90cd-efab-34567890abcd</Id>\n"
            "      <ChunkList>\n"
            "        <Chunk><Path>CPL_test.xml</Path></Chunk>\n"
            "      </ChunkList>\n"
            "    </Asset>\n"
            "  </AssetList>\n"
            "</AssetMap>\n",
        )
        _write(
            vd / "VOLINDEX.xml",
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<VolumeIndex xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">\n'
            "  <Index>1</Index>\n"
            "</VolumeIndex>\n",
        )
        _write(
            vd / "PKL_test.xml",
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">\n'
            "  <Id>urn:uuid:b2c3d4e5-f6a7-890b-cdef-234567890abc</Id>\n"
            "  <AnnotationText>Test PKL</AnnotationText>\n"
            "  <IssueDate>2024-01-01T00:00:00+00:00</IssueDate>\n"
            "  <Issuer>DcpDoctor Test Suite</Issuer>\n"
            "  <Creator>DcpDoctor</Creator>\n"
            "  <AssetList>\n"
            "    <Asset>\n"
            "      <Id>urn:uuid:c3d4e5f6-a7b8-90cd-efab-34567890abcd</Id>\n"
            "      <AnnotationText>Test CPL</AnnotationText>\n"
            "      <Hash>placeholder</Hash>\n"
            "      <Size>1234</Size>\n"
            "      <Type>text/xml;asdcpKind=CPL</Type>\n"
            "    </Asset>\n"
            "  </AssetList>\n"
            "</PackingList>\n",
        )
        _write(
            vd / "CPL_test.xml",
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">\n'
            "  <Id>urn:uuid:c3d4e5f6-a7b8-90cd-efab-34567890abcd</Id>\n"
            "  <AnnotationText>Test CPL</AnnotationText>\n"
            "  <IssueDate>2024-01-01T00:00:00+00:00</IssueDate>\n"
            "  <Issuer>DcpDoctor Test Suite</Issuer>\n"
            "  <Creator>DcpDoctor</Creator>\n"
            '  <ContentTitleText>DCPDOCTOR-TEST_FTR-1_F_EN-XX_US_51_2K_TEST_20240101_DCR_SMPTE_OV</ContentTitleText>\n'
            "  <ContentKind>feature</ContentKind>\n"
            "  <ContentVersion>\n"
            "    <Id>urn:uuid:d4e5f6a7-b890-cdef-abcd-4567890abcde</Id>\n"
            "    <LabelText>Test Version 1</LabelText>\n"
            "  </ContentVersion>\n"
            "  <RatingList/>\n"
            "  <ReelList>\n"
            "    <Reel>\n"
            "      <Id>urn:uuid:e5f6a7b8-90cd-efab-bcde-567890abcdef</Id>\n"
            "      <AssetList>\n"
            "      </AssetList>\n"
            "    </Reel>\n"
            "  </ReelList>\n"
            "</CompositionPlaylist>\n",
        )
        info(f"Created {vd.relative_to(SCRIPT_DIR)}")

    # ── Minimal Interop ───────────────────────────────────────────────────────
    interop = base / "minimal" / "minimal_interop"
    if not interop.exists():
        _write(
            interop / "ASSETMAP",
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<AssetMap xmlns="http://www.digicine.com/PROTO-ASDCP-AM-20040311#">\n'
            "  <Id>urn:uuid:f0000000-0000-0000-0000-000000000001</Id>\n"
            "  <VolumeCount>1</VolumeCount>\n"
            "  <IssueDate>2024-01-01T00:00:00+00:00</IssueDate>\n"
            "  <Issuer>DcpDoctor</Issuer>\n"
            "  <Creator>DcpDoctor</Creator>\n"
            "  <AssetList>\n"
            "    <Asset>\n"
            "      <Id>urn:uuid:f0000000-0000-0000-0000-000000000002</Id>\n"
            "      <PackingList>true</PackingList>\n"
            "      <ChunkList>\n"
            "        <Chunk><Path>PKL_interop.xml</Path></Chunk>\n"
            "      </ChunkList>\n"
            "    </Asset>\n"
            "  </AssetList>\n"
            "</AssetMap>\n",
        )
        _write(
            interop / "PKL_interop.xml",
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<PackingList xmlns="http://www.digicine.com/PROTO-ASDCP-PKL-20040311#">\n'
            "  <Id>urn:uuid:f0000000-0000-0000-0000-000000000002</Id>\n"
            "  <AnnotationText>Interop Test PKL</AnnotationText>\n"
            "  <IssueDate>2024-01-01T00:00:00+00:00</IssueDate>\n"
            "  <Issuer>DcpDoctor</Issuer>\n"
            "  <Creator>DcpDoctor</Creator>\n"
            "  <AssetList/>\n"
            "</PackingList>\n",
        )
        info(f"Created {interop.relative_to(SCRIPT_DIR)}")

    # ── Invalid: Missing ASSETMAP ───────────────────────────────────────────
    no_am = base / "invalid" / "missing_assetmap"
    if not no_am.exists():
        no_am.mkdir(parents=True, exist_ok=True)
        shutil.copy2(vd / "PKL_test.xml", no_am / "PKL_test.xml")
        shutil.copy2(vd / "CPL_test.xml", no_am / "CPL_test.xml")
        info(f"Created {no_am.relative_to(SCRIPT_DIR)} (missing ASSETMAP)")

    # ── Invalid: Malformed XML ──────────────────────────────────────────────
    bad_xml = base / "invalid" / "bad_xml"
    if not bad_xml.exists():
        bad_xml.mkdir(parents=True, exist_ok=True)
        shutil.copy2(vd / "ASSETMAP.xml", bad_xml / "ASSETMAP.xml")
        shutil.copy2(vd / "VOLINDEX.xml", bad_xml / "VOLINDEX.xml")
        shutil.copy2(vd / "PKL_test.xml", bad_xml / "PKL_test.xml")
        (bad_xml / "CPL_test.xml").write_text(
            "<<<NOT VALID XML>>>", encoding="utf-8"
        )
        info(f"Created {bad_xml.relative_to(SCRIPT_DIR)} (malformed CPL)")

    # ── Invalid: Bad hash ───────────────────────────────────────────────────
    bad_hash = base / "invalid" / "bad_hash"
    if not bad_hash.exists():
        bad_hash.mkdir(parents=True, exist_ok=True)
        shutil.copy2(vd / "ASSETMAP.xml", bad_hash / "ASSETMAP.xml")
        shutil.copy2(vd / "VOLINDEX.xml", bad_hash / "VOLINDEX.xml")
        shutil.copy2(vd / "CPL_test.xml", bad_hash / "CPL_test.xml")
        pkl_text = (vd / "PKL_test.xml").read_text(encoding="utf-8")
        (bad_hash / "PKL_test.xml").write_text(
            pkl_text.replace("placeholder", "AAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            encoding="utf-8",
        )
        info(f"Created {bad_hash.relative_to(SCRIPT_DIR)} (wrong hash)")

    # ── Invalid: Empty DCP ──────────────────────────────────────────────────
    empty = base / "invalid" / "empty_dcp"
    if not empty.exists():
        _write(
            empty / "ASSETMAP.xml",
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">\n'
            "  <Id>urn:uuid:00000000-0000-0000-0000-000000000001</Id>\n"
            "  <VolumeCount>1</VolumeCount>\n"
            "  <IssueDate>2024-01-01T00:00:00+00:00</IssueDate>\n"
            "  <Issuer>DcpDoctor</Issuer>\n"
            "  <Creator>DcpDoctor</Creator>\n"
            "  <AssetList/>\n"
            "</AssetMap>\n",
        )
        info(f"Created {empty.relative_to(SCRIPT_DIR)} (empty asset list)")

    info(f"Synthetic test DCPs ready in {base.relative_to(SCRIPT_DIR)}/")


# ── Main ────────────────────────────────────────────────────────────────────
def main() -> None:
    parser = argparse.ArgumentParser(
        description="Download and create test DCP content for dcpdoctor."
    )
    parser.add_argument(
        "--all", action="store_true", help="Download everything (several GB)"
    )
    parser.add_argument(
        "--isdcf",
        action="store_true",
        help="ISDCF Bv2.1 SMPTE test content (encrypted)",
    )
    parser.add_argument(
        "--framing", action="store_true", help="ISDCF framing chart info"
    )
    parser.add_argument(
        "--hfr", action="store_true", help="Fox HFR frame rate tests"
    )
    parser.add_argument(
        "--stereo3d",
        action="store_true",
        help="3D stereo subtitle tests",
    )
    parser.add_argument(
        "--synthetic",
        action="store_true",
        help="Create minimal synthetic test DCPs (no download)",
    )
    args = parser.parse_args()

    # Default to --synthetic if nothing specified
    if not any(vars(args).values()):
        create_synthetic()
        return

    if args.all or args.isdcf:
        fetch_isdcf()
    if args.all or args.framing:
        fetch_framing()
    if args.all or args.hfr:
        fetch_hfr()
    if args.all or args.stereo3d:
        fetch_stereo3d()
    if args.all or args.synthetic:
        create_synthetic()

    info("Done! Run dcpdoctor against test DCPs:")
    info("  dcpdoctor tests/dcps/synthetic/minimal/minimal_smpte_2k")
    info("  dcpdoctor tests/dcps/synthetic/invalid/bad_xml")


if __name__ == "__main__":
    main()
