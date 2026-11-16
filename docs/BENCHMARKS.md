# Benchmarks

All numbers below are **real measurements** produced by the benchmark harness
in `crates/pigeon-core/tests/bench.rs`. Nothing on this page is estimated.

Reproduce them yourself:

```sh
cargo test -p pigeon-core --release --test bench -- --ignored --nocapture
```

**Test conditions:** Windows 11 x64, Rust 1.99.1 (release profile, `opt-level = 3`,
LTO), `pigeon-core` 0.1.0. The QR decoder in this harness is `rqrr` (test-only
dependency); the browser uses ZXing WASM, which has different (generally
better) decode performance.

## Results

### Byte-mode QR capacity (bytes per frame, measured)

| Version | V10 | V15 | V20 | V21 | V23 | V25 | V26 | V29 | V33 | V40 |
|---|---|---|---|---|---|---|---|---|---|---|
| ECL L | 271 | 520 | 858 | 929 | 1091 | 1273 | 1367 | 1628 | 2068 | 2953 |
| ECL M | 213 | 412 | 666 | 711 | 857 | 997 | 1059 | 1264 | 1628 | 2331 |

Per-frame wire overhead is 24 bytes (20-byte header + 4-byte payload id), so
a V23-L frame carries 1091 − 24 = 1067 payload bytes.

### Encode 1 MiB, incompressible data, 30% redundancy (auto QR version → V23-L)

| Stage | Time | Notes |
|---|---|---|
| Fragment + fountain encode | 128 ms | 1284 frames (987 source + repair) |
| QR generation | 1995 ms | ≈ 0.6 frames/ms (1.6 ms/frame) |
| GIF assembly (702 px frames) | 19 909 ms | ≈ 16 ms/frame, dominated by LZW |
| **GIF size** | **30 410 KiB** | ≈ 23.7 KiB/frame |

The GIF assembly cost and size scale linearly with frame count; both are
dominated by GIF LZW compression of full-frame bitmaps. This is the honest
cost of the "GIF as a portable container" design (see §Practical limits).

### Decode 1 MiB transfer, 20% simulated frame loss

| Metric | Value |
|---|---|
| Frames parsed + QR-decoded | 1284 parsed, 1028 decoded in 109 848 ms (rqrr) |
| Reconstruction | verified (SHA-256 match) after sufficient unique packets |

The browser's ZXing WASM decoder is faster than rqrr per frame; the
end-to-end browser roundtrip (encode → download → import → verified
reconstruction) completes in ~1.5 s for a 291-byte file and ~3 s for a 10 KB
binary file on the same machine (see `apps/web/tests/e2e/roundtrip.spec.ts`).

### Multi-block RaptorQ (release mode)

An 8 MiB object (8192 symbols, single block partitioning) round-trips
shuffled and out-of-order in **0.37 s** (`multi_block_roundtrip_out_of_order`,
`cargo test -p pigeon-core --release multi_block -- --ignored`).

## Practical limits derived from these measurements

- **GIF mode is best for small files.** At ~24 KiB/frame (700 px frames), a
  100 KB file with 30% redundancy produces ≈ 130 frames ≈ 3 MB of GIF. A
  1 MiB file produces ≈ 30 MB of GIF. The UI shows the frame count before
  download so the trade-off is visible.
- **QR generation is not the bottleneck.** Matrices encode on the fly at
  ~1.6 ms/frame; the GIF container and the scanning step dominate.
- **Optical throughput** at 4 fps with ~1067-byte payloads and ~80% capture
  is ≈ 3.4 KiB/s — comparable to AirQR's documented ~8 KB/s (different frame
  geometry). Optical transfer is for secrets and small documents, not bulk
  data.
- **Decoder headroom**: 20% frame loss is comfortably recovered at 30%
  redundancy for transfers of a few hundred frames; see the deterministic
  loss tests (`optical_roundtrip_with_frame_loss`).

## Known measurement caveats

- Debug builds are 50–100× slower for RaptorQ; always benchmark release
  builds. The large multi-block test is `#[ignore]`d in debug for this reason.
- GIF LZW output depends on frame dimensions; smaller scales shrink GIFs
  super-linearly but reduce scanner tolerance.
