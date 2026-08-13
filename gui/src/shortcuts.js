const OVERLAY_BINDING = "Ctrl+K";
const OVERLAY_LABEL = "Show keyboard shortcuts";
const OVERLAY_CATEGORY = "Help";
const CAPTURE_PROMPT = "press new shortcut";
const UNBOUND_LABEL = "unassigned";
const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta"]);
const TEXT_ENTRY_TAGS = new Set(["INPUT", "SELECT", "TEXTAREA"]);

const OVERLAY_STYLE = `
.shortcuts-overlay {
  position: fixed;
  inset: 0;
  z-index: 10000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.5);
}
.shortcuts-overlay[hidden] {
  display: none;
}
.shortcuts-dialog {
  width: min(560px, 90vw);
  max-height: 80vh;
  overflow-y: auto;
  padding: 16px 20px 20px;
  background: var(--surface, #1a1a24);
  color: var(--text, #eaeaea);
  border: 1px solid var(--border, #2a2a40);
  border-radius: var(--radius-lg, 10px);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}
.shortcuts-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.shortcuts-header h2 {
  margin: 0;
  font-size: 1rem;
}
.shortcuts-hint,
.shortcuts-warning {
  margin: 6px 0 0;
  font-size: 0.75rem;
  color: var(--text-muted, #8888a0);
}
.shortcuts-warning {
  color: #f59e0b;
}
.shortcuts-warning[hidden] {
  display: none;
}
.shortcuts-category {
  margin: 16px 0 6px;
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted, #8888a0);
}
.shortcuts-row {
  display: grid;
  grid-template-columns: 1fr auto 22px;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
}
.shortcuts-label {
  font-size: 0.85rem;
}
.shortcuts-binding {
  padding: 3px 8px;
  font-family: monospace;
  font-size: 0.78rem;
  background: var(--surface-alt, #222233);
  color: var(--text, #eaeaea);
  border: 1px solid var(--border-light, #363650);
  border-radius: var(--radius, 6px);
}
button.shortcuts-binding {
  cursor: pointer;
}
button.shortcuts-binding:hover {
  background: var(--surface-hover, #2a2a3d);
}
.shortcuts-binding.capturing {
  color: var(--accent, #60a5fa);
  border-color: var(--primary, #3b82f6);
}
.shortcuts-binding.unbound {
  color: var(--text-muted, #8888a0);
}
.shortcuts-reset,
.shortcuts-reset-all {
  background: none;
  color: var(--text-muted, #8888a0);
  border: 1px solid var(--border-light, #363650);
  border-radius: var(--radius, 6px);
  cursor: pointer;
}
.shortcuts-reset {
  width: 22px;
  height: 22px;
  padding: 0;
  font-size: 0.8rem;
}
.shortcuts-reset-all {
  padding: 4px 10px;
  font-size: 0.75rem;
}
.shortcuts-reset:hover,
.shortcuts-reset-all:hover {
  background: var(--surface-hover, #2a2a3d);
  color: var(--text, #eaeaea);
}
`;

let storageKey = "";
let actions = [];
let onChange = null;
let overrides = {};
let overlay = null;
let listElement = null;
let warningElement = null;
let capturingId = null;

export function initShortcuts(config) {
  storageKey = config.storageKey;
  actions = config.actions;
  onChange = config.onChange || null;
  overrides = loadOverrides();
  buildOverlay();
  document.addEventListener("keydown", onKeyDown);
}

export function getBinding(id) {
  const action = actions.find((candidate) => candidate.id === id);
  return action ? bindingOf(action) : null;
}

function loadOverrides() {
  let stored = null;
  try {
    stored = JSON.parse(localStorage.getItem(storageKey));
  } catch {
    return {};
  }
  if (!stored || typeof stored !== "object" || Array.isArray(stored)) return {};
  const known = {};
  for (const action of actions) {
    const binding = stored[action.id];
    if (typeof binding === "string" || binding === null) known[action.id] = binding;
  }
  return known;
}

function saveOverrides() {
  localStorage.setItem(storageKey, JSON.stringify(overrides));
}

function defaultBinding(action) {
  return action.binding || null;
}

function bindingOf(action) {
  return action.id in overrides ? overrides[action.id] : defaultBinding(action);
}

function commitChange() {
  saveOverrides();
  renderList();
  if (onChange) onChange();
}

function setBinding(action, binding) {
  if (binding === defaultBinding(action)) delete overrides[action.id];
  else overrides[action.id] = binding;
  commitChange();
}

function resetAction(action) {
  delete overrides[action.id];
  commitChange();
}

function keyName(key) {
  if (key === " ") return "Space";
  return key.length === 1 ? key.toUpperCase() : key;
}

function bindingFromEvent(event) {
  if (MODIFIER_KEYS.has(event.key)) return null;
  const parts = [];
  if (event.ctrlKey || event.metaKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  parts.push(keyName(event.key));
  return parts.join("+");
}

function isTextEntry(target) {
  return TEXT_ENTRY_TAGS.has(target.tagName) || target.isContentEditable;
}

function conflictLabel(binding, exceptId) {
  if (binding === OVERLAY_BINDING) return OVERLAY_LABEL;
  const clash = actions.find((action) => action.id !== exceptId && bindingOf(action) === binding);
  return clash ? clash.label : null;
}

function onKeyDown(event) {
  if (!overlay.hidden) {
    handleOverlayKey(event);
    return;
  }
  if (isTextEntry(event.target)) return;

  const binding = bindingFromEvent(event);
  if (!binding) return;
  if (binding === OVERLAY_BINDING) {
    event.preventDefault();
    openOverlay();
    return;
  }
  for (const action of actions) {
    if (bindingOf(action) !== binding) continue;
    if (action.when && !action.when()) continue;
    event.preventDefault();
    action.handler();
    return;
  }
}

function handleOverlayKey(event) {
  if (!capturingId) {
    if (event.key === "Escape" || bindingFromEvent(event) === OVERLAY_BINDING) {
      event.preventDefault();
      closeOverlay();
    }
    return;
  }

  event.preventDefault();
  const action = actions.find((candidate) => candidate.id === capturingId);
  if (event.key === "Escape") {
    endCapture();
    return;
  }
  if (event.key === "Backspace" || event.key === "Delete") {
    capturingId = null;
    showWarning("");
    setBinding(action, null);
    return;
  }

  const binding = bindingFromEvent(event);
  if (!binding) return;
  const conflict = conflictLabel(binding, capturingId);
  if (conflict) {
    showWarning(`${binding} is already used by ${conflict}`);
    return;
  }
  capturingId = null;
  showWarning("");
  setBinding(action, binding);
}

function startCapture(id) {
  capturingId = id;
  showWarning("");
  renderList();
}

function endCapture() {
  capturingId = null;
  showWarning("");
  renderList();
}

function showWarning(text) {
  warningElement.textContent = text;
  warningElement.hidden = !text;
}

function openOverlay() {
  renderList();
  overlay.hidden = false;
}

function closeOverlay() {
  capturingId = null;
  showWarning("");
  overlay.hidden = true;
}

function buildOverlay() {
  const style = document.createElement("style");
  style.textContent = OVERLAY_STYLE;
  document.head.append(style);

  overlay = document.createElement("div");
  overlay.className = "shortcuts-overlay";
  overlay.hidden = true;
  overlay.innerHTML = `
    <div class="shortcuts-dialog">
      <div class="shortcuts-header">
        <h2>Keyboard Shortcuts</h2>
        <button type="button" class="shortcuts-reset-all">Reset all</button>
      </div>
      <p class="shortcuts-hint">Click a shortcut to change it, Backspace clears it, Escape cancels.</p>
      <p class="shortcuts-warning" hidden></p>
      <div class="shortcuts-list"></div>
    </div>`;

  listElement = overlay.querySelector(".shortcuts-list");
  warningElement = overlay.querySelector(".shortcuts-warning");
  overlay.querySelector(".shortcuts-reset-all").addEventListener("click", () => {
    overrides = {};
    capturingId = null;
    showWarning("");
    commitChange();
  });
  overlay.addEventListener("mousedown", (event) => {
    if (event.target === overlay) closeOverlay();
  });
  document.body.append(overlay);
}

function renderList() {
  listElement.textContent = "";
  const groups = new Map();
  for (const action of actions) {
    if (!groups.has(action.category)) groups.set(action.category, []);
    groups.get(action.category).push(action);
  }
  for (const [category, members] of groups) {
    appendGroup(category, members.map(actionRow));
  }
  appendGroup(OVERLAY_CATEGORY, [overlayRow()]);
}

function appendGroup(category, rows) {
  const heading = document.createElement("div");
  heading.className = "shortcuts-category";
  heading.textContent = category;
  listElement.append(heading, ...rows);
}

function actionRow(action) {
  const row = document.createElement("div");
  row.className = "shortcuts-row";

  const label = document.createElement("span");
  label.className = "shortcuts-label";
  label.textContent = action.label;

  const capturing = capturingId === action.id;
  const binding = bindingOf(action);
  const trigger = document.createElement("button");
  trigger.type = "button";
  trigger.className = "shortcuts-binding";
  trigger.textContent = capturing ? CAPTURE_PROMPT : binding || UNBOUND_LABEL;
  if (capturing) trigger.classList.add("capturing");
  else if (!binding) trigger.classList.add("unbound");
  trigger.addEventListener("click", () => startCapture(action.id));

  row.append(label, trigger);

  if (action.id in overrides) {
    const reset = document.createElement("button");
    reset.type = "button";
    reset.className = "shortcuts-reset";
    reset.title = "Reset to default";
    reset.textContent = "↺";
    reset.addEventListener("click", () => resetAction(action));
    row.append(reset);
  }
  return row;
}

function overlayRow() {
  const row = document.createElement("div");
  row.className = "shortcuts-row";

  const label = document.createElement("span");
  label.className = "shortcuts-label";
  label.textContent = OVERLAY_LABEL;

  const binding = document.createElement("span");
  binding.className = "shortcuts-binding";
  binding.textContent = OVERLAY_BINDING;

  row.append(label, binding);
  return row;
}
