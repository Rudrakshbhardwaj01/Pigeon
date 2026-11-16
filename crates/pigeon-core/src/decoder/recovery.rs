//! Recovery engine: one state machine per transfer, accumulating unique
//! fountain symbols until RaptorQ reconstruction succeeds.

use std::collections::HashSet;

use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation};

use crate::error::PigeonError;
use crate::protocol::metadata::TransferMetadata;
use crate::protocol::{MAX_BUFFERED_PACKETS, MAX_DECODE_TRANSFER_BYTES};

use super::packet::IncomingPacket;
use super::reconstruction::ReconstructedFile;

/// Current state of a single receiving transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferPhase {
    /// Waiting for the metadata packet that carries the OTI.
    AwaitingMetadata,
    /// Metadata known; collecting symbols.
    Collecting,
    /// RaptorQ succeeded; awaiting or performing verification.
    Reconstructed,
    /// Reconstruction or verification failed terminally.
    Failed(String),
    /// Fully verified file available.
    Verified,
}

/// Result of ingesting one packet into a transfer.
#[derive(Debug, Clone)]
pub struct IngestOutcome {
    pub accepted: bool,
    pub duplicate: bool,
    pub unique_symbols: usize,
    pub needed_symbols: Option<usize>,
    pub phase: TransferPhase,
}

pub(super) struct TransferRx {
    pub id: u64,
    pub meta: Option<TransferMetadata>,
    pub oti: Option<ObjectTransmissionInformation>,
    decoder: Option<Decoder>,
    seen: HashSet<u32>,
    /// Symbols received before the metadata packet arrived (bounded).
    buffered: Vec<(u32, Vec<u8>)>,
    pub unique_symbols: usize,
    pub duplicates: usize,
    pub needed_symbols: Option<usize>,
    pub phase: TransferPhase,
    /// Successfully reconstructed (pre-decompression) object bytes.
    reconstructed: Option<Vec<u8>>,
    /// Verified file, produced by finalize; held until take_file().
    pub(super) verified_file: Option<ReconstructedFile>,
}

impl TransferRx {
    pub(super) fn new(id: u64) -> Self {
        Self {
            id,
            meta: None,
            oti: None,
            decoder: None,
            seen: HashSet::new(),
            buffered: Vec::new(),
            unique_symbols: 0,
            duplicates: 0,
            needed_symbols: None,
            phase: TransferPhase::AwaitingMetadata,
            reconstructed: None,
            verified_file: None,
        }
    }

    pub(super) fn ingest(&mut self, packet: IncomingPacket) -> Result<IngestOutcome, PigeonError> {
        let duplicate = match packet {
            IncomingPacket::Metadata { oti, meta, .. } => {
                self.apply_metadata(oti, meta)?;
                false
            }
            IncomingPacket::Symbol {
                payload_id, symbols, ..
            } => self.apply_symbol(payload_id, symbols)?,
        };
        Ok(self.outcome(true, duplicate))
    }

    fn apply_metadata(&mut self, oti: [u8; 12], meta: TransferMetadata) -> Result<(), PigeonError> {
        // Metadata packets repeat while looping; keep the first authoritative one.
        if self.meta.is_some() {
            return Ok(());
        }
        let config = ObjectTransmissionInformation::deserialize(&oti);
        if config.transfer_length() > MAX_DECODE_TRANSFER_BYTES {
            return Err(PigeonError::TransferTooLarge {
                declared: config.transfer_length(),
                max: MAX_DECODE_TRANSFER_BYTES,
            });
        }
        // Empty files complete with the metadata alone.
        if meta.original_size == 0 {
            self.meta = Some(meta);
            self.oti = Some(config);
            self.phase = TransferPhase::Reconstructed;
            self.reconstructed = Some(Vec::new());
            return Ok(());
        }
        self.meta = Some(meta);
        self.oti = Some(config);
        self.needed_symbols = Some(config.transfer_length().div_ceil(config.symbol_size() as u64) as usize);
        let mut decoder = Decoder::new(config);
        let buffered = std::mem::take(&mut self.buffered);
        for (payload_id, symbols) in buffered {
            let raw = payload_id.to_be_bytes();
            let id = raptorq::PayloadId::deserialize(&raw);
            if let Some(data) = decoder.decode(EncodingPacket::new(id, symbols)) {
                self.reconstructed = Some(data);
                self.phase = TransferPhase::Reconstructed;
            }
        }
        self.decoder = Some(decoder);
        if self.reconstructed.is_some() {
            self.phase = TransferPhase::Reconstructed;
        } else {
            self.phase = TransferPhase::Collecting;
        }
        Ok(())
    }

    fn apply_symbol(&mut self, payload_id: u32, symbols: Vec<u8>) -> Result<bool, PigeonError> {
        if matches!(self.phase, TransferPhase::Failed(_) | TransferPhase::Verified) {
            return Ok(false);
        }
        if self.seen.contains(&payload_id) {
            self.duplicates += 1;
            return Ok(true);
        }
        self.seen.insert(payload_id);
        self.unique_symbols += 1;

        match &mut self.decoder {
            Some(decoder) => {
                // Inverse of the u32::from_be_bytes(packed) used on the parse side.
                let raw = payload_id.to_be_bytes();
                let id = raptorq::PayloadId::deserialize(&raw);
                if let Some(data) = decoder.decode(EncodingPacket::new(id, symbols)) {
                    self.reconstructed = Some(data);
                    self.phase = TransferPhase::Reconstructed;
                }
            }
            None => {
                // Symbols ahead of metadata: bounded buffering.
                if self.buffered.len() < MAX_BUFFERED_PACKETS {
                    self.buffered.push((payload_id, symbols));
                }
            }
        }
        Ok(false)
    }

    fn outcome(&self, accepted: bool, duplicate: bool) -> IngestOutcome {
        IngestOutcome {
            accepted,
            duplicate,
            unique_symbols: self.unique_symbols,
            needed_symbols: self.needed_symbols,
            phase: self.phase.clone(),
        }
    }

    /// Takes the reconstructed object bytes (compressed form) if available.
    pub(super) fn take_reconstructed(&mut self) -> Option<Vec<u8>> {
        self.reconstructed.take()
    }

    /// Stores a verified file for later retrieval via take_file().
    pub(super) fn store_verified(&mut self, file: ReconstructedFile) {
        self.verified_file = Some(file);
    }

    /// Moves the verified file out (if present).
    pub(super) fn take_verified_file(&mut self) -> Option<ReconstructedFile> {
        self.verified_file.take()
    }

    /// Number of symbols buffered ahead of metadata.
    #[must_use]
    pub(super) fn buffered_len(&self) -> usize {
        self.buffered.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::{encode_file, EncodeConfig};

    /// Encodes `data` (compression disabled so symbol counts are predictable)
    /// and returns (transfer id, metadata packet, symbol frames).
    fn make_transfer(data: &[u8]) -> (u64, Vec<u8>, Vec<Vec<u8>>) {
        let cfg = EncodeConfig {
            compression: crate::encoder::CompressionMode::Off,
            ..Default::default()
        };
        let t = encode_file(data, cfg).expect("encodes");
        (t.stats.transfer_id, t.metadata_packet.clone(), t.frame_sequence[1..].to_vec())
    }

    #[test]
    fn symbols_before_metadata_are_buffered() {
        // ~24 KiB of compressible data -> several symbol frames
        let (id, meta_pkt, frames) = make_transfer(&vec![b'k'; 24_000]);
        assert!(frames.len() >= 3);
        let mut rx = TransferRx::new(id);
        // feed three symbol packets first
        for f in &frames[..3] {
            let pkt = crate::decoder::packet::parse_incoming(f).expect("parses");
            rx.ingest(pkt).expect("ok");
        }
        assert_eq!(rx.phase, TransferPhase::AwaitingMetadata);
        assert_eq!(rx.unique_symbols, 3);
        // now metadata drains the buffer
        let meta_pkt = crate::decoder::packet::parse_incoming(&meta_pkt).expect("parses");
        rx.ingest(meta_pkt).expect("ok");
        assert!(matches!(rx.phase, TransferPhase::Collecting | TransferPhase::Reconstructed));
    }

    #[test]
    fn duplicates_do_not_inflate_progress() {
        let (id, meta_pkt, frames) = make_transfer(&vec![b'd'; 8_000]);
        let mut rx = TransferRx::new(id);
        let meta_pkt = crate::decoder::packet::parse_incoming(&meta_pkt).expect("parses");
        rx.ingest(meta_pkt).expect("ok");
        let first = crate::decoder::packet::parse_incoming(&frames[0]).expect("parses");
        rx.ingest(first).expect("ok");
        let dup = crate::decoder::packet::parse_incoming(&frames[0]).expect("parses");
        let out = rx.ingest(dup).expect("ok");
        assert!(out.duplicate);
        assert_eq!(rx.unique_symbols, 1);
        assert_eq!(rx.duplicates, 1);
    }
}
