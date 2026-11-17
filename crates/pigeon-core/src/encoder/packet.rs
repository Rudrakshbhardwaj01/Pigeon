//! Wire packet assembly on the encoder side.
//!
//! Two dialects exist:
//! - **v1 (plain)**: payloads are sent as-is. Byte-compatible with the
//!   original protocol; older receivers can reconstruct these transfers.
//! - **v2 (encrypted)**: payloads are wrapped in an authenticated-encryption
//!   envelope (`crate::crypto::envelope`) whose associated data is the packet
//!   header bytes `[2..16]`, binding every packet to its transfer identity.

use crate::crypto::envelope as env;
use crate::crypto::kdf::KEY_LEN;
use crate::error::PigeonError;
use crate::protocol::header::write_packet_v1;
use crate::protocol::metadata::{
    serialize_metadata_packet, serialize_metadata_payload, Compression, TransferMetadata,
};
use crate::protocol::{FLAG_ENCRYPTED, FLAG_METADATA, VERSION_V2};

use super::fountain::FountainEncoder;

/// The header bytes `[2..16]` used as envelope associated data: version,
/// flags, transfer id and payload length. Must be built from the *final*
/// header of the packet carrying the envelope so authentication covers it.
fn header_aad(version: u8, flags: u8, transfer_id: u64, payload_len: usize) -> [u8; 14] {
    let mut aad = [0u8; 14];
    aad[0] = version;
    aad[1] = flags;
    aad[2..10].copy_from_slice(&transfer_id.to_le_bytes());
    aad[10..14].copy_from_slice(&(payload_len as u32).to_le_bytes());
    aad
}

/// Builds the wire bytes for one v1 (unencrypted) FEC symbol packet.
#[must_use]
pub fn build_symbol_packet(transfer_id: u64, packet: &raptorq::EncodingPacket) -> Vec<u8> {
    // flags: FEC scheme 0 (RaptorQ), not metadata
    let payload = packet.serialize(); // payload id (4B) + symbols
    write_packet_v1(0, transfer_id, &payload)
}

/// Builds the wire bytes for one v2 (encrypted) FEC symbol packet.
pub fn build_encrypted_symbol_packet(
    transfer_id: u64,
    key: &[u8; KEY_LEN],
    packet: &raptorq::EncodingPacket,
) -> Result<Vec<u8>, PigeonError> {
    let inner = packet.serialize();
    // Two-pass: the AAD includes the final payload length, which is a
    // function of the envelope that itself depends on the AAD only through
    // the length field — compute the length first, then seal.
    let payload_len = env::SYMBOL_ENVELOPE_FIXED + inner.len() + crate::crypto::aead::TAG_LEN;
    let aad = header_aad(VERSION_V2, FLAG_ENCRYPTED, transfer_id, payload_len);
    let payload = env::seal_symbol_payload(key, &aad, &inner)?;
    debug_assert_eq!(payload.len(), payload_len);
    Ok(crate::protocol::header::write_packet(
        VERSION_V2,
        FLAG_ENCRYPTED,
        transfer_id,
        &payload,
    ))
}

/// Builds the wire bytes for the self-contained v1 metadata packet.
pub fn build_metadata_packet(
    transfer_id: u64,
    fountain: &FountainEncoder,
    meta: &TransferMetadata,
) -> Result<Vec<u8>, PigeonError> {
    let oti = fountain.oti();
    serialize_metadata_packet(transfer_id, &oti, meta)
}

/// Builds the wire bytes for the self-contained v2 (encrypted) metadata
/// packet. The inner payload (OTI, filename, size, SHA-256, compression) is
/// sealed with the KDF parameters and salt the receiver needs to derive the
/// key.
pub fn build_encrypted_metadata_packet(
    transfer_id: u64,
    key: &[u8; KEY_LEN],
    kdf: &crate::crypto::KdfParams,
    salt: &[u8; 16],
    fountain: &FountainEncoder,
    meta: &TransferMetadata,
) -> Result<Vec<u8>, PigeonError> {
    let oti = fountain.oti();
    let inner = serialize_metadata_payload(&oti, meta)?;
    let flags = FLAG_METADATA | FLAG_ENCRYPTED;
    let payload_len = env::METADATA_ENVELOPE_FIXED + inner.len() + crate::crypto::aead::TAG_LEN;
    let aad = header_aad(VERSION_V2, flags, transfer_id, payload_len);
    let payload = env::seal_metadata_payload(key, &aad, kdf, salt, &inner)?;
    debug_assert_eq!(payload.len(), payload_len);
    Ok(crate::protocol::header::write_packet(
        VERSION_V2,
        flags,
        transfer_id,
        &payload,
    ))
}

/// Builds a v1 metadata packet for the degenerate empty-file transfer.
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
    use crate::crypto::{DecryptionKey, MetadataEnvelope};
    use crate::integrity::sha256;
    use crate::protocol::header::parse_packet;
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

    fn test_key() -> ([u8; KEY_LEN], [u8; 16], crate::crypto::KdfParams) {
        let kdf = crate::crypto::KdfParams {
            mem_kib: 8 * 1024,
            passes: 1,
            lanes: 1,
        };
        let salt = [0x21u8; 16];
        let env = MetadataEnvelope { kdf, salt };
        let key = DecryptionKey::derive("test-passphrase", &env).expect("derives");
        let mut buf = [0u8; KEY_LEN];
        buf.copy_from_slice(key.as_bytes());
        (buf, salt, kdf)
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
    fn encrypted_symbol_packet_roundtrips_through_decoder() {
        let data = b"encrypted payload!!".to_vec();
        let (key, salt, kdf) = test_key();
        let fountain = FountainEncoder::new(&data, 1024);
        let pkt = fountain.transmission_sequence(0).remove(0);
        let wire = build_encrypted_symbol_packet(0xABCD, &key, &pkt).expect("builds");

        let (header, payload) = parse_packet(&wire).expect("valid v2 packet");
        assert_eq!(header.transfer_id, 0xABCD);
        assert_eq!(header.flags, FLAG_ENCRYPTED);

        // Receiver side: parse envelope, open with AAD, decode the symbol.
        let aad = header_aad(
            header.version,
            header.flags,
            header.transfer_id,
            payload.len(),
        );
        let sealed = env::parse_symbol_envelope(payload).expect("envelope");
        let inner = crate::crypto::aead::open(&key, &aad, &sealed).expect("authenticates");
        let packet = raptorq::EncodingPacket::deserialize(&inner);
        let mut dec = Decoder::new(ObjectTransmissionInformation::deserialize(&fountain.oti()));
        assert_eq!(dec.decode(packet), Some(data));

        // Wrong passphrase must fail authentication, not produce garbage.
        let wrong = DecryptionKey::derive("other-passphrase", &MetadataEnvelope { kdf, salt })
            .expect("derives");
        assert!(crate::crypto::aead::open(wrong.as_bytes(), &aad, &sealed).is_err());
    }

    #[test]
    fn metadata_packet_parses_back() {
        let fountain = FountainEncoder::new(b"data", 1024);
        let wire = build_metadata_packet(7, &fountain, &meta()).expect("builds");
        let (header, payload) = parse_packet(&wire).expect("valid");
        assert_eq!(header.flags, FLAG_METADATA);
        let (oti, got) =
            crate::protocol::metadata::parse_metadata_payload(payload).expect("parses");
        assert_eq!(oti, fountain.oti());
        assert_eq!(got, meta());
    }

    #[test]
    fn encrypted_metadata_packet_parses_back() {
        let (key, salt, kdf) = test_key();
        let fountain = FountainEncoder::new(b"data", 1024);
        let wire = build_encrypted_metadata_packet(0xBEEF, &key, &kdf, &salt, &fountain, &meta())
            .expect("builds");

        let (header, payload) = parse_packet(&wire).expect("valid v2 packet");
        assert_eq!(header.flags, FLAG_METADATA | FLAG_ENCRYPTED);

        let aad = header_aad(
            header.version,
            header.flags,
            header.transfer_id,
            payload.len(),
        );
        let (env_parsed, sealed) = env::parse_metadata_envelope(payload).expect("envelope");
        assert_eq!(env_parsed.salt, salt);
        assert_eq!(env_parsed.kdf, kdf);
        let inner = crate::crypto::aead::open(&key, &aad, &sealed).expect("authenticates");
        let (oti, got) = crate::protocol::metadata::parse_metadata_payload(&inner).expect("parses");
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
