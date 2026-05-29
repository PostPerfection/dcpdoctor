import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";

// ── DOM Elements ──────────────────────────────────
const dropZone = document.getElementById("drop-zone");
const folderInput = document.getElementById("folder-input");
const dcpPath = document.getElementById("dcp-path");
const btnValidate = document.getElementById("btn-validate");
const progressSection = document.getElementById("progress-section");
const progressFill = document.getElementById("progress-fill");
const progressText = document.getElementById("progress-text");
const resultsSection = document.getElementById("results-section");
const summaryCard = document.getElementById("summary-card");
const summaryIcon = document.getElementById("summary-icon");
const summaryTitle = document.getElementById("summary-title");
const summaryDetail = document.getElementById("summary-detail");
const statErrors = document.getElementById("stat-errors");
const statWarnings = document.getElementById("stat-warnings");
const statInfo = document.getElementById("stat-info");
const resultsBody = document.getElementById("results-body");
const noResults = document.getElementById("no-results");
const versionLabel = document.getElementById("version-label");

// ── Version ───────────────────────────────────────
async function loadVersion() {
  try {
    const ver = await invoke("get_version");
    versionLabel.textContent = ver;
  } catch {
    versionLabel.textContent = "";
  }
}
loadVersion();

// ── Drop Zone ─────────────────────────────────────
dropZone.addEventListener("click", async () => {
  const selected = await open({ directory: true, title: "Select DCP folder" });
  if (selected) {
    dcpPath.value = selected;
  }
});

dropZone.addEventListener("dragover", (e) => {
  e.preventDefault();
  dropZone.classList.add("drag-over");
});

dropZone.addEventListener("dragleave", () => {
  dropZone.classList.remove("drag-over");
});

// Use Tauri's native drag-drop event to get real filesystem paths
getCurrentWindow().onDragDropEvent((event) => {
  if (event.payload.type === "over") {
    dropZone.classList.add("drag-over");
  } else if (event.payload.type === "leave") {
    dropZone.classList.remove("drag-over");
  } else if (event.payload.type === "drop") {
    dropZone.classList.remove("drag-over");
    if (event.payload.paths && event.payload.paths.length > 0) {
      dcpPath.value = event.payload.paths[0];
    }
  }
});

// Prevent default browser drop behavior
dropZone.addEventListener("drop", (e) => {
  e.preventDefault();
});

folderInput.addEventListener("change", () => {
  // Unused — folder selection now uses native dialog
});

// ── Validation ────────────────────────────────────
btnValidate.addEventListener("click", () => runValidation());
dcpPath.addEventListener("keydown", (e) => {
  if (e.key === "Enter") runValidation();
});

function getSelectedFlags() {
  const checkboxes = document.querySelectorAll('.option-chip input[type="checkbox"]:checked');
  return Array.from(checkboxes).map((cb) => cb.value);
}

async function runValidation() {
  const path = dcpPath.value.trim();
  if (!path) {
    dcpPath.focus();
    dcpPath.style.borderColor = "var(--red)";
    setTimeout(() => (dcpPath.style.borderColor = ""), 1500);
    return;
  }

  const flags = getSelectedFlags();

  // Show progress
  resultsSection.classList.add("hidden");
  progressSection.classList.remove("hidden");
  btnValidate.disabled = true;
  progressFill.style.width = "30%";
  progressText.textContent = "Validating DCP...";

  try {
    progressFill.style.width = "60%";
    progressText.textContent = "Running checks...";

    const response = await invoke("validate_dcp", { path, flags });

    progressFill.style.width = "100%";
    progressText.textContent = "Complete";
    await sleep(300);

    showResults(response);
  } catch (err) {
    progressSection.classList.add("hidden");
    alert("Validation failed: " + (err.message || err));
  } finally {
    btnValidate.disabled = false;
  }
}

function showResults(response) {
  progressSection.classList.add("hidden");
  resultsSection.classList.remove("hidden");

  const { results, summary, exit_code } = response;

  const errors = results.filter((r) => r.severity === "error").length;
  const warnings = results.filter((r) => r.severity === "warning").length;
  const infos = results.filter((r) => r.severity === "info").length;

  // Summary card
  summaryCard.className = "summary-card";
  if (errors > 0) {
    summaryCard.classList.add("invalid");
    summaryIcon.textContent = "✗";
    summaryTitle.textContent = "Validation Failed";
  } else if (warnings > 0) {
    summaryCard.classList.add("warnings-only");
    summaryIcon.textContent = "⚠";
    summaryTitle.textContent = "Passed with Warnings";
  } else {
    summaryCard.classList.add("valid");
    summaryIcon.textContent = "✓";
    summaryTitle.textContent = "DCP is Valid";
  }
  summaryDetail.textContent = summary;

  // Stats
  statErrors.textContent = errors;
  statWarnings.textContent = warnings;
  statInfo.textContent = infos;

  // Table
  resultsBody.innerHTML = "";
  if (results.length === 0) {
    noResults.classList.remove("hidden");
    document.querySelector(".results-table").classList.add("hidden");
  } else {
    noResults.classList.add("hidden");
    document.querySelector(".results-table").classList.remove("hidden");

    results.forEach((r) => {
      const tr = document.createElement("tr");
      tr.dataset.severity = r.severity;

      const badgeClass =
        r.severity === "error"
          ? "badge-error"
          : r.severity === "warning"
            ? "badge-warning"
            : "badge-info";

      tr.innerHTML = `
        <td><span class="badge ${badgeClass}">${escapeHtml(r.severity)}</span></td>
        <td><span class="code-tag">${escapeHtml(r.code)}</span></td>
        <td>${escapeHtml(r.message)}</td>
        <td><span class="file-path" title="${escapeHtml(r.file)}">${escapeHtml(shortPath(r.file))}</span></td>
      `;
      resultsBody.appendChild(tr);
    });
  }
}

// ── Filter Tabs ───────────────────────────────────
document.querySelectorAll(".filter-tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".filter-tab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");

    const filter = tab.dataset.filter;
    document.querySelectorAll("#results-body tr").forEach((tr) => {
      if (filter === "all" || tr.dataset.severity === filter) {
        tr.classList.remove("hidden");
      } else {
        tr.classList.add("hidden");
      }
    });
  });
});

// ── Helpers ───────────────────────────────────────
function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

function shortPath(path) {
  if (!path) return "";
  const parts = path.replace(/\\/g, "/").split("/");
  return parts.length > 2 ? "..." + parts.slice(-2).join("/") : path;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ── Demo Results (dev mode without Tauri backend) ─
function getDemoResults() {
  return {
    results: [
      { severity: "error", code: "cpl_hash_mismatch", message: "CPL hash does not match PKL entry", file: "CPL_abc123.xml" },
      { severity: "error", code: "mxf_incomplete", message: "MXF frame count does not match CPL duration", file: "j2c_track01.mxf" },
      { severity: "warning", code: "annotation_missing", message: "AnnotationText is empty in CPL", file: "CPL_abc123.xml" },
      { severity: "warning", code: "loudness_exceeds", message: "Dialogue loudness exceeds -24 LKFS target (-21.3 LKFS)", file: "pcm_track01.mxf" },
      { severity: "info", code: "resolution_2k", message: "Detected 2K DCI resolution (2048x1080)", file: "j2c_track01.mxf" },
      { severity: "info", code: "color_xyz", message: "Color space: XYZ (CIE 1931)", file: "j2c_track01.mxf" },
    ],
    summary: "2 error(s), 2 warning(s) found.",
    exit_code: 1,
  };
}

// ── Preferences ───────────────────────────────────
const PREFS_KEY = "dcpdoctor-preferences";

function loadPreferences() {
  try {
    const saved = JSON.parse(localStorage.getItem(PREFS_KEY));
    if (!saved) return;
    if (saved.standard) document.getElementById("pref-standard").value = saved.standard;
    if (saved.hashes !== undefined) document.getElementById("pref-hashes").checked = saved.hashes;
    if (saved.schemas !== undefined) document.getElementById("pref-schemas").checked = saved.schemas;
    if (saved.bitrate !== undefined) document.getElementById("pref-bitrate").checked = saved.bitrate;
    if (saved.loudness !== undefined) document.getElementById("pref-loudness").checked = saved.loudness;
    if (saved.maxBitrate) document.getElementById("pref-max-bitrate").value = saved.maxBitrate;
    if (saved.reportFormat) document.getElementById("pref-report-format").value = saved.reportFormat;
    if (saved.outputDir) document.getElementById("pref-output-dir").value = saved.outputDir;
    if (saved.schemaDir) document.getElementById("pref-schema-dir").value = saved.schemaDir;
  } catch { /* ignore */ }
}

function savePreferences() {
  const prefs = {
    standard: document.getElementById("pref-standard").value,
    hashes: document.getElementById("pref-hashes").checked,
    schemas: document.getElementById("pref-schemas").checked,
    bitrate: document.getElementById("pref-bitrate").checked,
    loudness: document.getElementById("pref-loudness").checked,
    maxBitrate: document.getElementById("pref-max-bitrate").value,
    reportFormat: document.getElementById("pref-report-format").value,
    outputDir: document.getElementById("pref-output-dir").value,
    schemaDir: document.getElementById("pref-schema-dir").value,
  };
  localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
}

document.getElementById("prefs-toggle")?.addEventListener("click", () => {
  const section = document.getElementById("preferences-section");
  section.classList.toggle("hidden");
});

document.getElementById("preferences-form")?.addEventListener("submit", (e) => {
  e.preventDefault();
  savePreferences();
  document.getElementById("preferences-section").classList.add("hidden");
});

document.getElementById("pref-reset")?.addEventListener("click", () => {
  localStorage.removeItem(PREFS_KEY);
  document.getElementById("pref-standard").value = "SMPTE";
  document.getElementById("pref-hashes").checked = true;
  document.getElementById("pref-schemas").checked = true;
  document.getElementById("pref-bitrate").checked = true;
  document.getElementById("pref-loudness").checked = true;
  document.getElementById("pref-max-bitrate").value = "250";
  document.getElementById("pref-report-format").value = "html";
  document.getElementById("pref-output-dir").value = "";
  document.getElementById("pref-schema-dir").value = "";
});

loadPreferences();
