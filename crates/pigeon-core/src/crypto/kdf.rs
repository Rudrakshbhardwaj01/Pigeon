//! Passphrase key derivation: Argon2id with explicit, validated parameters.
//!
//! KDF parameters travel in the (plaintext) metadata envelope so future
//! encoders can raise costs without a protocol change. Because the parameters
//! are attacker-controlled on the receive path, they are validated against
//! hard caps *before* any memory is allocated for derivation.

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::error::PigeonError;

/// Argon2id — the only KDF this engine implements (KDF id 1 on the wire).
pub const KDF_ID_ARGON2ID: u8 = 1;

/// Length of the derived AEAD key (XChaCha20-Poly1305 key size).
pub const KEY_LEN: usize = 32;

/// Hard floor on attacker-supplied memory cost (KiB). Below this the
/// parameters are considered invalid rather than merely weak.
pub const KDF_MIN_MEM_KIB: u32 = 8 * 1024;
/// Hard ceiling on attacker-supplied memory cost (KiB). A hostile sender
/// cannot force a receiver into an unbounded derivation.
pub const KDF_MAX_MEM_KIB: u32 = 64 * 1024;
/// Hard bounds on attacker-supplied iterations and lanes.
pub const KDF_MIN_PASSES: u32 = 1;
pub const KDF_MAX_PASSES: u32 = 16;
pub const KDF_MIN_LANES: u32 = 1;
pub const KDF_MAX_LANES: u32 = 8;

/// Default Argon2id cost parameters (OWASP interactive-login recommendation:
/// m = 19 MiB, t = 2, p = 1). These are the parameters the encoder uses
/// unless explicitly overridden.
pub const KDF_DEFAULT_MEM_KIB: u32 = 19 * 1024;
pub const KDF_DEFAULT_PASSES: u32 = 2;
pub const KDF_DEFAULT_LANES: u32 = 1;

/// Validated Argon2id cost parameters, wire-representable and capped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory cost in KiB.
    pub mem_kib: u32,
    /// Time cost (iterations).
    pub passes: u32,
    /// Parallelism (lanes).
    pub lanes: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            mem_kib: KDF_DEFAULT_MEM_KIB,
            passes: KDF_DEFAULT_PASSES,
            lanes: KDF_DEFAULT_LANES,
        }
    }
}

impl KdfParams {
    /// Validates parameters against the hard caps and range checks.
    ///
    /// Used both for encoder-side configuration and for hostile
    /// decoder-side parameters; the same limits apply in both directions.
    pub fn validated(mem_kib: u32, passes: u32, lanes: u32) -> Result<Self, PigeonError> {
        let in_range = |v: u32, lo: u32, hi: u32| (lo..=hi).contains(&v);
        if !in_range(mem_kib, KDF_MIN_MEM_KIB, KDF_MAX_MEM_KIB) {
            return Err(PigeonError::KdfParamsExcessive {
                detail: format!(
                    "Argon2id memory cost {mem_kib} KiB is outside the accepted range {}..{} KiB",
                    KDF_MIN_MEM_KIB, KDF_MAX_MEM_KIB
                ),
            });
        }
        if !in_range(passes, KDF_MIN_PASSES, KDF_MAX_PASSES) {
            return Err(PigeonError::KdfParamsExcessive {
                detail: format!(
                    "Argon2id pass count {passes} is outside the accepted range {KDF_MIN_PASSES}..{KDF_MAX_PASSES}"
                ),
            });
        }
        if !in_range(lanes, KDF_MIN_LANES, KDF_MAX_LANES) {
            return Err(PigeonError::KdfParamsExcessive {
                detail: format!(
                    "Argon2id lane count {lanes} is outside the accepted range {KDF_MIN_LANES}..{KDF_MAX_LANES}"
                ),
            });
        }
        Ok(Self {
            mem_kib,
            passes,
            lanes,
        })
    }

    /// Derives the 32-byte AEAD key from a passphrase and salt.
    ///
    /// The returned key is `Zeroizing`: the buffer is wiped on drop. This is
    /// best-effort hygiene — neither the compiler nor the browser runtime
    /// guarantees that no copy of the key material ever existed elsewhere.
    pub fn derive_key(
        &self,
        passphrase: &str,
        salt: &[u8; 16],
    ) -> Result<Zeroizing<[u8; KEY_LEN]>, PigeonError> {
        // Re-validate so no caller can bypass the caps with a hand-built struct.
        let params = Self::validated(self.mem_kib, self.passes, self.lanes)?;
        // Params::new validates internally as well; caps above are the wire-level guard.
        let params = Params::new(params.mem_kib, params.passes, params.lanes, Some(KEY_LEN))
            .map_err(|e| PigeonError::CryptoInit(format!("invalid Argon2id parameters: {e}")))?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        argon
            .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
            .map_err(|e| PigeonError::CryptoInit(format!("Argon2id derivation failed: {e}")))?;
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_are_valid() {
        let p = KdfParams::default();
        assert!(KdfParams::validated(p.mem_kib, p.passes, p.lanes).is_ok());
    }

    #[test]
    fn excessive_memory_rejected() {
        let err = KdfParams::validated(KDF_MAX_MEM_KIB + 1, 2, 1).unwrap_err();
        assert!(matches!(err, PigeonError::KdfParamsExcessive { .. }));
        assert!(KdfParams::validated(KDF_MIN_MEM_KIB - 1, 2, 1).is_err());
    }

    #[test]
    fn excessive_passes_and_lanes_rejected() {
        assert!(KdfParams::validated(19 * 1024, KDF_MAX_PASSES + 1, 1).is_err());
        assert!(KdfParams::validated(19 * 1024, 2, KDF_MAX_LANES + 1).is_err());
        assert!(KdfParams::validated(19 * 1024, 0, 1).is_err());
        assert!(KdfParams::validated(19 * 1024, 2, 0).is_err());
    }

    #[test]
    fn derivation_is_deterministic_and_length_correct() {
        let p = KdfParams {
            mem_kib: 8 * 1024,
            passes: 1,
            lanes: 1,
        };
        let salt = [7u8; 16];
        let k1 = p
            .derive_key("correct horse battery staple", &salt)
            .expect("derives");
        let k2 = p
            .derive_key("correct horse battery staple", &salt)
            .expect("derives");
        assert_eq!(k1.as_ref(), k2.as_ref());
        assert_ne!(*k1, [0u8; 32]);

        let k3 = p.derive_key("wrong passphrase", &salt).expect("derives");
        assert_ne!(k1.as_ref(), k3.as_ref());
    }

    #[test]
    fn salt_changes_the_key() {
        let p = KdfParams {
            mem_kib: 8 * 1024,
            passes: 1,
            lanes: 1,
        };
        let k1 = p.derive_key("pw", &[1u8; 16]).expect("derives");
        let k2 = p.derive_key("pw", &[2u8; 16]).expect("derives");
        assert_ne!(k1.as_ref(), k2.as_ref());
    }

    #[test]
    fn derived_key_ignores_out_of_range_struct_fields() {
        // A hand-built struct with fields above the caps must fail on derive.
        let p = KdfParams {
            mem_kib: u32::MAX,
            passes: 2,
            lanes: 1,
        };
        assert!(p.derive_key("pw", &[0u8; 16]).is_err());
    }
}
