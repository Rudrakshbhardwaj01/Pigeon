//! Pigeon core — environment-neutral engine for optical file transfer.
//!
//! Owns the binary wire protocol, forward error correction (RaptorQ / RFC 6330),
//! QR-code generation, animated-GIF encoding, GIF parsing, packet validation,
//! reconstruction and integrity verification. Contains no browser or OS-specific
//! code; the `pigeon-wasm` crate exposes this engine to JavaScript.

pub mod crypto;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod integrity;
pub mod protocol;

pub use decoder::{DecodeLimits, DecoderEngine};
pub use encoder::{CompressionMode, EncodeConfig, QrEcl};
pub use error::PigeonError;
pub use protocol::metadata::sanitize_filename;

/// Highest wire protocol version this engine speaks (v2 = encrypted
/// transfers). Unencrypted transfers still emit byte-compatible v1 packets
/// so older receivers can reconstruct them; see `docs/PROTOCOL.md`.
pub const PROTOCOL_VERSION: u8 = protocol::VERSION;
