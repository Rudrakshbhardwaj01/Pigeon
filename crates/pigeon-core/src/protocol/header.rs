//! Packet header framing: writing and strictly parsing the 20-byte header.

use crate::error::PigeonError;
use crate::integrity::crc32c_bytes;

use crc32c::crc32c_append;

use super::{fec_scheme, validate_flags, HEADER_LEN, MAGIC, MAX_PACKET_PAYLOAD, VERSION};

/// Parsed view of a packet header (excluding magic bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHeader {
    pub version: u8,
    pub flags: u8,
    pub transfer_id: u64,
    pub payload_len: u32,
}

/// Serializes a header with magic and CRC into a buffer that already contains
/// the payload at offset [`HEADER_LEN`].
///
/// CRC32C covers bytes `[2..16]` (version, flags, transfer id, payload length)
/// followed by the payload. The magic and the CRC field itself are outside the
/// checksum, so packets can be scanned by magic and validated in one pass.
#[must_use]
pub fn write_packet(flags: u8, transfer_id: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(flags);
    out.extend_from_slice(&transfer_id.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]); // CRC placeholder
    out.extend_from_slice(payload);
    let crc = crc32c_append(crc32c_bytes(&out[2..16]), payload);
    out[16..HEADER_LEN].copy_from_slice(&crc.to_le_bytes());
    out
}

/// Strictly parses and validates a wire packet (header + CRC presence check).
///
/// The payload is returned as a slice of `bytes` so callers avoid copies.
pub fn parse_packet(bytes: &[u8]) -> Result<(PacketHeader, &[u8]), PigeonError> {
    if bytes.len() < HEADER_LEN {
        return Err(PigeonError::PacketTooShort {
            got: bytes.len(),
            need: HEADER_LEN,
        });
    }
    if bytes[0..2] != MAGIC {
        return Err(PigeonError::BadMagic);
    }
    let version = bytes[2];
    if version != VERSION {
        return Err(PigeonError::UnsupportedVersion {
            got: version,
            supported: VERSION,
        });
    }
    let flags = bytes[3];
    validate_flags(flags)?;

    let transfer_id = u64::from_le_bytes(bytes[4..12].try_into().expect("fixed 8-byte slice"));
    let payload_len = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed 4-byte slice"))
        as usize;
    if payload_len > MAX_PACKET_PAYLOAD {
        return Err(PigeonError::PayloadTooLarge(payload_len, MAX_PACKET_PAYLOAD));
    }
    let declared_end = HEADER_LEN
        .checked_add(payload_len)
        .ok_or(PigeonError::PayloadTooLarge(payload_len, MAX_PACKET_PAYLOAD))?;
    if bytes.len() != declared_end {
        return Err(PigeonError::BadPayloadLength {
            declared: payload_len,
            actual: bytes.len() - HEADER_LEN,
        });
    }

    let payload = &bytes[HEADER_LEN..];
    let crc_stored = u32::from_le_bytes(bytes[16..HEADER_LEN].try_into().expect("fixed 4-byte slice"));
    let crc_actual = crc32c_append(crc32c_bytes(&bytes[2..16]), payload);
    if crc_stored != crc_actual {
        return Err(PigeonError::BadCrc);
    }

    let header = PacketHeader {
        version,
        flags,
        transfer_id,
        payload_len: payload_len as u32,
    };
    Ok((header, payload))
}

/// Returns the FEC scheme for a parsed header.
#[must_use]
pub fn header_fec_scheme(header: &PacketHeader) -> u8 {
    fec_scheme(header.flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_fields() {
        let payload = b"hello pigeon";
        let bytes = write_packet(0x01, 0x1122_3344_5566_7788, payload);
        let (header, got) = parse_packet(&bytes).expect("valid packet");
        assert_eq!(header.version, VERSION);
        assert_eq!(header.flags, 0x01);
        assert_eq!(header.transfer_id, 0x1122_3344_5566_7788);
        assert_eq!(header.payload_len, payload.len() as u32);
        assert_eq!(got, payload);
    }

    #[test]
    fn flipped_bit_fails_crc() {
        let mut bytes = write_packet(0, 42, b"data");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        assert!(matches!(parse_packet(&bytes), Err(PigeonError::BadCrc)));
    }

    #[test]
    fn corrupted_header_fails_crc() {
        let mut bytes = write_packet(0, 42, b"data");
        bytes[4] ^= 0xFF; // transfer id corruption
        assert!(matches!(parse_packet(&bytes), Err(PigeonError::BadCrc)));
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = write_packet(0, 42, b"data");
        bytes[1] = b'X';
        assert!(matches!(parse_packet(&bytes), Err(PigeonError::BadMagic)));
    }

    #[test]
    fn unknown_version_rejected() {
        let mut bytes = write_packet(0, 42, b"data");
        bytes[2] = 0x7F;
        // version byte participates in the CRC, so rewrite it honestly
        bytes[2] = 0x7F;
        let crc = crc32c_bytes(&bytes[2..HEADER_LEN]);
        bytes[16..HEADER_LEN].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(
            parse_packet(&bytes),
            Err(PigeonError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn trailing_garbage_rejected() {
        let mut bytes = write_packet(0, 42, b"data");
        bytes.push(0);
        assert!(matches!(
            parse_packet(&bytes),
            Err(PigeonError::BadPayloadLength { .. })
        ));
    }
}
