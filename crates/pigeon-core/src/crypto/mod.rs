//! Cryptographic layer for protocol v2 encrypted transfers.
//!
//! Construction (no primitives invented here):
//!
//! - **Key derivation**: Argon2id, 32-byte output, parameters transmitted in
//!   the plaintext metadata envelope and validated against hard caps before
//!   use ([`kdf`]).
//! - **Encryption**: XChaCha20-Poly1305 with a fresh random 192-bit nonce per
//!   sealed value; the Poly1305 tag authenticates ciphertext plus associated
//!   data ([`aead`]).
//! - **Framing**: per-packet envelopes ([`envelope`]) whose associated data
//!   is the packet header's `[2..16]` slice — version, flags, transfer id and
//!   payload length. This binds every encrypted payload to its transfer, so
//!   packets cannot be moved between transfers without failing
//!   authentication.
//!
//! Two sealing layers exist in an encrypted transfer:
//!
//! 1. **Object layer** — the (possibly compressed) file is sealed once; the
//!    sealed blob (`nonce || ciphertext || tag`, AAD = transfer id) is what
//!    gets fountain-coded. Reconstruction therefore yields an authenticated
//!    plaintext object only after every byte of it is present and its tag
//!    verifies.
//! 2. **Packet layer** — each wire packet's payload (metadata or symbol) is
//!    individually sealed so packets are authenticated the moment they are
//!    received, before they touch any decoder state.

pub mod aead;
pub mod envelope;
pub mod kdf;

use crate::error::PigeonError;

pub use envelope::{DecryptionKey, MetadataEnvelope};
pub use kdf::{KdfParams, KEY_LEN};

/// Generates the 16-byte KDF salt for a new encrypted transfer.
pub fn random_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).unwrap_or_else(|e| panic!("CSPRNG unavailable: {e}"));
    salt
}

/// Seals the transfer object: `nonce || ciphertext || tag`, bound to the
/// transfer id. This blob is what the fountain code transmits.
pub fn seal_object(
    key: &[u8; KEY_LEN],
    transfer_id: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, PigeonError> {
    aead::seal(key, &transfer_id.to_le_bytes(), plaintext)
}

/// Opens the transfer object after fountain reconstruction.
pub fn open_object(
    key: &[u8; KEY_LEN],
    transfer_id: u64,
    sealed: &[u8],
) -> Result<Vec<u8>, PigeonError> {
    aead::open(key, &transfer_id.to_le_bytes(), sealed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_seal_opens_with_matching_transfer_id_only() {
        let env = MetadataEnvelope {
            kdf: KdfParams {
                mem_kib: 8 * 1024,
                passes: 1,
                lanes: 1,
            },
            salt: [3u8; 16],
        };
        let key = DecryptionKey::derive("pw", &env).expect("derives");
        let sealed = seal_object(key.as_bytes(), 0x1234, b"file bytes here").expect("seals");
        assert_eq!(
            open_object(key.as_bytes(), 0x1234, &sealed).expect("opens"),
            b"file bytes here"
        );
        // Same key, different transfer id: authentication must fail. This is
        // the property that defeats cross-transfer packet/object substitution.
        assert!(open_object(key.as_bytes(), 0x5678, &sealed).is_err());
    }
}
