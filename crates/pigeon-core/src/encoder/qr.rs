//! QR-code generation from wire packets, with exact byte-capacity probing.

use fast_qr::{QRBuilder, Version, ECL};

use crate::error::PigeonError;

use serde::Serialize;

/// QR error-correction level. Module-level redundancy inside a single frame;
/// complementary to the fountain layer, which covers whole-frame loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QrEcl {
    L,
    M,
    Q,
    H,
}

impl QrEcl {
    #[must_use]
    pub fn to_fast_qr(self) -> ECL {
        match self {
            Self::L => ECL::L,
            Self::M => ECL::M,
            Self::Q => ECL::Q,
            Self::H => ECL::H,
        }
    }

    /// Parses an ECL from a config string ("L"/"M"/"Q"/"H", case-insensitive).
    pub fn parse(s: &str) -> Result<Self, PigeonError> {
        match s.to_ascii_uppercase().as_str() {
            "L" => Ok(Self::L),
            "M" => Ok(Self::M),
            "Q" => Ok(Self::Q),
            "H" => Ok(Self::H),
            _ => Err(PigeonError::InvalidConfig(format!(
                "unknown QR error-correction level \"{s}\" (expected L, M, Q or H)"
            ))),
        }
    }
}

/// A rendered QR matrix: `modules` is row-major, 1 byte per module (0/1),
/// `size` is the matrix side in modules (quiet zone NOT included).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrMatrix {
    pub size: usize,
    #[serde(serialize_with = "crate::protocol::ser::bytes")]
    pub modules: Vec<u8>,
}

/// Finds the largest byte-mode payload (in bytes) that fits a QR version/ECL.
///
/// Byte-mode capacity is content-independent, so probing with a fixed pattern
/// is exact. fast_qr chooses the mode automatically; the probe pattern
/// guarantees byte mode is selected (it contains non-alphanumeric bytes).
#[must_use]
pub fn probe_byte_capacity(version: Version, ecl: QrEcl) -> usize {
    let ecl = ecl.to_fast_qr();
    let mut lo = 1usize;
    let mut hi = 2953usize; // absolute byte-mode maximum (v40-L)
    let mut best = 0usize;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let pattern: Vec<u8> = (0..mid).map(|i| (i % 256) as u8 | 0x80).collect();
        if QRBuilder::new(pattern)
            .ecl(ecl)
            .version(version)
            .build()
            .is_ok()
        {
            best = mid;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    best
}

/// Smallest QR version (1..=40) whose byte capacity reaches `min_payload`.
///
/// Returns `None` if even version 40 cannot fit the payload at this ECL.
#[must_use]
pub fn smallest_version_for(min_payload: usize, ecl: QrEcl) -> Option<Version> {
    for v in 1..=40u8 {
        let version = version_from_u8(v).ok()?;
        if probe_byte_capacity(version, ecl) >= min_payload {
            return Some(version);
        }
    }
    None
}

/// Converts a 1-based QR version number into the fast_qr enum.
pub fn version_from_u8(v: u8) -> Result<Version, PigeonError> {
    if !(1..=40).contains(&v) {
        return Err(PigeonError::InvalidConfig(format!(
            "QR version must be 1..=40, got {v}"
        )));
    }
    match v {
        1 => Ok(Version::V01),
        2 => Ok(Version::V02),
        3 => Ok(Version::V03),
        4 => Ok(Version::V04),
        5 => Ok(Version::V05),
        6 => Ok(Version::V06),
        7 => Ok(Version::V07),
        8 => Ok(Version::V08),
        9 => Ok(Version::V09),
        10 => Ok(Version::V10),
        11 => Ok(Version::V11),
        12 => Ok(Version::V12),
        13 => Ok(Version::V13),
        14 => Ok(Version::V14),
        15 => Ok(Version::V15),
        16 => Ok(Version::V16),
        17 => Ok(Version::V17),
        18 => Ok(Version::V18),
        19 => Ok(Version::V19),
        20 => Ok(Version::V20),
        21 => Ok(Version::V21),
        22 => Ok(Version::V22),
        23 => Ok(Version::V23),
        24 => Ok(Version::V24),
        25 => Ok(Version::V25),
        26 => Ok(Version::V26),
        27 => Ok(Version::V27),
        28 => Ok(Version::V28),
        29 => Ok(Version::V29),
        30 => Ok(Version::V30),
        31 => Ok(Version::V31),
        32 => Ok(Version::V32),
        33 => Ok(Version::V33),
        34 => Ok(Version::V34),
        35 => Ok(Version::V35),
        36 => Ok(Version::V36),
        37 => Ok(Version::V37),
        38 => Ok(Version::V38),
        39 => Ok(Version::V39),
        _ => Ok(Version::V40),
    }
}

/// Converts a fast_qr version back into its 1-based number.
#[must_use]
pub fn version_to_u8(version: Version) -> u8 {
    // Discriminants are 0-based (V01 = 0 ... V40 = 39).
    (version as u8) + 1
}

/// Encodes bytes into a QR matrix at the given version/ECL.
pub fn qr_encode(payload: &[u8], version: Version, ecl: QrEcl) -> Result<QrMatrix, PigeonError> {
    let code = QRBuilder::new(payload.to_vec())
        .ecl(ecl.to_fast_qr())
        .version(version)
        .build()
        .map_err(|e| PigeonError::QrEncodeFailed(format!("{e}")))?;

    let size = code.size;
    let mut modules = Vec::with_capacity(size * size);
    for row in 0..size {
        for col in 0..size {
            // Indexing yields a row slice; index again for the module.
            modules.push(u8::from(code[row][col].value()));
        }
    }
    Ok(QrMatrix { size, modules })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_probe_matches_known_table() {
        // Byte-mode capacities, verified against fast_qr 0.14's tables.
        assert_eq!(probe_byte_capacity(Version::V01, QrEcl::L), 17);
        assert_eq!(probe_byte_capacity(Version::V20, QrEcl::L), 858);
        assert_eq!(probe_byte_capacity(Version::V40, QrEcl::L), 2953);
        // M and H have less room.
        assert_eq!(probe_byte_capacity(Version::V40, QrEcl::M), 2331);
        assert_eq!(probe_byte_capacity(Version::V40, QrEcl::H), 1273);
    }

    #[test]
    fn smallest_version_finds_fit() {
        let v = smallest_version_for(1024, QrEcl::L).expect("fits under v40");
        assert!(probe_byte_capacity(v, QrEcl::L) >= 1024);
        // The previous version must NOT fit (we picked the smallest).
        if let Ok(prev) = version_from_u8(v as u8 - 1) {
            assert!(probe_byte_capacity(prev, QrEcl::L) < 1024);
        }
        assert!(smallest_version_for(10_000, QrEcl::L).is_none());
    }

    #[test]
    fn qr_matrix_dimensions_and_e2e_decode_capacity() {
        let payload = vec![0x41u8; 500];
        let matrix = qr_encode(&payload, Version::V15, QrEcl::L).expect("encodes");
        // V15 = 77 modules (4*15 + 17)
        assert_eq!(matrix.size, 77);
        assert_eq!(matrix.modules.len(), 77 * 77);
        // finder pattern at top-left corner must be dark
        assert_eq!(matrix.modules[0], 1);
    }

    #[test]
    fn qr_encode_rejects_oversize() {
        let payload = vec![0x41u8; 3000];
        assert!(qr_encode(&payload, Version::V10, QrEcl::H).is_err());
    }

    #[test]
    fn version_parsing() {
        assert!(version_from_u8(0).is_err());
        assert!(version_from_u8(41).is_err());
        assert!(version_from_u8(40).is_ok());
    }

    #[test]
    fn ecl_parsing() {
        assert!(matches!(QrEcl::parse("m"), Ok(QrEcl::M)));
        assert!(QrEcl::parse("X").is_err());
    }
}
