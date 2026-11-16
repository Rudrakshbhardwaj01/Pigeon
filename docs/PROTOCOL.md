# Pigeon Wire Protocol — v1

Status: **Stable** (implemented in `crates/pigeon-core/src/protocol/`).
Encoding: all multi-byte integers are **little-endian** unless noted.

---

## 1. Overview

A Pigeon transfer is a stream of self-contained **packets**. Each packet is
encoded into exactly one QR code; QR frames appear as frames of an animated
GIF (file → GIF → file), which a receiver can either import directly or
capture optically with its camera while the GIF loops on a screen. Every QR
frame is independently decodable — there are no inter-frame dependencies.

Two packet types exist:

1. **Metadata packet** — describes the transfer (file name, size, checksum,
   compression, FEC parameters). Sent first and re-sent periodically so that
   receivers joining mid-loop still learn the parameters.
2. **Symbol packet** — carries one RaptorQ (RFC 6330) encoding symbol.

The receiver reconstructs the file from *any* sufficient set of unique
symbol packets, in any order, with duplicates and losses tolerated.

## 2. Packet layout

Every packet has a fixed 20-byte header followed by the payload:

```text
offset  size  field
0       2     magic = 0x50 0x47 ("PG")
2       1     protocol version (v1 = 0x01)
3       1     flags
4       8     transfer id (u64 LE, random per transfer)
12      4     payload length N (u32 LE)
16      4     CRC32C checksum (u32 LE)
20      N     payload
```

**CRC32C (Castagnoli)** is computed over bytes `[2..16]` (version, flags,
transfer id, payload length) **followed by the payload**. The magic and the
CRC field itself are outside the checksum, so a scanner can locate packets by
magic and verify them in a single pass.

### 2.1 Flags byte

```text
bit 0      : 1 = metadata packet, 0 = symbol packet
bits 1-2   : FEC scheme (0 = RaptorQ per RFC 6330)
bits 3-7   : reserved — must be 0 in v1; receivers reject unknown bits
```

### 2.2 Limits (v1)

| Limit | Value |
|---|---|
| Maximum packet payload | 4096 bytes |
| Filename (UTF-8, sanitized) | ≤ 255 bytes |
| MIME type | ≤ 127 bytes |
| Encoder file-size cap (hard) | 256 MiB |
| Decoder declared-transfer cap (hard) | 512 MiB |
| Decompression output cap (hard) | 512 MiB |
| GIF file size on import | ≤ 256 MiB |
| GIF frame dimensions on import | ≤ 2048 px per side |
| GIF frame count on import | ≤ 20 000 |
| Tracked concurrent transfers | ≤ 16 |
| Symbols buffered before metadata | ≤ 4096 |

## 3. Symbol packet payload

```text
offset  size  field
0       4     payload id (u32 BE) — raptorq PayloadId serialization
4       T     RaptorQ encoding symbol bytes (exactly one symbol, T bytes)
```

The `payload id` is the big-endian serialization of
`PayloadId::serialize()` from the `raptorq` crate: source block number in
the top byte, encoding symbol id (ESI) in the low 24 bits. A payload id
uniquely identifies one symbol of one source block; receivers use it for
deduplication.

The symbol size `T` is chosen at encode time from the QR capacity (see §5)
and is recorded in the metadata packet's OTI. Each symbol packet carries
exactly one symbol, so a symbol packet's payload is `4 + T` bytes.

## 4. Metadata packet payload

```text
offset  size  field
0       12    RaptorQ object transmission information (OTI, crate format)
12      1     compression id (0 = none, 1 = zlib)
13      8     original file size (u64 LE) — size after decompression
21      32    SHA-256 of the original file
53      2     filename length F (u16 LE, bytes)
55      2     MIME length M (u16 LE, bytes)
57      F     filename (UTF-8)
57+F    M     MIME type (ASCII, lowercase)
```

The 12-byte OTI is the `ObjectTransmissionInformation::serialize()` output:
transfer length (u64), symbol size (u16), sub-block count (u16), source
blocks (u8), symbol alignment (u8). It is authoritative: the decoder derives
the symbol size, block structure and required symbol count from it.

**Empty files** (original size 0) are complete upon receiving the metadata
packet alone; no symbol packets exist for such transfers. The OTI in that
case uses explicit parameters (transfer length 0, symbol size 8, 1 block,
alignment 1) because the raptorq library rejects zero-length objects.

### 4.1 Filename sanitization

Filenames are untrusted input. The encoder sanitizes its own input and the
decoder sanitizes again on receipt: path separators and characters illegal
on Windows (`/ \ : < > " | ? *`) and control characters become `_`; leading
dots/whitespace are stripped (blocking `..` traversal and hidden files);
Windows device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9) are prefixed;
length is capped at 255 bytes on a UTF-8 char boundary; empty results become
`pigeon-file`.

## 5. Frame sequencing

The transmit sequence is:

1. metadata packet,
2. all symbol packets (source symbols first per block, then repair symbols;
   blocks interleaved round-robin so partial capture spreads loss evenly),
3. the metadata packet re-inserted every 500 symbol frames for long
   transfers.

Loops (GIF playback) repeat the whole sequence forever. A receiver therefore
reaches the metadata within one loop even when it joined late or captures
optically. All packet identities are stable across loops, so duplicate
metadata packets are idempotent.

## 6. Reconstruction rules

- A transfer is identified by its 64-bit transfer id; transfers never mix.
- Duplicates (same payload id) are ignored and counted separately — they do
  not inflate progress.
- Symbol packets that arrive before the metadata packet are buffered (bounded
  at 4096) and replayed once the OTI arrives.
- RaptorQ decoding succeeds once every source block has enough symbols
  (typically K + 1–2% overhead). Reconstruction failure is never hidden.
- After RaptorQ succeeds: decompression (if zlib, with a strict output cap =
  declared original size) then SHA-256 verification. **A file is offered for
  download only when the SHA-256 matches.**

## 7. Version compatibility

- Receivers MUST reject packets whose version byte differs from 0x01 with an
  explicit `unsupported protocol version` error (never silent salvage).
- Receivers MUST reject packets with unknown flag bits set.
- Unknown compression ids and unknown FEC schemes are hard errors.
- Forward-compatible additions must use the reserved flag space and bump the
  version byte when wire-incompatible.
