// Precise TypeScript contracts for the Pigeon WASM engine.
// The generated `pigeon_wasm.d.ts` types most returns as `any`; these are the
// shapes actually produced by serde-wasm-bindgen (camelCase keys).

export interface EncodeStats {
  transferId: string;
  originalSize: number;
  encodedSize: number;
  compressionUsed: boolean;
  originalSha256: string;
  filename: string;
  mimeType: string | null;
  qrVersion: number;
  qrEcl: string;
  packetPayloadBytes: number;
  sourceSymbols: number;
  repairSymbols: number;
  totalFrames: number;
  frameDelayMs: number;
  neededPackets: number;
}

export interface QrMatrix {
  size: number;
  modules: Uint8Array;
}

export interface TransferStatus {
  transferId: string;
  phase: string;
  filename: string | null;
  mimeType: string | null;
  originalSize: number | null;
  expectedSha256: string | null;
  uniqueSymbols: number;
  neededSymbols: number | null;
  progress: number | null;
  duplicates: number;
  bufferedSymbols: number;
}

export interface IngestReport {
  valid: boolean;
  duplicate: boolean;
  reason: string | null;
  transferId: string | null;
  status: TransferStatus | null;
}

export interface DecoderCounters {
  packetsSeen: number;
  invalidPackets: number;
  duplicatePackets: number;
  transfers: number;
  rejections: [string, number][];
}

export interface ReconstructedFile {
  filename: string;
  mimeType: string | null;
  data: Uint8Array;
  sha256: string;
  originalSize: number;
}

export interface DecodedGifFrame {
  index: number;
  delayMs: number;
  width: number;
  height: number;
  data: Uint8Array;
}

export interface EncodeParams {
  filename?: string;
  mimeType?: string | null;
  compression?: "auto" | "on" | "off";
  overheadPercent?: number;
  qrEcl?: "l" | "m" | "q" | "h";
  qrVersion?: number;
  frameDelayMs?: number;
  maxFileSize?: number;
  targetFramePx?: number;
}

export interface QrFrameSession {
  totalFrames(): number;
  nextFrame(): QrMatrix | null;
}

export interface GifBuilder {
  writeMatrix(matrix: QrMatrix): void;
  framesWritten(): number;
  finish(): Uint8Array;
}

export interface Receiver {
  ingestPacket(bytes: Uint8Array): IngestReport;
  statusOf(transferId: string): TransferStatus | null;
  activeStatus(): TransferStatus | null;
  allStatuses(): TransferStatus[];
  counters(): DecoderCounters;
  takeFile(transferId: string): ReconstructedFile | null;
  reset(): void;
}

export interface GifFrameStream {
  dimensions(): [number, number];
  nextFrame(): DecodedGifFrame | null;
}

export interface PreparedTransfer {
  stats: EncodeStats;
  session: QrFrameSession;
}

export interface GifEncodeResult {
  gif: Uint8Array;
  stats: EncodeStats;
}
