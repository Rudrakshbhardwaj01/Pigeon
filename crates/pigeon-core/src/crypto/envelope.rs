//! Packet-payload encryption envelopes (protocol v2).
//!
//! Every v2 packet's payload is an envelope: a short plaintext header naming
//! the cipher and (for metadata packets) the KDF parameters, followed by the
//! AEAD-sealed inner payload. The envelope is self-describing so a receiver
//! can derive a key from a passphrase before touching any ciphertext.
//!
//! **Associated data**: the caller passes the packet header bytes `[2..16]`
//! (version, flags, transfer id, payload length) as AAD. This binds the
//! sealed payload to the packet's transfer identity: a packet cannot be
//! moved between transfers, or have its version/flags/length tampered with,
//! without failing authentication. CRC32C covers the same range — the AEAD
//! tag is the cryptographically strong guarantee on top of it.
//!
//! Layouts (all integers little-endian):
//!
//! ```text
//! symbol envelope
//! 0    1  envelope version (= 1)
//! 1    1  cipher id (1 = XChaCha20-Poly1305)
//! 2   24  per-packet nonce (random)
//! 26   2  inner payload length N
//! 28   N  ciphertext of the inner payload
//!      16  Poly1305 tag (follows the ciphertext)
//!
//! metadata envelope
//! 0    1  envelope version (= 1)
//! 1    1  cipher id (1 = XChaCha20-Poly1305)
//! 2    1  KDF id (1 = Argon2id)
//! 3    4  Argon2id memory cost, KiB
//! 7    4  Argon2id passes
//! 11   4  Argon2id lanes
//! 15  16  KDF salt
//! 31  24  per-packet nonce (random)
//! 55   2  inner payload length N
//! 57   N  ciphertext of the inner payload
//!      16  Poly1305 tag (follows the ciphertext)
//! ```

use zeroize::Zeroizing;

use crate::error::PigeonError;

use super::aead;
use super::kdf::{self, KdfParams, KEY_LEN};

/// Envelope format version (independent of the packet protocol version).
pub const ENVELOPE_VERSION: u8 = 1;

/// Fixed header size of a symbol envelope (before ciphertext + tag).
pub const SYMBOL_ENVELOPE_FIXED: usize = 1 + 1 + aead::NONCE_LEN + 2;
/// Fixed header size of a metadata envelope (before ciphertext + tag).
pub const METADATA_ENVELOPE_FIXED: usize = 1 + 1 + 1 + 4 + 4 + 4 + 16 + aead::NONCE_LEN + 2;

/// Maximum inner payload an envelope can express (u16 length field).
const MAX_INNER_LEN: usize = u16::MAX as usize;

/// A parsed metadata envelope: everything needed to derive a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEnvelope {
    pub kdf: KdfParams,
    pub salt: [u8; 16],
}

/// Validates the two leading envelope bytes common to both envelope types.
fn check_prefix(bytes: &[u8], expected_len: usize) -> Result<(), PigeonError> {
    if bytes.len() < expected_len {
        return Err(PigeonError::DecryptionFailed);
    }
    if bytes[0] != ENVELOPE_VERSION {
        return Err(PigeonError::BadMetadata(format!(
            "unknown encryption envelope version {}",
            bytes[0]
        )));
    }
    if bytes[1] != aead::CIPHER_ID_XCHACHA20_POLY1305 {
        return Err(PigeonError::UnsupportedCipher(bytes[1]));
    }
    Ok(())
}

/// 16-byte read of little-endian u32s (KDF parameter block).
fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed 4-byte slice"),
    )
}

/// Seal-encrypts a symbol packet's inner payload into the symbol envelope.
pub fn seal_symbol_payload(
    key: &[u8; KEY_LEN],
    aad: &[u8],
    inner: &[u8],
) -> Result<Vec<u8>, PigeonError> {
    if inner.len() > MAX_INNER_LEN {
        return Err(PigeonError::PayloadTooLarge(inner.len(), MAX_INNER_LEN));
    }
    let sealed = aead::seal(key, aad, inner)?; // nonce || ciphertext || tag
    let mut out = Vec::with_capacity(SYMBOL_ENVELOPE_FIXED + sealed.len() + aead::TAG_LEN);
    out.push(ENVELOPE_VERSION);
    out.push(aead::CIPHER_ID_XCHACHA20_POLY1305);
    out.extend_from_slice(&sealed[..aead::NONCE_LEN]);
    out.extend_from_slice(&((inner.len() as u16).to_le_bytes()));
    out.extend_from_slice(&sealed[aead::NONCE_LEN..]);
    Ok(out)
}

/// Seal-encrypts a metadata packet's inner payload, embedding the KDF
/// parameters and salt the receiver will need to derive the key.
pub fn seal_metadata_payload(
    key: &[u8; KEY_LEN],
    aad: &[u8],
    kdf: &KdfParams,
    salt: &[u8; 16],
    inner: &[u8],
) -> Result<Vec<u8>, PigeonError> {
    if inner.len() > MAX_INNER_LEN {
        return Err(PigeonError::PayloadTooLarge(inner.len(), MAX_INNER_LEN));
    }
    let sealed = aead::seal(key, aad, inner)?; // nonce || ciphertext || tag
    let mut out = Vec::with_capacity(METADATA_ENVELOPE_FIXED + sealed.len() + aead::TAG_LEN);
    out.push(ENVELOPE_VERSION);
    out.push(aead::CIPHER_ID_XCHACHA20_POLY1305);
    out.push(kdf::KDF_ID_ARGON2ID);
    out.extend_from_slice(&kdf.mem_kib.to_le_bytes());
    out.extend_from_slice(&kdf.passes.to_le_bytes());
    out.extend_from_slice(&kdf.lanes.to_le_bytes());
    out.extend_from_slice(salt);
    out.extend_from_slice(&sealed[..aead::NONCE_LEN]);
    out.extend_from_slice(&((inner.len() as u16).to_le_bytes()));
    out.extend_from_slice(&sealed[aead::NONCE_LEN..]);
    Ok(out)
}

/// Reassembles `nonce || ciphertext || tag` from an envelope body in which
/// the nonce is stored separately from the length-prefixed ciphertext.
fn reassemble_sealed(nonce: &[u8], ct_tag: &[u8]) -> Vec<u8> {
    let mut sealed = Vec::with_capacity(aead::NONCE_LEN + ct_tag.len());
    sealed.extend_from_slice(nonce);
    sealed.extend_from_slice(ct_tag);
    sealed
}

/// Parses a symbol envelope and returns the sealed value
/// (nonce || ciphertext || tag) ready for [`aead::open`].
pub fn parse_symbol_envelope(bytes: &[u8]) -> Result<Vec<u8>, PigeonError> {
    check_prefix(bytes, SYMBOL_ENVELOPE_FIXED)?;
    let ct_tag = &bytes[SYMBOL_ENVELOPE_FIXED..];
    let inner_len =
        u16::from_le_bytes(bytes[26..28].try_into().expect("fixed 2-byte slice")) as usize;
    if ct_tag.len() != inner_len + aead::TAG_LEN {
        return Err(PigeonError::BadPayloadLength {
            declared: inner_len,
            actual: ct_tag.len() - aead::TAG_LEN.min(ct_tag.len()),
        });
    }
    Ok(reassemble_sealed(&bytes[2..26], ct_tag))
}

/// Parses a metadata envelope.
///
/// KDF parameters are validated against the hard caps immediately — a
/// hostile sender cannot make a receiver allocate an unbounded derivation.
pub fn parse_metadata_envelope(bytes: &[u8]) -> Result<(MetadataEnvelope, Vec<u8>), PigeonError> {
    check_prefix(bytes, METADATA_ENVELOPE_FIXED)?;
    if bytes[2] != kdf::KDF_ID_ARGON2ID {
        return Err(PigeonError::UnsupportedKdf(bytes[2]));
    }
    let kdf = KdfParams::validated(
        read_u32_le(bytes, 3),
        read_u32_le(bytes, 7),
        read_u32_le(bytes, 11),
    )?;
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&bytes[15..31]);

    let ct_tag = &bytes[METADATA_ENVELOPE_FIXED..];
    let inner_len =
        u16::from_le_bytes(bytes[55..57].try_into().expect("fixed 2-byte slice")) as usize;
    if ct_tag.len() != inner_len + aead::TAG_LEN {
        return Err(PigeonError::BadPayloadLength {
            declared: inner_len,
            actual: ct_tag.len() - aead::TAG_LEN.min(ct_tag.len()),
        });
    }
    Ok((
        MetadataEnvelope { kdf, salt },
        reassemble_sealed(&bytes[31..55], ct_tag),
    ))
}

/// A derived, wipe-on-drop decryption key plus the parameters it came from.
#[derive(Clone)]
pub struct DecryptionKey {
    key: Zeroizing<[u8; KEY_LEN]>,
}

impl DecryptionKey {
    /// Derives a key from a passphrase using validated envelope parameters.
    pub fn derive(passphrase: &str, envelope: &MetadataEnvelope) -> Result<Self, PigeonError> {
        let key = envelope.kdf.derive_key(passphrase, &envelope.salt)?;
        Ok(Self { key })
    }

    /// Raw key access for the AEAD layer.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> DecryptionKey {
        let env = MetadataEnvelope {
            kdf: KdfParams {
                mem_kib: 8 * 1024,
                passes: 1,
                lanes: 1,
            },
            salt: [5u8; 16],
        };
        DecryptionKey::derive("passphrase", &env).expect("derives")
    }

    #[test]
    fn symbol_envelope_roundtrip() {
        let k = key();
        let inner = b"raptorq payload-id + symbols".to_vec();
        let env = seal_symbol_payload(k.as_bytes(), b"aad", &inner).expect("seals");
        assert_eq!(
            env.len(),
            SYMBOL_ENVELOPE_FIXED + inner.len() + aead::TAG_LEN
        );
        assert_eq!(env[0], ENVELOPE_VERSION);
        assert_eq!(env[1], aead::CIPHER_ID_XCHACHA20_POLY1305);

        let sealed = parse_symbol_envelope(&env).expect("parses");
        assert_eq!(
            aead::open(k.as_bytes(), b"aad", &sealed).expect("opens"),
            inner
        );
    }

    #[test]
    fn metadata_envelope_roundtrip() {
        let kdf = KdfParams::default();
        let salt = [0x11u8; 16];
        let k =
            DecryptionKey::derive("passphrase", &MetadataEnvelope { kdf, salt }).expect("derives");
        let inner = vec![0x42u8; 100];
        let env = seal_metadata_payload(k.as_bytes(), b"aad", &kdf, &salt, &inner).expect("seals");
        assert_eq!(
            env.len(),
            METADATA_ENVELOPE_FIXED + inner.len() + aead::TAG_LEN
        );

        let (got_env, sealed) = parse_metadata_envelope(&env).expect("parses");
        assert_eq!(got_env.kdf, kdf);
        assert_eq!(got_env.salt, salt);
        assert_eq!(
            aead::open(k.as_bytes(), b"aad", &sealed).expect("opens"),
            inner
        );
    }

    #[test]
    fn metadata_envelope_caps_hostile_kdf_params() {
        // Hand-build an envelope declaring a 4 GiB derivation.
        let mut bytes = vec![0u8; METADATA_ENVELOPE_FIXED + 64];
        bytes[0] = ENVELOPE_VERSION;
        bytes[1] = aead::CIPHER_ID_XCHACHA20_POLY1305;
        bytes[2] = kdf::KDF_ID_ARGON2ID;
        bytes[3..7].copy_from_slice(&(4u32 * 1024 * 1024).to_le_bytes()); // 4 GiB in KiB
        bytes[7..11].copy_from_slice(&2u32.to_le_bytes());
        bytes[11..15].copy_from_slice(&1u32.to_le_bytes());
        let err = parse_metadata_envelope(&bytes).unwrap_err();
        assert!(matches!(err, PigeonError::KdfParamsExcessive { .. }));
    }

    #[test]
    fn metadata_envelope_rejects_unknown_kdf_and_cipher() {
        let mut bytes = vec![0u8; METADATA_ENVELOPE_FIXED + 64];
        bytes[0] = ENVELOPE_VERSION;
        bytes[1] = 0x99; // unknown cipher
        assert!(matches!(
            parse_metadata_envelope(&bytes),
            Err(PigeonError::UnsupportedCipher(0x99))
        ));

        let mut bytes2 = vec![0u8; METADATA_ENVELOPE_FIXED + 64];
        bytes2[0] = ENVELOPE_VERSION;
        bytes2[1] = aead::CIPHER_ID_XCHACHA20_POLY1305;
        bytes2[2] = 0x77; // unknown KDF
        assert!(matches!(
            parse_metadata_envelope(&bytes2),
            Err(PigeonError::UnsupportedKdf(0x77))
        ));

        let mut bytes3 = vec![0u8; METADATA_ENVELOPE_FIXED + 64];
        bytes3[0] = 0x05; // unknown envelope version
        assert!(parse_metadata_envelope(&bytes3).is_err());
    }

    #[test]
    fn envelope_length_field_must_match() {
        let k = key();
        let mut env = seal_symbol_payload(k.as_bytes(), b"", b"hello").expect("seals");
        // Corrupt the declared inner length.
        env[26] ^= 0x01;
        assert!(parse_symbol_envelope(&env).is_err());
    }

    #[test]
    fn tampered_ciphertext_rejected_through_envelope() {
        let k = key();
        let mut env = seal_symbol_payload(k.as_bytes(), b"aad", b"payload").expect("seals");
        let last = env.len() - 1;
        env[last] ^= 0xFF;
        let sealed = parse_symbol_envelope(&env).expect("parses");
        assert!(matches!(
            aead::open(k.as_bytes(), b"aad", &sealed),
            Err(PigeonError::DecryptionFailed)
        ));
    }

    #[test]
    fn wrong_passphrase_produces_different_key() {
        let env = MetadataEnvelope {
            kdf: KdfParams {
                mem_kib: 8 * 1024,
                passes: 1,
                lanes: 1,
            },
            salt: [5u8; 16],
        };
        let right = DecryptionKey::derive("right", &env).expect("derives");
        let wrong = DecryptionKey::derive("wrong", &env).expect("derives");
        let sealed = seal_symbol_payload(right.as_bytes(), b"", b"secret").expect("seals");
        let sealed = parse_symbol_envelope(&sealed).expect("parses");
        assert!(matches!(
            aead::open(wrong.as_bytes(), b"", &sealed),
            Err(PigeonError::DecryptionFailed)
        ));
    }
}
