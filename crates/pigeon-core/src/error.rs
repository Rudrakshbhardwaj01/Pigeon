use thiserror::Error;

/// Every failure mode of the Pigeon engine, with user-presentable messages.
#[derive(Debug, Error)]
pub enum PigeonError {
    // ---- wire protocol ----
    #[error("packet is too short to contain a Pigeon header ({got} bytes, need {need})")]
    PacketTooShort { got: usize, need: usize },
    #[error("packet does not start with the Pigeon magic bytes")]
    BadMagic,
    #[error("unsupported protocol version {got} (this engine speaks version {supported})")]
    UnsupportedVersion { got: u8, supported: u8 },
    #[error("packet header contains unknown flag bits: 0x{bits:02x}")]
    BadFlags { bits: u8 },
    #[error("packet failed its CRC32C integrity check")]
    BadCrc,
    #[error("packet header payload length {declared} does not match actual {actual}")]
    BadPayloadLength { declared: usize, actual: usize },
    #[error("packet payload length {0} exceeds the limit of {1}")]
    PayloadTooLarge(usize, usize),
    #[error("invalid FEC packet payload: {0}")]
    BadFecPayload(&'static str),
    #[error("invalid metadata packet: {0}")]
    BadMetadata(String),

    // ---- metadata / files ----
    #[error("filename is required")]
    FilenameEmpty,
    #[error("filename exceeds {max} bytes after sanitization")]
    FilenameTooLong { max: usize },
    #[error("MIME type exceeds {max} bytes")]
    MimeTooLong { max: usize },
    #[error("input file is {size} bytes, above the configured limit of {max}")]
    FileTooLarge { size: u64, max: u64 },
    #[error("declared transfer size {declared} exceeds the decoder limit of {max}")]
    TransferTooLarge { declared: u64, max: u64 },
    #[error("invalid encoding configuration: {0}")]
    InvalidConfig(String),

    // ---- compression ----
    #[error("compression failed: {0}")]
    CompressionFailed(String),
    #[error("decompressed output exceeds the safety limit of {limit} bytes")]
    DecompressionLimitExceeded { limit: u64 },
    #[error("decompressed size {got} does not match declared original size {expected}")]
    DecompressedSizeMismatch { expected: u64, got: u64 },

    // ---- reconstruction ----
    #[error("transfer is incomplete: {unique} unique symbols received, {needed} needed")]
    TransferIncomplete { unique: usize, needed: usize },
    #[error("no active transfer with the given identifier")]
    TransferNotFound,
    #[error("too many concurrent transfers (limit {max}); transfer {id:#018x} rejected")]
    TooManyTransfers { id: u64, max: usize },
    #[error("checksum verification failed: expected {expected}, computed {computed}")]
    ChecksumMismatch { expected: String, computed: String },

    // ---- cryptography (protocol v2) ----
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed: the passphrase is wrong or the transfer is corrupted")]
    DecryptionFailed,
    #[error("this transfer is encrypted; a passphrase is required")]
    PassphraseRequired,
    #[error("unsupported cipher id {0}")]
    UnsupportedCipher(u8),
    #[error("unsupported KDF id {0}")]
    UnsupportedKdf(u8),
    #[error("invalid KDF parameters: {detail}")]
    KdfParamsExcessive { detail: String },
    #[error("cryptographic subsystem error: {0}")]
    CryptoInit(String),

    // ---- QR / GIF ----
    #[error("QR encoding failed: {0}")]
    QrEncodeFailed(String),
    #[error("payload of {payload} bytes does not fit QR version {version}")]
    QrCapacityExceeded { payload: usize, version: u8 },
    #[error("GIF encoding failed: {0}")]
    GifEncodeFailed(String),
    #[error("not a GIF file (bad signature)")]
    GifBadSignature,
    #[error("GIF frame dimensions {0}x{1} exceed the limit of {2}")]
    GifDimensionsTooLarge(u16, u16, u32),
    #[error("GIF file is {size} bytes, above the limit of {max}")]
    GifFileTooLarge { size: u64, max: u64 },
    #[error("GIF frame limit exceeded (more than {max} frames)")]
    GifFrameLimitExceeded { max: usize },
    #[error("GIF frame {index} is malformed: {reason}")]
    GifFrameMalformed { index: usize, reason: String },

    #[error("internal engine error: {0}")]
    Internal(String),
}
