//! The send-side pipeline: file → compress → fragment → fountain encode →
//! wire packets → (QR matrices / animated GIF).

pub mod fragment;
pub mod fountain;
pub mod gif;
pub mod packet;
pub mod qr;

use serde::Serialize;

use crate::error::PigeonError;
use crate::integrity::sha256;
use crate::protocol::metadata::{sanitize_filename, Compression, TransferMetadata};
use crate::protocol::{new_transfer_id, MAX_ENCODE_FILE_SIZE};

pub use self::fragment::CompressionMode;
pub use self::qr::{QrEcl, QrMatrix};
use self::fountain::FountainEncoder;
use fast_qr::Version;

/// How often the metadata packet is re-emitted inside the frame sequence so a
/// receiver that joins mid-loop still learns the OTI and file identity.
pub const METADATA_REPEAT_FRAMES: usize = 500;

/// Tunable encoding configuration (deserialized from JavaScript).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
            return Err(PigeonError::InvalidConfig("max_file_size must be positive".into()));
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

    // 2. Choose the QR geometry and derive the per-frame payload budget.
    let version = match config.qr_version {
        Some(v) => qr::version_from_u8(v)?,
        None => qr::smallest_version_for(1024, config.qr_ecl).ok_or_else(|| {
            PigeonError::QrCapacityExceeded {
                payload: 1024,
                version: 40,
            }
        })?,
    };
    // Header (20) + payload id (4) come out of the QR byte capacity.
    let qr_capacity = qr::probe_byte_capacity(version, config.qr_ecl);
    let packet_payload = qr_capacity
        .checked_sub(crate::protocol::HEADER_LEN + 4)
        .filter(|v| *v > 0)
        .ok_or(PigeonError::QrCapacityExceeded {
            payload: crate::protocol::HEADER_LEN + 4,
            version: qr::version_to_u8(version),
        })?;

    // 3. Fountain encode the (possibly compressed) payload.
    let fountain = FountainEncoder::new(&payload, packet_payload);
    let needed = fountain.source_symbol_count();

    let metadata = TransferMetadata {
        filename: config.filename.clone(),
        mime_type: config.mime_type.clone(),
        compression,
        original_size: data.len() as u64,
        sha256: original_sha,
    };

    let metadata_packet = packet::build_metadata_packet(transfer_id, &fountain, &metadata)?;
    let symbol_packets: Vec<Vec<u8>> = fountain
        .transmission_sequence(config.overhead_percent)
        .iter()
        .map(|p| packet::build_symbol_packet(transfer_id, p))
        .collect();

    // 4. Transmit order: metadata first, symbols, then periodic metadata repeats.
    let mut frame_sequence: Vec<Vec<u8>> = Vec::with_capacity(symbol_packets.len() + 2 + symbol_packets.len() / METADATA_REPEAT_FRAMES);
    frame_sequence.push(metadata_packet.clone());
    for (i, pkt) in symbol_packets.iter().enumerate() {
        frame_sequence.push(pkt.clone());
        if (i + 1) % METADATA_REPEAT_FRAMES == 0 && i + 1 < symbol_packets.len() {
            frame_sequence.push(metadata_packet.clone());
        }
    }
    // An empty file needs no symbols; its single metadata packet completes it.

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
    let gif_bytes = gif::encode_gif_streaming(
        first,
        &mut session,
        scale,
        stats.frame_delay_ms,
    )?;
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

    #[test]
    fn encode_small_file_shape() {
        let data = b"hello pigeon, this is a test".to_vec();
        let transfer = encode_file(&data, config("hello.txt")).expect("encodes");
        assert!(transfer.stats.total_frames >= 2); // metadata + at least 1 symbol
        assert_eq!(transfer.frame_sequence[0], transfer.metadata_packet);
        assert_eq!(transfer.stats.source_symbols, transfer.stats.needed_packets);
        assert_eq!(transfer.stats.original_sha256, crate::integrity::sha256_hex(&data));
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
        assert_eq!(transfer.packet_frames.len(), 1 + transfer.stats.repair_symbols);
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
        assert!(metadata_count >= 2, "metadata should repeat for long transfers");
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
