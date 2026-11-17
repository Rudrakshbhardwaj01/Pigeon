//! Optional zlib compression with an auto heuristic and strict limits.

use crate::error::PigeonError;
use crate::protocol::metadata::Compression;
use crate::protocol::MAX_DECOMPRESSED_SIZE;

use serde::Serialize;

/// User-facing compression selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompressionMode {
    /// Compress unless the content looks already compressed or won't shrink.
    Auto,
    /// Always attempt compression (still bounded by the shrink heuristic).
    On,
    /// Never compress.
    Off,
}

/// Signatures of formats that are already compressed; re-compressing wastes CPU.
const COMPRESSED_MAGIC: [&[u8]; 14] = [
    b"PK\x03\x04",       // zip (and zip-based office formats)
    b"\x1f\x8b",         // gzip
    b"\x89PNG",          // png
    b"7z\xbc\xaf",       // 7z
    b"\xfd7zXZ",         // xz
    b"\x28\xb5\x2f\xfd", // zstd
    b"OggS",             // ogg
    b"fLaC",             // flac
    b"Rar!",             // rar
    b"BZh",              // bzip2
    b"\x00\x00\x00",     // mp4 starts with a size box (ftyp follows); media is compressed
    b"ftyp",
    b"ID3",  // mp3 metadata
    b"wOFF", // woff fonts
];

/// Heuristic: does this start with a signature of an already-compressed format?
fn looks_compressed(data: &[u8]) -> bool {
    COMPRESSED_MAGIC
        .iter()
        .any(|m| data.len() >= m.len() && &data[..m.len()] == *m)
}

/// JPEG has no fixed magic string long enough above; check its own.
fn is_jpeg(data: &[u8]) -> bool {
    data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF
}

/// Prepares the payload for FEC encoding.
///
/// Returns the data to transmit plus the compression algorithm actually used.
/// In `Auto` mode compression is skipped for files that look already compressed
/// or shrink by less than 5%. Output size is always bounded by the input size
/// plus a small margin (zlib cannot expand arbitrary input unboundedly, and we
/// discard any result that grew).
pub fn prepare_payload(
    data: &[u8],
    mode: CompressionMode,
) -> Result<(Vec<u8>, Compression), PigeonError> {
    if data.is_empty() || mode == CompressionMode::Off {
        return Ok((data.to_vec(), Compression::None));
    }
    if mode == CompressionMode::Auto && (looks_compressed(data) || is_jpeg(data)) {
        return Ok((data.to_vec(), Compression::None));
    }

    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(data.len()),
        flate2::Compression::fast(),
    );
    encoder
        .write_all(data)
        .map_err(|e| PigeonError::CompressionFailed(e.to_string()))?;
    let compressed = encoder
        .finish()
        .map_err(|e| PigeonError::CompressionFailed(e.to_string()))?;

    // Keep compression only when it clearly wins.
    let worthwhile = compressed.len() < data.len().saturating_mul(95) / 100 && data.len() > 1024;
    if compressed.len() > data.len() || !worthwhile {
        return Ok((data.to_vec(), Compression::None));
    }
    Ok((compressed, Compression::Zlib))
}

/// Decompresses zlib data with a strict output cap and exact-size check.
///
/// `expected` is the declared original size from the (untrusted) metadata; the
/// decompressor is hard-limited to it plus one byte so a bomb cannot allocate
/// unbounded memory before the mismatch is detected.
pub fn decompress_bounded(compressed: &[u8], expected: u64) -> Result<Vec<u8>, PigeonError> {
    use std::io::Read;
    if expected > MAX_DECOMPRESSED_SIZE {
        return Err(PigeonError::DecompressionLimitExceeded {
            limit: MAX_DECOMPRESSED_SIZE,
        });
    }
    let cap = expected.min(MAX_DECOMPRESSED_SIZE);
    let mut decoder = flate2::read::ZlibDecoder::new(compressed).take(cap + 1);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| PigeonError::CompressionFailed(format!("malformed zlib stream: {e}")))?;
    if out.len() as u64 > cap {
        return Err(PigeonError::DecompressionLimitExceeded { limit: cap });
    }
    if out.len() as u64 != expected {
        return Err(PigeonError::DecompressedSizeMismatch {
            expected,
            got: out.len() as u64,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressible_data_compresses() {
        let data = vec![b'A'; 100_000];
        let (payload, algo) = prepare_payload(&data, CompressionMode::Auto).expect("ok");
        assert_eq!(algo, Compression::Zlib);
        assert!(payload.len() < data.len() / 10);
    }

    #[test]
    fn already_compressed_formats_are_skipped() {
        let mut zip = b"PK\x03\x04".to_vec();
        zip.extend(std::iter::repeat_n(b'A', 100_000));
        let (payload, algo) = prepare_payload(&zip, CompressionMode::Auto).expect("ok");
        assert_eq!(algo, Compression::None);
        assert_eq!(payload.len(), zip.len());
    }

    #[test]
    fn jpeg_is_skipped() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF];
        jpeg.extend(std::iter::repeat_n(b'B', 50_000));
        let (_, algo) = prepare_payload(&jpeg, CompressionMode::Auto).expect("ok");
        assert_eq!(algo, Compression::None);
    }

    #[test]
    fn incompressible_random_data_stays_raw() {
        // xorshift-generated bytes: no constant-magic bias
        let mut state = 0x9E3779B97F4A7C15u64;
        let data: Vec<u8> = (0..50_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let (payload, algo) = prepare_payload(&data, CompressionMode::Auto).expect("ok");
        assert_eq!(algo, Compression::None);
        assert_eq!(payload.len(), data.len());
    }

    #[test]
    fn small_files_stay_raw() {
        let data = vec![b'A'; 512];
        let (_, algo) = prepare_payload(&data, CompressionMode::Auto).expect("ok");
        assert_eq!(algo, Compression::None);
    }

    #[test]
    fn off_mode_never_compresses() {
        let data = vec![b'A'; 100_000];
        let (payload, algo) = prepare_payload(&data, CompressionMode::Off).expect("ok");
        assert_eq!(algo, Compression::None);
        assert_eq!(payload.len(), data.len());
    }

    #[test]
    fn decompress_roundtrip() {
        let original = vec![b'A'; 100_000];
        let (compressed, _) = prepare_payload(&original, CompressionMode::On).expect("ok");
        let out = decompress_bounded(&compressed, original.len() as u64).expect("decompresses");
        assert_eq!(out, original);
    }

    #[test]
    fn decompression_bomb_is_capped() {
        // zlib stream that would expand to 100 MB of 'A'
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder
            .write_all(&vec![b'A'; 100 * 1024 * 1024])
            .expect("write");
        let bomb = encoder.finish().expect("finish");
        let result = decompress_bounded(&bomb, 1000); // claims 1000 bytes
                                                      // Either the hard limit trips during the bounded read or the exact-size
                                                      // check fails afterwards; both reject the bomb.
        assert!(matches!(
            result,
            Err(PigeonError::DecompressionLimitExceeded { .. }
                | PigeonError::DecompressedSizeMismatch { .. })
        ));
    }

    #[test]
    fn malformed_zlib_rejected() {
        assert!(decompress_bounded(&[0xFF, 0xFF, 0xFF], 10).is_err());
    }

    #[test]
    fn declared_size_above_hard_limit_rejected() {
        let result = decompress_bounded(&[], MAX_DECOMPRESSED_SIZE + 1);
        assert!(matches!(
            result,
            Err(PigeonError::DecompressionLimitExceeded { .. })
        ));
    }
}
