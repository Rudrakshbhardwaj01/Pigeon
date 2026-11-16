selfHostZXing();

// Camera worker: QR-decodes video frames via ZXing WASM.
// Raw packet bytes are returned to the main thread, which forwards them to the
// decode worker — the Rust core still validates and interprets everything.

import { readBarcodes, selfHostZXing } from "../lib/zxing";

export interface DecodeFrameRequest {
  kind: "decode-frame";
  id: number;
  bitmap: ImageBitmap;
  targetWidth: number;
}

export interface FrameResult {
  id: number;
  kind: "frame-result";
  packets: Uint8Array[];
}

export interface CameraErr {
  id: number;
  kind: "err";
  message: string;
}

export type CameraResponse = FrameResult | CameraErr;

type BarcodeResult = { bytes?: Uint8Array | null; text?: string };

function rawBytes(result: BarcodeResult): Uint8Array | null {
  if (result.bytes instanceof Uint8Array) {
    return result.bytes;
  }
  if (typeof result.text === "string" && result.text.length > 0) {
    const out = new Uint8Array(result.text.length);
    for (let i = 0; i < result.text.length; i++) {
      out[i] = result.text.charCodeAt(i) & 0xff;
    }
    return out;
  }
  return null;
}

async function handle(request: DecodeFrameRequest): Promise<void> {
  const bitmap = request.bitmap;
  const scale = Math.min(1, request.targetWidth / bitmap.width);
  const w = Math.max(1, Math.round(bitmap.width * scale));
  const h = Math.max(1, Math.round(bitmap.height * scale));

  const canvas = new OffscreenCanvas(w, h);
  const ctx = canvas.getContext("2d", { willReadFrequently: true }) as OffscreenCanvasRenderingContext2D | null;
  if (!ctx) {
    post({ id: request.id, kind: "err", message: "OffscreenCanvas 2D unavailable" });
    bitmap.close();
    return;
  }
  ctx.drawImage(bitmap, 0, 0, w, h);
  bitmap.close();
  const imageData = ctx.getImageData(0, 0, w, h);

  let results: BarcodeResult[] = [];
  try {
    results = (await readBarcodes(imageData, {
      formats: ["QRCode"],
      tryHarder: true,
      maxNumberOfSymbols: 1,
    })) as Array<{ bytes?: Uint8Array | null; text?: string }>;
  } catch {
    // no code in frame
  }

  const packets: Uint8Array[] = [];
  for (const result of results) {
    const bytes = rawBytes(result);
    if (bytes && bytes.length > 0) {
      packets.push(bytes);
    }
  }
  post({ id: request.id, kind: "frame-result", packets });
}

function post(message: CameraResponse): void {
  (self as unknown as Worker).postMessage(message);
}

self.onmessage = (event: MessageEvent<DecodeFrameRequest>) => {
  handle(event.data).catch((err: unknown) => {
    post({
      id: (event.data as DecodeFrameRequest).id,
      kind: "err",
      message: err instanceof Error ? err.message : String(err),
    });
  });
};
