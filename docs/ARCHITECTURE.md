# Architecture

Pigeon turns files into animated QR GIFs and reconstructs them exactly on
another device using fountain coding. All protocol logic lives in Rust
compiled to WebAssembly; the frontend is a thin browser shell.

## System overview

```text
┌────────────────────────── apps/web (React + TS) ───────────────────────────┐
│  SendPage                       ReceivePage                                │
│      │                               │  │                                   │
│      ▼                               ▼  ▼                                  │
│  encode.worker       decode.worker  camera.worker (QR decode only)         │
│      │                    │                  │                              │
│      ▼                    ▼                  ▼                              │
│  ┌──────────────── pigeon_wasm (Rust → WASM) ────────────────┐             │
│  │  pigeon-core: protocol · RaptorQ · QR · GIF · recovery    │             │
│  └───────────────────────────────────────────────────────────┘             │
│                          zxing-wasm (self-hosted): QR DECODE only           │
└─────────────────────────────────────────────────────────────────────────────┘
```

- **`crates/pigeon-core`** — environment-neutral engine. No browser, OS or
  allocator dependencies. Owns everything that affects correctness:
  wire protocol, CRC32C, metadata + filename sanitization, RaptorQ FEC
  (`raptorq` crate, RFC 6330), QR generation (`fast_qr`), GIF encode/parse
  (`gif` crate), compression (`flate2`, bounded), SHA-256 verification
  (`sha2`), authenticated encryption (`argon2` + `chacha20poly1305`, see the
  `crypto` module and `docs/PROTOCOL.md` §8), and the multi-transfer
  recovery state machine.
- **`crates/pigeon-wasm`** — thin `wasm-bindgen` wrapper producing the
  `pigeon_wasm` package (web target). Translation only: JS params in via
  serde-wasm-bindgen, results out as camelCase objects; `Vec<u8>` fields are
  forced through `serialize_bytes` so they arrive as `Uint8Array`. u64
  transfer ids cross the boundary as 16-char hex strings (JS numbers lose
  precision above 2^53).
- **`apps/web`** — React 19 + Vite. Three module workers:
  - `encode.worker` — owns `prepare_transfer` + `GifBuilder`; interleaves QR
    generation and GIF writing frame by frame so the UI gets real progress
    and cancellation stays responsive.
  - `decode.worker` — owns the `Receiver` state machine (multi-transfer) and
    the GIF import flow: parse a frame → QR-decode via ZXing → ingest raw
    packet bytes into Rust. One frame is in memory at a time.
  - `camera.worker` — QR-decodes video frames (via `OffscreenCanvas`) and
    returns raw QR bytes; it holds no protocol state. The main thread
    forwards packets to `decode.worker`, so the Rust core validates
    everything.
- **`zxing-wasm`** — browser QR decoder, self-hosted: the `.wasm` binary is
  imported via `?url` (bundled, hashed, precached by the service worker) and
  `prepareZXingModule({ overrides: { locateFile } })` prevents the package's
  default jsDelivr CDN fetch. ZXing never sees protocol state.

## Entry points

| Entry | Trigger | What happens |
|---|---|---|
| `src/main.tsx` | page load | mounts `App` (Send/Receive routing, no router dep) |
| `encode_gif` (wasm) | one-shot encode | file → compressed payload → RaptorQ packets → QR matrices → GIF bytes |
| `prepare_transfer` + `GifBuilder`/`QrFrameSession` (wasm) | worker-driven encode | same pipeline, but QR matrices stream one at a time |
| `Receiver.ingestPacket` (wasm) | every decoded QR payload | validate → route by transfer id → dedupe → RaptorQ decode → auto-verify |
| `GifFrameStream` (wasm) | GIF import | streaming parse + compositing, one RGBA frame per call |

## Send pipeline (end to end)

1. **Prepare** (`encoder::encode_file`): SHA-256 the file → optional zlib
   compression with the auto heuristic (skips known-compressed magic bytes;
   keeps zlib only if > 5% smaller and > 1 KiB) → **optional encryption**
   (Argon2id key from the passphrase, then the whole object sealed once with
   XChaCha20-Poly1305 bound to the transfer id) → choose QR geometry
   (smallest version holding ≥ 1024 bytes plus wire overheads at the chosen
   ECL; capacity probed exactly via binary search) → `FountainEncoder` with
   `symbol_size = qr_capacity − 24 − envelope overhead` → build the metadata
   packet (OTI, compression id, original size, SHA-256, sanitized filename,
   MIME) and symbol packets, interleaved round-robin across source blocks
   with the configured repair overhead; metadata re-emitted every 500 frames.
   Encrypted transfers wrap every packet payload in a per-packet envelope
   and emit protocol v2 headers; unencrypted transfers emit byte-compatible
   v1.
2. **Render** (worker loop): pull one `QrMatrix` per frame from
   `QrFrameSession`, write into `GifBuilder` (two-colour global palette,
   quiet zone 4, `Repeat::Infinite`), posting progress every 25 frames.
3. **Deliver**: GIF bytes transferred to the main thread; user downloads via
   an object URL. The GIF loops forever in any image viewer.

Before any of this, `estimate_transfer` (same geometry math, no fountain or
rendering work) produces the UI's pre-encode estimate: frame-count bounds,
loop duration and a GIF size estimate with stated assumptions. Presets
("Balanced", "Maximum reliability", "Faster transmission", "Camera-friendly")
map UI choices onto concrete engine parameters; manual changes select
"Custom" so the active preset never lies.

## Receive pipeline (end to end)

1. **Acquire**: GIF import (`GifFrameStream.nextFrame` → RGBA canvas) or
   camera frames (video → `createImageBitmap` → camera.worker →
   `OffscreenCanvas` → ZXing `readBarcodes`).
2. **QR decode**: ZXing returns raw bytes (`bytes` when available, latin-1
   text fallback).
3. **Validate** (`decoder::packet::parse_incoming`): magic, version
   (unknown ⇒ explicit error), flags (v1 never encrypted; v2 always), exact
   payload length, CRC32C, then metadata/symbol structure — or, for v2,
   envelope structure (cipher/KDF ids, KDF caps, inner length) without
   touching ciphertext beyond that. Invalid packets are counted with a
   stable reason (`corrupted-content`, `unsupported-protocol-version`,
   `encrypted-envelope-invalid`, `not-a-pigeon-packet`, …) and never touch
   transfer state.
4. **Recover** (`decoder::recovery::TransferRx`): route by transfer id
   (≤ 16 concurrent); duplicates counted, not re-decoded; symbols ahead of
   metadata buffered (≤ 4096); `raptorq::Decoder.decode` per packet until
   reconstruction completes. Encrypted transfers add a passphrase gate:
   sealed packets buffer until the metadata envelope's KDF parameters +
   passphrase yield a key (`Receiver.setPassphrase`, remembered for later
   transfers), then authentication failure is counted, never fatal, and the
   gate stays open for retry.
5. **Verify** (`decoder::reconstruction::finalize` / `finalize_sealed`):
   declared transfer length check (on the sealed object for v2) → object
   AEAD open bound to the transfer id (v2) → bounded decompression
   (cap = declared original size) → SHA-256 against the metadata manifest.
   Only a matching file enters `Verified` and can be taken (`takeFile`) for
   download. Failures move the transfer to `Failed` with the reason
   surfaced in the status.

## Data flow contracts

- Encode params cross the boundary as a camelCase JS object
  (`EncodeParams`), deserialized with serde defaults (all optional).
- Status/report objects (`EncodeStats`, `TransferStatus`, `IngestReport`,
  `DecoderCounters`, `ReconstructedFile`, `DecodedGifFrame`) are camelCase,
  `transferId` as hex string, byte fields as `Uint8Array`.
- Worker RPC (`src/lib/worker-client.ts`): promise-based with progress
  events; responses are matched by the request envelope id; `ArrayBuffer`s
  are transferred where a full copy would double memory.
- The wire format itself is specified in `docs/PROTOCOL.md`.

## Memory management

- Whole-object RaptorQ requires the (possibly compressed) payload in memory:
  encoder soft limit 64 MiB (configurable to the 256 MiB engine cap),
  decoder hard cap 512 MiB on declared transfer length. Documented, never
  claimed unlimited.
- GIF import streams: one composited frame at a time; frame/dimension/file
  caps enforced up front (20 000 frames, 2048 px, 256 MiB).
- GIF writing streams the bitmap; QR matrices are generated one at a time in
  the encode worker.
- Blob URLs are revoked after download triggers; camera tracks stop on
  unmount; workers are terminated when pages unmount.

## Cross-cutting behavior

- **Offline**: PWA service worker (vite-plugin-pwa `injectManifest` with the
  custom `src/sw.ts`) precaches all app assets including both WASM binaries,
  storing responses Vary-free so page-originated requests always match; no
  CDN references exist in the bundle. CSP (`index.html`) restricts
  everything to `'self'` plus `wasm-unsafe-eval`; `connect-src` has no
  remote origins.
- **Errors**: a typed `PigeonError` crosses the boundary as messages; the UI
  renders honest states (indeterminate progress when the needed-symbol count
  is unknown; explicit "not enough frames" instead of partial output).
- **Threading**: all WASM work happens in workers; the main thread only
  renders canvas/React state and forwards packets.

## Repository map

| Path | Responsibility |
|---|---|
| `crates/pigeon-core/src/protocol/` | wire format, limits, metadata, sanitization, serde boundary helpers |
| `crates/pigeon-core/src/crypto/` | Argon2id KDF, XChaCha20-Poly1305 AEAD, packet envelopes, object sealing |
| `crates/pigeon-core/src/encoder/` | compression, fountain, packet build, QR, GIF, estimator, pipeline |
| `crates/pigeon-core/src/decoder/` | packet parse, recovery state machine, GIF stream, reconstruction, engine |
| `crates/pigeon-core/src/integrity/` | SHA-256, CRC32C, hex |
| `crates/pigeon-wasm/src/lib.rs` | wasm-bindgen API surface |
| `apps/web/src/wasm/` | generated engine + typed bridge (`bridge.ts`, `wasm-types.ts`) |
| `apps/web/src/workers/` | encode / decode / camera workers |
| `apps/web/src/lib/` | worker RPC, formatting, download, ZXing self-host |
| `apps/web/src/features/` | Send / Receive UI (presets, estimator, passphrase gates) |
| `apps/web/tests/e2e/` | Playwright roundtrip + offline + encryption suites |
