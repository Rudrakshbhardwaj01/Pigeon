//! Recovery engine: one state machine per transfer, accumulating unique
//! fountain symbols until RaptorQ reconstruction succeeds.
//!
//! Encrypted transfers (protocol v2) add a passphrase gate: packets arrive
//! as authenticated-encryption envelopes and can only be opened once a key
//! is derived from the passphrase and the salt carried in the (encrypted)
//! metadata envelope. Until then the transfer sits in `AwaitingPassphrase`
//! and buffers sealed packets, bounded by the same limits as plaintext.
//! Authentication failure (wrong passphrase or corrupted packet) never
//! touches decoder state — it is counted and reported.

use std::collections::HashSet;

use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation};
use zeroize::Zeroizing;

use crate::crypto::aead;
use crate::crypto::envelope::{DecryptionKey, MetadataEnvelope};
use crate::error::PigeonError;
use crate::protocol::metadata::{parse_metadata_payload, TransferMetadata};
use crate::protocol::{MAX_BUFFERED_PACKETS, MAX_DECODE_TRANSFER_BYTES};

use super::packet::IncomingPacket;
use super::reconstruction::ReconstructedFile;

/// Current state of a single receiving transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferPhase {
    /// Waiting for the metadata packet that carries the OTI.
    AwaitingMetadata,
    /// Encrypted transfer: metadata envelope received, but no passphrase has
    /// been provided (or derivation failed), so the key is still unknown.
    AwaitingPassphrase,
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

/// A sealed symbol packet held until the decryption key is available.
#[derive(Debug, Clone)]
struct SealedSymbol {
    aad: [u8; 14],
    sealed: Vec<u8>,
}

/// The pending encrypted metadata packet (first one wins; duplicates are
/// idempotent), retained until a passphrase unlocks it.
#[derive(Debug, Clone)]
struct PendingMetadata {
    aad: [u8; 14],
    sealed: Vec<u8>,
}

pub(super) struct TransferRx {
    pub id: u64,
    pub encrypted: bool,
    pub meta: Option<TransferMetadata>,
    pub oti: Option<ObjectTransmissionInformation>,
    decoder: Option<Decoder>,
    seen: HashSet<u32>,
    /// Plaintext symbols received before the metadata packet arrived (bounded).
    buffered: Vec<(u32, Vec<u8>)>,
    /// Sealed packets (symbols, or the first metadata packet) held while the
    /// decryption key is unavailable (bounded).
    sealed_pending: Vec<SealedSymbol>,
    pending_metadata: Option<PendingMetadata>,
    /// KDF parameters and salt from the pending encrypted metadata packet.
    pending_envelope: Option<MetadataEnvelope>,
    /// Passphrase entered before/without metadata; used as soon as an
    /// encrypted metadata envelope arrives.
    passphrase: Option<Zeroizing<String>>,
    /// Key derived from the passphrase + the metadata envelope's KDF params.
    crypto: Option<DecryptionKey>,
    pub unique_symbols: usize,
    pub duplicates: usize,
    /// Packets whose authentication failed after a key was available
    /// (wrong passphrase signature or corruption).
    pub failed_decryptions: usize,
    pub needed_symbols: Option<usize>,
    pub phase: TransferPhase,
    /// Successfully reconstructed (pre-decompression) object bytes. For
    /// encrypted transfers this is the *sealed* object.
    reconstructed: Option<Vec<u8>>,
    /// Verified file, produced by finalize; held until take_file().
    pub(super) verified_file: Option<ReconstructedFile>,
}

impl TransferRx {
    pub(super) fn new(id: u64) -> Self {
        Self {
            id,
            encrypted: false,
            meta: None,
            oti: None,
            decoder: None,
            seen: HashSet::new(),
            buffered: Vec::new(),
            sealed_pending: Vec::new(),
            pending_metadata: None,
            pending_envelope: None,
            passphrase: None,
            crypto: None,
            unique_symbols: 0,
            duplicates: 0,
            failed_decryptions: 0,
            needed_symbols: None,
            phase: TransferPhase::AwaitingMetadata,
            reconstructed: None,
            verified_file: None,
        }
    }

    /// Creates a receiver seeded with a passphrase the user entered before
    /// this transfer's first packet arrived.
    pub(super) fn with_session_passphrase(id: u64, passphrase: Option<Zeroizing<String>>) -> Self {
        let mut rx = Self::new(id);
        rx.passphrase = passphrase;
        rx
    }

    pub(super) fn ingest(&mut self, packet: IncomingPacket) -> Result<IngestOutcome, PigeonError> {
        let duplicate = match packet {
            IncomingPacket::Metadata { oti, meta, .. } => {
                self.apply_metadata(oti, meta)?;
                false
            }
            IncomingPacket::Symbol {
                payload_id,
                symbols,
                ..
            } => {
                let mut inner = payload_id.to_be_bytes().to_vec();
                inner.extend_from_slice(&symbols);
                self.apply_symbol_bytes(&inner)?
            }
            IncomingPacket::EncryptedMetadata {
                envelope,
                sealed,
                aad,
                ..
            } => {
                self.apply_encrypted_metadata(envelope, sealed, aad)?;
                false
            }
            IncomingPacket::EncryptedSymbol { sealed, aad, .. } => {
                self.apply_sealed_symbol(sealed, aad)?
            }
        };
        Ok(self.outcome(true, duplicate))
    }

    /// Stores or applies the passphrase for this transfer.
    ///
    /// Returns `true` when the passphrase unlocked a pending encrypted
    /// metadata packet (the transfer now proceeds normally). A wrong
    /// passphrase returns `DecryptionFailed` but leaves the pending
    /// metadata intact so the caller can retry.
    pub(super) fn set_passphrase(&mut self, passphrase: &str) -> Result<bool, PigeonError> {
        self.passphrase = Some(Zeroizing::new(passphrase.to_string()));
        if self.crypto.is_some() {
            return Ok(false); // already unlocked
        }
        let (envelope, pending) = match (
            self.pending_envelope.as_ref(),
            self.pending_metadata.as_ref(),
        ) {
            (Some(env), Some(meta)) => (env.clone(), meta.clone()),
            _ => return Ok(false),
        };
        let result = self.unlock(envelope, pending.aad, pending.sealed);
        if result.is_ok() {
            // Success: the pending metadata is consumed.
            self.pending_metadata = None;
            self.pending_envelope = None;
        } else {
            // Failure (wrong passphrase): keep the pending metadata so the
            // passphrase gate stays open for a retry.
            self.failed_decryptions += 1;
        }
        result?;
        Ok(true)
    }

    /// Opens the sealed metadata packet with a freshly derived key and
    /// applies the decrypted transfer metadata.
    fn unlock(
        &mut self,
        envelope: MetadataEnvelope,
        aad: [u8; 14],
        sealed: Vec<u8>,
    ) -> Result<(), PigeonError> {
        let passphrase = self
            .passphrase
            .clone()
            .ok_or(PigeonError::PassphraseRequired)?;
        let key = DecryptionKey::derive(&passphrase, &envelope)?;
        let inner = aead::open(key.as_bytes(), &aad, &sealed)?;
        let (oti, meta) = parse_metadata_payload(&inner)?;
        self.crypto = Some(key);
        self.pending_envelope = None;
        self.apply_metadata(oti, meta)?;

        // Replay sealed symbols that arrived before the key existed.
        for symbol in std::mem::take(&mut self.sealed_pending) {
            self.apply_sealed_symbol(symbol.sealed, symbol.aad)?;
        }
        Ok(())
    }

    fn apply_encrypted_metadata(
        &mut self,
        envelope: MetadataEnvelope,
        sealed: Vec<u8>,
        aad: [u8; 14],
    ) -> Result<(), PigeonError> {
        // Metadata packets repeat while looping; the first is authoritative.
        if self.pending_metadata.is_some() || self.encrypted {
            return Ok(());
        }
        self.encrypted = true;
        if let Some(passphrase) = self.passphrase.clone() {
            // A passphrase was entered before the metadata arrived: derive
            // the key immediately. On failure the transfer stays gated for
            // retry — a wrong passphrase must never kill the session.
            let key = DecryptionKey::derive(&passphrase, &envelope);
            match key
                .as_ref()
                .map(|k| aead::open(k.as_bytes(), &aad, &sealed))
            {
                Ok(Ok(inner)) => {
                    let key = key.expect("key derived when open succeeded");
                    let (oti, meta) = parse_metadata_payload(&inner)?;
                    self.crypto = Some(key);
                    self.apply_metadata(oti, meta)?;
                    for symbol in std::mem::take(&mut self.sealed_pending) {
                        self.apply_sealed_symbol(symbol.sealed, symbol.aad)?;
                    }
                    return Ok(());
                }
                Ok(Err(_)) | Err(_) => {
                    self.failed_decryptions += 1;
                }
            }
        }
        self.pending_envelope = Some(envelope);
        self.pending_metadata = Some(PendingMetadata { aad, sealed });
        self.phase = TransferPhase::AwaitingPassphrase;
        Ok(())
    }

    fn apply_sealed_symbol(&mut self, sealed: Vec<u8>, aad: [u8; 14]) -> Result<bool, PigeonError> {
        match &self.crypto {
            Some(key) => {
                let inner = match aead::open(key.as_bytes(), &aad, &sealed) {
                    Ok(inner) => inner,
                    Err(_) => {
                        // Wrong passphrase (if several transfers share one
                        // passphrase) or corruption: counted, never fatal.
                        self.failed_decryptions += 1;
                        return Ok(false);
                    }
                };
                self.apply_symbol_bytes(&inner)
            }
            None => {
                // No key yet: buffer the sealed packet (bounded).
                if self.sealed_pending.len() < MAX_BUFFERED_PACKETS {
                    self.sealed_pending.push(SealedSymbol { aad, sealed });
                }
                if matches!(self.phase, TransferPhase::AwaitingMetadata) {
                    self.phase = TransferPhase::AwaitingPassphrase;
                }
                Ok(false)
            }
        }
    }

    /// Feeds one decrypted RaptorQ packet (payload id + symbols) into the
    /// decode path: deduplication and fountain decoding.
    fn apply_symbol_bytes(&mut self, inner: &[u8]) -> Result<bool, PigeonError> {
        if matches!(
            self.phase,
            TransferPhase::Failed(_) | TransferPhase::Verified
        ) {
            return Ok(false);
        }
        if inner.len() < 4 {
            return Err(PigeonError::BadFecPayload(
                "payload shorter than a payload id",
            ));
        }
        let raw_id: [u8; 4] = inner[0..4].try_into().expect("fixed 4-byte slice");
        let payload_id = u32::from_be_bytes(raw_id);

        if self.seen.contains(&payload_id) {
            self.duplicates += 1;
            return Ok(true);
        }
        self.seen.insert(payload_id);
        self.unique_symbols += 1;

        match &mut self.decoder {
            Some(decoder) => {
                let id = raptorq::PayloadId::deserialize(&raw_id);
                if let Some(data) = decoder.decode(EncodingPacket::new(id, inner[4..].to_vec())) {
                    self.reconstructed = Some(data);
                    self.phase = TransferPhase::Reconstructed;
                }
            }
            None => {
                // Symbols ahead of metadata: bounded buffering.
                if self.buffered.len() < MAX_BUFFERED_PACKETS {
                    self.buffered.push((payload_id, inner[4..].to_vec()));
                }
            }
        }
        Ok(false)
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
        // Empty *unencrypted* files complete with the metadata alone. Empty
        // encrypted files still carry a 40-byte sealed object (nonce + tag)
        // that must be received and authenticated like any other symbol.
        if meta.original_size == 0 && !self.encrypted {
            self.meta = Some(meta);
            self.oti = Some(config);
            self.phase = TransferPhase::Reconstructed;
            self.reconstructed = Some(Vec::new());
            return Ok(());
        }
        self.meta = Some(meta);
        self.oti = Some(config);
        self.needed_symbols = Some(
            config
                .transfer_length()
                .div_ceil(config.symbol_size() as u64) as usize,
        );
        let mut decoder = Decoder::new(config);
        let buffered = std::mem::take(&mut self.buffered);
        for (payload_id, symbols) in buffered {
            let raw = payload_id.to_be_bytes();
            let id = raptorq::PayloadId::deserialize(&raw);
            if let Some(data) = decoder.decode(EncodingPacket::new(id, symbols)) {
                self.reconstructed = Some(data);
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

    fn outcome(&self, accepted: bool, duplicate: bool) -> IngestOutcome {
        IngestOutcome {
            accepted,
            duplicate,
            unique_symbols: self.unique_symbols,
            needed_symbols: self.needed_symbols,
            phase: self.phase.clone(),
        }
    }

    /// Takes the reconstructed object bytes (compressed, or still sealed for
    /// encrypted transfers) if available.
    pub(super) fn take_reconstructed(&mut self) -> Option<Vec<u8>> {
        self.reconstructed.take()
    }

    /// The decryption key, when the transfer has been unlocked.
    pub(super) fn crypto(&self) -> Option<&DecryptionKey> {
        self.crypto.as_ref()
    }

    /// Stores a verified file for later retrieval via take_file().
    pub(super) fn store_verified(&mut self, file: ReconstructedFile) {
        self.verified_file = Some(file);
    }

    /// Moves the verified file out (if present).
    pub(super) fn take_verified_file(&mut self) -> Option<ReconstructedFile> {
        self.verified_file.take()
    }

    /// Number of packets buffered ahead of metadata (plaintext) or of the
    /// decryption key (sealed).
    #[must_use]
    pub(super) fn buffered_len(&self) -> usize {
        self.buffered.len() + self.sealed_pending.len()
    }
}
