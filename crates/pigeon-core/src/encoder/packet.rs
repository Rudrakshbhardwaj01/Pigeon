//! Wire packet assembly on the encoder side.

use crate::error::PigeonError;
use crate::protocol::header::write_packet;
use crate::protocol::metadata::{serialize_metadata_packet, Compression, TransferMetadata};

use super::fountain::FountainEncoder;

/// Builds the wire bytes for one FEC symbol packet.
#[must_use]
pub fn build_symbol_packet(transfer_id: u64, packet: &raptorq::EncodingPacket) -> Vec<u8> {
    // flags: FEC scheme 0 (RaptorQ), not metadata
    let payload = packet.serialize(); // payload id (4B) + symbols
    write_packet(0, transfer_id, &payload)
}

/// Builds the wire bytes for the self-contained metadata packet.
pub fn build_metadata_packet(
    transfer_id: u64,
    fountain: &FountainEncoder,
    meta: &TransferMetadata,
) -> Result<Vec<u8>, PigeonError> {
    let oti = fountain.oti();
    serialize_metadata_packet(transfer_id, &oti, meta)
}

/// Builds a metadata packet for the degenerate empty-file transfer.
pub fn build_empty_file_metadata_packet(
    transfer_id: u64,
    meta: &TransferMetadata,
) -> Result<Vec<u8>, PigeonError> {
    debug_assert_eq!(meta.original_size, 0);
    debug_assert_eq!(meta.compression, Compression::None);
    let oti = FountainEncoder::empty_oti();
    serialize_metadata_packet(transfer_id, &oti, meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::sha256;
    use crate::protocol::header::parse_packet;
    use crate::protocol::FLAG_METADATA;
    use raptorq::Decoder;
    use raptorq::ObjectTransmissionInformation;

    fn meta() -> TransferMetadata {
        TransferMetadata {
            filename: "t.bin".into(),
            mime_type: None,
            compression: Compression::None,
            original_size: 8,
            sha256: sha256(b"01234567"),
        }
    }

    #[test]
    fn symbol_packet_roundtrips_through_decoder() {
        let data = b"payload!!".to_vec();
        let fountain = FountainEncoder::new(&data, 1024);
        let pkt = fountain.transmission_sequence(0).remove(0);
        let wire = build_symbol_packet(0x1234, &pkt);

        let (header, payload) = parse_packet(&wire).expect("valid wire packet");
        assert_eq!(header.transfer_id, 0x1234);
        assert_ne!(header.flags & FLAG_METADATA, FLAG_METADATA);

        let packet = raptorq::EncodingPacket::deserialize(payload);
        let mut dec = Decoder::new(ObjectTransmissionInformation::deserialize(&fountain.oti()));
        assert_eq!(dec.decode(packet), Some(data));
    }

    #[test]
    fn metadata_packet_parses_back() {
        let fountain = FountainEncoder::new(b"data", 1024);
        let wire = build_metadata_packet(7, &fountain, &meta()).expect("builds");
        let (header, payload) = parse_packet(&wire).expect("valid");
        assert_eq!(header.flags, FLAG_METADATA);
        let (oti, got) = crate::protocol::metadata::parse_metadata_payload(payload).expect("parses");
        assert_eq!(oti, fountain.oti());
        assert_eq!(got, meta());
    }

    #[test]
    fn empty_file_packet_parses_back() {
        let mut m = meta();
        m.original_size = 0;
        m.sha256 = sha256(b"");
        let wire = build_empty_file_metadata_packet(9, &m).expect("builds");
        let (_, payload) = parse_packet(&wire).expect("valid");
        let (_, got) = crate::protocol::metadata::parse_metadata_payload(payload).expect("parses");
        assert_eq!(got.original_size, 0);
    }
}
