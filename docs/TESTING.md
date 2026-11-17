# Testing

## Test suites

| Suite | Location | Command | What it covers |
|---|---|---|---|
| Rust unit tests | `crates/pigeon-core/src/**` | `cargo test -p pigeon-core --lib` | protocol framing, CRC, metadata, sanitization, compression bounds, fountain, QR, GIF, recovery state machine, engine |
| Rust optical roundtrip | `crates/pigeon-core/tests/roundtrip.rs` | `cargo test -p pigeon-core --test roundtrip` | file → wire → QR → GIF → parse → QR-decode (rqrr) → ingest → reconstruct, byte-exact |
| Rust benchmarks | `crates/pigeon-core/tests/bench.rs` | `cargo test -p pigeon-core --release --test bench -- --ignored --nocapture` | measured timings/capacities (see `docs/BENCHMARKS.md`) |
| Large multi-block RaptorQ | `fountain.rs::multi_block_roundtrip_out_of_order` | `cargo test -p pigeon-core --release multi_block -- --ignored` | 8 MiB object, shuffled, out of order |
| Frontend unit tests | `apps/web/src/**/*.test.tsx` | `npm run test:web` | formatting, dropzone behavior (incl. GIF-only rejection), accessible components, progress states |
| Browser end-to-end | `apps/web/tests/e2e/roundtrip.spec.ts` | `npm run test:e2e` | real-browser roundtrip incl. offline operation |

`npm run test:rust` runs the Rust suites; `npm run test:web` and
`npm run test:e2e` run the frontend suites (all from the repo root).

## The roundtrip proof

The single most important test is the complete optical pipeline, executed in
two independent environments:

1. **Native** (`tests/roundtrip.rs`): encode → animated GIF → parse frames →
   QR-decode with `rqrr` (with deterministic frame drop, downscaling, frame
   shuffling, and multiple concurrent transfers) → reconstruct → compare
   bytes and SHA-256. Byte-exact equality is asserted.
2. **Browser** (`tests/e2e/roundtrip.spec.ts`): the same flow through the
   real UI — pick a file, encode, download the GIF from the download event,
   import it in the Receive tab, verify, download the reconstructed file and
   compare bytes with the fixture. Includes a test that performs the whole
   roundtrip **offline** after the service worker has cached the app
   (`context.setOffline(true)`).

## Scenarios covered

- Empty file (metadata-only transfer in v1; one sealed-object symbol in v2).
- 1-byte file.
- Binary data containing every byte value 0–255 (catches text/UTF-8
  assumptions in QR payloads).
- Unicode filenames (Chinese), Windows-illegal characters, over-long names.
- Compression: compressible payload actually compressed; media-like magic
  bytes skipped; bomb capped.
- Duplicates: same payload id twice → counted, progress not inflated.
- Out-of-order frames (shuffled sequences, reversed ingestion).
- Missing frames: 20% deterministic loss recovered with redundancy;
  insufficient frames reported as honestly incomplete (`take_file` returns
  `None`, phase stays `collecting`).
- Corrupted frames: bit flips rejected as `corrupted-content`; garbage
  rejected as `not-a-pigeon-packet`; both counted without breaking the
  transfer.
- Concurrent transfers: interleaved ingestion, no cross-contamination.
- Resource limits: oversized GIFs, excessive transfers, decompression bombs.
- Version compatibility: unknown version byte rejected explicitly; v1
  packets claiming encryption and v2 packets without the flag are rejected.
- **Encryption (protocol v2)**: full passphrase roundtrip (including
  passphrases entered before the first frame arrives); wrong-passphrase
  rejection with retry preserved; tampered ciphertext counted, never fatal;
  spliced packets cannot benefit the spoofed or the real transfer (AAD
  binding); KDF parameter caps (hostile 4 GiB envelope rejected before
  allocation); unknown cipher/KDF ids rejected; encrypted empty files;
  multi-transfer isolation with per-transfer salts; **encrypted optical
  roundtrip** through the real QR/GIF pipeline with the filename verified
  hidden pre-decryption; estimator exactness against a real encode.
- Browser: wasm engine initializes, no page errors on load, non-GIF files
  rejected with a visible message, valid-but-foreign GIFs reported honestly,
  camera tab opens (Playwright fake media stream), **encrypted transfer
  roundtrips in the real UI with wrong-passphrase rejection then byte-exact
  recovery**.

## Reproducing the loss-simulation scenarios

```sh
# native: 20% deterministic frame loss recovered by FEC
cargo test -p pigeon-core --test roundtrip optical_roundtrip_with_frame_loss -- --nocapture

# native: insufficient frames → honest incomplete state
cargo test -p pigeon-core --test roundtrip incomplete_transfer_reports_honestly

# native: encrypted transfer through the full optical pipeline
cargo test -p pigeon-core --test roundtrip encrypted_optical_roundtrip_with_passphrase

# browser: full roundtrip offline (service worker precache required)
npm run test:e2e -- -g offline

# browser: encrypted roundtrip incl. wrong-passphrase rejection
npm run test:e2e -- -g encrypted

# browser: incomplete GIF resolves with an honest partial report
npm run test:e2e -- -g "incomplete Pigeon GIF"
```

## Camera testing caveat

Camera tests use Playwright's Chromium fake media stream; they verify that
the camera path initializes and decodes without a physical camera. **Real
device camera scanning was not performed as part of this delivery.**
