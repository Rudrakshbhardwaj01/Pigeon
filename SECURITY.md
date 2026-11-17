# Security Policy

## Supported versions

Only the latest release branch receives security fixes. Pigeon is a
client-side application; there is no server component to patch. Users should
run the newest published version — the PWA service worker pins the first
installed build, so re-visit the app once to pick up updates.

## Reporting a vulnerability

**Do not open a public issue for a security vulnerability.**

Use GitHub's *Report a vulnerability* button on the repository's Security
tab (Private vulnerability reporting). If that is unavailable, contact the
maintainer directly and include:

- A description of the issue and its impact.
- Reproduction steps or a proof of concept (crafted GIFs, packet bytes, or
  payloads are far more useful than descriptions alone).
- The affected version/commit and browser(s) tested.

You will get an acknowledgement within a week and a judgement of severity
within two weeks. Fixes will be released as soon as practical, with credit
unless you prefer anonymity.

## Scope

In scope:

- The Rust engine's parsing and reconstruction paths (`crates/pigeon-core`):
  packet framing, metadata, GIF import, decompression, reconstruction,
  the cryptographic envelope (protocol v2).
- The WASM boundary (`crates/pigeon-wasm`).
- The web application's browser security posture: CSP, service worker,
  worker messaging, storage, download handling, offline behaviour.
- Breaks of the local-only promise: any runtime network request, any file
  content leaving the device.

Out of scope:

- Compromise of the user's browser/OS (Pigeon runs with page privileges).
- Physical observation of the optical channel (the GIF is broadcast by
  design; see `docs/THREAT_MODEL.md` T1/T9). Unencrypted transfers are
  plaintext by definition — this is documented, not a vulnerability.
- The hosting origin's TLS/bundle integrity when served by a third party.
- Social engineering of users into sharing passphrases over the same channel
  as the GIF.

## Design guarantees and known limits

The full statements live in [`docs/SECURITY.md`](docs/SECURITY.md) and
[`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md). Summary:

- Integrity: SHA-256 manifest verification gates every download; CRC32C and
  AEAD authentication happen before any state change.
- Confidentiality (encrypted transfers): Argon2id + XChaCha20-Poly1305 with
  the transfer identity as associated data. Metadata is inside the envelope.
  Leakage surface: timing, volume, transfer id, KDF parameters, salt.
- Confidentiality (unencrypted transfers): none — treat the GIF as the
  secret itself.
- Key material hygiene: passphrases are zeroized after use where the runtime
  allows; the browser cannot guarantee erasure of every intermediate copy.
