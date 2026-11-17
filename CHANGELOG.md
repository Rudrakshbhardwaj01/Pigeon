# Changelog

All notable changes to Pigeon are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the project aims for [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The wire protocol has its own version byte — see `docs/PROTOCOL.md` — and
protocol changes are called out explicitly in the entries below.

## [Unreleased]

### Added

- **Encrypted transfers (protocol v2)** — optional passphrase protection for
  the whole transfer:
  - Argon2id key derivation (OWASP-recommended defaults: m = 19 MiB, t = 2,
    p = 1) with the KDF parameters transmitted in the plaintext metadata
    envelope and hard-capped on receipt before any derivation runs.
  - XChaCha20-Poly1305 authenticated encryption at two layers: the whole
    (possibly compressed) object is sealed once (nonce + ciphertext + tag,
    bound to the transfer id) before fountain coding, and every wire packet's
    payload is individually sealed with a fresh random 192-bit nonce.
  - Packet headers `[2..16]` (version, flags, transfer id, length) are the
    AEAD associated data, so packets cannot be moved between transfers or
    have their identity tampered with without failing authentication.
  - File contents, filename, MIME type, size and checksum are all inside the
    encrypted envelope — metadata is protected by default.
  - Wrong passphrases are rejected without killing the receive session: the
    transfer pauses at `awaiting-passphrase` and can be retried.
  - Unencrypted transfers remain byte-compatible protocol v1; older receivers
    reconstruct them unchanged and reject v2 packets with an explicit
    unsupported-version error.
- **Encryption UI** — passphrase entry with confirmation, show/hide, honest
  length-based strength hints, an encrypted-transfer badge in the results,
  and a passphrase gate on both GIF import and camera receiving.
- **Transfer presets** — Balanced, Maximum reliability, Faster transmission
  and Camera-friendly, each mapping to concrete engine parameters; manual
  changes select "Custom" so the active preset never lies.
- **Pre-encode transfer estimator** — QR geometry, upper-bound frame count
  (with an optimistic compressible bound), loop duration and a GIF size
  estimate with stated assumptions, computed by the engine (not guessed in
  the UI) and refreshed live as parameters change.
- **Continuous integration** — GitHub Actions workflow covering rustfmt,
  clippy (`-D warnings`), the Rust test suite, a wasm-pack compile check,
  TypeScript type check, ESLint, Vitest, the production build, the full
  Playwright suite (including the offline roundtrip), and advisory dependency
  audits. Rust toolchain pinned via `rust-toolchain.toml`.
- **Open-source infrastructure** — this changelog, a contribution guide
  (`CONTRIBUTING.md`), a security policy (`SECURITY.md`), and issue templates.

### Changed

- The QR geometry for encrypted transfers selects a slightly larger QR
  version to hold the per-packet envelope (unencrypted geometry is
  unchanged).
- The service-worker cache version is unchanged; the WASM engine gains the
  crypto module and grows accordingly.

### Security

- `docs/SECURITY.md` and `docs/THREAT_MODEL.md` updated for the encrypted
  mode, including the documented leakage surface (timing, volume, transfer
  id, KDF parameters, salt) and the limits of browser-side key hygiene.

## [0.1.0] — 2026-11-16

Initial release.

- Offline-first optical file transfer: file → animated QR GIF → file,
  byte-exact, SHA-256 verified.
- Binary wire protocol v1: 20-byte header (magic, version, flags, transfer
  id, payload length, CRC32C) with strict parse rules and resource limits.
- RaptorQ (RFC 6330) fountain coding with configurable redundancy (0–200 %),
  round-robin source-block interleaving, duplicate handling and bounded
  pre-metadata buffering.
- Streaming QR generation (fast_qr) and animated GIF assembly with a global
  two-colour palette; streaming GIF parsing with hostile-input caps.
- Optional zlib compression with a magic-byte auto heuristic and hard
  decompression bounds.
- React + TypeScript web application with dedicated encode/decode/camera Web
  Workers, live progress, honest incomplete-transfer reporting, camera
  device selection and cancellation.
- Fully offline PWA: custom precache service worker (Vary-free responses),
  self-hosted ZXing QR decoder, strict CSP with no remote origins.
- Test suites: Rust unit + native optical roundtrips (loss, reordering,
  downscaling, multi-transfer), Vitest, and Playwright E2E including a
  complete offline roundtrip.
