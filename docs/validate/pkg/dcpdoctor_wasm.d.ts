/* tslint:disable */
/* eslint-disable */

/**
 * Streaming SHA-1 hasher for incremental hashing of large files.
 */
export class Sha1Hasher {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Finalize and return the SHA-1 digest as base64.
     */
    finalize(): string;
    /**
     * Create a new streaming SHA-1 hasher.
     */
    constructor();
    /**
     * Feed a chunk of bytes into the hasher.
     */
    update(chunk: Uint8Array): void;
}

/**
 * Compute SHA-1 hash of raw bytes, returned as base64.
 */
export function sha1_base64(data: Uint8Array): string;

/**
 * Main entry point: validate a DCP from a set of file entries.
 *
 * Accepts a JSON array of FileEntry objects and returns a ValidationResult as JSON.
 */
export function validate_dcp(files_json: string): string;

/**
 * Get the version of dcpdoctor-wasm.
 */
export function version(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_sha1hasher_free: (a: number, b: number) => void;
    readonly sha1_base64: (a: number, b: number) => [number, number];
    readonly sha1hasher_finalize: (a: number) => [number, number];
    readonly sha1hasher_new: () => number;
    readonly sha1hasher_update: (a: number, b: number, c: number) => void;
    readonly validate_dcp: (a: number, b: number) => [number, number];
    readonly version: () => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
