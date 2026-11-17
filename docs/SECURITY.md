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

### Confidentiality (mode-dependent)

**Unencrypted transfers (protocol v1) do not encrypt.** Anyone who receives
the GIF can decode the file. The checksum authenticates *content*, not the
*sender*: SHA-256 and CRC32C are not signatures. If a GIF could be swapped
in transit (e.g. an attacker replaces it on a shared drive) they can
substitute a different valid transfer with its own consistent manifest.
Treat an unencrypted GIF as the secret itself.

**Encrypted transfers (protocol v2, optional)** provide authenticated
confidentiality with established primitives:

- **KDF:** Argon2id, 32-byte key, OWASP-recommended defaults (m = 19 MiB,
  t = 2, p = 1). The parameters and salt travel in the plaintext metadata
  envelope so future encoders can raise costs; receivers validate them
  against hard caps (memory ≤ 64 MiB, passes ≤ 16, lanes ≤ 8) before any
  allocation, so a hostile sender cannot force an unbounded derivation.
- **AEAD:** XChaCha20-Poly1305 with a fresh random 192-bit nonce per sealed
  value (object layer and every packet layer).
- **Transfer binding:** the AEAD associated data is the packet header bytes
  `[2..16]` — version, flags, transfer id, length. Packets cannot be moved
  between transfers or have their identity tampered with without failing
  authentication; the reconstructed object is likewise bound to the transfer
  id before decompression.
- **Metadata protection:** filename, MIME type, size, checksum and FEC
  parameters are inside the encrypted metadata envelope. Until the correct
  passphrase is provided, the receiver cannot read any of them (the UI
  shows an encrypted-transfer gate, not partial information).
- **Ordering:** authentication happens before plaintext use at every layer —
  packet tags before decoder state changes, the object tag before
  decompression, the SHA-256 before the download is offered.
- **Failure behaviour:** a wrong passphrase is rejected without destroying
  receive state; the transfer pauses in `awaiting-passphrase` and can be
  retried. Per-packet authentication failures are counted and surfaced.

**What encryption does NOT provide (documented limits):**

1. **Sender authentication.** A passphrase is shared knowledge; anyone with
   it can create a valid encrypted transfer. Possession of the passphrase
   authenticates nothing about the sender's identity.
2. **Passphrase strength.** The construction is only as strong as the
   passphrase; the KDF raises attack cost but cannot rescue "1234". The UI
   states this honestly.
3. **Metadata leakage.** Transfer id, salt, KDF parameters, packet volume
   and timing remain visible — they are inherently outside the envelope.
4. **Memory erasure guarantees.** Passphrases and keys are zeroized on drop
   where the runtime allows, but the browser cannot guarantee that no
   intermediate copy ever existed (see also the WASM linear-memory model).
5. **Channel observation.** The optical channel is broadcast; a camera in
   the room captures what a screen shows regardless of encryption.

### Input handling (hostile by default)

Every imported GIF, decoded QR payload and metadata field is treated as
untrusted:

- GIF: signature check, ≤ 256 MiB file, ≤ 2048 px frames, ≤ 20 000 frames,
  streaming parse (one frame in memory), malformed frames error instead of
  looping.
- Packets: exact-length checks, CRC32C, reserved-flag rejection, version
  rejection, payload ≤ 4096 bytes.
- Envelopes (v2): cipher/KDF id whitelists, KDF parameter caps before
  allocation, exact inner-length consistency, AEAD authentication before any
  plaintext reaches decoder state.
- Metadata: bounded filename (≤ 255 bytes) and MIME (≤ 127 bytes),
  compression-id whitelist, declared transfer size ≤ 512 MiB.
- Decompression: zlib output capped at the declared original size (read
  hard-limited, so bombs cannot allocate before the mismatch is detected).
- Filenames are sanitized again on receipt (path separators, control chars,
  Windows-illegal characters, leading dots, device names, char-boundary-safe
  length cap) — downloads cannot escape the chosen filename.
- Unknown protocol versions, unknown flag bits, unknown cipher/KDF ids and
  out-of-range KDF parameters are hard errors, never silently salvaged.

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

1. A malicious sender or recipient: encryption proves knowledge of the
   passphrase, not a real-world identity. For sender binding, verify the
   SHA-256 out-of-band (encrypted or not) or hand-deliver the passphrase
   over a different channel than the GIF.
2. Weak passphrases: the KDF raises the cost of guessing but cannot rescue
   one. Length and uniqueness remain the user's responsibility.
3. Compromise of the browser itself (Pigeon runs with page privileges).
4. Physical observation of the screen/camera channel by a third party.
5. Denial of service via pathological inputs beyond the caps (the caps bound
   it; a 256 MiB hostile GIF still costs CPU to walk).
6. Tampering with the app bundle in transit — host Pigeon from a trusted
   origin (HTTPS); the service worker pins the first-installed version.

## Reporting

See [`SECURITY.md`](../SECURITY.md) at the repository root for the
vulnerability reporting process and scope.
