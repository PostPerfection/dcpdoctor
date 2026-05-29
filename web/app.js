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

// Abort controller for cancelling in-progress validation
let abortController = null;

// Hash queue state
let hashQueue = []; // { path, size, file (File obj), expectedHash, selected, status, progress }
let hashAbort = null;

cancelBtn.addEventListener('click', () => {
    if (abortController) {
        abortController.abort();
    }
    resetToDropZone();
});

function resetToDropZone() {
    abortController = null;
    progress.classList.add('hidden');
    results.classList.add('hidden');
    hashQueueSection.classList.add('hidden');
    dropZone.classList.remove('hidden');
    progressFill.style.width = '0%';
    progressText.textContent = 'Reading files...';
    hashQueue = [];
}

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
        const entry = items[0].webkitGetAsEntry();
        if (entry && entry.isDirectory) {
            await processDirectory(entry);
        } else {
            // Try using getAsFileSystemHandle for modern browsers
            if (items[0].getAsFileSystemHandle) {
                const handle = await items[0].getAsFileSystemHandle();
                if (handle.kind === 'directory') {
                    await processDirectoryHandle(handle);
                }
            }
        }
    }
});

// Hidden file input fallback for browsers without showDirectoryPicker (Brave, Firefox, Safari)
const fileInput = document.createElement('input');
fileInput.type = 'file';
fileInput.setAttribute('webkitdirectory', '');
fileInput.setAttribute('directory', '');
fileInput.style.display = 'none';
document.body.appendChild(fileInput);

fileInput.addEventListener('change', async () => {
    if (fileInput.files.length === 0) return;
    await processFileList(fileInput.files);
    fileInput.value = '';
});

// File picker button
pickBtn.addEventListener('click', async (e) => {
    e.stopPropagation();
    if ('showDirectoryPicker' in window) {
        try {
            const dirHandle = await window.showDirectoryPicker();
            await processDirectoryHandle(dirHandle);
        } catch (err) {
            if (err.name !== 'AbortError') {
                console.error('Directory picker error:', err);
            }
        }
    } else {
        // Fallback: use hidden file input with webkitdirectory
        fileInput.click();
    }
});

// Click on drop zone also opens picker
dropZone.addEventListener('click', (e) => {
    if (e.target !== pickBtn) {
        pickBtn.click();
    }
});

// Process a FileList from <input webkitdirectory> (Brave/Firefox/Safari fallback)
async function processFileList(fileList) {
    abortController = new AbortController();
    showProgress();
    const files = [];

    for (let i = 0; i < fileList.length; i++) {
        if (abortController.signal.aborted) return;
        const file = fileList[i];
        // webkitRelativePath gives "DirName/subdir/file.xml"
        const relPath = file.webkitRelativePath;
        // Strip the top-level directory name to get paths relative to DCP root
        const parts = relPath.split('/');
        const path = parts.slice(1).join('/');
        if (path.startsWith('.')) continue;
        updateProgress(`Reading ${path}... (${i + 1}/${fileList.length})`, (i / fileList.length) * 80);
        // Yield to event loop every file so UI stays responsive
        await new Promise(r => setTimeout(r, 0));
        const content = await readFileContent(file, path);
        files.push(content);
    }

    if (!abortController.signal.aborted) {
        await runValidation(files);
    }
}

// Process a FileSystemDirectoryHandle (modern API)
async function processDirectoryHandle(dirHandle) {
    abortController = new AbortController();
    showProgress();
    const files = [];
    let fileCount = 0;

    async function readDir(handle, prefix = '') {
        for await (const [name, entry] of handle.entries()) {
            if (abortController.signal.aborted) return;
            if (name.startsWith('.')) continue;
            const path = prefix ? `${prefix}/${name}` : name;
            if (entry.kind === 'file') {
                fileCount++;
                updateProgress(`Reading ${path}...`, -1);
                // Yield to event loop so UI stays responsive
                await new Promise(r => setTimeout(r, 0));
                const file = await entry.getFile();
                const content = await readFileContent(file, path);
                files.push(content);
            } else if (entry.kind === 'directory') {
                await readDir(entry, path);
            }
        }
    }

    await readDir(dirHandle);
    if (!abortController.signal.aborted) {
        await runValidation(files);
    }
}

// Process a webkitGetAsEntry directory (fallback)
async function processDirectory(entry) {
    abortController = new AbortController();
    showProgress();
    const files = [];

    async function readDir(dirEntry, prefix = '') {
        return new Promise((resolve) => {
            const reader = dirEntry.createReader();
            reader.readEntries(async (entries) => {
                for (const entry of entries) {
                    if (abortController.signal.aborted) break;
                    if (entry.name.startsWith('.')) continue;
                    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
                    if (entry.isFile) {
                        updateProgress(`Reading ${path}...`, -1);
                        const file = await getFile(entry);
                        const content = await readFileContent(file, path);
                        files.push(content);
                    } else if (entry.isDirectory) {
                        await readDir(entry, path);
                    }
                }
                resolve();
            });
        });
    }

    await readDir(entry);
    if (!abortController.signal.aborted) {
        await runValidation(files);
    }
}

function getFile(fileEntry) {
    return new Promise((resolve) => fileEntry.file(resolve));
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
        // Binary essence file — record path/size, keep File reference for hash queue
        return { path, content: null, is_base64: false, size: file.size, skipped: true, file };
    }
}

async function runValidation(files) {
    if (!wasmReady) {
        await initWasm();
    }

    updateProgress('Validating DCP...', 50);
    
    // Give UI a chance to update
    await new Promise(r => setTimeout(r, 10));

    // Strip File references before sending to WASM (not serializable)
    const filesForWasm = files.map(f => ({
        path: f.path, content: f.content, is_base64: f.is_base64, size: f.size, skipped: f.skipped
    }));
    const filesJson = JSON.stringify(filesForWasm);
    const resultJson = validate_dcp(filesJson);
    const result = JSON.parse(resultJson);

    updateProgress('Done!', 100);
    setTimeout(() => {
        showResults(result);
        // Populate hash queue from binary files that have expected hashes
        populateHashQueue(files, result);
    }, 300);
}

function showProgress() {
    dropZone.classList.add('hidden');
    progress.classList.remove('hidden');
    results.classList.add('hidden');
}

function updateProgress(text, percent) {
    progressText.textContent = text;
    if (percent >= 0) {
        progressFill.style.width = `${percent}%`;
    }
}

function showResults(result) {
    progress.classList.add('hidden');
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
        results.classList.add('hidden');
        hashQueueSection.classList.add('hidden');
        dropZone.classList.remove('hidden');
    };
    notesList.appendChild(btn);
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
