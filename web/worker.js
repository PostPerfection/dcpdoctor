// Web Worker for DCP/IMP validation
// Each worker instance validates one package independently
// Phase 1: Structural validation (metadata only)
// Phase 2: Streaming hash verification of binary files
// Phase 3: MXF header inspection (picture/sound descriptor validation)

import init, { validate_dcp, Sha1Hasher, validate_mxf_file, validate_imf_supplemental } from './pkg/dcpdoctor_wasm.js';

// cross-ref notes that OV resolution supersedes for a supplemental CPL
const OV_REF_CODES = new Set(['cross_ref_broken', 'supplemental_ov_not_provided']);
const ASSETMAP_NAMES = new Set(['ASSETMAP.xml', 'ASSETMAP']);

let wasmReady = false;

const CHUNK_SIZE = 4 * 1024 * 1024; // 4 MB chunks for hashing
const MXF_HEADER_SIZE = 2 * 1024 * 1024; // Read first 2 MB of MXF for header + first J2K frame

self.onmessage = async (e) => {
    const { type, id, files, binaryFiles, ovAssetIds } = e.data;

    if (type === 'validate') {
        try {
            if (!wasmReady) {
                await init();
                wasmReady = true;
            }

            // Phase 1: Structural validation
            self.postMessage({ type: 'progress', id, detail: 'Validating structure…', percent: null });

            const filesJson = JSON.stringify(files);
            const resultJson = validate_dcp(filesJson);
            const result = JSON.parse(resultJson);

            // Phase 1b: OV-aware supplemental resolution. When an OV package's
            // asset ids are supplied, re-resolve each IMF CPL's cross-package
            // references against the OV so a VF no longer false-fails as broken.
            if (ovAssetIds && ovAssetIds.length > 0) {
                const amEntry = files.find(f => f.content && ASSETMAP_NAMES.has((f.path.split('/').pop() || f.path)));
                const assetmapXml = amEntry ? amEntry.content : '';
                const ovIdsJson = JSON.stringify(ovAssetIds);
                for (const f of files) {
                    if (!f.content || !f.content.includes('CompositionPlaylist') || !f.content.includes('2067')) continue;
                    const ovNotes = JSON.parse(validate_imf_supplemental(f.content, assetmapXml, ovIdsJson, f.path));
                    // replace the non-OV ref notes for this CPL with the OV-aware ones
                    result.notes = result.notes.filter(n => !(n.file === f.path && OV_REF_CODES.has(n.code)));
                    for (const n of ovNotes) {
                        if (OV_REF_CODES.has(n.code)) result.notes.push(n);
                    }
                }
                result.valid = !result.notes.some(n => n.severity === 'error');
            }

            // Phase 2: Hash verification for binary files
            const assetHashes = result.asset_hashes || {};
            const hashPaths = Object.keys(assetHashes);
            if (hashPaths.length > 0 && binaryFiles) {
                let hashesVerified = result.summary.hashes_verified || 0;
                let hashesFailed = result.summary.hashes_failed || 0;
                let hashesSkipped = result.summary.hashes_skipped || 0;
                const totalBytes = hashPaths.reduce((sum, p) => {
                    const f = binaryFiles[p];
                    return sum + (f ? f.size : 0);
                }, 0);
                let bytesHashed = 0;

                for (let i = 0; i < hashPaths.length; i++) {
                    const path = hashPaths[i];
                    const file = binaryFiles[path];
                    const expectedHash = assetHashes[path];

                    if (!file) {
                        // File not available for hashing
                        continue;
                    }

                    const shortName = path.split('/').pop() || path;
                    self.postMessage({
                        type: 'progress', id,
                        detail: `Hashing ${shortName} (${i + 1}/${hashPaths.length})`,
                        percent: totalBytes > 0 ? (bytesHashed / totalBytes) * 100 : 0
                    });

                    // Stream-hash the file in chunks
                    const computed = await hashFile(file, (chunkBytes) => {
                        bytesHashed += chunkBytes;
                        self.postMessage({
                            type: 'progress', id,
                            detail: `Hashing ${shortName} (${i + 1}/${hashPaths.length})`,
                            percent: totalBytes > 0 ? (bytesHashed / totalBytes) * 100 : 0
                        });
                    });

                    if (computed === expectedHash) {
                        hashesVerified++;
                    } else {
                        hashesFailed++;
                        result.notes.push({
                            severity: 'error',
                            code: 'pkl_hash_mismatch',
                            message: `Hash mismatch for ${path}: expected ${expectedHash}, computed ${computed}`,
                            file: path
                        });
                    }

                    hashesSkipped--;
                }

                // Update summary
                result.summary.hashes_verified = hashesVerified;
                result.summary.hashes_failed = hashesFailed;
                result.summary.hashes_skipped = Math.max(0, hashesSkipped);

                // Remove the "skipped" info note if all hashes were checked
                if (result.summary.hashes_skipped === 0) {
                    result.notes = result.notes.filter(n => n.code !== 'hashes_skipped');
                }

                // Update validity based on hash failures
                const hasErrors = result.notes.some(n => n.severity === 'error');
                result.valid = !hasErrors;
            }

            // Phase 3: MXF header inspection
            if (binaryFiles) {
                const mxfPaths = Object.keys(binaryFiles).filter(p => p.toLowerCase().endsWith('.mxf'));
                for (let i = 0; i < mxfPaths.length; i++) {
                    const path = mxfPaths[i];
                    const file = binaryFiles[path];
                    if (!file) continue;

                    const shortName = path.split('/').pop() || path;
                    self.postMessage({
                        type: 'progress', id,
                        detail: `Inspecting ${shortName} (${i + 1}/${mxfPaths.length})`,
                        percent: null
                    });

                    // Read first 1 MB for header parsing
                    const headerSize = Math.min(file.size, MXF_HEADER_SIZE);
                    const blob = file.slice(0, headerSize);
                    const buffer = await blob.arrayBuffer();
                    const headerBytes = new Uint8Array(buffer);

                    // Validate MXF header via WASM
                    const notesJson = validate_mxf_file(headerBytes, path);
                    const mxfNotes = JSON.parse(notesJson);
                    if (mxfNotes.length > 0) {
                        result.notes.push(...mxfNotes);
                        // Re-check validity
                        const hasErrors = result.notes.some(n => n.severity === 'error');
                        result.valid = !hasErrors;
                    }
                }
            }

            self.postMessage({ type: 'done', id, result });
        } catch (err) {
            self.postMessage({ type: 'error', id, error: err.message || String(err) });
        }
    }
};

async function hashFile(file, onChunk) {
    const hasher = new Sha1Hasher();
    let offset = 0;
    const size = file.size;

    while (offset < size) {
        const end = Math.min(offset + CHUNK_SIZE, size);
        const blob = file.slice(offset, end);
        const buffer = await blob.arrayBuffer();
        hasher.update(new Uint8Array(buffer));
        const chunkLen = end - offset;
        offset = end;
        onChunk(chunkLen);
    }

    return hasher.finalize();
}
