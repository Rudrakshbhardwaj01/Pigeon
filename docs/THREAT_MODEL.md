# Threat Model

Scope: the Pigeon web application (browser), its WASM engine, and the
optical/file channels it consumes. Out of scope: the hosting infrastructure
(beyond the bundle-integrity note), the user's device security, and
post-download handling of files.

## Assets

1. User files (confidentiality + integrity).
2. The integrity of the transfer (receiver reconstructs what the sender
   sent).
3. User privacy (no leakage of file contents or transfer metadata to third
   parties).
4. Browser session health (no hangs, exhaustion, or persistent compromise).

## Actors

| Actor | Capability | Interest |
|---|---|---|
| Sender / receiver | legitimate users | move a file exactly |
| Eavesdropper | sees the screen, the GIF file, or the camera view | read the file (it is unencrypted) |
| Interceptor | modifies the GIF in transit (shared drive, messaging platform) | substitute a payload |
| Malformed-input author | crafts GIFs/QR payloads targeting the decoder | crash, exhaust, or corrupt the receiver |
| Platform (messaging app) | re-encodes media, resizes frames, converts to video | not adversarial, but lossy |

## Threats and mitigations

### T1 — File disclosure through the GIF (HIGH likelihood, HIGH impact)
The GIF is plaintext by design; anyone holding it can reconstruct the file.
**Mitigation: none inside Pigeon** — the UI and README state it plainly.
Users must treat the GIF as the secret itself (hand-delivery, encrypted
container around it, or future protocol-level encryption).

### T2 — Transfer substitution (MEDIUM likelihood, HIGH impact)
An interceptor swaps the GIF; a substitute carries its own valid SHA-256
manifest, so the receiver cannot detect substitution from checksums alone.
**Mitigation: out-of-band comparison.** The UI surfaces the sender's SHA-256
before download; a receiver who can verify the hash out-of-band (e.g. read
over phone) defeats substitution. Future: authenticated encryption binds the
manifest to a sender key.

### T3 — Malicious GIF / packet input (HIGH likelihood, MEDIUM impact)
Crafted input aims to exhaust memory/CPU or corrupt state.
**Mitigations:** strict caps (GIF 256 MiB / 2048 px / 20k frames; packets
4096 B payload; 16 transfers; 4096 buffered pre-metadata), streaming frame
handling, exact-length + CRC validation before state changes, bounded zlib
output, version/flag rejection. Failures surface as errors, never silent
salvage. Residual risk: CPU time spent rejecting hostile input is bounded by
the caps but not zero.

### T4 — Filename attacks (MEDIUM likelihood, LOW impact)
`../../.bashrc`, device names, over-long names, control characters.
**Mitigation:** double sanitization (encoder + receiver), traversal-safe
transformation, device-name prefixing, UTF-8-boundary length cap; the UI
never uses received names for paths beyond the download filename.

### T5 — Decompression bombs (MEDIUM likelihood, MEDIUM impact)
A tiny compressed object declaring a huge original size.
**Mitigation:** decompressor reads at most `declared_size + 1` bytes
(hard-limited before allocation grows), rejects size mismatch, hard cap
512 MiB, and the decoder rejects declared transfers above the cap outright.

### T6 — Messaging-platform corruption (HIGH likelihood, LOW–MEDIUM impact)
WhatsApp/Telegram re-encode GIFs to video, resize or drop frames.
**Mitigation:** full-frame two-colour frames (no inter-frame dependency),
fountain redundancy over frames, honest partial-failure reporting, and
explicit guidance to send as document/file. Detected corruption is reported
per category (`corrupted-content`, `not-a-pigeon-packet`).

### T7 — Bundle/network compromise (LOW likelihood, HIGH impact)
If the app is served from a hostile origin or a CDN adds a script, the
"local-only" guarantee dies.
**Mitigation:** no third-party origins in the bundle; CSP `default-src
'self'` with no remote `connect-src`; service worker precache pins assets
after first install. Users should serve over HTTPS or localhost.

### T8 — Resource abuse via concurrent transfers (LOW likelihood, LOW impact)
Many simultaneous transfers inflate memory. **Mitigation:** 16-transfer
limit with a hard error (`too many concurrent transfers`), reset action in
the UI.

### T9 — Screen/camera channel observation (contextual, HIGH impact)
The optical channel is broadcast: anything visible on the sender's screen is
visible to cameras in the room. **Mitigation: none possible in software;**
documented as inherent to the medium.

## Assumptions

- The user's browser and OS are not compromised.
- SHA-256 and CRC32C remain cryptographically/behaviorally sound for their
  roles (content identity / accidental-corruption detection).
- The user serves the app from an origin they trust.
