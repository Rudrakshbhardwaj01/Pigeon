# Security

Pigeon is a security-sensitive local file processor. This document states
what the implementation actually does and does not guarantee.

## Core properties

### Local processing (verifiable)

- Files are processed exclusively in browser memory (WASM + workers).
- No accounts, no backend, no telemetry, no analytics scripts.
- The bundle contains **no remote origins**: the CSP in `index.html` sets
  `default-src 'self'; connect-src 'self' blob: data:` and both WASM
  binaries (engine + ZXing) are bundled and precached locally. The service
  worker caches only the app's own assets. "Your files never leave this
  device" is enforced by `connect-src` and can be verified by inspecting the
  service worker and network log.

### Integrity (strong)

- **SHA-256** of the original file travels in the metadata packet; the
  receiver reconstructs, decompresses and verifies before offering the
  download. A mismatched file is never presented as success.
- **CRC32C** on every packet detects accidental corruption; corrupt packets
  are rejected before the recovery engine (counted, reported).
- Fountain coding tolerates missing frames; reconstruction failure is
  always explicit.

### Confidentiality (NOT provided)

- **Pigeon does not encrypt.** Anyone who receives the GIF can decode the
  file. The checksum authenticates *content*, not the *sender*: SHA-256 and
  CRC32C are not signatures. If a GIF could be swapped in transit (e.g. an
  attacker replaces it on a shared drive) they can substitute a different
  valid transfer with its own consistent manifest.
- The protocol reserves flag space for authenticated encryption
  (e.g. XChaCha20-Poly1305) as a future additive change; none is implemented.

### Input handling (hostile by default)

Every imported GIF, decoded QR payload and metadata field is treated as
untrusted:

- GIF: signature check, ≤ 256 MiB file, ≤ 2048 px frames, ≤ 20 000 frames,
  streaming parse (one frame in memory), malformed frames error instead of
  looping.
- Packets: exact-length checks, CRC32C, reserved-flag rejection, version
  rejection, payload ≤ 4096 bytes.
- Metadata: bounded filename (≤ 255 bytes) and MIME (≤ 127 bytes),
  compression-id whitelist, declared transfer size ≤ 512 MiB.
- Decompression: zlib output capped at the declared original size (read
  hard-limited, so bombs cannot allocate before the mismatch is detected).
- Filenames are sanitized again on receipt (path separators, control chars,
  Windows-illegal characters, leading dots, device names, char-boundary-safe
  length cap) — downloads cannot escape the chosen filename.
- Unknown protocol versions and unknown flag bits are hard errors, never
  silently salvaged.

### Resource exhaustion

Documented, enforced limits (see `docs/PROTOCOL.md` §2.2): packet payload,
transfer count (16), pre-metadata buffering (4096 packets), GIF caps,
decompression caps, encoder file cap. Decoding work is streamed
(frame-at-a-time) so hostile inputs are bounded per step, not just in total.

### Browser hygiene

- Restrictive CSP (`default-src 'self'`, `wasm-unsafe-eval` for
  instantiation, no `unsafe-eval`, no remote script or connect origins).
  `frame-ancestors` is intentionally not in the meta CSP (ignored there;
  set it in hosting headers if required).
- No `eval`, no `innerHTML` with untrusted data, no third-party scripts.
- Blob URLs revoked after use; camera tracks stopped on unmount; workers
  terminated with their pages.
- Worker messages are validated typed contracts; QR bytes cross workers as
  `Uint8Array`s without touching `document` scope.
- Error reporting shows failure categories, never file contents or raw
  packet dumps.

## What this does NOT protect against

1. A malicious recipient or sender (no authentication, no encryption).
2. Compromise of the browser itself (Pigeon runs with page privileges).
3. Physical observation of the screen/camera channel by a third party.
4. Denial of service via pathological inputs beyond the caps (the caps bound
   it; a 256 MiB hostile GIF still costs CPU to walk).
5. Tampering with the app bundle in transit — host Pigeon from a trusted
   origin (HTTPS); the service worker pins the first-installed version.

## Reporting

This is an open-source project without a paid security process; report
issues on the project tracker with reproduction steps.
