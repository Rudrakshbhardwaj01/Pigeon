//! The receive-side engine: multi-transfer packet ingestion, deduplication,
//! fountain recovery and verified reconstruction.

pub mod gif;
pub mod packet;
pub mod reconstruction;
pub mod recovery;

use serde::Serialize;

use crate::error::PigeonError;
use crate::protocol::MAX_CONCURRENT_TRANSFERS;

use self::packet::{parse_incoming, rejection_reason, RejectionReason};
use self::recovery::{IngestOutcome, TransferPhase, TransferRx};
use self::reconstruction::ReconstructedFile;

/// Decoder resource limits (defaults come from the protocol module).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeLimits {
    pub max_transfer_bytes: u64,
    pub max_transfers: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_transfer_bytes: crate::protocol::MAX_DECODE_TRANSFER_BYTES,
            max_transfers: MAX_CONCURRENT_TRANSFERS,
        }
    }
}

/// Statistics about one tracked transfer, as reported to the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferStatus {
    #[serde(serialize_with = "crate::protocol::ser::u64_hex")]
    pub transfer_id: u64,
    pub phase: String,
    /// Terminal failure reason, when phase == "failed".
    pub failure_reason: Option<String>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub original_size: Option<u64>,
    pub expected_sha256: Option<String>,
    pub unique_symbols: usize,
    pub needed_symbols: Option<usize>,
    /// Progress in 0..=1, only when the needed count is known and the
    /// transfer is not yet verified. `None` = unknown (awaiting metadata).
    pub progress: Option<f64>,
    pub duplicates: usize,
    pub buffered_symbols: usize,
}

impl TransferStatus {
    fn from_rx(rx: &TransferRx) -> Self {
        let phase = match &rx.phase {
            TransferPhase::AwaitingMetadata => "awaiting-metadata",
            TransferPhase::Collecting => "collecting",
            TransferPhase::Reconstructed => "decoding",
            TransferPhase::Failed(_) => "failed",
            TransferPhase::Verified => "verified",
        };
        let progress = rx.needed_symbols.map(|needed| {
            if matches!(rx.phase, TransferPhase::Verified) {
                1.0
            } else {
                (rx.unique_symbols as f64 / needed as f64).min(1.0)
            }
        });
        Self {
            transfer_id: rx.id,
            phase: phase.into(),
            failure_reason: match &rx.phase {
                TransferPhase::Failed(msg) => Some(msg.clone()),
                _ => None,
            },
            filename: rx.meta.as_ref().map(|m| m.filename.clone()),
            mime_type: rx.meta.as_ref().and_then(|m| m.mime_type.clone()),
            original_size: rx.meta.as_ref().map(|m| m.original_size),
            expected_sha256: rx
                .meta
                .as_ref()
                .map(|m| crate::integrity::hex(&m.sha256)),
            unique_symbols: rx.unique_symbols,
            needed_symbols: rx.needed_symbols,
            progress,
            duplicates: rx.duplicates,
            buffered_symbols: rx.buffered_len(),
        }
    }
}

/// Report for a single ingested packet (or rejection).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub valid: bool,
    pub duplicate: bool,
    pub reason: Option<&'static str>,
    #[serde(serialize_with = "crate::protocol::ser::opt_u64_hex")]
    pub transfer_id: Option<u64>,
    /// Aggregated status of the transfer this packet belongs to.
    pub status: Option<TransferStatus>,
}

/// The public decoder engine.
pub struct DecoderEngine {
    transfers: Vec<TransferRx>,
    limits: DecodeLimits,
    total_packets_seen: usize,
    total_invalid: usize,
    total_duplicates: usize,
    rejection_counts: std::collections::HashMap<&'static str, usize>,
}

impl Default for DecoderEngine {
    fn default() -> Self {
        Self::new(DecodeLimits::default())
    }
}

impl DecoderEngine {
    #[must_use]
    pub fn new(limits: DecodeLimits) -> Self {
        Self {
            transfers: Vec::new(),
            limits,
            total_packets_seen: 0,
            total_invalid: 0,
            total_duplicates: 0,
            rejection_counts: std::collections::HashMap::new(),
        }
    }

    /// Ingests one candidate packet (e.g. decoded QR content).
    ///
    /// Invalid packets never touch transfer state; they only bump counters.
    pub fn ingest_packet(&mut self, bytes: &[u8]) -> Result<IngestReport, PigeonError> {
        self.total_packets_seen += 1;

        let packet = match parse_incoming(bytes) {
            Ok(p) => p,
            Err(err) => {
                self.total_invalid += 1;
                let RejectionReason(reason) = rejection_reason(&err);
                *self.rejection_counts.entry(reason).or_insert(0) += 1;
                return Ok(IngestReport {
                    valid: false,
                    duplicate: false,
                    reason: Some(reason),
                    transfer_id: packet::peek_transfer_id(bytes),
                    status: None,
                });
            }
        };

        let transfer_id = match &packet {
            packet::IncomingPacket::Metadata { transfer_id, .. } => *transfer_id,
            packet::IncomingPacket::Symbol { transfer_id, .. } => *transfer_id,
        };

        let rx = match self
            .transfers
            .iter_mut()
            .find(|t| t.id == transfer_id)
        {
            Some(rx) => rx,
            None => {
                if self.transfers.len() >= self.limits.max_transfers {
                    return Err(PigeonError::TooManyTransfers {
                        id: transfer_id,
                        max: self.limits.max_transfers,
                    });
                }
                self.transfers.push(recovery::TransferRx::new(transfer_id));
                self.transfers.last_mut().expect("just pushed")
            }
        };

        let outcome: IngestOutcome = rx.ingest(packet)?;
        if outcome.duplicate {
            self.total_duplicates += 1;
        }
        if matches!(rx.phase, TransferPhase::Reconstructed) {
            // Auto-finalize reconstruction as soon as RaptorQ succeeds.
            Self::finalize_transfer(rx)?;
        }
        let status = TransferStatus::from_rx(rx);
        Ok(IngestReport {
            valid: outcome.accepted,
            duplicate: outcome.duplicate,
            reason: None,
            transfer_id: Some(transfer_id),
            status: Some(status),
        })
    }

    /// Performs decompression + checksum verification for a reconstructed
    /// transfer, moving it to `Verified` or `Failed`.
    fn finalize_transfer(rx: &mut TransferRx) -> Result<(), PigeonError> {
        let (meta, oti) = match (&rx.meta, &rx.oti) {
            (Some(m), Some(o)) => (m.clone(), *o),
            _ => return Err(PigeonError::Internal("finalize without metadata".into())),
        };
        let raw = match rx.take_reconstructed() {
            Some(raw) => raw,
            None => return Ok(()),
        };
        match reconstruction::finalize(raw, &meta, &oti) {
            Ok(file) => {
                rx.store_verified(file);
                rx.phase = TransferPhase::Verified;
                Ok(())
            }
            Err(err) => {
                rx.phase = TransferPhase::Failed(err.to_string());
                Ok(())
            }
        }
    }

    /// Status of a specific transfer.
    #[must_use]
    pub fn status(&self, transfer_id: u64) -> Option<TransferStatus> {
        self.transfers
            .iter()
            .find(|t| t.id == transfer_id)
            .map(TransferStatus::from_rx)
    }

    /// Status of the currently most relevant transfer (latest activity).
    #[must_use]
    pub fn active_status(&self) -> Option<TransferStatus> {
        self.transfers.last().map(TransferStatus::from_rx)
    }

    /// All tracked transfers.
    #[must_use]
    pub fn all_statuses(&self) -> Vec<TransferStatus> {
        self.transfers.iter().map(TransferStatus::from_rx).collect()
    }

    /// Aggregate counters.
    #[must_use]
    pub fn counters(&self) -> DecoderCounters {
        DecoderCounters {
            packets_seen: self.total_packets_seen,
            invalid_packets: self.total_invalid,
            duplicate_packets: self.total_duplicates,
            transfers: self.transfers.len(),
            rejections: self
                .rejection_counts
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
        }
    }

    /// Takes the verified file out of the engine for download.
    ///
    /// Returns `Ok(None)` while the transfer is incomplete; a terminal error
    /// if reconstruction or verification failed.
    pub fn take_file(&mut self, transfer_id: u64) -> Result<Option<ReconstructedFile>, PigeonError> {
        let idx = self
            .transfers
            .iter()
            .position(|t| t.id == transfer_id)
            .ok_or(PigeonError::TransferNotFound)?;
        let rx = &mut self.transfers[idx];
        match &rx.phase {
            TransferPhase::Failed(msg) => Err(PigeonError::Internal(msg.clone())),
            TransferPhase::Verified => Ok(rx.take_verified_file()),
            _ => Ok(None),
        }
    }

    /// Clears all state (start a fresh receive session).
    pub fn reset(&mut self) {
        *self = Self::new(self.limits);
    }
}

/// Aggregate counters for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecoderCounters {
    pub packets_seen: usize,
    pub invalid_packets: usize,
    pub duplicate_packets: usize,
    pub transfers: usize,
    pub rejections: Vec<(String, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::{encode_file, EncodeConfig};

    fn encode(data: &[u8], name: &str) -> crate::encoder::EncodedTransfer {
        let cfg = EncodeConfig {
            filename: name.into(),
            mime_type: Some("text/plain".into()),
            ..Default::default()
        };
        encode_file(data, cfg).expect("encodes")
    }

    fn feed_all(engine: &mut DecoderEngine, transfer: &crate::encoder::EncodedTransfer) {
        for frame in &transfer.frame_sequence {
            engine.ingest_packet(frame).expect("ingest");
        }
    }

    #[test]
    fn complete_roundtrip() {
        let data = b"reconstruct me exactly, byte for byte!".to_vec();
        let transfer = encode(&data, "roundtrip.txt");
        let mut engine = DecoderEngine::default();
        feed_all(&mut engine, &transfer);

        let id = transfer.stats.transfer_id;
        let status = engine.status(id).expect("tracked");
        assert_eq!(status.phase, "verified");
        assert_eq!(status.progress, Some(1.0));
        assert_eq!(status.original_size, Some(data.len() as u64));

        let file = engine.take_file(id).expect("ok").expect("file");
        assert_eq!(file.data, data);
        assert_eq!(file.filename, "roundtrip.txt");
        assert_eq!(file.mime_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn out_of_order_ingestion() {
        let data = vec![42u8; 20_000];
        let transfer = encode(&data, "shuffled.bin");
        let mut engine = DecoderEngine::default();
        // Reverse order: metadata last.
        for frame in transfer.frame_sequence.iter().rev() {
            engine.ingest_packet(frame).expect("ingest");
        }
        let id = transfer.stats.transfer_id;
        assert_eq!(engine.status(id).expect("tracked").phase, "verified");
        assert_eq!(engine.take_file(id).expect("ok").expect("file").data, data);
    }

    #[test]
    fn lossy_but_recoverable() {
        let data: Vec<u8> = (0..60_000u32).map(|i| (i % 251) as u8).collect();
        let transfer = encode(&data, "lossy.bin");
        let mut engine = DecoderEngine::default();
        // Drop 20% of symbol frames (never metadata).
        for (i, frame) in transfer.frame_sequence.iter().enumerate() {
            if i > 0 && i % 5 == 0 {
                continue;
            }
            engine.ingest_packet(frame).expect("ingest");
        }
        let id = transfer.stats.transfer_id;
        assert_eq!(engine.status(id).expect("tracked").phase, "verified");
        assert_eq!(engine.take_file(id).expect("ok").expect("file").data, data);
    }

    #[test]
    fn unrecoverable_reports_incomplete() {
        // High-entropy data (no compression) so the transfer needs many symbols.
        let mut state = 0x9E3779B97F4A7C15u64;
        let data: Vec<u8> = (0..40_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let cfg = EncodeConfig {
            filename: "incomplete.bin".into(),
            compression: crate::encoder::CompressionMode::Off,
            overhead_percent: 0,
            ..Default::default()
        };
        let transfer = encode_file(&data, cfg).expect("encodes");
        assert!(transfer.stats.needed_packets >= 10);
        let mut engine = DecoderEngine::default();
        // Deliver only the first symbol frame: far too few to reconstruct.
        engine
            .ingest_packet(&transfer.frame_sequence[0])
            .expect("metadata");
        engine
            .ingest_packet(&transfer.frame_sequence[1])
            .expect("one symbol");
        let id = transfer.stats.transfer_id;
        let status = engine.status(id).expect("tracked");
        assert_ne!(status.phase, "verified");
        assert!(status.unique_symbols < status.needed_symbols.unwrap());
        assert!(engine.take_file(id).expect("ok").is_none());
    }

    #[test]
    fn corrupted_packets_counted_not_fatal() {
        let data = b"corruption tolerance".to_vec();
        let transfer = encode(&data, "corrupt.txt");
        let mut engine = DecoderEngine::default();

        let mut corrupted = transfer.packet_frames[0].clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x01;
        let report = engine.ingest_packet(&corrupted).expect("report");
        assert!(!report.valid);
        assert_eq!(report.reason, Some("corrupted-content"));

        let report2 = engine.ingest_packet(b"not a pigeon packet at all, too long for header check maybe").expect("report");
        assert!(!report2.valid);

        feed_all(&mut engine, &transfer);
        let counters = engine.counters();
        assert_eq!(counters.invalid_packets, 2);
        let id = transfer.stats.transfer_id;
        assert_eq!(engine.status(id).expect("tracked").phase, "verified");
    }

    #[test]
    fn mixed_transfers_do_not_mix() {
        let a = encode(b"transfer A", "a.txt");
        let b = encode(b"transfer B", "b.txt");
        let mut engine = DecoderEngine::default();
        // Interleave everything.
        let mut frames: Vec<&Vec<u8>> = a.frame_sequence.iter().chain(b.frame_sequence.iter()).collect();
        frames.reverse();
        for f in frames {
            engine.ingest_packet(f).expect("ingest");
        }
        let fa = engine.take_file(a.stats.transfer_id).expect("ok").expect("a");
        let fb = engine.take_file(b.stats.transfer_id).expect("ok").expect("b");
        assert_eq!(fa.data, b"transfer A");
        assert_eq!(fb.data, b"transfer B");
        assert_eq!(engine.counters().transfers, 2);
    }

    #[test]
    fn empty_file_completes_with_metadata_alone() {
        let transfer = encode(&[], "empty.txt");
        let mut engine = DecoderEngine::default();
        engine.ingest_packet(&transfer.metadata_packet).expect("ingest");
        let id = transfer.stats.transfer_id;
        let status = engine.status(id).expect("tracked");
        assert_eq!(status.phase, "verified", "reason: {:?}", status.failure_reason);
        let file = engine.take_file(id).expect("ok").expect("file");
        assert!(file.data.is_empty());
    }

    #[test]
    fn transfer_limit_enforced() {
        let mut engine = DecoderEngine::new(DecodeLimits {
            max_transfer_bytes: 512 * 1024 * 1024,
            max_transfers: 2,
        });
        let a = encode(b"A", "a");
        let b = encode(b"B", "b");
        let c = encode(b"C", "c");
        engine.ingest_packet(&a.metadata_packet).expect("ok");
        engine.ingest_packet(&b.metadata_packet).expect("ok");
        assert!(matches!(
            engine.ingest_packet(&c.metadata_packet),
            Err(PigeonError::TooManyTransfers { .. })
        ));
    }

    #[test]
    fn reset_clears_everything() {
        let transfer = encode(b"reset me", "r.txt");
        let mut engine = DecoderEngine::default();
        feed_all(&mut engine, &transfer);
        engine.reset();
        assert_eq!(engine.counters().transfers, 0);
        assert!(engine.take_file(transfer.stats.transfer_id).is_err());
    }

    #[test]
    fn unicode_filename_survives_decode() {
        let data = b"unicode filename test".to_vec();
        let transfer = encode(&data, "文档 说明.pdf");
        let mut engine = DecoderEngine::default();
        feed_all(&mut engine, &transfer);
        let file = engine.take_file(transfer.stats.transfer_id).expect("ok").expect("file");
        assert_eq!(file.filename, "文档 说明.pdf");
        assert_eq!(file.data, data);
    }
}
