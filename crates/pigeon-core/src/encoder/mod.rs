//! The send-side pipeline: file → compress → (encrypt) → fragment → fountain
//! encode → wire packets → (QR matrices / animated GIF).
//!
//! Encryption (protocol v2) is optional and controlled by
//! [`EncodeConfig::encryption`]. When enabled, the whole (possibly
//! compressed) object is sealed once with XChaCha20-Poly1305 (key from
//! Argon2id, AAD = transfer id) and the sealed blob is what the fountain
//! code transmits; every wire packet's payload is additionally wrapped in a
//! per-packet envelope. Unencrypted transfers remain byte-compatible v1.

pub mod fountain;
pub mod fragment;
pub mod gif;
pub mod packet;
pub mod qr;

use serde::Serialize;
use zeroize::Zeroizing;

use crate::crypto;
use crate::error::PigeonError;
use crate::integrity::sha256;
use crate::protocol::metadata::{sanitize_filename, Compression, TransferMetadata};
use crate::protocol::{new_transfer_id, MAX_ENCODE_FILE_SIZE};

use self::fountain::FountainEncoder;
pub use self::fragment::CompressionMode;
pub use self::qr::{QrEcl, QrMatrix};
use fast_qr::Version;

/// How often the metadata packet is re-emitted inside the frame sequence so a
/// receiver that joins mid-loop still learns the OTI and file identity.
pub const METADATA_REPEAT_FRAMES: usize = 500;

/// Target per-frame payload budget (symbol size in bytes) before the wire
/// overheads (header, payload id, envelope) are subtracted from the QR
/// capacity.
const TARGET_SYMBOL_BYTES: usize = 1024;

/// Optional encryption settings for a transfer (protocol v2).
#[derive(Debug, Clone)]
pub struct EncryptionConfig {
    /// The transfer passphrase. Wiped on drop (best-effort hygiene).
    pub passphrase: Zeroizing<String>,
    /// Argon2id cost parameters (defaults to the OWASP recommendation).
    pub kdf: crypto::KdfParams,
}

/// Tunable encoding configuration (deserialized from JavaScript).
#[derive(Debug, Clone)]
pub struct EncodeConfig {
    pub filename: String,
    pub mime_type: Option<String>,
    pub compression: CompressionMode,
    /// Extra repair symbols as a percentage of source symbols (0..=200).
    pub overhead_percent: u32,
    pub qr_ecl: QrEcl,
    /// Explicit QR version 1..=40, or None for automatic selection.
    pub qr_version: Option<u8>,
    /// GIF/live frame duration in milliseconds (60..=2000).
    pub frame_delay_ms: u32,
    /// Soft per-file size limit; must not exceed [`MAX_ENCODE_FILE_SIZE`].
    pub max_file_size: u64,
    /// When set, the transfer is encrypted (protocol v2). `None` produces a
    /// byte-compatible v1 transfer.
    pub encryption: Option<EncryptionConfig>,
}

impl Default for EncodeConfig {
    fn default() -> Self {
        Self {
            filename: "pigeon-file".into(),
            mime_type: None,
            compression: CompressionMode::Auto,
            overhead_percent: 30,
            qr_ecl: QrEcl::L,
            qr_version: None,
            frame_delay_ms: 250,
            max_file_size: 64 * 1024 * 1024,
            encryption: None,
        }
    }
}

impl EncodeConfig {
    /// Validates and normalizes the configuration.
    pub fn validated(mut self) -> Result<Self, PigeonError> {
        self.filename = sanitize_filename(&self.filename);
        if self.filename.is_empty() {
            return Err(PigeonError::FilenameEmpty);
        }
        if let Some(mime) = &self.mime_type {
            if mime.len() > crate::protocol::MAX_MIME_BYTES {
                return Err(PigeonError::MimeTooLong {
                    max: crate::protocol::MAX_MIME_BYTES,
                });
            }
        }
        if self.overhead_percent > 200 {
            return Err(PigeonError::InvalidConfig(format!(
                "overhead_percent {} out of range 0..=200",
                self.overhead_percent
            )));
        }
        if self.frame_delay_ms < 60 || self.frame_delay_ms > 2000 {
            return Err(PigeonError::InvalidConfig(format!(
                "frame_delay_ms {} out of range 60..=2000",
                self.frame_delay_ms
            )));
        }
        if self.max_file_size > MAX_ENCODE_FILE_SIZE {
            return Err(PigeonError::InvalidConfig(format!(
                "max_file_size {} exceeds the engine cap of {MAX_ENCODE_FILE_SIZE}",
                self.max_file_size
            )));
        }
        if self.max_file_size == 0 {
            return Err(PigeonError::InvalidConfig(
                "max_file_size must be positive".into(),
            ));
        }
        if let Some(enc) = &self.encryption {
            if enc.passphrase.is_empty() {
                return Err(PigeonError::InvalidConfig(
                    "an encryption passphrase is required when encryption is enabled; \
                     omit encryption for an unencrypted transfer"
                        .into(),
                ));
            }
            // Same validation as the receive path applies to the send path.
            crypto::KdfParams::validated(enc.kdf.mem_kib, enc.kdf.passes, enc.kdf.lanes)?;
        }
        Ok(self)
    }
}

/// Aggregate statistics about a completed encoding.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeStats {
    #[serde(serialize_with = "crate::protocol::ser::u64_hex")]
    pub transfer_id: u64,
    /// Byte size of the original input file.
    pub original_size: u64,
    /// Byte size after compression (== original size when uncompressed).
    pub encoded_size: u64,
    pub compression_used: bool,
    pub original_sha256: String,
    pub filename: String,
    pub mime_type: Option<String>,
    pub qr_version: u8,
    pub qr_ecl: String,
    /// Payload bytes carried by one QR frame.
    pub packet_payload_bytes: usize,
    pub source_symbols: usize,
    pub repair_symbols: usize,
    /// Total QR frames in the sequence (metadata repeats included).
    pub total_frames: usize,
    pub frame_delay_ms: u32,
    /// Required unique packets for reconstruction (source symbols).
    pub needed_packets: usize,
    /// True when the transfer is encrypted (protocol v2).
    pub encrypted: bool,
}

/// Result of encoding a file into the frame sequence (before rendering).
#[derive(Debug, Clone)]
pub struct EncodedTransfer {
    pub stats: EncodeStats,
    /// Wire bytes of the metadata packet.
    pub metadata_packet: Vec<u8>,
    /// Wire bytes of each symbol packet, in transmit order.
    pub packet_frames: Vec<Vec<u8>>,
    /// Full transmit sequence (metadata + packets, metadata re-emitted periodically).
    pub frame_sequence: Vec<Vec<u8>>,
}

/// Encodes a file into the Pigeon frame sequence.
///
/// The sequence starts with the metadata packet, then all symbol packets,
/// re-emitting the metadata every [`METADATA_REPEAT_FRAMES`] frames.
pub fn encode_file(data: &[u8], config: EncodeConfig) -> Result<EncodedTransfer, PigeonError> {
    let config = config.validated()?;
    if data.len() as u64 > config.max_file_size {
        return Err(PigeonError::FileTooLarge {
            size: data.len() as u64,
            max: config.max_file_size,
        });
    }

    let transfer_id = new_transfer_id();
    let original_sha = sha256(data);

    // 1. Optional compression with the auto heuristic.
    let (payload, compression) = fragment::prepare_payload(data, config.compression)?;

    // 2. Optional encryption: derive the key and seal the whole object. The
    //    sealed blob (nonce || ciphertext || tag, bound to the transfer id)
    //    is what the fountain code transmits; reconstruction yields nothing
    //    until its tag verifies.
    let encryption = match &config.encryption {
        Some(enc) => {
            let salt = crypto::random_salt();
            let envelope = crypto::MetadataEnvelope { kdf: enc.kdf, salt };
            let key = crypto::DecryptionKey::derive(&enc.passphrase, &envelope)?;
            Some((key, salt, enc.kdf))
        }
        None => None,
    };
    let payload = match &encryption {
        Some((key, _, _)) => crypto::seal_object(key.as_bytes(), transfer_id, &payload)?,
        None => payload,
    };
    let envelope_overhead = if encryption.is_some() {
        crate::crypto::envelope::SYMBOL_ENVELOPE_FIXED + crate::crypto::aead::TAG_LEN
    } else {
        0
    };

    // 3. Choose the QR geometry and derive the per-frame payload budget.
    let version = match config.qr_version {
        Some(v) => qr::version_from_u8(v)?,
        None => qr::smallest_version_for(TARGET_SYMBOL_BYTES + envelope_overhead, config.qr_ecl)
            .ok_or_else(|| PigeonError::QrCapacityExceeded {
                payload: TARGET_SYMBOL_BYTES + envelope_overhead,
                version: 40,
            })?,
    };
    // Header (20) + payload id (4) + symbol envelope (v2) come out of the QR
    // byte capacity; what remains is the fountain symbol size.
    let qr_capacity = qr::probe_byte_capacity(version, config.qr_ecl);
    let packet_payload = qr_capacity
        .checked_sub(crate::protocol::HEADER_LEN + 4 + envelope_overhead)
        .filter(|v| *v > 0)
        .ok_or(PigeonError::QrCapacityExceeded {
            payload: crate::protocol::HEADER_LEN + 4 + envelope_overhead,
            version: qr::version_to_u8(version),
        })?;

    // 4. Fountain encode the (possibly compressed, possibly sealed) payload.
    let fountain = FountainEncoder::new(&payload, packet_payload);
    let needed = fountain.source_symbol_count();

    let metadata = TransferMetadata {
        filename: config.filename.clone(),
        mime_type: config.mime_type.clone(),
        compression,
        original_size: data.len() as u64,
        sha256: original_sha,
    };

    let metadata_packet = match &encryption {
        Some((key, salt, kdf)) => packet::build_encrypted_metadata_packet(
            transfer_id,
            key.as_bytes(),
            kdf,
            salt,
            &fountain,
            &metadata,
        )?,
        None => packet::build_metadata_packet(transfer_id, &fountain, &metadata)?,
    };
    let symbol_packets: Vec<Vec<u8>> = fountain
        .transmission_sequence(config.overhead_percent)
        .iter()
        .map(|p| match &encryption {
            Some((key, _, _)) => {
                packet::build_encrypted_symbol_packet(transfer_id, key.as_bytes(), p)
            }
            None => Ok(packet::build_symbol_packet(transfer_id, p)),
        })
        .collect::<Result<Vec<_>, _>>()?;

    // 5. Transmit order: metadata first, symbols, then periodic metadata repeats.
    let mut frame_sequence: Vec<Vec<u8>> = Vec::with_capacity(
        symbol_packets.len() + 2 + symbol_packets.len() / METADATA_REPEAT_FRAMES,
    );
    frame_sequence.push(metadata_packet.clone());
    for (i, pkt) in symbol_packets.iter().enumerate() {
        frame_sequence.push(pkt.clone());
        if (i + 1) % METADATA_REPEAT_FRAMES == 0 && i + 1 < symbol_packets.len() {
            frame_sequence.push(metadata_packet.clone());
        }
    }
    // An unencrypted empty file needs no symbols; its single metadata packet
    // completes it. An encrypted empty file transmits its 40-byte sealed
    // object (nonce + tag) as one symbol, so the receiver authenticates it.

    let repair = symbol_packets.len().saturating_sub(needed);
    let stats = EncodeStats {
        transfer_id,
        original_size: data.len() as u64,
        encoded_size: payload.len() as u64,
        compression_used: compression == Compression::Zlib,
        original_sha256: crate::integrity::sha256_hex(data),
        filename: metadata.filename.clone(),
        mime_type: metadata.mime_type.clone(),
        qr_version: qr::version_to_u8(version),
        qr_ecl: format!("{:?}", config.qr_ecl).to_lowercase(),
        packet_payload_bytes: packet_payload,
        source_symbols: needed,
        repair_symbols: repair,
        total_frames: frame_sequence.len(),
        frame_delay_ms: config.frame_delay_ms,
        needed_packets: needed,
        encrypted: encryption.is_some(),
    };

    Ok(EncodedTransfer {
        stats,
        metadata_packet,
        packet_frames: symbol_packets,
        frame_sequence,
    })
}

/// Encodes every frame of a transfer into QR matrices, in transmit order.
pub fn encode_qr_matrices(
    transfer: &EncodedTransfer,
    _config: &EncodeConfig,
) -> Result<Vec<QrMatrix>, PigeonError> {
    let version = qr::version_from_u8(transfer.stats.qr_version)?;
    let ecl = QrEcl::parse(&transfer.stats.qr_ecl)?;
    transfer
        .frame_sequence
        .iter()
        .map(|frame| qr::qr_encode(frame, version, ecl))
        .collect()
}

/// Streaming QR generation over a transfer's frame sequence.
///
/// Holds only the transfer bytes plus the current matrix, so live playback
/// and GIF writing never materialize every matrix at once.
pub struct QrSession {
    transfer: EncodedTransfer,
    version: Version,
    ecl: QrEcl,
    cursor: usize,
}

impl QrSession {
    pub fn new(transfer: EncodedTransfer) -> Result<Self, PigeonError> {
        let version = qr::version_from_u8(transfer.stats.qr_version)?;
        let ecl = QrEcl::parse(&transfer.stats.qr_ecl)?;
        Ok(Self {
            transfer,
            version,
            ecl,
            cursor: 0,
        })
    }

    #[must_use]
    pub fn total_frames(&self) -> usize {
        self.transfer.frame_sequence.len()
    }

    #[must_use]
    pub fn stats(&self) -> &EncodeStats {
        &self.transfer.stats
    }

    /// Encodes the next frame into a QR matrix; `Ok(None)` when done.
    pub fn next_matrix(&mut self) -> Result<Option<QrMatrix>, PigeonError> {
        if self.cursor >= self.transfer.frame_sequence.len() {
            return Ok(None);
        }
        let frame = &self.transfer.frame_sequence[self.cursor];
        self.cursor += 1;
        qr::qr_encode(frame, self.version, self.ecl).map(Some)
    }
}

impl Iterator for QrSession {
    type Item = Result<QrMatrix, PigeonError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_matrix().transpose()
    }
}

/// Pre-encode size/throughput estimate for a file of a known byte size.
///
/// Every number is an explicit estimate: compression outcomes and GIF LZW
/// ratios vary with content, so the receiver-facing numbers are bounds with
/// the assumptions stated, never invented precision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferEstimate {
    pub qr_version: u8,
    pub qr_ecl: String,
    /// Payload bytes carried by one QR frame (symbol + wire overheads).
    pub packet_payload_bytes: usize,
    /// QR frames for an incompressible input of this size (upper bound,
    /// metadata repeats included).
    pub frames_incompressible: usize,
    /// QR frames if compression halves the input (roughly the best case for
    /// text-like data; media skips compression entirely).
    pub frames_compressible_optimistic: usize,
    pub encrypted: bool,
    /// Loop duration in seconds at the configured frame delay
    /// (incompressible upper bound).
    pub loop_seconds: u64,
    /// GIF size estimate in bytes at the default 800px frame target, using
    /// the measured ≈0.05 bytes/pixel two-colour LZW figure from
    /// docs/BENCHMARKS.md. Expect ±30% either way with content.
    pub gif_bytes_estimate: u64,
}

/// Estimates the transfer shape for an input of `input_size` bytes without
/// touching the data. Mirrors the geometry selection of [`encode_file`] —
/// QR version, symbol size, envelope overheads — but performs no fountain
/// or rendering work.
pub fn estimate_transfer(
    input_size: u64,
    config: &EncodeConfig,
) -> Result<TransferEstimate, PigeonError> {
    let config = config.clone().validated()?;
    let encrypted = config.encryption.is_some();
    let envelope_overhead = if encrypted {
        crate::crypto::envelope::SYMBOL_ENVELOPE_FIXED + crate::crypto::aead::TAG_LEN
    } else {
        0
    };

    let version = match config.qr_version {
        Some(v) => qr::version_from_u8(v)?,
        None => qr::smallest_version_for(TARGET_SYMBOL_BYTES + envelope_overhead, config.qr_ecl)
            .ok_or_else(|| PigeonError::QrCapacityExceeded {
                payload: TARGET_SYMBOL_BYTES + envelope_overhead,
                version: 40,
            })?,
    };
    let qr_capacity = qr::probe_byte_capacity(version, config.qr_ecl);
    let packet_payload = qr_capacity
        .checked_sub(crate::protocol::HEADER_LEN + 4 + envelope_overhead)
        .filter(|v| *v > 0)
        .ok_or(PigeonError::QrCapacityExceeded {
            payload: crate::protocol::HEADER_LEN + 4 + envelope_overhead,
            version: qr::version_to_u8(version),
        })?;

    // Source symbols for a given (possibly compressed) object size, with the
    // configured redundancy applied, plus the metadata packet and its
    // periodic re-emissions — the same transmit order as encode_file.
    let frames_for = |object_bytes: u64| -> usize {
        let source = object_bytes.div_ceil(packet_payload as u64).max(1) as usize;
        let with_repair =
            (source as u64 * (100 + config.overhead_percent as u64) / 100).max(1) as usize;
        let repeats = with_repair / METADATA_REPEAT_FRAMES;
        1 + with_repair + repeats
    };
    let frames_incompressible = frames_for(input_size);
    let frames_compressible_optimistic = frames_for(input_size / 2);

    // Frame pixel geometry at the default render target (800 px): reuse the
    // exact scale logic the GIF writer uses.
    let modules = (qr::version_to_u8(version) as usize * 4 + 17) + 2 * gif::QUIET_MODULES;
    let scale = gif::scale_for_target_px(800, modules) as usize;
    let frame_px = modules * scale;
    // Measured two-colour LZW figure: ≈0.05 bytes per pixel.
    let bytes_per_frame = (frame_px as f64) * (frame_px as f64) * 0.05;
    let gif_bytes_estimate = (frames_incompressible as f64 * bytes_per_frame).max(1.0) as u64;

    Ok(TransferEstimate {
        qr_version: qr::version_to_u8(version),
        qr_ecl: format!("{:?}", config.qr_ecl).to_lowercase(),
        packet_payload_bytes: packet_payload,
        frames_incompressible,
        frames_compressible_optimistic,
        encrypted,
        loop_seconds: (frames_incompressible as u64 * config.frame_delay_ms as u64) / 1000,
        gif_bytes_estimate,
    })
}

/// Rendering parameters for GIF/live output derived from a transfer.
#[derive(Debug, Clone, Copy)]
pub struct RenderParams {
    pub frame_delay_ms: u32,
    pub target_frame_px: u32,
}

impl RenderParams {
    /// Derives rendering parameters from encode stats.
    #[must_use]
    pub fn from_stats(stats: &EncodeStats) -> Self {
        Self {
            frame_delay_ms: stats.frame_delay_ms,
            target_frame_px: 800,
        }
    }
}

/// Full send pipeline for the GIF export path: file → animated GIF bytes.
///
/// QR matrices are generated and written one frame at a time.
pub fn encode_gif_bytes(
    data: &[u8],
    config: EncodeConfig,
    target_frame_px: u32,
) -> Result<(Vec<u8>, EncodeStats), PigeonError> {
    let transfer = encode_file(data, config)?;
    let stats = transfer.stats.clone();
    let mut session = QrSession::new(transfer)?;

    let first = session
        .next_matrix()?
        .ok_or_else(|| PigeonError::InvalidConfig("transfer has no frames".into()))?;
    let modules = first.size + 2 * gif::QUIET_MODULES;
    let scale = gif::scale_for_target_px(target_frame_px, modules);
    let gif_bytes = gif::encode_gif_streaming(first, &mut session, scale, stats.frame_delay_ms)?;
    Ok((gif_bytes, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(name: &str) -> EncodeConfig {
        EncodeConfig {
            filename: name.into(),
            mime_type: Some("text/plain".into()),
            ..Default::default()
        }
    }

    fn encrypted_config(name: &str, passphrase: &str) -> EncodeConfig {
        EncodeConfig {
            filename: name.into(),
            mime_type: Some("text/plain".into()),
            encryption: Some(EncryptionConfig {
                passphrase: Zeroizing::new(passphrase.into()),
                kdf: crypto::KdfParams {
                    mem_kib: 8 * 1024,
                    passes: 1,
                    lanes: 1,
                },
            }),
            ..Default::default()
        }
    }

    #[test]
    fn encrypted_transfer_uses_v2_packets() {
        let transfer =
            encode_file(b"secret contents", encrypted_config("secret.txt", "pw")).expect("encodes");
        assert!(transfer.stats.encrypted);
        // First frame: v2 metadata with the encryption flag.
        let (header, _) =
            crate::protocol::header::parse_packet(&transfer.metadata_packet).expect("valid");
        assert_eq!(header.version, crate::protocol::VERSION_V2);
        assert_eq!(
            header.flags,
            crate::protocol::FLAG_METADATA | crate::protocol::FLAG_ENCRYPTED
        );
        // Symbol frames: v2, encrypted, no metadata bit.
        for pkt in &transfer.packet_frames {
            let (h, _) = crate::protocol::header::parse_packet(pkt).expect("valid");
            assert_eq!(h.version, crate::protocol::VERSION_V2);
            assert_eq!(h.flags, crate::protocol::FLAG_ENCRYPTED);
        }
    }

    #[test]
    fn encrypted_metadata_hides_the_filename() {
        let transfer = encode_file(
            b"secret contents",
            encrypted_config("tax-records.pdf", "pw"),
        )
        .expect("encodes");
        // The v1 plaintext metadata structure must not appear anywhere in
        // the encrypted metadata packet or any symbol packet.
        for frame in &transfer.frame_sequence {
            assert!(!frame
                .windows(b"tax-records.pdf".len())
                .any(|w| w == b"tax-records.pdf"));
        }
    }

    #[test]
    fn empty_encrypted_file_has_one_symbol() {
        let transfer = encode_file(&[], encrypted_config("empty.txt", "pw")).expect("encodes");
        assert_eq!(
            transfer.stats.total_frames, 2,
            "metadata + one sealed-object symbol"
        );
        assert_eq!(transfer.stats.needed_packets, 1);
    }

    #[test]
    fn empty_passphrase_rejected() {
        let cfg = EncodeConfig {
            filename: "x.txt".into(),
            encryption: Some(EncryptionConfig {
                passphrase: Zeroizing::new(String::new()),
                kdf: crypto::KdfParams::default(),
            }),
            ..Default::default()
        };
        assert!(matches!(
            encode_file(b"x", cfg),
            Err(PigeonError::InvalidConfig(_))
        ));
    }

    #[test]
    fn encrypted_unencrypted_sizes_differ_reasonably() {
        // Encryption adds ~72 bytes per packet (envelope + tag + object
        // sealing); frames may grow by one, never by orders of magnitude.
        let data = vec![0x5Au8; 20_000];
        let plain = encode_file(&data, config("p.bin")).expect("plain");
        let cipher = encode_file(&data, encrypted_config("c.bin", "pw")).expect("cipher");
        assert!(
            cipher.stats.total_frames <= plain.stats.total_frames + 2,
            "plain {} frames vs encrypted {} frames",
            plain.stats.total_frames,
            cipher.stats.total_frames
        );
    }

    #[test]
    fn estimate_matches_actual_encode_for_incompressible_input() {
        // High-entropy data: compression stays off, so the estimate's
        // incompressible bound must predict the real frame count exactly.
        let mut state = 0x9E3779B97F4A7C15u64;
        let data: Vec<u8> = (0..50_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let cfg = EncodeConfig {
            filename: "est.bin".into(),
            compression: CompressionMode::Off,
            ..Default::default()
        };
        let estimate = estimate_transfer(data.len() as u64, &cfg).expect("estimates");
        let transfer = encode_file(&data, cfg).expect("encodes");
        assert_eq!(estimate.frames_incompressible, transfer.stats.total_frames);
        assert_eq!(estimate.qr_version, transfer.stats.qr_version);
        assert_eq!(
            estimate.packet_payload_bytes,
            transfer.stats.packet_payload_bytes
        );
        assert!(!estimate.encrypted);
        assert!(estimate.gif_bytes_estimate > 0);
    }

    #[test]
    fn estimate_reports_encryption_and_bounds() {
        let cfg = EncodeConfig {
            filename: "est.bin".into(),
            encryption: Some(EncryptionConfig {
                passphrase: Zeroizing::new("pw".into()),
                kdf: crypto::KdfParams {
                    mem_kib: 8 * 1024,
                    passes: 1,
                    lanes: 1,
                },
            }),
            ..Default::default()
        };
        let plain_cfg = EncodeConfig {
            filename: "est.bin".into(),
            ..Default::default()
        };
        let enc = estimate_transfer(1_000_000, &cfg).expect("estimates");
        let plain = estimate_transfer(1_000_000, &plain_cfg).expect("estimates");
        assert!(enc.encrypted);
        assert!(!plain.encrypted);
        // Encrypted packets carry less payload, so the frame bound can only
        // be equal or higher for the same input.
        assert!(enc.frames_incompressible >= plain.frames_incompressible);
        assert!(enc.frames_compressible_optimistic <= enc.frames_incompressible);
        assert!(enc.loop_seconds >= plain.loop_seconds);
    }

    #[test]
    fn encode_small_file_shape() {
        let data = b"hello pigeon, this is a test".to_vec();
        let transfer = encode_file(&data, config("hello.txt")).expect("encodes");
        assert!(transfer.stats.total_frames >= 2); // metadata + at least 1 symbol
        assert_eq!(transfer.frame_sequence[0], transfer.metadata_packet);
        assert_eq!(transfer.stats.source_symbols, transfer.stats.needed_packets);
        assert_eq!(
            transfer.stats.original_sha256,
            crate::integrity::sha256_hex(&data)
        );
    }

    #[test]
    fn empty_file_is_metadata_only() {
        let transfer = encode_file(&[], config("empty.txt")).expect("encodes");
        assert_eq!(transfer.stats.total_frames, 1);
        assert_eq!(transfer.stats.needed_packets, 0);
    }

    #[test]
    fn one_byte_file_needs_one_symbol() {
        let transfer = encode_file(&[0x42], config("one.bin")).expect("encodes");
        assert_eq!(transfer.stats.source_symbols, 1);
        assert_eq!(
            transfer.packet_frames.len(),
            1 + transfer.stats.repair_symbols
        );
    }

    #[test]
    fn metadata_repeats_periodically() {
        // Compression disabled so the transfer genuinely spans hundreds of frames.
        let mut state = 0x9E3779B97F4A7C15u64;
        let data: Vec<u8> = (0..600_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let cfg = EncodeConfig {
            filename: "big.bin".into(),
            compression: CompressionMode::Off,
            ..Default::default()
        };
        let transfer = encode_file(&data, cfg).expect("encodes");
        let metadata_count = transfer
            .frame_sequence
            .iter()
            .filter(|f| **f == transfer.metadata_packet)
            .count();
        assert!(
            metadata_count >= 2,
            "metadata should repeat for long transfers"
        );
    }

    #[test]
    fn file_size_limit_enforced() {
        let data = vec![0u8; 1000];
        let cfg = EncodeConfig {
            max_file_size: 100,
            ..Default::default()
        };
        assert!(matches!(
            encode_file(&data, cfg),
            Err(PigeonError::FileTooLarge { .. })
        ));
    }

    #[test]
    fn invalid_config_rejected() {
        let bad = EncodeConfig {
            overhead_percent: 500,
            ..Default::default()
        };
        assert!(encode_file(b"x", bad).is_err());
        let bad_delay = EncodeConfig {
            frame_delay_ms: 10,
            ..Default::default()
        };
        assert!(encode_file(b"x", bad_delay).is_err());
    }

    #[test]
    fn unicode_filename_flows_through() {
        let transfer = encode_file(b"data", config("文档 报告.pdf")).expect("encodes");
        assert_eq!(transfer.stats.filename, "文档 报告.pdf");
    }

    #[test]
    fn full_gif_pipeline_roundtrip_bytes() {
        let data = b"the quick brown fox jumps over the lazy dog".to_vec();
        let (gif_bytes, stats) = encode_gif_bytes(&data, config("fox.txt"), 600).expect("encodes");
        assert!(gif_bytes.starts_with(b"GIF89a"));
        assert!(stats.total_frames >= 2);
    }
}
