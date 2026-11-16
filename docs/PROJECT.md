# PROJECT

## Identity

**Pigeon** — offline-first optical file transfer. Pigeon turns any file into
an animated QR-code GIF that can travel through ordinary file-sharing
channels (WhatsApp as document, Telegram, e-mail, Drive, USB) and be
reconstructed byte-for-byte on another device with no network, no server, no
account, and no pairing. The receiving device can also capture the frames
with its camera while the GIF loops on a screen.

The pigeon references carrier pigeons: the message travels on its own,
without infrastructure.

**Status:** functional end-to-end (both receive paths); see
`docs/IMPLEMENTATION_STATUS.md` for verified capabilities and known gaps.

## Scope and requirements

### Implemented

- File → animated QR GIF → file, byte-exact (SHA-256 verified), via GIF
  import on the receiving device.
- Camera receiving: point the camera at a screen playing a Pigeon GIF and
  collect fountain packets in real time, in any order.
- Versioned binary wire protocol (v1) with CRC32C packet integrity, random
  64-bit transfer ids, self-contained metadata packets, and periodic
  metadata re-emission for late joiners.
- RaptorQ (RFC 6330) fountain coding with configurable redundancy (0–200%);
  any sufficient subset of unique packets reconstructs the file in any
  order; duplicates and losses tolerated.
- Optional zlib compression with an auto heuristic (skips already-compressed
  formats) and strict decompression bounds.
- GIF import pipeline: streaming parse, per-frame QR decode, honest partial
  results.
- Local-first browser app: PWA (offline after first load), CSP with no
  remote origins, self-hosted WASM, workers for all heavy processing.
- Progressive, accessible UI (keyboard navigation, focus states, honest
  progress semantics, reduced-motion support).

### Explicit non-goals (by design)

- No encryption/authentication (protocol reserves space for a future
  additive scheme) — the GIF is plaintext; treat it as the secret itself.
- No backend, accounts, database, telemetry, or feedback channel from
  receiver to sender.
- No bulk-data ambitions: optical throughput is ~3–8 KiB/s; GIF containers
  cost ~24 KiB/frame. Pigeon targets secrets and small documents.

### Constraints

- Whole-object FEC requires the payload in memory: encoder default 64 MiB
  (configurable up to the 256 MiB cap); decoder accepts declared transfers
  up to 512 MiB.
- Camera scanning requires HTTPS or localhost (browser MediaDevices policy).
- QR decoding uses ZXing WASM in the browser; protocol validation always
  happens in the Rust core.

## Technology stack

- **Rust core** (`pigeon-core`): `raptorq` (RFC 6330 FEC), `fast_qr`
  (QR encode), `gif` (GIF encode/parse), `crc32c`, `sha2`, `flate2`,
  `getrandom`, `thiserror`, `serde`.
- **WASM bindings** (`pigeon-wasm`): `wasm-bindgen`, `serde-wasm-bindgen`,
  built with `wasm-pack` (web target).
- **Frontend**: React 19, TypeScript (strict), Vite, `zxing-wasm` (QR decode,
  self-hosted), `lucide-react` icons, `vite-plugin-pwa` (workbox), Vitest +
  Testing Library, Playwright (Chromium).

## Development and operation

Prerequisites: **Node.js ≥ 20 with npm** (the sole package manager — npm
workspaces wire the root to `apps/*`). The compiled WASM engine is committed
at `apps/web/src/wasm/pigeon-wasm/`; Rust with `wasm32-unknown-unknown` +
wasm-pack is only needed to regenerate it (`npm run build:wasm`).

```sh
# one-time setup
npm install

# develop / serve
npm run dev              # vite dev server → http://127.0.0.1:5173
npm run build            # production build → apps/web/dist
npm run preview          # serve dist → http://127.0.0.1:4173

# tests
npm run test:rust        # cargo test --workspace (lib + roundtrip suites)
cargo test -p pigeon-core --release --test bench -- --ignored --nocapture
npm run test:web         # vitest
npm run test:e2e         # playwright (starts preview server automatically)

# quality
npm run typecheck        # tsc -b
```

Key locations: `apps/web/src/` (UI), `crates/pigeon-core/src/` (engine),
`docs/` (protocol, architecture, security, threat model, benchmarks, status).

## Assumptions and limitations

- GIF size grows ≈ 24 KiB/frame at 700 px frames (measured — see
  `docs/BENCHMARKS.md`); the UI surfaces the frame count before download.
- Optical throughput at 4 fps is ~3 KiB/s with the default geometry.
- Real-device camera scanning has not been exercised (Playwright fake media
  stream only); see `IMPLEMENTATION_STATUS.md`.
