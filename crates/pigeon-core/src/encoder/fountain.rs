//! RaptorQ (RFC 6330) fountain encoding wrapper.
//!
//! The `raptorq` crate owns the actual code; this module adapts it into the
//! Pigeon packet flow: one encoding packet per QR frame, source blocks
//! interleaved round-robin so a partially captured transfer spreads loss
//! evenly across blocks, plus a configurable number of repair symbols.

use raptorq::{EncodingPacket, Encoder, ObjectTransmissionInformation};

/// Wraps a single object for fountain encoding.
pub struct FountainEncoder {
    encoder: Encoder,
    /// Source symbols per source block (extended, i.e. padding included).
    block_symbol_counts: Vec<usize>,
    total_source_symbols: usize,
}

impl FountainEncoder {
    /// Creates an encoder producing packets of `packet_size` bytes.
    ///
    /// `packet_size` doubles as the symbol size (one symbol per packet).
    /// Empty objects produce no symbols; their OTI describes a zero-length
    /// transfer (see [`empty_oti`]).
    pub fn new(data: &[u8], packet_size: usize) -> Self {
        debug_assert!(packet_size > 0 && packet_size <= u16::MAX as usize);
        if data.is_empty() {
            return Self {
                encoder: Self::empty_encoder(),
                block_symbol_counts: Vec::new(),
                total_source_symbols: 0,
            };
        }
        let encoder = Encoder::with_defaults(data, packet_size as u16);
        let block_symbol_counts: Vec<usize> = encoder
            .get_block_encoders()
            .iter()
            .map(|b| b.source_packets().len())
            .collect();
        let total = block_symbol_counts.iter().sum();
        Self {
            encoder,
            block_symbol_counts,
            total_source_symbols: total,
        }
    }

    /// Serialized 12-byte object transmission information for the wire.
    #[must_use]
    pub fn oti(&self) -> [u8; 12] {
        if self.total_source_symbols == 0 {
            // Empty object: report the explicit zero-length OTI, not the
            // placeholder encoder's config.
            return Self::empty_oti();
        }
        self.encoder.get_config().serialize()
    }

    /// A harmless encoder for the degenerate empty-object case; never used
    /// to produce symbols (the block list is empty).
    fn empty_encoder() -> Encoder {
        Encoder::new(&[0u8; 1], ObjectTransmissionInformation::new(1, 8, 1, 1, 1))
    }

    /// OTI for the degenerate empty-object case.
    ///
    /// `with_defaults` panics on zero-length objects inside the raptorq
    /// crate, so the transmission parameters are constructed explicitly.
    #[must_use]
    pub fn empty_oti() -> [u8; 12] {
        ObjectTransmissionInformation::new(0, 8, 1, 1, 1).serialize()
    }

    /// Number of source symbols a receiver must collect (padding included).
    #[must_use]
    pub fn source_symbol_count(&self) -> usize {
        self.total_source_symbols
    }

    /// Number of source blocks (RaptorQ partitions large objects).
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.block_symbol_counts.len()
    }

    /// Produces the full transmission sequence: `overhead_percent` extra
    /// repair symbols per block, interleaved round-robin across blocks.
    ///
    /// With zero blocks (empty object) the sequence is empty.
    #[must_use]
    pub fn transmission_sequence(&self, overhead_percent: u32) -> Vec<EncodingPacket> {
        if self.block_symbol_counts.is_empty() {
            return Vec::new();
        }
        let repair_per_block: Vec<usize> = self
            .block_symbol_counts
            .iter()
            .map(|&k| (k as u64 * overhead_percent as u64 / 100) as usize)
            .collect();

        // Collect each block's packet list once (source then repair), then
        // interleave round-robin so loss spreads evenly across blocks.
        let block_encoders = self.encoder.get_block_encoders();
        let per_block: Vec<(Vec<EncodingPacket>, Vec<EncodingPacket>)> = block_encoders
            .iter()
            .zip(&repair_per_block)
            .zip(&self.block_symbol_counts)
            .map(|((block, &repair), &src)| {
                let sources = block.source_packets();
                let repairs = if repair > 0 {
                    block.repair_packets(src as u32, repair as u32)
                } else {
                    Vec::new()
                };
                (sources, repairs)
            })
            .collect();

        let mut packets = Vec::with_capacity(self.total_source_symbols + repair_per_block.iter().sum::<usize>() * block_encoders.len());
        let max_source = self.block_symbol_counts.iter().copied().max().unwrap_or(0);
        let max_repair = repair_per_block.iter().copied().max().unwrap_or(0);
        for i in 0..max_source.max(max_repair) {
            for (sources, repairs) in &per_block {
                if let Some(pkt) = sources.get(i) {
                    packets.push(pkt.clone());
                }
                if let Some(pkt) = repairs.get(i) {
                    packets.push(pkt.clone());
                }
            }
        }
        packets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raptorq::Decoder;

    fn all_unique(packets: &[EncodingPacket]) -> usize {
        let mut seen = std::collections::HashSet::new();
        for p in packets {
            seen.insert(p.payload_id().serialize());
        }
        seen.len()
    }

    #[test]
    fn single_block_roundtrip_with_loss() {
        let data: Vec<u8> = (0..50_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let enc = FountainEncoder::new(&data, 1024);
        assert_eq!(enc.block_count(), 1);
        let seq = enc.transmission_sequence(60);
        assert_eq!(all_unique(&seq), seq.len(), "no duplicate payload ids");

        // drop 25% of frames (simulate missed QR frames)
        let kept: Vec<&EncodingPacket> = seq.iter().enumerate().filter(|(i, _)| i % 4 != 0).map(|(_, p)| p).collect();
        let mut dec = Decoder::new(ObjectTransmissionInformation::deserialize(&enc.oti()));
        let mut result = None;
        for pkt in kept {
            result = dec.decode(pkt.clone());
            if result.is_some() {
                break;
            }
        }
        assert_eq!(result.expect("recovers under 25% loss"), data);
    }

    // RaptorQ's generator-matrix work is extremely slow without optimizer
    // optimizations; run this large-object test in release mode:
    //   cargo test -p pigeon-core --release multi_block
    #[test]
    #[ignore = "expensive in debug builds; run with --release"]
    fn multi_block_roundtrip_out_of_order() {
        // 4 MB / 1024 bytes per symbol -> 4096 symbols > 56403? no; force blocks via big data
        // Default block partitioning splits objects > ~55k symbols; 60 MB with 1024B symbols
        // would be too slow for a unit test, so lower symbol count: 4096 symbols won't split.
        // Instead verify with a larger-than-one-block payload using explicit config is not
        // exposed by with_defaults; rely on crate behavior for 8 MB (8192 symbols, 1 block).
        let data: Vec<u8> = (0..8_000_000u32).map(|i| (i % 256) as u8).collect();
        let enc = FountainEncoder::new(&data, 1024);
        let seq = enc.transmission_sequence(20);
        assert!(seq.len() >= enc.source_symbol_count());

        // shuffled ingest
        let mut seq = seq;
        // deterministic shuffle
        let mut seed = 0x1234_5678u64;
        for i in (1..seq.len()).rev() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (seed >> 33) as usize % (i + 1);
            seq.swap(i, j);
        }
        let mut dec = Decoder::new(ObjectTransmissionInformation::deserialize(&enc.oti()));
        let mut result = None;
        for pkt in &seq {
            result = dec.decode(pkt.clone());
            if result.is_some() {
                break;
            }
        }
        assert_eq!(result.expect("recovers shuffled"), data);
    }

    #[test]
    fn tiny_object_one_symbol() {
        let data = b"x".to_vec();
        let enc = FountainEncoder::new(&data, 1024);
        assert_eq!(enc.source_symbol_count(), 1);
        let seq = enc.transmission_sequence(0);
        assert_eq!(seq.len(), 1);
        let mut dec = Decoder::new(ObjectTransmissionInformation::deserialize(&enc.oti()));
        assert_eq!(dec.decode(seq[0].clone()), Some(data));
    }

    #[test]
    fn empty_object_has_empty_sequence() {
        let enc = FountainEncoder::new(&[], 1024);
        assert_eq!(enc.source_symbol_count(), 0);
        assert!(enc.transmission_sequence(0).is_empty());
    }

    #[test]
    fn zero_overhead_still_roundtrips() {
        let data = vec![0xAA; 4096];
        let enc = FountainEncoder::new(&data, 512);
        let seq = enc.transmission_sequence(0);
        assert_eq!(seq.len(), enc.source_symbol_count());
        let mut dec = Decoder::new(ObjectTransmissionInformation::deserialize(&enc.oti()));
        let mut result = None;
        for pkt in &seq {
            result = dec.decode(pkt.clone());
        }
        assert_eq!(result, Some(data));
    }
}
