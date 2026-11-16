/* tslint:disable */
/* eslint-disable */

/**
 * Incremental GIF writer for the send pipeline. Pair with a
 * [`QrFrameSession`] to interleave QR generation and report progress.
 *
 * `start(matrix, scale, delayMs)` writes the first frame (canvas size is
 * fixed from it); `writeMatrix` appends; `finish` returns the bytes.
 */
export class GifBuilder {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Completes the GIF and returns the bytes (transferable-capable buffer).
     */
    finish(): Uint8Array;
    /**
     * Frames written so far.
     */
    framesWritten(): number;
    constructor(first_modules: Uint8Array | number[], first_size: number, scale: number, delay_ms: number);
    /**
     * Appends a frame. `modules` is a 1-byte-per-module row-major matrix.
     */
    writeMatrix(modules: Uint8Array | number[], size: number): void;
}

/**
 * Streaming GIF frame extractor for the import path.
 */
export class GifFrameStream {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * `[width, height]` of the composited canvas frames.
     */
    dimensions(): Uint32Array;
    /**
     * Opens a GIF (validates signature, dimensions and size limits).
     */
    constructor(bytes: Uint8Array | number[]);
    /**
     * Next composited frame `{ index, delayMs, width, height, data: Uint8Array }`
     * or `null` at end of stream.
     */
    nextFrame(): any;
}

/**
 * Incremental QR generation over a prepared transfer. One matrix lives in
 * memory at a time; pull frames with [`next_frame`].
 */
export class QrFrameSession {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Next QR frame as `{ size, modules: Uint8Array }` (1 byte per module),
     * or `null` when the sequence is exhausted.
     */
    nextFrame(): any;
    /**
     * Total QR frames in the sequence.
     */
    totalFrames(): number;
}

/**
 * Receive engine bound to JS: ingest packets, watch progress, take files.
 */
export class Receiver {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Status of the most recently active transfer or `null`.
     */
    activeStatus(): any;
    /**
     * All tracked transfers.
     */
    allStatuses(): any;
    /**
     * Aggregate counters.
     */
    counters(): any;
    /**
     * Ingests one candidate packet (decoded QR content).
     * Returns `{ valid, duplicate, reason?, transferId?, status? }`.
     */
    ingestPacket(bytes: Uint8Array | number[]): any;
    /**
     * Creates a receiver. `max_transfer_bytes` bounds each transfer's
     * declared size (resource guard).
     */
    constructor(max_transfer_bytes?: any | null);
    /**
     * Clears all receive state.
     */
    reset(): void;
    /**
     * Status of one transfer (`transferId` is a 16-char hex string) or `null`.
     */
    statusOf(transfer_id: string): any;
    /**
     * Takes the verified file for a completed transfer (`transferId` is a
     * 16-char hex string). Returns `null` while incomplete; throws on
     * verification failure.
     */
    takeFile(transfer_id: string): any;
}

/**
 * One-shot send pipeline: file bytes → animated QR GIF.
 *
 * Returns `{ gif: Uint8Array, stats }`.
 */
export function encode_gif(data: Uint8Array | number[], params: any): any;

/**
 * Prepares a transfer: wire packets are encoded up front, QR matrices are
 * generated one at a time by the returned session (used for live playback
 * and for progress-reporting GIF generation).
 *
 * Returns `{ stats, session }`.
 */
export function prepare_transfer(data: Uint8Array | number[], params: any): any;

/**
 * The wire protocol version this engine implements.
 */
export function protocol_version(): number;

/**
 * Sanity helper exposed for tests: sanitizes a received filename.
 */
export function sanitizeFilename(name: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_gifbuilder_free: (a: number, b: number) => void;
    readonly __wbg_gifframestream_free: (a: number, b: number) => void;
    readonly __wbg_qrframesession_free: (a: number, b: number) => void;
    readonly __wbg_receiver_free: (a: number, b: number) => void;
    readonly encode_gif: (a: number, b: number, c: any) => [number, number, number];
    readonly gifbuilder_finish: (a: number) => [number, number, number, number];
    readonly gifbuilder_framesWritten: (a: number) => number;
    readonly gifbuilder_new: (a: number, b: number, c: number, d: number, e: number) => [number, number, number];
    readonly gifbuilder_writeMatrix: (a: number, b: number, c: number, d: number) => [number, number];
    readonly gifframestream_dimensions: (a: number) => [number, number];
    readonly gifframestream_new: (a: number, b: number) => [number, number, number];
    readonly gifframestream_nextFrame: (a: number) => [number, number, number];
    readonly prepare_transfer: (a: number, b: number, c: any) => [number, number, number];
    readonly protocol_version: () => number;
    readonly qrframesession_nextFrame: (a: number) => [number, number, number];
    readonly qrframesession_totalFrames: (a: number) => number;
    readonly receiver_activeStatus: (a: number) => [number, number, number];
    readonly receiver_allStatuses: (a: number) => [number, number, number];
    readonly receiver_counters: (a: number) => [number, number, number];
    readonly receiver_ingestPacket: (a: number, b: number, c: number) => [number, number, number];
    readonly receiver_new: (a: number) => number;
    readonly receiver_reset: (a: number) => void;
    readonly receiver_statusOf: (a: number, b: number, c: number) => [number, number, number];
    readonly receiver_takeFile: (a: number, b: number, c: number) => [number, number, number];
    readonly sanitizeFilename: (a: number, b: number) => [number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
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
