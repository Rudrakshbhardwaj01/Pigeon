//! Authenticated encryption: XChaCha20-Poly1305 with explicit associated data.
//!
//! Every sealed value carries a fresh random 24-byte nonce (XChaCha's
//! oversized nonce makes random-per-message nonces safe for any realistic
//! packet volume). The Poly1305 tag authenticates both the ciphertext and the
//! caller-supplied associated data, which Pigeon uses to cryptographically
//! bind packets to their transfer identity (see `envelope.rs`).

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::PigeonError;

/// XChaCha20-Poly1305 — the only AEAD this engine implements (cipher id 1).
pub const CIPHER_ID_XCHACHA20_POLY1305: u8 = 1;

/// Nonce length in bytes (XChaCha20: 192-bit extended nonce).
pub const NONCE_LEN: usize = 24;
/// Poly1305 authentication tag length in bytes.
pub const TAG_LEN: usize = 16;

/// Generates a fresh random nonce from the operating system CSPRNG.
pub fn random_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).unwrap_or_else(|e| panic!("CSPRNG unavailable: {e}"));
    nonce
}

/// Encrypts `plaintext`, returning `nonce || ciphertext || tag`.
///
/// `aad` is authenticated but not encrypted. Total output length is
/// `NONCE_LEN + plaintext.len() + TAG_LEN`.
pub fn seal(
    key: &[u8; super::kdf::KEY_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, PigeonError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = random_nonce();
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| PigeonError::EncryptionFailed)?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext); // ciphertext || tag
    Ok(out)
}

/// Opens a value produced by [`seal`] (layout `nonce || ciphertext || tag`).
///
/// Authentication happens *before* any plaintext is returned: on tag failure
/// the error is the only output and no partial plaintext is exposed.
pub fn open(
    key: &[u8; super::kdf::KEY_LEN],
    aad: &[u8],
    sealed: &[u8],
) -> Result<Vec<u8>, PigeonError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(PigeonError::DecryptionFailed);
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            XNonce::from_slice(&sealed[..NONCE_LEN]),
            Payload {
                msg: &sealed[NONCE_LEN..],
                aad,
            },
        )
        .map_err(|_| PigeonError::DecryptionFailed)
}

/// Seal variant that takes an explicit nonce (used to make the object layer's
/// nonce a deterministic function of nothing — encoders use [`random_nonce`]).
#[cfg(test)]
pub(crate) fn seal_with_nonce(
    key: &[u8; super::kdf::KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, PigeonError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| PigeonError::EncryptionFailed)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::kdf::{KdfParams, KEY_LEN};

    fn test_key() -> [u8; KEY_LEN] {
        // Fixed key for roundtrip tests; derivation itself is covered in kdf.rs.
        let params = KdfParams {
            mem_kib: 8 * 1024,
            passes: 1,
            lanes: 1,
        };
        let key = params
            .derive_key("test-passphrase", &[9u8; 16])
            .expect("derives");
        *key
    }

    #[test]
    fn seal_open_roundtrip() {
        let key = test_key();
        let sealed = seal(&key, b"aad", b"attack at dawn").expect("seals");
        assert_eq!(sealed.len(), NONCE_LEN + 14 + TAG_LEN);
        let opened = open(&key, b"aad", &sealed).expect("opens");
        assert_eq!(opened, b"attack at dawn");
    }

    #[test]
    fn empty_plaintext_roundtrip() {
        let key = test_key();
        let sealed = seal(&key, b"", b"").expect("seals");
        assert_eq!(sealed.len(), NONCE_LEN + TAG_LEN);
        assert_eq!(open(&key, b"", &sealed).expect("opens"), b"");
    }

    #[test]
    fn every_seal_uses_a_fresh_nonce() {
        let key = test_key();
        let a = seal(&key, b"", b"same message").expect("seals");
        let b = seal(&key, b"", b"same message").expect("seals");
        assert_ne!(a[..NONCE_LEN], b[..NONCE_LEN], "nonces must never repeat");
        assert_ne!(a, b, "ciphertexts must differ even for identical input");
    }

    #[test]
    fn wrong_key_fails_auth() {
        let key = test_key();
        let wrong = [0xEEu8; KEY_LEN];
        let sealed = seal(&key, b"", b"secret").expect("seals");
        assert!(matches!(
            open(&wrong, b"", &sealed),
            Err(PigeonError::DecryptionFailed)
        ));
    }

    #[test]
    fn tampered_ciphertext_fails_auth() {
        let key = test_key();
        let mut sealed = seal(&key, b"", b"secret").expect("seals");
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01; // flip a tag bit
        assert!(matches!(
            open(&key, b"", &sealed),
            Err(PigeonError::DecryptionFailed)
        ));
    }

    #[test]
    fn tampered_aad_fails_auth() {
        let key = test_key();
        let sealed = seal(&key, b"transfer-A", b"secret").expect("seals");
        assert!(matches!(
            open(&key, b"transfer-B", &sealed),
            Err(PigeonError::DecryptionFailed)
        ));
    }

    #[test]
    fn truncated_sealed_rejected() {
        let key = test_key();
        assert!(matches!(
            open(&key, b"", &[0u8; 10]),
            Err(PigeonError::DecryptionFailed)
        ));
        assert!(matches!(
            open(&key, b"", &[]),
            Err(PigeonError::DecryptionFailed)
        ));
    }

    #[test]
    fn nonce_test_seal_matches_seal_format() {
        // The deterministic variant (test-only) must produce openable output.
        let key = test_key();
        let nonce = [0x42u8; NONCE_LEN];
        let sealed = seal_with_nonce(&key, &nonce, b"aad", b"payload").expect("seals");
        assert_eq!(&sealed[..NONCE_LEN], &nonce);
        assert_eq!(open(&key, b"aad", &sealed).expect("opens"), b"payload");
    }
}
