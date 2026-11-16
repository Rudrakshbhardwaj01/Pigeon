//! Integrity primitives: SHA-256 content identity and CRC32C packet checksums.

use sha2::{Digest, Sha256};

/// SHA-256 digest of the input; used to establish content identity of files.
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Hex-encodes a SHA-256 digest.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&sha256(data))
}

/// Lowercase hex encoding of arbitrary bytes (used for digests).
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// CRC32C (Castagnoli) over the given bytes — accidental-corruption detection.
#[must_use]
pub fn crc32c_bytes(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // "abc" -> standard NIST test vector
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_empty() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn crc32c_known_vector() {
        // "123456789" -> Castagnoli check value 0xE3069283
        assert_eq!(crc32c_bytes(b"123456789"), 0xE306_9283);
    }
}
