//! Strict packet parsing on the receive path: untrusted bytes in, typed
//! events out. Everything invalid is rejected before touching recovery state.

use raptorq::PayloadId;

use crate::error::PigeonError;
use crate::protocol::header::{parse_packet, PacketHeader};
use crate::protocol::metadata::{parse_metadata_payload, TransferMetadata};
use crate::protocol::{fec_scheme, FLAG_METADATA, FEC_RAPTORQ, HEADER_LEN, MAGIC};

/// A validated packet ready to feed into the recovery engine.
#[derive(Debug)]
pub enum IncomingPacket {
    /// Metadata packet with the RaptorQ OTI (12 bytes) and file description.
    Metadata {
        transfer_id: u64,
        oti: [u8; 12],
        meta: TransferMetadata,
    },
    /// One RaptorQ encoding packet (payload id + symbols).
    Symbol {
        transfer_id: u64,
        payload_id: u32,
        symbols: Vec<u8>,
    },
}

/// Validates raw bytes (e.g. decoded QR content) as a Pigeon packet.
pub fn parse_incoming(bytes: &[u8]) -> Result<IncomingPacket, PigeonError> {
    if bytes.len() < HEADER_LEN || bytes[0..2] != MAGIC {
        // Distinguish "not ours at all" from protocol errors for reporting.
        return Err(if bytes.len() < HEADER_LEN {
            PigeonError::PacketTooShort {
                got: bytes.len(),
                need: HEADER_LEN,
            }
        } else {
            PigeonError::BadMagic
        });
    }

    let (header, payload) = parse_packet(bytes)?;
    if header.flags & FLAG_METADATA != 0 {
        let (oti, meta) = parse_metadata_payload(payload)?;
        return Ok(IncomingPacket::Metadata {
            transfer_id: header.transfer_id,
            oti,
            meta,
        });
    }

    if fec_scheme(header.flags) != FEC_RAPTORQ {
        return Err(PigeonError::BadFecPayload("unknown FEC scheme id"));
    }
    if payload.len() < 4 {
        return Err(PigeonError::BadFecPayload("payload shorter than a payload id"));
    }
    let payload_id = PayloadId::deserialize(
        payload[0..4]
            .try_into()
            .map_err(|_| PigeonError::BadFecPayload("payload id is 4 bytes"))?,
    );
    Ok(IncomingPacket::Symbol {
        transfer_id: header.transfer_id,
        payload_id: u32::from_be_bytes(payload_id.serialize()),
        symbols: payload[4..].to_vec(),
    })
}

/// The header of a packet, for pre-validation diagnostics.
pub fn peek_header(bytes: &[u8]) -> Option<PacketHeader> {
    if bytes.len() < HEADER_LEN || bytes[0..2] != MAGIC {
        return None;
    }
    Some(PacketHeader {
        version: bytes[2],
        flags: bytes[3],
        transfer_id: u64::from_le_bytes(bytes[4..12].try_into().ok()?),
        payload_len: u32::from_le_bytes(bytes[12..16].try_into().ok()?),
    })
}

/// Extracts the transfer id without full validation (for status display).
#[must_use]
pub fn peek_transfer_id(bytes: &[u8]) -> Option<u64> {
    peek_header(bytes).map(|h| h.transfer_id)
}

/// A rejected packet, with the reason, for UI statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectionReason(pub &'static str);

pub const REJ_MAGIC: RejectionReason = RejectionReason("not-a-pigeon-packet");
pub const REJ_VERSION: RejectionReason = RejectionReason("unsupported-protocol-version");
pub const REJ_CRC: RejectionReason = RejectionReason("corrupted-content");
pub const REJ_FORMAT: RejectionReason = RejectionReason("malformed-structure");
pub const REJ_LIMITS: RejectionReason = RejectionReason("resource-limit-exceeded");

/// Maps a parse error onto a stable rejection reason code.
#[must_use]
pub fn rejection_reason(err: &PigeonError) -> RejectionReason {
    match err {
        PigeonError::BadMagic | PigeonError::PacketTooShort { .. } => REJ_MAGIC,
        PigeonError::UnsupportedVersion { .. } => REJ_VERSION,
        PigeonError::BadCrc => REJ_CRC,
        PigeonError::PayloadTooLarge(..) | PigeonError::BadPayloadLength { .. } => REJ_LIMITS,
        _ => REJ_FORMAT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::{encode_file, EncodeConfig};
    use crate::protocol::metadata::{serialize_metadata_packet, Compression};

    #[test]
    fn symbol_packet_parses() {
        let transfer = encode_file(b"test-data-123", EncodeConfig::default()).expect("encodes");
        let wire = &transfer.packet_frames[0];
        match parse_incoming(wire).expect("parses") {
            IncomingPacket::Symbol { transfer_id, .. } => {
                assert_eq!(transfer_id, transfer.stats.transfer_id);
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn metadata_packet_parses() {
        let transfer = encode_file(b"test", EncodeConfig::default()).expect("encodes");
        match parse_incoming(&transfer.metadata_packet).expect("parses") {
            IncomingPacket::Metadata { meta, .. } => {
                assert_eq!(meta.filename, "pigeon-file");
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn garbage_rejected() {
        assert_eq!(rejection_reason(&parse_incoming(b"").unwrap_err()), REJ_MAGIC);
        assert_eq!(
            rejection_reason(&parse_incoming(b"hello world this is not a pigeon").unwrap_err()),
            REJ_MAGIC
        );
    }

    #[test]
    fn corrupted_payload_rejected_as_crc() {
        let transfer = encode_file(b"test", EncodeConfig::default()).expect("encodes");
        let mut wire = transfer.packet_frames[0].clone();
        let last = wire.len() - 1;
        wire[last] ^= 0xFF;
        assert_eq!(rejection_reason(&parse_incoming(&wire).unwrap_err()), REJ_CRC);
    }

    #[test]
    fn unknown_version_rejected_honestly() {
        // Build a structurally valid packet with an unknown version.
        let mut pkt = serialize_metadata_packet(1, &[0u8; 12], &{
            crate::protocol::metadata::TransferMetadata {
                filename: "a".into(),
                mime_type: None,
                compression: Compression::None,
                original_size: 0,
                sha256: [0u8; 32],
            }
        })
        .expect("builds");
        pkt[2] = 0x02;
        // fix CRC so we reach the version check
        let crc = crate::integrity::crc32c_bytes(&pkt[2..HEADER_LEN]);
        pkt[16..HEADER_LEN].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(
            parse_incoming(&pkt),
            Err(PigeonError::UnsupportedVersion { .. })
        ));
    }
}
