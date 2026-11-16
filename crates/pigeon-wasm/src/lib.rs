//! WebAssembly bindings for the Pigeon engine.
//!
//! Thin translation layer only: all protocol logic lives in `pigeon-core`.
//! Data crossing the boundary uses serde-wasm-bindgen with camelCase keys.

use serde::Serialize;
use wasm_bindgen::prelude::*;

use pigeon_core::decoder::{DecodeLimits, DecoderEngine, IngestReport, TransferStatus};
use pigeon_core::encoder::{CompressionMode, EncodeConfig, QrEcl};
use pigeon_core::error::PigeonError;

/// Deserialized encoding parameters from JavaScript (all optional except filename).
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct JsEncodeParams {
    filename: Option<String>,
    mime_type: Option<String>,
    compression: Option<String>,
    overhead_percent: Option<u32>,
    qr_ecl: Option<String>,
    qr_version: Option<u8>,
    frame_delay_ms: Option<u32>,
    max_file_size: Option<u64>,
    target_frame_px: Option<u32>,
}

impl JsEncodeParams {
    fn to_config(&self) -> Result<EncodeConfig, JsError> {
        let mut cfg = EncodeConfig::default();
        if let Some(f) = &self.filename {
            cfg.filename = f.clone();
        }
        cfg.mime_type = self.mime_type.clone();
        if let Some(c) = &self.compression {
            cfg.compression = match c.to_ascii_lowercase().as_str() {
                "auto" => CompressionMode::Auto,
                "on" => CompressionMode::On,
                "off" => CompressionMode::Off,
                other => {
                    return Err(JsError::new(&format!(
                        "compression must be auto|on|off, got \"{other}\""
                    )))
                }
            };
        }
        if let Some(p) = self.overhead_percent {
            cfg.overhead_percent = p;
        }
        if let Some(e) = &self.qr_ecl {
            cfg.qr_ecl = QrEcl::parse(e)?;
        }
        cfg.qr_version = self.qr_version;
        if let Some(d) = self.frame_delay_ms {
            cfg.frame_delay_ms = d;
        }
        if let Some(m) = self.max_file_size {
            cfg.max_file_size = m;
        }
        cfg.validated().map_err(map_err)
    }
}

fn map_err(err: PigeonError) -> JsError {
    JsError::new(&err.to_string())
}

fn parse_transfer_id(hex: &str) -> Result<u64, JsError> {
    u64::from_str_radix(hex, 16).map_err(|_| {
        JsError::new(&format!(
            "transfer id must be a 16-character hex string, got \"{hex}\""
        ))
    })
}

fn report_to_js(report: IngestReport) -> Result<JsValue, JsError> {
    let ser = serde_wasm_bindgen::Serializer::new();
    report.serialize(&ser).map_err(|e| JsError::new(&e.to_string()))
}

/// The wire protocol version this engine implements.
#[wasm_bindgen]
pub fn protocol_version() -> u8 {
    pigeon_core::PROTOCOL_VERSION
}

/// One-shot send pipeline: file bytes → animated QR GIF.
///
/// Returns `{ gif: Uint8Array, stats }`.
#[wasm_bindgen]
pub fn encode_gif(data: &[u8], params: JsValue) -> Result<JsValue, JsError> {
    let js: JsEncodeParams = if params.is_undefined() {
        JsEncodeParams::default()
    } else {
        serde_wasm_bindgen::from_value(params).map_err(|e| JsError::new(&e.to_string()))?
    };
    let config = js.to_config()?;
    let target_px = js.target_frame_px.unwrap_or(800);
    let (gif_bytes, stats) =
        pigeon_core::encoder::encode_gif_bytes(data, config, target_px).map_err(map_err)?;

    let ser = serde_wasm_bindgen::Serializer::new();
    let stats_js = stats.serialize(&ser).map_err(|e| JsError::new(&e.to_string()))?;
    let obj = js_sys::Object::new();
    js_sys::Reflect::set(&obj, &"gif".into(), &gif_bytes.into())
        .map_err(|_| JsError::new("failed to attach gif bytes"))?;
    js_sys::Reflect::set(&obj, &"stats".into(), &stats_js)
        .map_err(|_| JsError::new("failed to attach stats"))?;
    Ok(obj.into())
}

/// Prepares a transfer: wire packets are encoded up front, QR matrices are
/// generated one at a time by the returned session (used for live playback
/// and for progress-reporting GIF generation).
///
/// Returns `{ stats, session }`.
#[wasm_bindgen]
pub fn prepare_transfer(data: &[u8], params: JsValue) -> Result<JsValue, JsError> {
    let js: JsEncodeParams = if params.is_undefined() {
        JsEncodeParams::default()
    } else {
        serde_wasm_bindgen::from_value(params).map_err(|e| JsError::new(&e.to_string()))?
    };
    let config = js.to_config()?;
    let transfer = pigeon_core::encoder::encode_file(data, config).map_err(map_err)?;
    let session = QrFrameSession::new(transfer)?;

    let ser = serde_wasm_bindgen::Serializer::new();
    let stats_js = session
        .stats()
        .serialize(&ser)
        .map_err(|e| JsError::new(&e.to_string()))?;
    let obj = js_sys::Object::new();
    js_sys::Reflect::set(&obj, &"stats".into(), &stats_js)
        .map_err(|_| JsError::new("failed to attach stats"))?;
    js_sys::Reflect::set(&obj, &"session".into(), &session.into())
        .map_err(|_| JsError::new("failed to attach session"))?;
    Ok(obj.into())
}

/// Incremental GIF writer for the send pipeline. Pair with a
/// [`QrFrameSession`] to interleave QR generation and report progress.
///
/// `start(matrix, scale, delayMs)` writes the first frame (canvas size is
/// fixed from it); `writeMatrix` appends; `finish` returns the bytes.
#[wasm_bindgen]
pub struct GifBuilder {
    inner: Option<pigeon_core::encoder::gif::GifBuilder>,
}

#[wasm_bindgen]
impl GifBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new(first_modules: &[u8], first_size: usize, scale: u16, delay_ms: u32) -> Result<GifBuilder, JsError> {
        let matrix = pigeon_core::encoder::QrMatrix {
            size: first_size,
            modules: first_modules.to_vec(),
        };
        let inner = pigeon_core::encoder::gif::GifBuilder::start(&matrix, scale, delay_ms)
            .map_err(map_err)?;
        Ok(Self { inner: Some(inner) })
    }

    /// Appends a frame. `modules` is a 1-byte-per-module row-major matrix.
    #[wasm_bindgen(js_name = writeMatrix)]
    pub fn write_matrix(&mut self, modules: &[u8], size: usize) -> Result<(), JsError> {
        let m = pigeon_core::encoder::QrMatrix {
            size,
            modules: modules.to_vec(),
        };
        let builder = self
            .inner
            .as_mut()
            .ok_or_else(|| JsError::new("gif builder already finished"))?;
        builder.write_matrix(&m).map_err(map_err)
    }

    /// Frames written so far.
    #[wasm_bindgen(js_name = framesWritten)]
    pub fn frames_written(&self) -> usize {
        self.inner.as_ref().map_or(0, |b| b.frames_written())
    }

    /// Completes the GIF and returns the bytes (transferable-capable buffer).
    pub fn finish(&mut self) -> Result<Vec<u8>, JsError> {
        let builder = self
            .inner
            .take()
            .ok_or_else(|| JsError::new("gif builder already finished"))?;
        builder.finish().map_err(map_err)
    }
}

/// Incremental QR generation over a prepared transfer. One matrix lives in
/// memory at a time; pull frames with [`next_frame`].
#[wasm_bindgen]
pub struct QrFrameSession {
    inner: Option<pigeon_core::encoder::QrSession>,
}

#[wasm_bindgen]
impl QrFrameSession {
    fn new(transfer: pigeon_core::encoder::EncodedTransfer) -> Result<Self, JsError> {
        let session = pigeon_core::encoder::QrSession::new(transfer).map_err(map_err)?;
        Ok(Self {
            inner: Some(session),
        })
    }

    fn stats(&self) -> &pigeon_core::encoder::EncodeStats {
        self.inner
            .as_ref()
            .map(|s| s.stats())
            .expect("session alive for stats access")
    }

    /// Total QR frames in the sequence.
    #[wasm_bindgen(js_name = totalFrames)]
    pub fn total_frames(&self) -> usize {
        self.inner.as_ref().map_or(0, |s| s.total_frames())
    }

    /// Next QR frame as `{ size, modules: Uint8Array }` (1 byte per module),
    /// or `null` when the sequence is exhausted.
    #[wasm_bindgen(js_name = nextFrame)]
    pub fn next_frame(&mut self) -> Result<JsValue, JsError> {
        let session = self
            .inner
            .as_mut()
            .ok_or_else(|| JsError::new("session closed"))?;
        match session.next_matrix().map_err(map_err)? {
            None => Ok(JsValue::NULL),
            Some(matrix) => {
                let ser = serde_wasm_bindgen::Serializer::new();
                matrix.serialize(&ser).map_err(|e| JsError::new(&e.to_string()))
            }
        }
    }
}

/// Receive engine bound to JS: ingest packets, watch progress, take files.
#[wasm_bindgen]
pub struct Receiver {
    inner: DecoderEngine,
}

#[wasm_bindgen]
impl Receiver {
    /// Creates a receiver. `max_transfer_bytes` bounds each transfer's
    /// declared size (resource guard).
    #[wasm_bindgen(constructor)]
    pub fn new(max_transfer_bytes: Option<JsValue>) -> Self {
        // JS callers pass a Number; values above 2^53 are out of scope for
        // the soft limit and the engine cap applies anyway.
        let max_bytes: Option<u64> = max_transfer_bytes.map(|v| v.as_f64().map(|f| f as u64)).flatten();
        let limits = DecodeLimits {
            max_transfer_bytes: max_bytes
                .unwrap_or(pigeon_core::protocol::MAX_DECODE_TRANSFER_BYTES),
            max_transfers: pigeon_core::protocol::MAX_CONCURRENT_TRANSFERS,
        };
        Self {
            inner: DecoderEngine::new(limits),
        }
    }

    /// Ingests one candidate packet (decoded QR content).
    /// Returns `{ valid, duplicate, reason?, transferId?, status? }`.
    #[wasm_bindgen(js_name = ingestPacket)]
    pub fn ingest_packet(&mut self, bytes: &[u8]) -> Result<JsValue, JsError> {
        let report = self.inner.ingest_packet(bytes).map_err(map_err)?;
        report_to_js(report)
    }

    /// Status of one transfer (`transferId` is a 16-char hex string) or `null`.
    #[wasm_bindgen(js_name = statusOf)]
    pub fn status_of(&self, transfer_id: &str) -> Result<JsValue, JsError> {
        let ser = serde_wasm_bindgen::Serializer::new();
        let id = parse_transfer_id(transfer_id)?;
        match self.inner.status(id) {
            Some(status) => status.serialize(&ser).map_err(|e| JsError::new(&e.to_string())),
            None => Ok(JsValue::NULL),
        }
    }

    /// Status of the most recently active transfer or `null`.
    #[wasm_bindgen(js_name = activeStatus)]
    pub fn active_status(&self) -> Result<JsValue, JsError> {
        let ser = serde_wasm_bindgen::Serializer::new();
        match self.inner.active_status() {
            Some(status) => status.serialize(&ser).map_err(|e| JsError::new(&e.to_string())),
            None => Ok(JsValue::NULL),
        }
    }

    /// All tracked transfers.
    #[wasm_bindgen(js_name = allStatuses)]
    pub fn all_statuses(&self) -> Result<JsValue, JsError> {
        let ser = serde_wasm_bindgen::Serializer::new();
        let statuses: Vec<TransferStatus> = self.inner.all_statuses();
        statuses.serialize(&ser).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Aggregate counters.
    #[wasm_bindgen(js_name = counters)]
    pub fn counters(&self) -> Result<JsValue, JsError> {
        let ser = serde_wasm_bindgen::Serializer::new();
        self.inner
            .counters()
            .serialize(&ser)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Takes the verified file for a completed transfer (`transferId` is a
    /// 16-char hex string). Returns `null` while incomplete; throws on
    /// verification failure.
    #[wasm_bindgen(js_name = takeFile)]
    pub fn take_file(&mut self, transfer_id: &str) -> Result<JsValue, JsError> {
        let ser = serde_wasm_bindgen::Serializer::new();
        let id = parse_transfer_id(transfer_id)?;
        match self.inner.take_file(id).map_err(map_err)? {
            Some(file) => file.serialize(&ser).map_err(|e| JsError::new(&e.to_string())),
            None => Ok(JsValue::NULL),
        }
    }

    /// Clears all receive state.
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

/// Streaming GIF frame extractor for the import path.
#[wasm_bindgen]
pub struct GifFrameStream {
    inner: pigeon_core::decoder::gif::GifFrameStream,
}

#[wasm_bindgen]
impl GifFrameStream {
    /// Opens a GIF (validates signature, dimensions and size limits).
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<GifFrameStream, JsError> {
        let inner = pigeon_core::decoder::gif::GifFrameStream::new(bytes).map_err(map_err)?;
        Ok(Self { inner })
    }

    /// `[width, height]` of the composited canvas frames.
    #[wasm_bindgen(js_name = dimensions)]
    pub fn dimensions(&self) -> Vec<u32> {
        let (w, h) = self.inner.dimensions();
        vec![u32::from(w), u32::from(h)]
    }

    /// Next composited frame `{ index, delayMs, width, height, data: Uint8Array }`
    /// or `null` at end of stream.
    #[wasm_bindgen(js_name = nextFrame)]
    pub fn next_frame(&mut self) -> Result<JsValue, JsError> {
        let ser = serde_wasm_bindgen::Serializer::new();
        match self.inner.next_frame().map_err(map_err)? {
            None => Ok(JsValue::NULL),
            Some(frame) => frame.serialize(&ser).map_err(|e| JsError::new(&e.to_string())),
        }
    }
}

/// Sanity helper exposed for tests: sanitizes a received filename.
#[wasm_bindgen(js_name = sanitizeFilename)]
pub fn sanitize_filename(name: &str) -> String {
    pigeon_core::protocol::metadata::sanitize_filename(name)
}
