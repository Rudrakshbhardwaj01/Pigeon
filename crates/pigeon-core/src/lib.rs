//! Pigeon core — environment-neutral engine for optical file transfer.
//!
//! Owns the binary wire protocol, forward error correction (RaptorQ / RFC 6330),
//! QR-code generation, animated-GIF encoding, GIF parsing, packet validation,
//! reconstruction and integrity verification. Contains no browser or OS-specific
//! code; the `pigeon-wasm` crate exposes this engine to JavaScript.

pub mod decoder;
pub mod encoder;
pub mod error;
pub mod integrity;
pub mod protocol;

pub use decoder::{DecodeLimits, DecoderEngine};
pub use encoder::{CompressionMode, EncodeConfig, QrEcl};
pub use error::PigeonError;
pub use protocol::metadata::sanitize_filename;

/// Wire protocol version implemented by this engine.
pub const PROTOCOL_VERSION: u8 = protocol::VERSION;
