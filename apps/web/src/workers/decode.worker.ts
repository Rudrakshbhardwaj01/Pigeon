selfHostZXing();

// Decode worker: owns the Receiver state machine and the GIF import flow.
// GIF frames are parsed and QR-decoded one at a time (bounded memory), with
// per-frame progress posted back to the UI.

import { readBarcodes, selfHostZXing } from "../lib/zxing";
import { initEngine, newGifFrameStream, newReceiver } from "../wasm/bridge";
import type {
  IngestReport,
  ReconstructedFile,
  Receiver,
  TransferStatus,
} from "../wasm/bridge";

export interface ImportGifRequest {
  kind: "import-gif";
  data: ArrayBuffer;
}

export interface IngestPacketsRequest {
  kind: "ingest-packets";
  packets: Uint8Array[];
}

export interface TakeFileRequest {
  kind: "take-file";
  transferId: string;
}

export interface ResetRequest {
  kind: "reset";
}

export interface CancelImportRequest {
  kind: "cancel-import";
}

export interface StatusRequest {
  kind: "status";
}

export interface SetPassphraseRequest {
  kind: "set-passphrase";
  passphrase: string;
}

export type DecodeRequest =
  | ImportGifRequest
  | IngestPacketsRequest
  | TakeFileRequest
  | ResetRequest
  | CancelImportRequest
  | StatusRequest
  | SetPassphraseRequest;

export interface DecodeProgress {
  id: number;
  kind: "progress";
  phase: "parsing" | "collecting" | "verifying" | "done";
  framesSeen: number;
  validPackets: number;
  invalidPackets: number;
  duplicatePackets: number;
  uniqueSymbols: number;
  neededSymbols: number | null;
  progress: number | null;
  filename: string | null;
  mimeType: string | null;
  originalSize: number | null;
  encrypted: boolean;
  failedDecryptions: number;
}

export type DecodeResponse =
  | { id: number; kind: "ok"; result: unknown; transfer?: ArrayBuffer[] }
  | { id: number; kind: "err"; message: string }
  | DecodeProgress;

let ready: Promise<void> | null = null;
let receiver: Receiver | null = null;
let cancelled = false;

function post(message: DecodeResponse, transfer?: Transferable[]): void {
  (self as unknown as Worker).postMessage(message, transfer ?? []);
}

function ensureEngine(): Promise<void> {
  if (!ready) {
    ready = initEngine().then(() => {
      receiver = newReceiver();
    });
  }
  return ready;
}

function requireReceiver(): Receiver {
  if (!receiver) {
    throw new Error("receiver not initialized");
  }
  return receiver;
}

function snapshot(
  phase: DecodeProgress["phase"],
  framesSeen: number,
  lastReport: IngestReport | null
): DecodeProgress {
  const rx = requireReceiver();
  const counters = rx.counters();
  const status: TransferStatus | null = lastReport?.status ?? rx.activeStatus();
  return {
    // Callers overwrite `id` with the request envelope id before posting.
    id: 0,
    kind: "progress",
    phase,
    framesSeen,
    validPackets: counters.packetsSeen - counters.invalidPackets,
    invalidPackets: counters.invalidPackets,
    duplicatePackets: counters.duplicatePackets,
    uniqueSymbols: status?.uniqueSymbols ?? 0,
    neededSymbols: status?.neededSymbols ?? null,
    progress: status?.progress ?? null,
    filename: status?.filename ?? null,
    mimeType: status?.mimeType ?? null,
    originalSize: status?.originalSize ?? null,
    encrypted: status?.encrypted ?? false,
    failedDecryptions: status?.failedDecryptions ?? 0,
  };
}

/** Ensures the file's bytes live in a standalone, transferable ArrayBuffer. */
function fileWithTransferableData(file: ReconstructedFile): {
  file: ReconstructedFile;
  transfer: ArrayBuffer;
} {
  const data = new Uint8Array(file.data); // exact-size copy into a fresh buffer
  return { file: { ...file, data }, transfer: data.buffer as ArrayBuffer };
}

/** Extracts raw packet bytes from a ZXing result. */
function rawBytes(result: { bytes?: Uint8Array | null; text?: string }): Uint8Array | null {
  if (result.bytes instanceof Uint8Array) {
    return result.bytes;
  }
  if (typeof result.text === "string" && result.text.length > 0) {
    // Fallback: latin-1 encode (safe when ZXing surfaced bytes as text).
    const out = new Uint8Array(result.text.length);
    for (let i = 0; i < result.text.length; i++) {
      out[i] = result.text.charCodeAt(i) & 0xff;
    }
    return out;
  }
  return null;
}

type BarcodeResult = { bytes?: Uint8Array | null; text?: string };
type BarcodeInput = Parameters<typeof readBarcodes>[0];

function toBarcodeInput(data: Uint8Array, width: number, height: number): BarcodeInput {
  return {
    data: new Uint8ClampedArray(data.buffer.slice(0)),
    width,
    height,
  } as unknown as BarcodeInput;
}

async function handle(request: DecodeRequest, requestId: number): Promise<void> {
  await ensureEngine();

  if (request.kind === "reset") {
    cancelled = true;
    requireReceiver().reset();
    post({ id: requestId, kind: "ok", result: null });
    return;
  }

  if (request.kind === "cancel-import") {
    // Fire-and-forget: an in-flight import-gif loop notices the flag between
    // frames and reports "cancelled". No response is expected.
    cancelled = true;
    return;
  }

  if (request.kind === "status") {
    const rx = requireReceiver();
    post({
      id: requestId,
      kind: "ok",
      result: {
        counters: rx.counters(),
        active: rx.activeStatus(),
        all: rx.allStatuses(),
      },
    });
    return;
  }

  if (request.kind === "set-passphrase") {
    // Unlocks paused transfers; the receiver remembers the passphrase for
    // transfers whose metadata has not arrived yet. The response carries the
    // post-unlock status so the UI can proceed to download when verified.
    const rx = requireReceiver();
    const outcome = rx.setPassphrase(request.passphrase);
    post({ id: requestId, kind: "ok", result: { outcome, active: rx.activeStatus() } });
    return;
  }

  if (request.kind === "take-file") {
    const file = requireReceiver().takeFile(request.transferId);
    if (!file) {
      post({ id: requestId, kind: "err", message: "transfer is not complete" });
      return;
    }
    const { file: out, transfer } = fileWithTransferableData(file);
    post({ id: requestId, kind: "ok", result: { file: out }, transfer: [transfer] }, [transfer]);
    return;
  }

  if (request.kind === "ingest-packets") {
    const rx = requireReceiver();
    let last: IngestReport | null = null;
    for (const packet of request.packets) {
      last = rx.ingestPacket(packet);
    }
    post({
      id: requestId,
      kind: "ok",
      result: { last, counters: rx.counters(), active: rx.activeStatus() },
    });
    return;
  }

  // ---- import-gif ----
  cancelled = false;
  const rx = requireReceiver();
  rx.reset();

  let stream;
  try {
    stream = newGifFrameStream(new Uint8Array(request.data));
  } catch (err) {
    post({
      id: requestId,
      kind: "err",
      message: err instanceof Error ? err.message : String(err),
    });
    return;
  }

  const [width, height] = stream.dimensions();
  // Bound decode work: downscale oversized frames before ZXing.
  const maxSide = 900;
  const factor = Math.max(1, Math.ceil(Math.max(width, height) / maxSide));
  let ctx: OffscreenCanvasRenderingContext2D | null = null;
  let canvas: OffscreenCanvas | null = null;
  if (factor > 1) {
    canvas = new OffscreenCanvas(Math.ceil(width / factor), Math.ceil(height / factor));
    ctx = canvas.getContext("2d") as OffscreenCanvasRenderingContext2D | null;
  }

  let framesSeen = 0;
  let lastReport: IngestReport | null = null;
  let lastPost = 0;

  for (;;) {
    if (cancelled) {
      post({ id: requestId, kind: "err", message: "cancelled" });
      return;
    }
    const frame = stream.nextFrame();
    if (!frame) {
      break;
    }
    framesSeen += 1;

    let input: BarcodeInput;
    if (ctx && canvas && factor > 1) {
      const img = new ImageData(new Uint8ClampedArray(frame.data), frame.width, frame.height);
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(img as unknown as CanvasImageSource, 0, 0, canvas.width, canvas.height);
      const small = ctx.getImageData(0, 0, canvas.width, canvas.height);
      input = {
        data: small.data,
        width: small.width,
        height: small.height,
      } as unknown as BarcodeInput;
    } else {
      input = toBarcodeInput(frame.data, frame.width, frame.height);
    }

    let results: BarcodeResult[] = [];
    try {
      results = (await readBarcodes(input, {
        formats: ["QRCode"],
        tryHarder: true,
        maxNumberOfSymbols: 1,
      })) as BarcodeResult[];
    } catch {
      // undecodable frame — continue
    }

    for (const result of results) {
      const bytes = rawBytes(result);
      if (bytes && bytes.length > 0) {
        try {
          const report = rx.ingestPacket(bytes);
          lastReport = report;
        } catch (err) {
          post({
            id: requestId,
            kind: "err",
            message: err instanceof Error ? err.message : String(err),
          });
          return;
        }
      }
    }

    const now = Date.now();
    if (now - lastPost > 120 || results.length > 0) {
      lastPost = now;
      const snap = snapshot("collecting", framesSeen, lastReport);
      snap.id = requestId;
      post(snap);
    }
  }

  // Extraction finished; report the honest outcome.
  const statuses = rx.allStatuses();
  if (statuses.length === 0) {
    post({
      id: requestId,
      kind: "err",
      message:
        "No Pigeon packets found in this GIF. It may not be a Pigeon transfer, or its frames were altered by a messaging platform. Send the GIF as a document/file attachment.",
    });
    return;
  }

  const status = rx.activeStatus() as TransferStatus;
  if (status.phase === "verified") {
    const file = rx.takeFile(status.transferId);
    if (file) {
      const { file: out, transfer } = fileWithTransferableData(file);
      post(
        { id: requestId, kind: "ok", result: { status, file: out }, transfer: [transfer] },
        [transfer]
      );
      return;
    }
  }
  if (status.phase === "failed") {
    post({
      id: requestId,
      kind: "err",
      message:
        "Recovery failed: the reconstructed data did not verify against its checksum. Capture more frames or re-export the GIF and try again.",
    });
    return;
  }

  // Incomplete: resolve the RPC with the honest status (no file) so the UI can
  // show exactly how much was captured instead of waiting forever.
  const snap = snapshot("done", framesSeen, lastReport);
  snap.id = requestId;
  post(snap);
  post({ id: requestId, kind: "ok", result: { status, file: null } });
}

self.onmessage = (event: MessageEvent<DecodeRequest & { id: number }>) => {
  handle(event.data, event.data.id).catch((err: unknown) => {
    const detail = err instanceof Error ? `${err.message}
${err.stack ?? ""}` : String(err);
    post({
      id: event.data.id,
      kind: "err",
      message: detail,
    });
  });
};
