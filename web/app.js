import init, { validate_dcp, version } from './pkg/dcpdoctor_wasm.js';

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
const results = document.getElementById('results');
const resultHeader = document.getElementById('result-header');
const resultIcon = document.getElementById('result-icon');
const resultTitle = document.getElementById('result-title');
const summary = document.getElementById('summary');
const notesList = document.getElementById('notes-list');

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
        alert('Your browser does not support the Directory Picker API. Please use Chrome, Edge, or Opera.');
    }
});

// Click on drop zone also opens picker
dropZone.addEventListener('click', (e) => {
    if (e.target !== pickBtn) {
        pickBtn.click();
    }
});

// Process a FileSystemDirectoryHandle (modern API)
async function processDirectoryHandle(dirHandle) {
    showProgress();
    const files = [];
    let fileCount = 0;

    async function readDir(handle, prefix = '') {
        for await (const [name, entry] of handle.entries()) {
            if (name.startsWith('.')) continue;
            const path = prefix ? `${prefix}/${name}` : name;
            if (entry.kind === 'file') {
                fileCount++;
                updateProgress(`Reading ${path}...`, -1);
                const file = await entry.getFile();
                const content = await readFileContent(file, path);
                files.push(content);
            } else if (entry.kind === 'directory') {
                await readDir(entry, path);
            }
        }
    }

    await readDir(dirHandle);
    await runValidation(files);
}

// Process a webkitGetAsEntry directory (fallback)
async function processDirectory(entry) {
    showProgress();
    const files = [];

    async function readDir(dirEntry, prefix = '') {
        return new Promise((resolve) => {
            const reader = dirEntry.createReader();
            reader.readEntries(async (entries) => {
                for (const entry of entries) {
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
    await runValidation(files);
}

function getFile(fileEntry) {
    return new Promise((resolve) => fileEntry.file(resolve));
}

async function readFileContent(file, path) {
    const isXml = path.toLowerCase().endsWith('.xml') || 
                  path === 'ASSETMAP' || path === 'VOLINDEX';
    
    if (isXml || file.size < 10 * 1024 * 1024) {
        // Read as text for XML, as base64 for binary (if small enough for hashing)
        if (isXml) {
            const text = await file.text();
            return { path, content: text, is_base64: false, size: file.size };
        } else {
            const buffer = await file.arrayBuffer();
            const base64 = arrayBufferToBase64(buffer);
            return { path, content: base64, is_base64: true, size: file.size };
        }
    } else {
        // Large binary files: still read for hashing but in chunks
        const buffer = await file.arrayBuffer();
        const base64 = arrayBufferToBase64(buffer);
        return { path, content: base64, is_base64: true, size: file.size };
    }
}

function arrayBufferToBase64(buffer) {
    const bytes = new Uint8Array(buffer);
    let binary = '';
    const chunkSize = 8192;
    for (let i = 0; i < bytes.length; i += chunkSize) {
        const chunk = bytes.subarray(i, i + chunkSize);
        binary += String.fromCharCode.apply(null, chunk);
    }
    return btoa(binary);
}

async function runValidation(files) {
    if (!wasmReady) {
        await initWasm();
    }

    updateProgress('Validating DCP...', 50);
    
    // Give UI a chance to update
    await new Promise(r => setTimeout(r, 10));

    const filesJson = JSON.stringify(files);
    const resultJson = validate_dcp(filesJson);
    const result = JSON.parse(resultJson);

    updateProgress('Done!', 100);
    setTimeout(() => showResults(result), 300);
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
        dropZone.classList.remove('hidden');
    };
    notesList.appendChild(btn);
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}
