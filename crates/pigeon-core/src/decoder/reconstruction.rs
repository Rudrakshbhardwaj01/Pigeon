//! Final reconstruction: decompression with strict limits and SHA-256
//! verification. A file is only "done" when its checksum matches.

use crate::crypto::{self, DecryptionKey};
use crate::error::PigeonError;
use crate::integrity::sha256;
use crate::protocol::metadata::{Compression, TransferMetadata};
use raptorq::ObjectTransmissionInformation;

/// A fully reconstructed and verified file.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconstructedFile {
    pub filename: String,
    pub mime_type: Option<String>,
    #[serde(serialize_with = "crate::protocol::ser::bytes")]
    pub data: Vec<u8>,
    pub sha256: String,
    pub original_size: u64,
}

/// Runs the final verification pipeline over a reconstructed *sealed* object
/// (encrypted transfer): length check against the OTI, object-layer AEAD
/// open (bound to the transfer id), then decompression + checksum
/// verification.
///
/// Authentication happens before anything else: a wrong passphrase or a
/// tampered object fails here and no plaintext is ever produced.
pub fn finalize_sealed(
    sealed: Vec<u8>,
    key: &DecryptionKey,
    transfer_id: u64,
    meta: &TransferMetadata,
    oti: &ObjectTransmissionInformation,
) -> Result<ReconstructedFile, PigeonError> {
    // The RaptorQ transfer length is authoritative for the *sealed* object.
    if sealed.len() as u64 != oti.transfer_length() {
        return Err(PigeonError::Internal(format!(
            "reconstructed object is {} bytes, OTI declared {}",
            sealed.len(),
            oti.transfer_length()
        )));
    }
    let plain = crypto::open_object(key.as_bytes(), transfer_id, &sealed)?;
    finalize_inner(plain, meta)
}

/// Runs the final verification pipeline over reconstructed (still compressed)
/// object bytes.
pub fn finalize(
    raw: Vec<u8>,
    meta: &TransferMetadata,
    oti: &ObjectTransmissionInformation,
) -> Result<ReconstructedFile, PigeonError> {
    // The RaptorQ transfer length is authoritative for the transmitted object.
    if raw.len() as u64 != oti.transfer_length() {
        return Err(PigeonError::Internal(format!(
            "reconstructed object is {} bytes, OTI declared {}",
            raw.len(),
            oti.transfer_length()
        )));
    }
    finalize_inner(raw, meta)
}

/// Decompression + checksum verification shared by both dialects. The OTI
/// length check happens in the callers: for encrypted transfers the OTI
/// describes the sealed object, so it must not be applied to the plaintext.
fn finalize_inner(
    compressed: Vec<u8>,
    meta: &TransferMetadata,
) -> Result<ReconstructedFile, PigeonError> {
    let data = match meta.compression {
        Compression::None => {
            if compressed.len() as u64 != meta.original_size {
                return Err(PigeonError::DecompressedSizeMismatch {
                    expected: meta.original_size,
                    got: compressed.len() as u64,
                });
            }
            compressed
        }
        Compression::Zlib => {
            
            super::super::encoder::fragment::decompress_bounded(
                &compressed,
                meta.original_size,
            )?
        }
    };

    let actual = sha256(&data);
    if actual != meta.sha256 {
        return Err(PigeonError::ChecksumMismatch {
            expected: crate::integrity::hex(&meta.sha256),
            computed: crate::integrity::hex(&actual),
        });
    }

    Ok(ReconstructedFile {
        filename: meta.filename.clone(),
        mime_type: meta.mime_type.clone(),
        original_size: data.len() as u64,
        data,
        sha256: crate::integrity::hex(&actual),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::fragment::{decompress_bounded, prepare_payload};
    use crate::encoder::CompressionMode;
    use raptorq::ObjectTransmissionInformation;

    fn oti_for(data: &[u8]) -> ObjectTransmissionInformation {
        ObjectTransmissionInformation::with_defaults(data.len() as u64, 1024)
    }

    fn meta_for(data: &[u8], compression: Compression) -> TransferMetadata {
        TransferMetadata {
            filename: "f.bin".into(),
            mime_type: None,
            compression,
            original_size: data.len() as u64,
            sha256: sha256(data),
        }
    }

    #[test]
    fn plain_finalize_verifies() {
        let data = b"plain data".to_vec();
        let file = finalize(
            data.clone(),
            &meta_for(&data, Compression::None),
            &oti_for(&data),
        )
        .expect("ok");
        assert_eq!(file.data, data);
        assert_eq!(file.sha256, crate::integrity::sha256_hex(&data));
    }

    #[test]
    fn compressed_finalize_verifies() {
        let data = vec![b'A'; 50_000];
        let (compressed, algo) = prepare_payload(&data, CompressionMode::On).expect("ok");
        assert_eq!(algo, Compression::Zlib);
        let meta = meta_for(&data, Compression::Zlib);
        let oti = oti_for(&compressed);
        let file = finalize(compressed, &meta, &oti).expect("ok");
        assert_eq!(file.data, data);
    }

    #[test]
    fn checksum_mismatch_is_fatal() {
        let data = b"original".to_vec();
        let mut meta = meta_for(&data, Compression::None);
        meta.sha256 = [0u8; 32];
        assert!(matches!(
            finalize(data, &meta, &oti_for(b"original")),
            Err(PigeonError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn size_mismatch_is_fatal() {
        let data = b"12345678".to_vec();
        let mut meta = meta_for(&data, Compression::None);
        meta.original_size = 99;
        assert!(matches!(
            finalize(data, &meta, &oti_for(b"12345678")),
            Err(PigeonError::DecompressedSizeMismatch { .. })
        ));
    }

    #[test]
    fn empty_file_finalizes() {
        let meta = meta_for(b"", Compression::None);
        // with_defaults(0, ...) panics inside raptorq; use the explicit empty OTI.
        let oti = raptorq::ObjectTransmissionInformation::deserialize(
            &crate::encoder::fountain::FountainEncoder::empty_oti(),
        );
        let file = finalize(Vec::new(), &meta, &oti).expect("ok");
        assert!(file.data.is_empty());
        assert_eq!(file.sha256, crate::integrity::sha256_hex(b""));
    }

    #[test]
    fn decompression_limit_still_applies_in_finalize() {
        let data = vec![b'A'; 20_000];
        let (compressed, _) = prepare_payload(&data, CompressionMode::On).expect("ok");
        let mut meta = meta_for(&data, Compression::Zlib);
        meta.original_size = 5; // lie about the size
        let oti = oti_for(&compressed);
        assert!(finalize(compressed, &meta, &oti).is_err());
        let _ = decompress_bounded; // keep import used
    }
}
