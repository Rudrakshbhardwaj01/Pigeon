//! Pigeon wire protocol: constants, limits and packet framing.
//!
//! The full specification lives in `docs/PROTOCOL.md`. Summary:
//!
//! ```text
//! offset  size  field
//! 0       2     magic = 0x50 0x47 ("PG")
//! 2       1     protocol version
//! 3       1     flags
//! 4       8     transfer id (u64 little-endian, random per transfer)
//! 12      4     payload length (u32 little-endian)
//! 16      4     CRC32C over bytes [2..16] plus the payload
//! 20      N     payload (N == payload length)
//! ```
//!
//! Versions: unencrypted transfers use **v1** (byte-compatible with the
//! original protocol so older receivers still reconstruct them). Encrypted
//! transfers use **v2**, where every payload is an authenticated-encryption
//! envelope (`crate::crypto`) and flag bit 3 is set. Receivers must treat
//! the two versions as separate dialects: v1 never carries the encryption
//! flag; v2 always does.

pub mod header;
pub mod metadata;

use crate::error::PigeonError;

/// Magic bytes every packet starts with: ASCII "PG".
pub const MAGIC: [u8; 2] = [0x50, 0x47];

/// Unencrypted wire protocol (original dialect).
pub const VERSION_V1: u8 = 1;
/// Encrypted wire protocol (envelope payloads, FLAG_ENCRYPTED always set).
pub const VERSION_V2: u8 = 2;
/// Highest version this engine speaks (also what the WASM API reports).
pub const VERSION: u8 = VERSION_V2;

/// Fixed header length in bytes.
pub const HEADER_LEN: usize = 20;

/// Flags bit 0: packet carries transfer metadata instead of FEC symbols.
pub const FLAG_METADATA: u8 = 0b0000_0001;
/// Flags bits 1-2: FEC scheme id (0 = RaptorQ per RFC 6330).
pub const FEC_RAPTORQ: u8 = 0;
pub const FEC_SCHEME_MASK: u8 = 0b0000_0110;
pub const FEC_SCHEME_SHIFT: u8 = 1;
/// Flags bit 3: the payload is an authenticated-encryption envelope (v2 only).
pub const FLAG_ENCRYPTED: u8 = 0b0000_1000;
/// Bits 4-7 are reserved and must be zero in v1 and v2.
pub const FLAG_RESERVED_MASK: u8 = 0b1111_0000;

// ---- resource limits enforced on untrusted input ----

/// Maximum accepted packet payload (metadata or symbols), bytes.
pub const MAX_PACKET_PAYLOAD: usize = 4096;
/// Maximum filename length after sanitization, UTF-8 bytes.
pub const MAX_FILENAME_BYTES: usize = 255;
/// Maximum MIME type length, ASCII bytes.
pub const MAX_MIME_BYTES: usize = 127;
/// Hard cap on encodable file size.
pub const MAX_ENCODE_FILE_SIZE: u64 = 256 * 1024 * 1024;
/// Hard cap on the declared transfer size the decoder will reconstruct.
pub const MAX_DECODE_TRANSFER_BYTES: u64 = 512 * 1024 * 1024;
/// Hard cap on decompression output (belt and braces against bombs).
pub const MAX_DECOMPRESSED_SIZE: u64 = 512 * 1024 * 1024;
/// Maximum accepted GIF file size.
pub const MAX_GIF_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum GIF frame width or height.
pub const MAX_GIF_DIMENSION: u32 = 2048;
/// Maximum GIF frames parsed per import.
pub const MAX_GIF_FRAMES: usize = 20_000;
/// Packets buffered per transfer before the metadata (OTI) packet arrives.
pub const MAX_BUFFERED_PACKETS: usize = 4096;
/// Maximum number of transfers tracked simultaneously.
pub const MAX_CONCURRENT_TRANSFERS: usize = 16;

/// Validates a reserved-flag set: v1 packets must carry only known bits.
pub fn validate_flags(flags: u8) -> Result<(), PigeonError> {
    let unknown = flags & FLAG_RESERVED_MASK;
    if unknown != 0 {
        return Err(PigeonError::BadFlags { bits: unknown });
    }
    Ok(())
}

/// Extracts the FEC scheme id from a flags byte.
#[must_use]
pub fn fec_scheme(flags: u8) -> u8 {
    (flags & FEC_SCHEME_MASK) >> FEC_SCHEME_SHIFT
}

/// Serializes a transfer id as a fixed-width lowercase hex string.
///
/// u64 does not survive the JS boundary as a number (precision loss above
/// 2^53), so the wire API represents transfer ids as hex text.
pub mod ser {
    use serde::Serializer;

    pub fn u64_hex<S: Serializer>(id: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{id:016x}"))
    }

    pub fn opt_u64_hex<S: Serializer>(id: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(v) => u64_hex(v, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Serializes a byte vector as an actual JS `Uint8Array`.
    ///
    /// serde's generic `Vec<u8>` impl goes through `serialize_seq`, which
    /// serde-wasm-bindgen renders as a plain number array — unusable for file
    /// payloads. Forcing `serialize_bytes` yields a `Uint8Array` view.
    pub fn bytes<S: Serializer>(v: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(v)
    }
}

/// Generates a fresh random 64-bit transfer identifier.
pub fn new_transfer_id() -> u64 {
    let mut buf = [0u8; 8];
    getrandom::getrandom(&mut buf).unwrap_or_else(|e| panic!("CSPRNG unavailable: {e}"));
    u64::from_le_bytes(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_flags_rejected() {
        assert!(validate_flags(0).is_ok());
        assert!(validate_flags(FLAG_METADATA).is_ok());
        assert!(validate_flags(FLAG_METADATA | (FEC_RAPTORQ << FEC_SCHEME_SHIFT)).is_ok());
        assert!(validate_flags(FLAG_ENCRYPTED).is_ok());
        assert!(validate_flags(0b0001_0000).is_err());
        assert!(validate_flags(0b1000_0000).is_err());
    }

    #[test]
    fn transfer_ids_differ() {
        let a = new_transfer_id();
        let b = new_transfer_id();
        assert_ne!(a, b);
    }
}
