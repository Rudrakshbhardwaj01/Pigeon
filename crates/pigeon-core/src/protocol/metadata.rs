//! Transfer metadata: the self-contained packet that describes the file being
//! transmitted, plus filename sanitization for received (untrusted) names.

use crate::error::PigeonError;

use super::{header::write_packet, FLAG_METADATA, MAX_FILENAME_BYTES, MAX_MIME_BYTES};

/// How the transmitted (post-FEC) payload was compressed before encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    None = 0,
    Zlib = 1,
}

impl Compression {
    /// Parses the one-byte compression id, rejecting unknown values.
    pub fn from_byte(v: u8) -> Result<Self, PigeonError> {
        match v {
            0 => Ok(Self::None),
            1 => Ok(Self::Zlib),
            _ => Err(PigeonError::BadMetadata(format!(
                "unknown compression algorithm id {v}"
            ))),
        }
    }
}

/// Everything the receiver needs besides FEC symbols to reconstruct the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferMetadata {
    /// Sanitized filename (1..=`MAX_FILENAME_BYTES` UTF-8 bytes).
    pub filename: String,
    /// Optional MIME type (`MAX_MIME_BYTES` bytes max).
    pub mime_type: Option<String>,
    /// Compression applied to the encoded object.
    pub compression: Compression,
    /// Size of the *original* (decompressed) file in bytes.
    pub original_size: u64,
    /// SHA-256 of the original file — the authoritative integrity check.
    pub sha256: [u8; 32],
}

/// Serialized layout (v1):
///
/// ```text
/// 0    12  RaptorQ object transmission information (opaque 12-byte OTI)
/// 12    1  compression id
/// 13    8  original size (u64 LE)
/// 21   32  SHA-256 of the original file
/// 53    2  filename length (u16 LE, bytes)
/// 55    2  MIME length (u16 LE, bytes)
/// 57    f  filename (UTF-8)
///       m  MIME (ASCII)
/// ```
pub fn serialize_metadata_packet(
    transfer_id: u64,
    oti: &[u8; 12],
    meta: &TransferMetadata,
) -> Result<Vec<u8>, PigeonError> {
    validate_metadata(meta)?;
    let fname = meta.filename.as_bytes();
    let mime = meta.mime_type.as_deref().unwrap_or("").as_bytes();

    let mut payload = Vec::with_capacity(57 + fname.len() + mime.len());
    payload.extend_from_slice(oti);
    payload.push(meta.compression as u8);
    payload.extend_from_slice(&meta.original_size.to_le_bytes());
    payload.extend_from_slice(&meta.sha256);
    payload.extend_from_slice(&(fname.len() as u16).to_le_bytes());
    payload.extend_from_slice(&(mime.len() as u16).to_le_bytes());
    payload.extend_from_slice(fname);
    payload.extend_from_slice(mime);

    Ok(write_packet(FLAG_METADATA, transfer_id, &payload))
}

/// Parses a metadata packet payload into the OTI bytes and metadata struct.
pub fn parse_metadata_payload(payload: &[u8]) -> Result<([u8; 12], TransferMetadata), PigeonError> {
    if payload.len() < 57 {
        return Err(PigeonError::BadMetadata(format!(
            "payload is {} bytes, need at least 57",
            payload.len()
        )));
    }
    let mut oti = [0u8; 12];
    oti.copy_from_slice(&payload[0..12]);
    let compression = Compression::from_byte(payload[12])?;
    let original_size = u64::from_le_bytes(payload[13..21].try_into().expect("fixed 8-byte slice"));
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&payload[21..53]);
    let fname_len = u16::from_le_bytes(payload[53..55].try_into().expect("fixed 2-byte slice")) as usize;
    let mime_len = u16::from_le_bytes(payload[55..57].try_into().expect("fixed 2-byte slice")) as usize;

    let total = 57usize
        .checked_add(fname_len)
        .and_then(|v| v.checked_add(mime_len))
        .ok_or_else(|| PigeonError::BadMetadata("length overflow".into()))?;
    if total != payload.len() {
        return Err(PigeonError::BadMetadata(format!(
            "declared string lengths ({fname_len} + {mime_len}) do not fit payload of {} bytes",
            payload.len()
        )));
    }
    let fname_bytes = &payload[57..57 + fname_len];
    if fname_bytes.len() > MAX_FILENAME_BYTES {
        return Err(PigeonError::BadMetadata(format!(
            "filename is {} bytes, limit {MAX_FILENAME_BYTES}",
            fname_bytes.len()
        )));
    }
    let filename = String::from_utf8(fname_bytes.to_vec())
        .map_err(|_| PigeonError::BadMetadata("filename is not valid UTF-8".into()))?;
    let filename = sanitize_filename(&filename);

    let mime_bytes = &payload[57 + fname_len..];
    if mime_bytes.len() > MAX_MIME_BYTES {
        return Err(PigeonError::BadMetadata(format!(
            "MIME type is {} bytes, limit {MAX_MIME_BYTES}",
            mime_bytes.len()
        )));
    }
    let mime_str = String::from_utf8(mime_bytes.to_vec())
        .map_err(|_| PigeonError::BadMetadata("MIME type is not valid UTF-8".into()))?;
    let mime_str = mime_str.trim().to_ascii_lowercase();
    let mime_type = if mime_str.is_empty() {
        None
    } else if mime_str.len() > MAX_MIME_BYTES || !mime_str.bytes().all(|b| b.is_ascii_graphic() || b == b'/') {
        return Err(PigeonError::BadMetadata("MIME type contains invalid characters".into()));
    } else {
        Some(mime_str)
    };

    let meta = TransferMetadata {
        filename,
        mime_type,
        compression,
        original_size,
        sha256,
    };
    validate_metadata(&meta)?;
    Ok((oti, meta))
}

fn validate_metadata(meta: &TransferMetadata) -> Result<(), PigeonError> {
    if meta.filename.is_empty() {
        return Err(PigeonError::FilenameEmpty);
    }
    if meta.filename.len() > MAX_FILENAME_BYTES {
        return Err(PigeonError::FilenameTooLong { max: MAX_FILENAME_BYTES });
    }
    if let Some(mime) = &meta.mime_type {
        if mime.len() > MAX_MIME_BYTES {
            return Err(PigeonError::MimeTooLong { max: MAX_MIME_BYTES });
        }
    }
    Ok(())
}

/// Characters Windows reserves as device names; never safe as a bare filename.
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Sanitizes an untrusted filename so it is safe to save under any desktop OS.
///
/// - path separators, drive letters, control characters and characters illegal
///   on Windows (`< > : " / \ | ? *`) become `_`
/// - leading dots and whitespace are stripped (no `..` traversal, no hidden files)
/// - Windows device names (CON, NUL, COM1, ...) are prefixed
/// - length is capped at [`MAX_FILENAME_BYTES`] UTF-8 bytes on a char boundary
/// - the result is non-empty; falls back to `pigeon-file`
#[must_use]
pub fn sanitize_filename(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(MAX_FILENAME_BYTES));
    for ch in input.chars() {
        let bad = matches!(ch, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
            || ch.is_control();
        out.push(if bad { '_' } else { ch });
    }
    let trimmed = out.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name: String = trimmed.chars().collect();

    // Windows device names: CON, CON.txt, com1.pdf, ...
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&base.as_str()) {
        name = format!("_{name}");
    }

    // Cap length on a UTF-8 char boundary.
    while name.len() > MAX_FILENAME_BYTES {
        name.pop();
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        "pigeon-file".to_string()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta() -> TransferMetadata {
        TransferMetadata {
            filename: "report.pdf".into(),
            mime_type: Some("application/pdf".into()),
            compression: Compression::None,
            original_size: 1234,
            sha256: [7u8; 32],
        }
    }

    #[test]
    fn metadata_roundtrip() {
        let meta = sample_meta();
        let oti = [0xA5u8; 12];
        let pkt = serialize_metadata_packet(0xDEAD_BEEF, &oti, &meta).expect("serializes");
        let (header, payload) = crate::protocol::header::parse_packet(&pkt).expect("valid packet");
        assert_eq!(header.transfer_id, 0xDEAD_BEEF);
        assert_eq!(header.flags, FLAG_METADATA);
        let (got_oti, got_meta) = parse_metadata_payload(payload).expect("parses");
        assert_eq!(got_oti, oti);
        assert_eq!(got_meta, meta);
    }

    #[test]
    fn metadata_unicode_roundtrip() {
        let meta = TransferMetadata {
            filename: "文档 最终版(3).pdf".into(),
            mime_type: None,
            compression: Compression::Zlib,
            original_size: 42,
            sha256: [0u8; 32],
        };
        let pkt = serialize_metadata_packet(1, &[0u8; 12], &meta).expect("serializes");
        let (_, payload) = crate::protocol::header::parse_packet(&pkt).expect("valid packet");
        let (_, got) = parse_metadata_payload(payload).expect("parses");
        assert_eq!(got, meta);
    }

    #[test]
    fn metadata_rejects_bad_compression_id() {
        let mut payload = vec![0u8; 57];
        payload[12] = 0xFF;
        assert!(parse_metadata_payload(&payload).is_err());
    }

    #[test]
    fn metadata_rejects_inconsistent_lengths() {
        let mut payload = vec![0u8; 57];
        payload[53..55].copy_from_slice(&100u16.to_le_bytes()); // filename 100 bytes not present
        assert!(parse_metadata_payload(&payload).is_err());
    }

    #[test]
    fn metadata_rejects_short_payload() {
        assert!(parse_metadata_payload(&[0u8; 10]).is_err());
    }

    #[test]
    fn sanitize_paths() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("C:\\temp\\evil.exe"), "C__temp_evil.exe");
        assert_eq!(sanitize_filename("/abs/path/file.txt"), "_abs_path_file.txt");
    }

    #[test]
    fn sanitize_traversal_and_hidden() {
        assert_eq!(sanitize_filename("..\\..\\secret"), "_.._secret");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        assert_eq!(sanitize_filename("   "), "pigeon-file");
        assert_eq!(sanitize_filename(""), "pigeon-file");
        assert_eq!(sanitize_filename("..."), "pigeon-file");
    }

    #[test]
    fn sanitize_control_chars() {
        assert_eq!(sanitize_filename("a\u{0000}b\u{0007}c"), "a_b_c");
        assert_eq!(sanitize_filename("new\nline.txt"), "new_line.txt");
    }

    #[test]
    fn sanitize_windows_device_names() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_filename("COM10"), "COM10"); // COM10 is fine
    }

    #[test]
    fn sanitize_preserves_unicode() {
        assert_eq!(sanitize_filename("文件.docx"), "文件.docx");
        assert_eq!(sanitize_filename("café résumé.md"), "café résumé.md");
    }

    #[test]
    fn sanitize_caps_length_on_char_boundary() {
        let long = "文".repeat(300); // 900 bytes
        let s = sanitize_filename(&long);
        assert!(s.len() <= MAX_FILENAME_BYTES);
        assert!(s.chars().count() <= MAX_FILENAME_BYTES / 3);
        assert_eq!(s, "文".repeat(85));
    }
}
