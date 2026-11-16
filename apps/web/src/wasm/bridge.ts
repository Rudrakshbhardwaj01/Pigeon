// Typed bridge over the wasm-pack generated bindings.
//
// The generated module types most serde returns as `any`; the wrappers here
// assert the precise shapes declared in `wasm-types.ts` (verified by the
// roundtrip tests against the Rust side).

import initModule, {
  GifBuilder as GenGifBuilder,
  GifFrameStream as GenGifFrameStream,
  Receiver as GenReceiver,
  encode_gif as genEncodeGif,
  prepare_transfer as genPrepareTransfer,
  sanitizeFilename as genSanitizeFilename,
  protocol_version as genProtocolVersion,
} from "./pigeon-wasm/pigeon_wasm";

import type {
  EncodeParams,
  GifBuilder,
  GifEncodeResult,
  GifFrameStream,
  PreparedTransfer,
  QrMatrix,
  Receiver,
} from "./wasm-types";

export type {
  EncodeParams,
  EncodeStats,
  GifBuilder,
  GifEncodeResult,
  GifFrameStream,
  IngestReport,
  DecodedGifFrame,
  DecoderCounters,
  PreparedTransfer,
  QrMatrix,
  QrFrameSession,
  ReconstructedFile,
  Receiver,
  TransferStatus,
} from "./wasm-types";

let initPromise: Promise<unknown> | null = null;

/** Initializes the engine exactly once per execution context (main or worker). */
export function initEngine(): Promise<unknown> {
  if (!initPromise) {
    initPromise = initModule().catch((err: unknown) => {
      initPromise = null;
      throw err;
    });
  }
  return initPromise;
}

export function protocolVersion(): number {
  return genProtocolVersion();
}

export function prepareTransfer(
  data: Uint8Array,
  params?: EncodeParams
): PreparedTransfer {
  return genPrepareTransfer(data, params) as PreparedTransfer;
}

export function encodeGif(data: Uint8Array, params?: EncodeParams): GifEncodeResult {
  return genEncodeGif(data, params) as GifEncodeResult;
}

export function newReceiver(maxTransferBytes?: number): Receiver {
  return new GenReceiver(maxTransferBytes) as unknown as Receiver;
}

export function newGifFrameStream(bytes: Uint8Array): GifFrameStream {
  const stream = new GenGifFrameStream(bytes) as unknown as {
    dimensions(): Uint32Array;
    nextFrame(): {
      index: number;
      delayMs: number;
      width: number;
      height: number;
      data: Uint8Array;
    } | null;
  };
  return {
    dimensions(): [number, number] {
      const [w, h] = stream.dimensions();
      return [w, h];
    },
    nextFrame: () => stream.nextFrame(),
  };
}

export function newGifBuilder(
  firstMatrix: QrMatrix,
  scale: number,
  delayMs: number
): GifBuilder {
  const builder = new GenGifBuilder(firstMatrix.modules, firstMatrix.size, scale, delayMs) as unknown as {
    writeMatrix(modules: Uint8Array, size: number): void;
    framesWritten(): number;
    finish(): Uint8Array;
  };
  return {
    writeMatrix: (m: QrMatrix) => builder.writeMatrix(m.modules, m.size),
    framesWritten: () => builder.framesWritten(),
    finish: () => builder.finish(),
  };
}

export const sanitizeFilename: (name: string) => string = genSanitizeFilename;
