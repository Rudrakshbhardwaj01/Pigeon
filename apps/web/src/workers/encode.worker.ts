// Encode worker: owns the WASM engine for the send pipeline.
// GIF generation interleaves QR encoding and GIF writing frame by frame so
// real progress can be reported and cancellation stays responsive.

import {
  initEngine,
  prepareTransfer,
  newGifBuilder,
  estimateTransfer,
} from "../wasm/bridge";
import type {
  EncodeParams,
  EncodeStats,
  QrMatrix,
} from "../wasm/bridge";

export interface EncodeGifRequest {
  kind: "encode-gif";
  data: ArrayBuffer;
  params: EncodeParams;
}

export interface EstimateRequest {
  kind: "estimate";
  size: number;
  params: EncodeParams;
}

export interface CancelRequest {
  kind: "cancel";
}

export type EncodeRequest = EncodeGifRequest | EstimateRequest | CancelRequest;

export interface EncodeProgress {
  id: number;
  kind: "progress";
  phase: "preparing" | "fountain" | "qr-gif" | "ready";
  framesDone?: number;
  totalFrames?: number;
}

export type EncodeResponse =
  | { id: number; kind: "ok"; result: unknown }
  | { id: number; kind: "err"; message: string }
  | EncodeProgress;

const QUIET_MODULES = 4;

let ready: Promise<unknown> | null = null;
let cancelled = false;

function post(message: EncodeResponse, transfer?: Transferable[]): void {
  (self as unknown as Worker).postMessage(message, transfer ?? []);
}

function ensureEngine(): Promise<unknown> {
  if (!ready) {
    ready = initEngine();
  }
  return ready;
}

async function handle(request: EncodeRequest, requestId: number): Promise<void> {
  await ensureEngine();

  if (request.kind === "cancel") {
    cancelled = true;
    return;
  }

  if (request.kind === "estimate") {
    // Cheap: QR-capacity probing only, no fountain or rendering work.
    const estimate = estimateTransfer(request.size, request.params);
    post({ id: requestId, kind: "ok", result: { estimate } });
    return;
  }

  cancelled = false;
  const params = request.params;

  post({ id: requestId, kind: "progress", phase: "preparing" });

  // Step 1: fragment + fountain encode (fast, single call into the engine).
  const { stats, session } = prepareTransfer(new Uint8Array(request.data), params);
  const totalFrames = session.totalFrames();
  post({ id: requestId, kind: "progress", phase: "fountain", totalFrames });

  // Step 2: interleave QR generation and GIF writing, reporting progress.
  const first = session.nextFrame();
  if (!first) {
    post({ id: requestId, kind: "err", message: "transfer produced no frames" });
    return;
  }
  const modules = first.size + 2 * QUIET_MODULES;
  const targetPx = params.targetFramePx ?? 800;
  const scale = Math.min(16, Math.max(2, Math.floor(targetPx / modules)));
  const builder = newGifBuilder(first, scale, stats.frameDelayMs);
  let done = 1;
  let matrix: QrMatrix | null = session.nextFrame();
  while (matrix) {
    if (cancelled) {
      post({ id: requestId, kind: "err", message: "cancelled" });
      return;
    }
    builder.writeMatrix(matrix);
    done += 1;
    if (done % 25 === 0 || done === totalFrames) {
      post({ id: requestId, kind: "progress", phase: "qr-gif", framesDone: done, totalFrames });
      // Yield to the event loop so a queued "cancel" message is handled
      // between frame batches instead of only after the whole GIF is built.
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    matrix = session.nextFrame();
  }
  const gif = builder.finish();

  // Hand the GIF buffer to the main thread without copying.
  const copy = gif.buffer.slice(
    gif.byteOffset,
    gif.byteOffset + gif.byteLength
  ) as ArrayBuffer;
  post(
    {
      id: requestId,
      kind: "ok",
      result: { gif: copy, stats: stats as EncodeStats },
    },
    [copy]
  );
}

self.onmessage = (event: MessageEvent<EncodeRequest & { id: number }>) => {
  handle(event.data, event.data.id).catch((err: unknown) => {
    post({
      id: event.data.id,
      kind: "err",
      message: err instanceof Error ? err.message : String(err),
    });
  });
};
