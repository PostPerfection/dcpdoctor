import init, { validate_dcp, version, Sha1Hasher } from './pkg/dcpdoctor_wasm.js';

let wasmReady = false;

async function initWasm() {
    await init();
    wasmReady = true;
    document.getElementById('version').textContent = `v${version()}`;
}

initWasm().catch(err => {
    console.error('Failed to load WASM:', err);
});

// DOM elements
const dropZone = document.getElementById('drop-zone');
const pickBtn = document.getElementById('pick-btn');
const progress = document.getElementById('progress');
const progressFill = document.getElementById('progress-fill');
const progressText = document.getElementById('progress-text');
const cancelBtn = document.getElementById('cancel-btn');
const results = document.getElementById('results');
const resultHeader = document.getElementById('result-header');
const resultIcon = document.getElementById('result-icon');
const resultTitle = document.getElementById('result-title');
const summary = document.getElementById('summary');
const notesList = document.getElementById('notes-list');

// DCP Queue DOM
const dcpQueueSection = document.getElementById('dcp-queue');
const dcpQueueBody = document.getElementById('dcp-queue-body');
const dcpAddMore = document.getElementById('dcp-add-more');

// Hash queue DOM
const hashQueueSection = document.getElementById('hash-queue');
const hashQueueList = document.getElementById('hash-queue-list');
const hashStartBtn = document.getElementById('hash-start');
const hashCancelBtn = document.getElementById('hash-cancel');
const hashSelectAll = document.getElementById('hash-select-all');
const hashClear = document.getElementById('hash-clear');
const hashStatus = document.getElementById('hash-status');
const hashOverallProgress = document.getElementById('hash-overall-progress');
const hashProgressFill = document.getElementById('hash-progress-fill');
const hashProgressText = document.getElementById('hash-progress-text');

// === DCP Queue State ===
let dcpQueue = []; // { id, name, fileCount, status, files, result }
let selectedDcpId = null;
let validationAbort = null; // AbortController for current validation
let nextDcpId = 1;
let pickInProgress = false; // Guard against multiple picker dialogs

// Hash queue state
let hashQueue = [];
let hashAbort = null;

cancelBtn.addEventListener('click', () => {
    if (validationAbort) {
        validationAbort.abort();
    }
    progress.classList.add('hidden');
    dropZone.classList.remove('hidden');
});

// Drag and drop
dropZone.addEventListener('dragover', (e) => {
    e.preventDefault();
    dropZone.classList.add('dragover');
});

dropZone.addEventListener('dragleave', () => {
    dropZone.classList.remove('dragover');
});

dropZone.addEventListener('drop', async (e) => {
    e.preventDefault();
    dropZone.classList.remove('dragover');
    const items = e.dataTransfer.items;
    if (items.length > 0) {
        // Collect entries synchronously before any await (DataTransfer clears after yield)
        const entries = [];
        for (let i = 0; i < items.length; i++) {
            const entry = items[i].webkitGetAsEntry();
            if (entry && entry.isDirectory) {
                entries.push({ type: 'entry', entry });
            }
        }
        // Now process asynchronously
        for (const { entry } of entries) {
            await enqueueDcpFromEntry(entry);
        }
    }
});

// Hidden file input fallback
const fileInput = document.createElement('input');
fileInput.type = 'file';
fileInput.setAttribute('webkitdirectory', '');
fileInput.setAttribute('directory', '');
fileInput.style.display = 'none';
document.body.appendChild(fileInput);

fileInput.addEventListener('change', async () => {
    if (fileInput.files.length === 0) return;
    pickInProgress = false; // The dialog has resolved
    await enqueueDcpFromFileList(fileInput.files);
    fileInput.value = '';
});

// File picker button
pickBtn.addEventListener('click', async (e) => {
    e.stopPropagation();
    if (pickInProgress) return;
    pickInProgress = true;
    try {
        if ('showDirectoryPicker' in window) {
            const dirHandle = await window.showDirectoryPicker();
            await enqueueDcpFromHandle(dirHandle);
        } else {
            fileInput.click();
        }
    } catch (err) {
        if (err.name !== 'AbortError') console.error('Directory picker error:', err);
    } finally {
        pickInProgress = false;
    }
});

dropZone.addEventListener('click', (e) => {
    if (e.target !== pickBtn && !pickInProgress) pickBtn.click();
});

// "Add DCP" button in queue header
dcpAddMore.addEventListener('click', async () => {
    if (pickInProgress) return;
    pickInProgress = true;
    try {
        if ('showDirectoryPicker' in window) {
            const dirHandle = await window.showDirectoryPicker();
            await enqueueDcpFromHandle(dirHandle);
        } else {
            fileInput.click();
        }
    } catch (err) {
        if (err.name !== 'AbortError') console.error('Directory picker error:', err);
    } finally {
        pickInProgress = false;
    }
});

// === Enqueue DCP from various sources ===

async function enqueueDcpFromHandle(dirHandle) {
    const dcpId = nextDcpId++;
    const item = { id: dcpId, name: dirHandle.name, fileCount: 0, status: 'reading', files: [], result: null };
    dcpQueue.push(item);
    showQueueUI();
    renderDcpQueue();

    const files = [];
    async function readDir(handle, prefix = '') {
        for await (const [name, entry] of handle.entries()) {
            if (name.startsWith('.')) continue;
            const path = prefix ? `${prefix}/${name}` : name;
            if (entry.kind === 'file') {
                const file = await entry.getFile();
                files.push(await readFileContent(file, path));
                // Yield to event loop so UI stays responsive
                await new Promise(r => setTimeout(r, 0));
            } else if (entry.kind === 'directory') {
                await readDir(entry, path);
            }
        }
    }

    await readDir(dirHandle);
    item.files = files;
    item.fileCount = files.length;
    item.status = 'pending';
    renderDcpQueue();
    processNextDcp();
}

async function enqueueDcpFromEntry(entry) {
    const dcpId = nextDcpId++;
    const item = { id: dcpId, name: entry.name, fileCount: 0, status: 'reading', files: [], result: null };
    dcpQueue.push(item);
    showQueueUI();
    renderDcpQueue();

    const files = [];
    async function readDir(dirEntry, prefix = '') {
        return new Promise((resolve) => {
            const reader = dirEntry.createReader();
            reader.readEntries(async (entries) => {
                for (const e of entries) {
                    if (e.name.startsWith('.')) continue;
                    const path = prefix ? `${prefix}/${e.name}` : e.name;
                    if (e.isFile) {
                        const file = await new Promise(r => e.file(r));
                        files.push(await readFileContent(file, path));
                        // Yield to event loop so UI stays responsive
                        await new Promise(r => setTimeout(r, 0));
                    } else if (e.isDirectory) {
                        await readDir(e, path);
                    }
                }
                resolve();
            });
        });
    }

    await readDir(entry);
    item.files = files;
    item.fileCount = files.length;
    item.status = 'pending';
    renderDcpQueue();
    processNextDcp();
}

async function enqueueDcpFromFileList(fileList) {
    const firstName = fileList[0]?.webkitRelativePath?.split('/')[0] || 'DCP';
    const dcpId = nextDcpId++;
    const item = { id: dcpId, name: firstName, fileCount: 0, status: 'reading', files: [], result: null };
    dcpQueue.push(item);
    showQueueUI();
    renderDcpQueue();

    const files = [];
    for (let i = 0; i < fileList.length; i++) {
        const file = fileList[i];
        const parts = file.webkitRelativePath.split('/');
        const path = parts.slice(1).join('/');
        if (path.startsWith('.')) continue;
        files.push(await readFileContent(file, path));
        // Yield to event loop every file so UI stays responsive
        if (i % 5 === 0) await new Promise(r => setTimeout(r, 0));
    }

    item.files = files;
    item.fileCount = files.length;
    item.status = 'pending';
    renderDcpQueue();
    processNextDcp();
}

// === DCP Queue Processing ===

let processingDcpId = null;

async function processNextDcp() {
    if (processingDcpId !== null) return; // Already running

    const next = dcpQueue.find(d => d.status === 'pending');
    if (!next) return;

    processingDcpId = next.id;
    next.status = 'validating';
    renderDcpQueue();

    validationAbort = new AbortController();

    if (!wasmReady) await initWasm();

    // Give UI a chance to update
    await new Promise(r => setTimeout(r, 10));

    if (validationAbort.signal.aborted) {
        next.status = 'pending';
        processingDcpId = null;
        validationAbort = null;
        renderDcpQueue();
        processNextDcp();
        return;
    }

    const filesForWasm = next.files.map(f => ({
        path: f.path, content: f.content, is_base64: f.is_base64, size: f.size, skipped: f.skipped
    }));
    const filesJson = JSON.stringify(filesForWasm);
    const resultJson = validate_dcp(filesJson);
    const result = JSON.parse(resultJson);

    if (validationAbort.signal.aborted) {
        next.status = 'pending';
        processingDcpId = null;
        validationAbort = null;
        renderDcpQueue();
        processNextDcp();
        return;
    }

    next.result = result;
    next.status = result.valid ? 'done' : 'error';
    processingDcpId = null;
    validationAbort = null;
    renderDcpQueue();

    // Auto-select the first completed DCP if nothing is selected
    if (!selectedDcpId) {
        selectDcp(next.id);
    }

    // Continue processing
    processNextDcp();
}

function interruptValidation() {
    if (validationAbort) {
        validationAbort.abort();
        const current = dcpQueue.find(d => d.id === processingDcpId);
        if (current) current.status = 'pending';
        processingDcpId = null;
        validationAbort = null;
        renderDcpQueue();
        // Re-process from new priority order
        processNextDcp();
    }
}

// === DCP Queue UI ===

function showQueueUI() {
    dropZone.classList.add('hidden');
    progress.classList.add('hidden');
    dcpQueueSection.classList.remove('hidden');
}

function renderDcpQueue() {
    dcpQueueBody.innerHTML = '';
    dcpQueue.forEach((item, idx) => {
        const tr = document.createElement('tr');
        tr.className = item.id === selectedDcpId ? 'active' : '';
        tr.dataset.id = item.id;

        const statusClass = item.status === 'done' ? 'done' :
                           item.status === 'error' ? 'error' :
                           item.status === 'validating' ? 'validating' : 'pending';
        const statusText = item.status === 'reading' ? 'Reading…' :
                          item.status === 'pending' ? 'Pending' :
                          item.status === 'validating' ? 'Validating…' :
                          item.status === 'done' ? '✓ Valid' :
                          item.status === 'error' ? '✗ Issues' : item.status;

        const canMove = item.status === 'pending';
        tr.innerHTML = `
            <td><strong>${escapeHtml(item.name)}</strong></td>
            <td>${item.fileCount || '…'}</td>
            <td><span class="status-badge ${statusClass}">${statusText}</span></td>
            <td class="priority-cell">
                ${canMove ? `
                    <button class="move-btn" data-idx="${idx}" data-dir="up" ${idx === 0 ? 'disabled' : ''}>▲</button>
                    <button class="move-btn" data-idx="${idx}" data-dir="down" ${idx === dcpQueue.length - 1 ? 'disabled' : ''}>▼</button>
                ` : ''}
            </td>
        `;

        // Click row to view results
        tr.addEventListener('click', (e) => {
            if (e.target.classList.contains('move-btn')) return;
            selectDcp(item.id);
        });

        dcpQueueBody.appendChild(tr);
    });

    // Priority move handlers
    dcpQueueBody.querySelectorAll('.move-btn').forEach(btn => {
        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            const idx = parseInt(e.target.dataset.idx);
            const dir = e.target.dataset.dir;
            if (dir === 'up' && idx > 0) {
                [dcpQueue[idx - 1], dcpQueue[idx]] = [dcpQueue[idx], dcpQueue[idx - 1]];
                interruptValidation();
            } else if (dir === 'down' && idx < dcpQueue.length - 1) {
                [dcpQueue[idx], dcpQueue[idx + 1]] = [dcpQueue[idx + 1], dcpQueue[idx]];
                interruptValidation();
            }
            renderDcpQueue();
        });
    });
}

function selectDcp(id) {
    const item = dcpQueue.find(d => d.id === id);
    if (!item) return;
    selectedDcpId = id;
    renderDcpQueue();

    if (item.result) {
        showResults(item.result, item.files);
    } else {
        results.classList.add('hidden');
        hashQueueSection.classList.add('hidden');
    }
}

// Determine if a file needs its content read (XML metadata) or just path/size (binary essence)
function isMetadataFile(path) {
    const lower = path.toLowerCase();
    return lower.endsWith('.xml') || 
           lower.endsWith('/assetmap') || lower === 'assetmap' ||
           lower.endsWith('/volindex') || lower === 'volindex';
}

async function readFileContent(file, path) {
    if (isMetadataFile(path)) {
        const text = await file.text();
        return { path, content: text, is_base64: false, size: file.size, skipped: false, file: null };
    } else {
        return { path, content: null, is_base64: false, size: file.size, skipped: true, file };
    }
}

function showResults(result, files) {
    results.classList.remove('hidden');

    // Header
    resultHeader.className = `result-header ${result.valid ? 'pass' : 'fail'}`;
    resultIcon.textContent = result.valid ? '✅' : '❌';
    resultTitle.textContent = result.valid ? 'DCP is valid' : 'Issues found';

    // Summary cards
    summary.innerHTML = `
        <div class="summary-card errors">
            <div class="value">${result.summary.errors}</div>
            <div class="label">Errors</div>
        </div>
        <div class="summary-card warnings">
            <div class="value">${result.summary.warnings}</div>
            <div class="label">Warnings</div>
        </div>
        <div class="summary-card info">
            <div class="value">${result.summary.info}</div>
            <div class="label">Info</div>
        </div>
        <div class="summary-card">
            <div class="value">${result.summary.files_checked}</div>
            <div class="label">Files</div>
        </div>
        <div class="summary-card hashes">
            <div class="value">${result.summary.hashes_verified}</div>
            <div class="label">Hashes OK</div>
        </div>
        ${result.summary.hashes_failed > 0 ? `
        <div class="summary-card errors">
            <div class="value">${result.summary.hashes_failed}</div>
            <div class="label">Hashes Failed</div>
        </div>` : ''}
        ${result.summary.hashes_skipped > 0 ? `
        <div class="summary-card">
            <div class="value">${result.summary.hashes_skipped}</div>
            <div class="label">Hashes Skipped</div>
        </div>` : ''}
    `;

    // Standard badge
    const standardBadge = result.standard !== 'unknown' ? 
        `<div class="summary-card"><div class="value" style="font-size:1rem">${result.standard.toUpperCase()}</div><div class="label">Standard</div></div>` : '';
    summary.innerHTML += standardBadge;

    // Notes
    notesList.innerHTML = '';
    if (result.notes.length === 0) {
        notesList.innerHTML = '<p style="color: var(--text-muted); text-align: center; padding: 2rem;">No issues found 🎉</p>';
    } else {
        for (const note of result.notes) {
            const el = document.createElement('div');
            el.className = 'note';
            el.innerHTML = `
                <span class="badge ${note.severity}">${note.severity}</span>
                <div>
                    <div class="message">${escapeHtml(note.message)}</div>
                    ${note.file ? `<div class="file">${escapeHtml(note.file)}</div>` : ''}
                </div>
            `;
            notesList.appendChild(el);
        }
    }

    // Show "validate another" button
    const btn = document.createElement('button');
    btn.className = 'pick-btn';
    btn.style.marginTop = '2rem';
    btn.textContent = 'Validate another DCP';
    btn.onclick = () => {
        if ('showDirectoryPicker' in window) {
            window.showDirectoryPicker().then(h => enqueueDcpFromHandle(h)).catch(() => {});
        } else {
            fileInput.click();
        }
    };
    notesList.appendChild(btn);

    // Populate hash queue for the selected DCP
    if (files) {
        populateHashQueue(files, result);
    }
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

// === Hash Queue ===

function populateHashQueue(files, result) {
    // Extract expected hashes from validation result
    const expectedHashes = result.asset_hashes || {};

    // Build queue from binary (skipped) files that have File references
    hashQueue = files
        .filter(f => f.skipped && f.file)
        .map(f => ({
            path: f.path,
            size: f.size,
            file: f.file,
            expectedHash: expectedHashes[f.path] || null,
            selected: true,
            status: 'pending', // pending | hashing | pass | fail | skipped
            progress: 0
        }));

    if (hashQueue.length === 0) return;

    hashQueueSection.classList.remove('hidden');
    renderHashQueue();
    updateHashControls();
}

function renderHashQueue() {
    hashQueueList.innerHTML = '';
    const isRunning = hashAbort !== null;
    hashQueue.forEach((item, idx) => {
        const el = document.createElement('div');
        el.className = 'hash-item';
        el.dataset.idx = idx;
        const canReorder = !isRunning && item.status === 'pending';
        el.innerHTML = `
            <input type="checkbox" ${item.selected ? 'checked' : ''} data-idx="${idx}" class="hash-cb" ${isRunning ? 'disabled' : ''}>
            <span class="hash-item-name" title="${escapeHtml(item.path)}">${escapeHtml(item.path)}</span>
            <span class="hash-item-size">${formatSize(item.size)}</span>
            ${canReorder ? `<span class="priority-btns">
                <button data-idx="${idx}" data-dir="up" class="move-btn" ${idx === 0 ? 'disabled' : ''}>▲</button>
                <button data-idx="${idx}" data-dir="down" class="move-btn" ${idx === hashQueue.length - 1 ? 'disabled' : ''}>▼</button>
            </span>` : '<span></span>'}
            <span class="hash-item-status ${item.status}">${statusLabel(item.status)}</span>
            ${item.status === 'hashing' ? `
            <div class="hash-item-progress">
                <div class="hash-item-progress-fill" style="width:${item.progress}%"></div>
            </div>` : ''}
        `;
        hashQueueList.appendChild(el);
    });

    // Checkbox handlers
    hashQueueList.querySelectorAll('.hash-cb').forEach(cb => {
        cb.addEventListener('change', (e) => {
            const idx = parseInt(e.target.dataset.idx);
            hashQueue[idx].selected = e.target.checked;
            updateHashControls();
        });
    });

    // Priority move handlers
    hashQueueList.querySelectorAll('.move-btn').forEach(btn => {
        btn.addEventListener('click', (e) => {
            const idx = parseInt(e.target.dataset.idx);
            const dir = e.target.dataset.dir;
            if (dir === 'up' && idx > 0) {
                [hashQueue[idx - 1], hashQueue[idx]] = [hashQueue[idx], hashQueue[idx - 1]];
            } else if (dir === 'down' && idx < hashQueue.length - 1) {
                [hashQueue[idx], hashQueue[idx + 1]] = [hashQueue[idx + 1], hashQueue[idx]];
            }
            renderHashQueue();
        });
    });
}

function formatSize(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1048576) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1073741824) return (bytes / 1048576).toFixed(1) + ' MB';
    return (bytes / 1073741824).toFixed(2) + ' GB';
}

function statusLabel(status) {
    switch (status) {
        case 'pending': return 'Pending';
        case 'hashing': return 'Hashing…';
        case 'pass': return '✓ Match';
        case 'fail': return '✗ Mismatch';
        case 'skipped': return 'Skipped';
        default: return status;
    }
}

function updateHashControls() {
    const selectedCount = hashQueue.filter(i => i.selected).length;
    const isRunning = hashAbort !== null;
    hashStartBtn.disabled = isRunning || selectedCount === 0;
    hashStartBtn.textContent = isRunning ? 'Running…' : 'Verify Selected';
    hashStatus.textContent = isRunning
        ? 'Verifying...'
        : `${selectedCount}/${hashQueue.length} selected`;
}

hashSelectAll.addEventListener('click', () => {
    hashQueue.forEach(i => i.selected = true);
    renderHashQueue();
    updateHashControls();
});

hashClear.addEventListener('click', () => {
    hashQueue.forEach(i => i.selected = false);
    renderHashQueue();
    updateHashControls();
});

hashStartBtn.addEventListener('click', () => {
    runHashVerification();
});

hashCancelBtn.addEventListener('click', () => {
    if (hashAbort) {
        hashAbort.abort();
    }
});

async function runHashVerification() {
    hashAbort = new AbortController();
    hashStartBtn.disabled = true;
    hashCancelBtn.classList.remove('hidden');
    hashOverallProgress.classList.remove('hidden');
    updateHashControls();
    renderHashQueue();

    const selected = hashQueue.filter(i => i.selected && i.status === 'pending');
    const totalBytes = selected.reduce((sum, i) => sum + i.size, 0);
    let doneBytes = 0;
    let passCount = 0;
    let failCount = 0;

    for (const item of selected) {
        if (hashAbort.signal.aborted) break;

        item.status = 'hashing';
        item.progress = 0;
        renderHashQueue();

        const computed = await hashFileStreaming(item.file, hashAbort.signal, (bytesDone) => {
            item.progress = Math.round((bytesDone / item.size) * 100);
            const overallPct = Math.round(((doneBytes + bytesDone) / totalBytes) * 100);
            hashProgressFill.style.width = `${overallPct}%`;
            hashProgressText.textContent = `Hashing ${item.path} (${item.progress}%)`;
            // Update just the progress bar in-place
            const el = hashQueueList.querySelector(`[data-idx="${hashQueue.indexOf(item)}"] .hash-item-progress-fill`);
            if (el) el.style.width = `${item.progress}%`;
        });

        if (hashAbort.signal.aborted) {
            // Reset current item back to pending
            item.status = 'pending';
            item.progress = 0;
            break;
        }

        doneBytes += item.size;

        if (item.expectedHash) {
            item.status = (computed === item.expectedHash) ? 'pass' : 'fail';
            if (item.status === 'pass') passCount++;
            else failCount++;
        } else {
            item.status = 'pass';
            passCount++;
        }
        item.progress = 100;
        renderHashQueue();
    }

    const cancelled = hashAbort.signal.aborted;
    hashAbort = null;
    hashCancelBtn.classList.add('hidden');
    renderHashQueue();
    updateHashControls();

    if (cancelled) {
        hashProgressText.textContent = `Cancelled — ${passCount} passed, ${failCount} failed`;
    } else {
        hashProgressFill.style.width = '100%';
        hashProgressText.textContent = `Done — ${passCount} passed, ${failCount} failed`;
    }
}

// === Streaming SHA-1 via WASM ===

const CHUNK_SIZE = 1048576; // 1 MB

async function hashFileStreaming(file, signal, onProgress) {
    const hasher = new Sha1Hasher();
    let offset = 0;

    while (offset < file.size) {
        if (signal.aborted) return null;
        const end = Math.min(offset + CHUNK_SIZE, file.size);
        const slice = file.slice(offset, end);
        const buf = await slice.arrayBuffer();
        hasher.update(new Uint8Array(buf));
        offset = end;
        onProgress(offset);
        // Yield to UI every chunk
        await new Promise(r => setTimeout(r, 0));
    }

    if (signal.aborted) return null;
    return hasher.finalize();
}
