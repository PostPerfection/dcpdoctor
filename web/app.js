import init, { version } from './pkg/dcpdoctor_wasm.js';

// Initialize WASM in main thread just for version display
init().then(() => {
    document.getElementById('version').textContent = `v${version()}`;
}).catch(err => console.error('WASM init failed:', err));

// === Concurrency cap: limit active workers to physical core count ===
const MAX_WORKERS = Math.max(1, Math.floor((navigator.hardwareConcurrency || 4) / 2));
let activeWorkers = 0;

// === State ===
let queue = []; // { id, name, fileCount, status, result, error, worker, files }
let nextId = 1;
let pickInProgress = false;

// === DOM ===
const dropZone = document.getElementById('drop-zone');
const pickBtn = document.getElementById('pick-btn');
const queueSection = document.getElementById('queue-section');
const queueBody = document.getElementById('queue-body');
const addMoreBtn = document.getElementById('add-more-btn');
const tooltip = document.getElementById('detail-tooltip');

// === File input fallback ===
const fileInput = document.createElement('input');
fileInput.type = 'file';
fileInput.setAttribute('webkitdirectory', '');
fileInput.setAttribute('directory', '');
fileInput.style.display = 'none';
document.body.appendChild(fileInput);

fileInput.addEventListener('change', async () => {
    if (fileInput.files.length === 0) return;
    pickInProgress = false;
    await enqueueFromFileList(fileInput.files);
    fileInput.value = '';
});

// === Event Listeners ===

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
    if (!items || items.length === 0) return;

    // Collect directory entries synchronously (DataTransfer clears after yield)
    const entries = [];
    for (let i = 0; i < items.length; i++) {
        const entry = items[i].webkitGetAsEntry();
        if (entry && entry.isDirectory) {
            entries.push(entry);
        }
    }

    for (const entry of entries) {
        await enqueueFromEntry(entry);
    }
});

pickBtn.addEventListener('click', async (e) => {
    e.stopPropagation();
    if (pickInProgress) return;
    pickInProgress = true;
    try {
        if ('showDirectoryPicker' in window) {
            const dirHandle = await window.showDirectoryPicker();
            await enqueueFromHandle(dirHandle);
        } else {
            fileInput.click();
        }
    } catch (err) {
        if (err.name !== 'AbortError') console.error(err);
    } finally {
        pickInProgress = false;
    }
});

dropZone.addEventListener('click', (e) => {
    if (e.target !== pickBtn && !pickInProgress) pickBtn.click();
});

addMoreBtn.addEventListener('click', async () => {
    if (pickInProgress) return;
    pickInProgress = true;
    try {
        if ('showDirectoryPicker' in window) {
            const dirHandle = await window.showDirectoryPicker();
            await enqueueFromHandle(dirHandle);
        } else {
            fileInput.click();
        }
    } catch (err) {
        if (err.name !== 'AbortError') console.error(err);
    } finally {
        pickInProgress = false;
    }
});

// === Enqueue Functions ===

async function enqueueFromHandle(dirHandle) {
    const item = createQueueItem(dirHandle.name);
    showQueue();

    const files = [];
    await readDirHandle(dirHandle, '', files, item);

    item.files = files;
    item.fileCount = files.length;
    item.status = 'pending';
    renderQueue();
    drainQueue();
}

async function readDirHandle(handle, prefix, files, item) {
    for await (const [name, entry] of handle.entries()) {
        if (name.startsWith('.')) continue;
        const path = prefix ? `${prefix}/${name}` : name;
        if (entry.kind === 'file') {
            const file = await entry.getFile();
            files.push(await readFileEntry(file, path));
            item.fileCount = files.length;
            if (files.length % 10 === 0) {
                renderQueue();
                await yieldToUI();
            }
        } else if (entry.kind === 'directory') {
            await readDirHandle(entry, path, files, item);
        }
    }
}

async function enqueueFromEntry(entry) {
    const item = createQueueItem(entry.name);
    showQueue();

    const files = [];
    await readDirEntry(entry, '', files, item);

    item.files = files;
    item.fileCount = files.length;
    item.status = 'pending';
    renderQueue();
    drainQueue();
}

function readDirEntry(dirEntry, prefix, files, item) {
    return new Promise((resolve) => {
        const reader = dirEntry.createReader();
        reader.readEntries(async (entries) => {
            for (const e of entries) {
                if (e.name.startsWith('.')) continue;
                const path = prefix ? `${prefix}/${e.name}` : e.name;
                if (e.isFile) {
                    const file = await new Promise(r => e.file(r));
                    files.push(await readFileEntry(file, path));
                    item.fileCount = files.length;
                    if (files.length % 10 === 0) {
                        renderQueue();
                        await yieldToUI();
                    }
                } else if (e.isDirectory) {
                    await readDirEntry(e, path, files, item);
                }
            }
            resolve();
        });
    });
}

async function enqueueFromFileList(fileList) {
    const firstName = fileList[0]?.webkitRelativePath?.split('/')[0] || 'Package';
    const item = createQueueItem(firstName);
    showQueue();

    const files = [];
    for (let i = 0; i < fileList.length; i++) {
        const file = fileList[i];
        const parts = file.webkitRelativePath.split('/');
        const path = parts.slice(1).join('/');
        if (path.startsWith('.')) continue;
        files.push(await readFileEntry(file, path));
        item.fileCount = files.length;
        if (i % 10 === 0) {
            renderQueue();
            await yieldToUI();
        }
    }

    item.files = files;
    item.fileCount = files.length;
    item.status = 'pending';
    renderQueue();
    drainQueue();
}

// === Helpers ===

function createQueueItem(name) {
    const item = {
        id: nextId++,
        name,
        fileCount: 0,
        status: 'reading', // reading | validating | done | failed | cancelled
        result: null,
        error: null,
        worker: null,
        files: null,
    };
    queue.push(item);
    renderQueue();
    return item;
}

function isMetadataFile(path) {
    const lower = path.toLowerCase();
    return lower.endsWith('.xml') ||
        lower.endsWith('/assetmap') || lower === 'assetmap' ||
        lower.endsWith('/volindex') || lower === 'volindex';
}

async function readFileEntry(file, path) {
    if (isMetadataFile(path)) {
        const text = await file.text();
        return { path, content: text, is_base64: false, size: file.size, skipped: false };
    } else {
        return { path, content: null, is_base64: false, size: file.size, skipped: true };
    }
}

function yieldToUI() {
    return new Promise(r => setTimeout(r, 0));
}

function showQueue() {
    dropZone.classList.add('hidden');
    queueSection.classList.remove('hidden');
}

// === Validation via Web Worker (with concurrency cap) ===

function drainQueue() {
    while (activeWorkers < MAX_WORKERS) {
        const next = queue.find(i => i.status === 'pending');
        if (!next) break;
        startValidation(next);
    }
}

function startValidation(item) {
    item.status = 'validating';
    activeWorkers++;
    renderQueue();

    const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
    item.worker = worker;

    worker.onmessage = (e) => {
        const { type, result, error } = e.data;

        if (type === 'progress') {
            item.status = 'validating';
            renderQueue();
        } else if (type === 'done') {
            item.status = result.valid ? 'done' : 'failed';
            item.result = result;
            item.worker = null;
            activeWorkers--;
            worker.terminate();
            renderQueue();
            drainQueue();
        } else if (type === 'error') {
            item.status = 'failed';
            item.error = error;
            item.worker = null;
            activeWorkers--;
            worker.terminate();
            renderQueue();
            drainQueue();
        }
    };

    worker.onerror = (err) => {
        item.status = 'failed';
        item.error = err.message || 'Worker error';
        item.worker = null;
        activeWorkers--;
        renderQueue();
        drainQueue();
    };

    // Send files to worker for validation
    const filesForWasm = item.files.map(f => ({
        path: f.path,
        content: f.content,
        is_base64: f.is_base64,
        size: f.size,
        skipped: f.skipped,
    }));

    worker.postMessage({ type: 'validate', id: item.id, files: filesForWasm });
}

function cancelItem(id) {
    const item = queue.find(i => i.id === id);
    if (!item) return;
    if (item.worker) {
        item.worker.terminate();
        item.worker = null;
        activeWorkers--;
    }
    item.status = 'cancelled';
    renderQueue();
    drainQueue();
}

// === Render ===

function renderQueue() {
    queueBody.innerHTML = '';

    for (const item of queue) {
        const tr = document.createElement('tr');
        tr.dataset.id = item.id;

        // Name
        const tdName = document.createElement('td');
        tdName.textContent = item.name;
        tdName.className = 'cell-name';

        // File count
        const tdFiles = document.createElement('td');
        tdFiles.textContent = item.fileCount || '…';

        // Progress
        const tdProgress = document.createElement('td');
        tdProgress.className = 'cell-progress';
        if (item.status === 'reading' || item.status === 'validating') {
            tdProgress.innerHTML = `<div class="progress-bar-mini"><div class="progress-fill-mini indeterminate"></div></div>`;
        } else if (item.status === 'cancelled') {
            tdProgress.innerHTML = `<div class="progress-bar-mini"><div class="progress-fill-mini cancelled-bar"></div></div>`;
        } else {
            tdProgress.innerHTML = `<div class="progress-bar-mini"><div class="progress-fill-mini" style="width:100%"></div></div>`;
        }

        // Result
        const tdResult = document.createElement('td');
        tdResult.className = 'cell-result';
        if (item.status === 'done') {
            tdResult.innerHTML = `<span class="result-badge pass">✓</span>`;
        } else if (item.status === 'failed') {
            tdResult.innerHTML = `<span class="result-badge fail">✗</span>`;
        } else if (item.status === 'cancelled') {
            tdResult.innerHTML = `<span class="result-badge cancelled">—</span>`;
        } else if (item.status === 'reading') {
            tdResult.innerHTML = `<span class="result-badge reading">…</span>`;
        } else {
            tdResult.innerHTML = `<span class="result-badge working">⏳</span>`;
        }

        // Actions
        const tdAction = document.createElement('td');
        tdAction.className = 'cell-action';
        if (item.status === 'reading' || item.status === 'validating') {
            const cancelBtn = document.createElement('button');
            cancelBtn.className = 'btn-cancel-small';
            cancelBtn.textContent = 'Cancel';
            cancelBtn.onclick = (e) => { e.stopPropagation(); cancelItem(item.id); };
            tdAction.appendChild(cancelBtn);
        }

        tr.appendChild(tdName);
        tr.appendChild(tdFiles);
        tr.appendChild(tdProgress);
        tr.appendChild(tdResult);
        tr.appendChild(tdAction);

        // Hover for details
        tr.addEventListener('mouseenter', (e) => showTooltip(item, e));
        tr.addEventListener('mouseleave', hideTooltip);
        tr.addEventListener('mousemove', (e) => positionTooltip(e));

        queueBody.appendChild(tr);
    }
}

// === Tooltip ===

function showTooltip(item, e) {
    let html = `<strong>${escapeHtml(item.name)}</strong><br>`;
    html += `Files: ${item.fileCount}<br>`;
    html += `Status: ${item.status}<br>`;

    if (item.result) {
        const r = item.result;
        html += `<br><strong>${r.valid ? '✓ Valid' : '✗ Invalid'}</strong><br>`;
        html += `Errors: ${r.summary.errors} | Warnings: ${r.summary.warnings} | Info: ${r.summary.info}<br>`;
        if (r.notes && r.notes.length > 0) {
            html += `<br><strong>Issues:</strong><br>`;
            const shown = r.notes.slice(0, 8);
            for (const note of shown) {
                const icon = note.severity === 'error' ? '🔴' : note.severity === 'warning' ? '🟡' : '🔵';
                html += `${icon} ${escapeHtml(note.message)}<br>`;
            }
            if (r.notes.length > 8) {
                html += `<em>…and ${r.notes.length - 8} more</em>`;
            }
        }
    } else if (item.error) {
        html += `<br><strong>Error:</strong> ${escapeHtml(item.error)}`;
    }

    tooltip.innerHTML = html;
    tooltip.classList.remove('hidden');
    positionTooltip(e);
}

function positionTooltip(e) {
    const x = e.clientX + 15;
    const y = e.clientY + 15;
    tooltip.style.left = `${x}px`;
    tooltip.style.top = `${y}px`;

    // Keep within viewport
    const rect = tooltip.getBoundingClientRect();
    if (rect.right > window.innerWidth) {
        tooltip.style.left = `${e.clientX - rect.width - 15}px`;
    }
    if (rect.bottom > window.innerHeight) {
        tooltip.style.top = `${e.clientY - rect.height - 15}px`;
    }
}

function hideTooltip() {
    tooltip.classList.add('hidden');
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}
