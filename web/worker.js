// Web Worker for DCP/IMP validation
// Each worker instance validates one package independently

import init, { validate_dcp, version } from './pkg/dcpdoctor_wasm.js';

let wasmReady = false;

self.onmessage = async (e) => {
    const { type, id, files } = e.data;

    if (type === 'validate') {
        try {
            if (!wasmReady) {
                await init();
                wasmReady = true;
            }

            self.postMessage({ type: 'progress', id, status: 'validating' });

            const filesJson = JSON.stringify(files);
            const resultJson = validate_dcp(filesJson);
            const result = JSON.parse(resultJson);

            self.postMessage({ type: 'done', id, result });
        } catch (err) {
            self.postMessage({ type: 'error', id, error: err.message || String(err) });
        }
    }
};
