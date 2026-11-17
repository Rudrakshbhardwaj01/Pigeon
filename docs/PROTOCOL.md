# Pigeon Wire Protocol — v1 and v2

Status: **Stable** (implemented in `crates/pigeon-core/src/protocol/` and
`crates/pigeon-core/src/crypto/`).
Encoding: all multi-byte integers are **little-endian** unless noted.

Two protocol versions exist:

- **v1** — the original dialect; payloads are plaintext. Unencrypted
  transfers are emitted as v1 and remain byte-compatible with older
  receivers.
- **v2** — the encrypted dialect; every payload is an authenticated-encryption
  envelope (§8). v2 requires flag bit 3 set; v1 forbids it. A receiver that
  does not understand v2 rejects v2 packets with an explicit
  unsupported-version error (never silent salvage).

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
bit 3      : 1 = payload is an encryption envelope (v2 only; v1 forbids it)
bits 4-7   : reserved — must be 0 in v1 and v2; receivers reject unknown bits
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

- Receivers MUST reject packets whose version byte is neither 0x01 nor 0x02
  with an explicit `unsupported protocol version` error (never silent
  salvage).
- v1 packets MUST NOT carry flag bit 3 (encryption); v2 packets MUST carry
  it. Mismatched combinations are hard errors.
- Receivers MUST reject packets with unknown flag bits set.
- Unknown compression ids, unknown FEC schemes, unknown cipher ids, unknown
  KDF ids and out-of-range KDF parameters are hard errors.
- Forward-compatible additions must use the reserved flag space and bump the
  version byte when wire-incompatible.

## 8. Encrypted transfers (v2)

Encrypted transfers wrap **every** packet payload in an authenticated
encryption envelope before the packet header is written. The construction
uses only established primitives (RustCrypto crates; no custom cryptography):

- **KDF:** Argon2id, 32-byte output key. Cost parameters travel in the
  plaintext metadata envelope (§8.2) and are validated against hard caps
  (memory 8–64 MiB, passes 1–16, lanes 1–8) **before** any derivation runs —
  a hostile sender cannot force an unbounded receiver allocation. Defaults:
  m = 19456 KiB, t = 2, p = 1 (OWASP recommendation).
- **AEAD:** XChaCha20-Poly1305, fresh random 192-bit nonce per sealed value.
  The Poly1305 tag authenticates ciphertext **and associated data**.

### 8.1 Associated data — the transfer binding

The AEAD associated data is the packet header bytes `[2..16]` — protocol
version, flags, transfer id and payload length. Consequences:

- A packet sealed for transfer A cannot be accepted by a receiver tracking
  it as transfer B (the tag binds the transfer id).
- Version, flags and payload length cannot be tampered with without breaking
  authentication. CRC32C covers the same range as a fast pre-check; the AEAD
  tag is the cryptographic guarantee.

### 8.2 Metadata envelope (v2 metadata packets)

```text
offset  size  field
0       1     envelope version (= 1)
1       1     cipher id (1 = XChaCha20-Poly1305)
2       1     KDF id (1 = Argon2id)
3       4     Argon2id memory cost, KiB (u32 LE)
7       4     Argon2id passes (u32 LE)
11      4     Argon2id lanes (u32 LE)
15      16    KDF salt
31      24    per-packet nonce (random)
55      2     inner payload length N (u16 LE)
57      N     ciphertext of the inner payload
        16    Poly1305 tag (follows the ciphertext)
```

The inner payload is exactly the §4 metadata payload (OTI, compression id,
original size, SHA-256, filename, MIME). Because it is sealed, the receiver
learns **nothing** about the file (name, size, type, checksum) until it
derives a key from the passphrase. The plaintext leakage surface of an
encrypted transfer is: transfer id, salt, KDF parameters, envelope/packet
sizes (and therefore coarse volume), and timing.

### 8.3 Symbol envelope (v2 symbol packets)

```text
offset  size  field
0       1     envelope version (= 1)
1       1     cipher id (1 = XChaCha20-Poly1305)
2       24    per-packet nonce (random)
26      2     inner payload length N (u16 LE)
28      N     ciphertext of the inner payload
        16    Poly1305 tag (follows the ciphertext)
```

The inner payload is exactly the §3 symbol payload (payload id + symbol).

### 8.4 Object sealing

Independently of the per-packet envelopes, the whole transmitted object —
the file after optional compression — is sealed once:

```text
sealed object = object nonce (24 B) || ciphertext || tag (16 B)
```

with the transfer id (u64 LE) as associated data. The sealed object is what
the fountain code transmits; the OTI therefore describes the sealed length.
Reconstruction yields the sealed blob, which is opened (authenticating
against the transfer id) **before** decompression and checksum verification.
The object tag rides at the end of the transmitted data and needs no extra
metadata field.

### 8.5 Receive rules for v2

- Envelope prefixes are parsed before decryption (ids, KDF caps, length
  consistency); violations are hard errors.
- A transfer whose metadata envelope has been seen but whose passphrase has
  not been provided pauses in an `awaiting-passphrase` state. Sealed symbol
  packets arriving before the key exist are buffered (same 4096 cap as v1
  pre-metadata buffering) and replayed once a key is derived.
- Authentication failure on a symbol packet (wrong passphrase, corruption)
  is counted per transfer and never mutates decoder state; a wrong
  passphrase keeps the gate open for retry.
- Deduplication of sealed packets is impossible before the key exists
  (payload ids are inside the ciphertext); duplicate detection resumes after
  decryption. This is a documented trade-off, bounded by the buffering cap.
- Empty files: an encrypted transfer of an empty file still transmits its
  40-byte sealed object (nonce + tag) as one symbol — unlike v1, which
  completes from metadata alone — so the receiver authenticates the object.

### 8.6 QR geometry impact

The per-packet envelope adds 44 bytes of wire overhead (envelope header +
tag), so v2 transfers select a slightly larger QR version for the same
symbol size target. v1 geometry is unchanged.
