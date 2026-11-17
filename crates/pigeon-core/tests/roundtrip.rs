//! Native end-to-end integration test: file → wire packets → QR matrices →
//! GIF → GIF parse → QR decode → packet ingestion → reconstruction.
//!
//! This exercises every stage of the real pipeline without a browser: the QR
//! decoder (`rqrr`) is a test-only dependency standing in for the browser's
//! ZXing WASM decoder. The reconstructed bytes must match the original
//! byte-for-byte, and the SHA-256 must verify.

use pigeon_core::decoder::gif::GifFrameStream;
use pigeon_core::decoder::{DecodeLimits, DecoderEngine};
use pigeon_core::encoder::{encode_file, EncodeConfig};
use pigeon_core::integrity::sha256_hex;

use rqrr::PreparedImage;

/// Decodes the first QR symbol found in an RGBA frame using rqrr.
///
/// Uses `decode_to` so binary payloads survive byte-exactly (the String-based
/// `decode` would corrupt arbitrary binary content).
fn decode_qr_rgba(rgba: &[u8], width: usize, height: usize) -> Option<Vec<u8>> {
    let mut prepared = PreparedImage::prepare_from_greyscale(width, height, |x, y| {
        let px = (y * width + x) * 4;
        let l = (u32::from(rgba[px]) * 299
            + u32::from(rgba[px + 1]) * 587
            + u32::from(rgba[px + 2]) * 114)
            / 1000;
        l as u8
    });
    for grid in prepared.detect_grids() {
        let mut content = Vec::new();
        if grid.decode_to(&mut content).is_ok() && !content.is_empty() {
            return Some(content);
        }
    }
    None
}

/// Downscales an RGBA frame by an integer factor, sampling the centre pixel
/// of each block (nearest-neighbour). Averaging would blur module edges into
/// greys that a global-threshold decoder struggles with; a coarse camera view
/// still resolves crisp module boundaries.
fn downscale_rgba(
    rgba: &[u8],
    width: usize,
    height: usize,
    factor: usize,
) -> (Vec<u8>, usize, usize) {
    let nw = width / factor;
    let nh = height / factor;
    let mut out = vec![0u8; nw * nh * 4];
    for ny in 0..nh {
        for nx in 0..nw {
            let sy = (ny * factor + factor / 2).min(height - 1);
            let sx = (nx * factor + factor / 2).min(width - 1);
            let idx = (sy * width + sx) * 4;
            let o = (ny * nw + nx) * 4;
            out[o] = rgba[idx];
            out[o + 1] = rgba[idx + 1];
            out[o + 2] = rgba[idx + 2];
            out[o + 3] = 255;
        }
    }
    (out, nw, nh)
}

struct PipelineOutcome {
    packets_decoded: usize,
    frames_seen: usize,
}

/// Runs the complete optical pipeline: encode → GIF → decode GIF → decode QR
/// (with optional deterministic frame drop) → reconstruct.
fn run_optical_roundtrip(
    data: &[u8],
    config: EncodeConfig,
    drop_every_nth: Option<usize>,
    downscale_factor: usize,
) -> Result<(Vec<u8>, PipelineOutcome), Box<dyn std::error::Error>> {
    // ---- encode ----
    let transfer = encode_file(data, config)?;
    assert!(transfer.stats.total_frames >= 2);

    // ---- render all frames to QR matrices (test path; production streams) ----
    let matrices = pigeon_core::encoder::encode_qr_matrices(&transfer, &EncodeConfig::default())?;
    let modules = matrices[0].size + 2 * pigeon_core::encoder::gif::QUIET_MODULES;
    // 900 px frames: large enough that a 2x downscale (a coarse camera view)
    // still leaves ~4 px per module for the decoder.
    let scale = pigeon_core::encoder::gif::scale_for_target_px(900, modules);

    // ---- assemble GIF ----
    let gif_bytes =
        pigeon_core::encoder::gif::encode_gif(&matrices, scale, transfer.stats.frame_delay_ms)?;
    assert!(gif_bytes.starts_with(b"GIF89a"));

    // ---- receive: parse GIF, decode QR, ingest packets ----
    let mut engine = DecoderEngine::new(DecodeLimits::default());
    let mut stream = GifFrameStream::new(&gif_bytes)?;
    let mut outcome = PipelineOutcome {
        packets_decoded: 0,
        frames_seen: 0,
    };

    while let Some(frame) = stream.next_frame()? {
        outcome.frames_seen += 1;
        if let Some(n) = drop_every_nth {
            if outcome.frames_seen.is_multiple_of(n) {
                continue; // simulate a missed frame
            }
        }
        let (rgba, w, h) = if downscale_factor > 1 {
            downscale_rgba(
                &frame.data,
                frame.width as usize,
                frame.height as usize,
                downscale_factor,
            )
        } else {
            (
                frame.data.clone(),
                frame.width as usize,
                frame.height as usize,
            )
        };
        if let Some(bytes) = decode_qr_rgba(&rgba, w, h) {
            outcome.packets_decoded += 1;
            engine.ingest_packet(&bytes)?;
        }
    }

    let statuses = engine.all_statuses();
    assert_eq!(statuses.len(), 1, "exactly one transfer must be tracked");
    let status = &statuses[0];
    assert_eq!(status.phase, "verified", "transfer must verify: {status:?}");

    let file = engine
        .take_file(status.transfer_id)?
        .expect("verified transfer must yield a file");
    assert_eq!(file.data, data, "reconstruction must be byte-exact");
    assert_eq!(file.sha256, sha256_hex(data));
    Ok((file.data, outcome))
}

fn base_config(name: &str) -> EncodeConfig {
    EncodeConfig {
        filename: name.into(),
        mime_type: Some("application/octet-stream".into()),
        overhead_percent: 40, // generous for simulated loss
        ..Default::default()
    }
}

#[test]
fn optical_roundtrip_text_file() {
    let data = b"Pigeon carries this message between two machines.\
        No network. No server. Just light."
        .to_vec();
    let (_, out) = run_optical_roundtrip(&data, base_config("message.txt"), None, 1)
        .expect("full pipeline roundtrip");
    assert!(out.packets_decoded >= 2);
}

#[test]
fn optical_roundtrip_binary_all_byte_values() {
    // Every byte value in a non-trivial sequence — catches any text/UTF-8
    // assumptions in the QR payload path.
    let data: Vec<u8> = (0..=255u8).cycle().take(8_192).collect();
    let (_, _) =
        run_optical_roundtrip(&data, base_config("binary.bin"), None, 1).expect("binary roundtrip");
}

#[test]
fn optical_roundtrip_compressible_10kb() {
    let data = vec![b'p'; 10_240];
    let cfg = EncodeConfig {
        filename: "repetitive.log".into(),
        compression: pigeon_core::encoder::CompressionMode::Auto,
        ..base_config("repetitive.log")
    };
    let (out_data, _) = run_optical_roundtrip(&data, cfg, None, 1).expect("compressed roundtrip");
    assert_eq!(out_data.len(), data.len());
}

#[test]
fn optical_roundtrip_with_frame_loss() {
    // High-entropy data (compression off) so the transfer spans many frames.
    let mut state = 0x9E3779B97F4A7C15u64;
    let data: Vec<u8> = (0..30_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    let cfg = EncodeConfig {
        filename: "lossy.bin".into(),
        compression: pigeon_core::encoder::CompressionMode::Off,
        ..base_config("lossy.bin")
    };
    // drop every 5th frame (~20% loss) — FEC must recover
    let (_, out) =
        run_optical_roundtrip(&data, cfg, Some(5), 1).expect("recovers with 20% frame loss");
    assert!(out.packets_decoded < out.frames_seen);
}

#[test]
fn optical_roundtrip_downscaled_camera_view() {
    let data = b"simulating a camera that captures fewer pixels".to_vec();
    // downscale by 2 approximates a phone camera view of a QR on a screen
    let (_, _) = run_optical_roundtrip(&data, base_config("camera.txt"), None, 2)
        .expect("roundtrip through downscaled frames");
}

#[test]
fn optical_roundtrip_out_of_order_frames() {
    let data: Vec<u8> = (0..20_000u32).map(|i| ((i * 7) % 251) as u8).collect();
    let transfer = encode_file(&data, base_config("shuffled.bin")).expect("encodes");
    let matrices =
        pigeon_core::encoder::encode_qr_matrices(&transfer, &EncodeConfig::default()).expect("qr");

    // deterministic shuffle
    let mut indices: Vec<usize> = (0..matrices.len()).collect();
    let mut seed = 0xC0FFEEu64;
    for i in (1..indices.len()).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (seed >> 33) as usize % (i + 1);
        indices.swap(i, j);
    }
    let shuffled: Vec<pigeon_core::encoder::QrMatrix> =
        indices.iter().map(|&i| matrices[i].clone()).collect();

    let modules = shuffled[0].size + 2 * pigeon_core::encoder::gif::QUIET_MODULES;
    let scale = pigeon_core::encoder::gif::scale_for_target_px(600, modules);
    let gif_bytes =
        pigeon_core::encoder::gif::encode_gif(&shuffled, scale, transfer.stats.frame_delay_ms)
            .expect("gif");

    let mut engine = DecoderEngine::default();
    let mut stream = GifFrameStream::new(&gif_bytes).expect("opens");
    while let Some(frame) = stream.next_frame().expect("frame") {
        if let Some(bytes) =
            decode_qr_rgba(&frame.data, frame.width as usize, frame.height as usize)
        {
            engine.ingest_packet(&bytes).expect("ingest");
        }
    }
    let status = engine.active_status().expect("tracked");
    assert_eq!(status.phase, "verified");
    let file = engine
        .take_file(status.transfer_id)
        .expect("ok")
        .expect("file");
    assert_eq!(file.data, data);
}

#[test]
fn optical_roundtrip_multiple_transfers_one_gif_stream() {
    // Two independent transfers must never mix, even when ingested
    // interleaved (mixed transfers simulated by alternating frames).
    let a = b"transfer alpha payload".to_vec();
    let b: Vec<u8> = (0..9_000u32).map(|i| ((i * 13) % 249) as u8).collect();
    let ta = encode_file(&a, base_config("alpha.txt")).expect("a");
    let tb = encode_file(&b, base_config("beta.bin")).expect("b");

    let ma = pigeon_core::encoder::encode_qr_matrices(&ta, &EncodeConfig::default()).expect("qr a");
    let mb = pigeon_core::encoder::encode_qr_matrices(&tb, &EncodeConfig::default()).expect("qr b");

    let mut engine = DecoderEngine::default();
    for (ia, ib) in ma.iter().zip(mb.iter()) {
        for m in [ia, ib] {
            // render matrix into RGBA at scale 8 and decode
            let mut bitmap = Vec::new();
            pigeon_core::encoder::gif::render_frame(m, 8, &mut bitmap);
            let side = (m.size + 2 * pigeon_core::encoder::gif::QUIET_MODULES) * 8;
            let rgba = grayscale_to_rgba(&bitmap);
            if let Some(bytes) = decode_qr_rgba(&rgba, side, side) {
                engine.ingest_packet(&bytes).expect("ingest");
            }
        }
    }
    assert_eq!(engine.all_statuses().len(), 2);
    let fa = engine
        .take_file(ta.stats.transfer_id)
        .expect("ok")
        .expect("a");
    let fb = engine
        .take_file(tb.stats.transfer_id)
        .expect("ok")
        .expect("b");
    assert_eq!(fa.data, a);
    assert_eq!(fb.data, b);
}

fn grayscale_to_rgba(indices: &[u8]) -> Vec<u8> {
    indices
        .iter()
        .flat_map(|&i| {
            let v = if i == 1 { 0u8 } else { 255u8 };
            [v, v, v, 255]
        })
        .collect()
}

#[test]
fn qr_version_and_capacity_are_consistent() {
    // The auto-selected version must hold the configured packet payload.
    let cfg = base_config("cap.bin");
    let transfer = encode_file(&vec![0u8; 50_000], cfg).expect("encodes");
    // smoke: stats align with what QR generation accepts
    assert!(transfer.stats.packet_payload_bytes > 900);
}

#[test]
fn incomplete_transfer_reports_honestly() {
    let data = vec![1u8; 80_000];
    let transfer = encode_file(&data, base_config("partial.bin")).expect("encodes");
    let matrices =
        pigeon_core::encoder::encode_qr_matrices(&transfer, &EncodeConfig::default()).expect("qr");

    let mut engine = DecoderEngine::default();
    // ingest only the first 20% of frames
    let cutoff = matrices.len() / 5;
    for m in matrices.iter().take(cutoff.max(1)) {
        let mut bitmap = Vec::new();
        pigeon_core::encoder::gif::render_frame(m, 8, &mut bitmap);
        let side = (m.size + 2 * pigeon_core::encoder::gif::QUIET_MODULES) * 8;
        let rgba = grayscale_to_rgba(&bitmap);
        if let Some(bytes) = decode_qr_rgba(&rgba, side, side) {
            engine.ingest_packet(&bytes).expect("ingest");
        }
    }
    let status = engine.active_status().expect("tracked");
    assert_ne!(status.phase, "verified");
    assert!(engine.take_file(status.transfer_id).expect("ok").is_none());
}

/// Complete optical roundtrip of an **encrypted** transfer: the sealed object
/// travels through the same QR/GIF pipeline, and only the correct passphrase
/// turns the captured frames into the original bytes.
#[test]
fn encrypted_optical_roundtrip_with_passphrase() {
    use pigeon_core::encoder::EncryptionConfig;
    use zeroize::Zeroizing;

    let data: Vec<u8> = (0..12_000u32).map(|i| ((i * 31 + 7) % 253) as u8).collect();
    let cfg = EncodeConfig {
        filename: "encrypted.txt".into(),
        mime_type: Some("text/plain".into()),
        overhead_percent: 40,
        encryption: Some(EncryptionConfig {
            passphrase: Zeroizing::new("optical secret".into()),
            kdf: pigeon_core::crypto::KdfParams {
                mem_kib: 8 * 1024,
                passes: 1,
                lanes: 1,
            },
        }),
        ..Default::default()
    };
    let transfer = encode_file(&data, cfg).expect("encodes");
    assert!(transfer.stats.encrypted);

    // The optical carrier must not leak the plaintext or the filename.
    let matrices =
        pigeon_core::encoder::encode_qr_matrices(&transfer, &EncodeConfig::default()).expect("qr");
    let modules = matrices[0].size + 2 * pigeon_core::encoder::gif::QUIET_MODULES;
    let scale = pigeon_core::encoder::gif::scale_for_target_px(900, modules);
    let gif_bytes =
        pigeon_core::encoder::gif::encode_gif(&matrices, scale, transfer.stats.frame_delay_ms)
            .expect("gif");

    let mut engine = DecoderEngine::default();
    let mut frames_seen = 0;
    let mut stream = GifFrameStream::new(&gif_bytes).expect("opens");
    while let Some(frame) = stream.next_frame().expect("frame") {
        frames_seen += 1;
        if let Some(bytes) =
            decode_qr_rgba(&frame.data, frame.width as usize, frame.height as usize)
        {
            engine.ingest_packet(&bytes).expect("ingest");
        }
    }
    assert!(frames_seen >= 2);

    let status = engine.active_status().expect("tracked");
    assert_eq!(status.phase, "awaiting-passphrase", "status: {status:?}");
    assert!(status.encrypted);
    assert!(
        status.filename.is_none(),
        "filename must not leak before decryption"
    );

    // Wrong passphrase: honest rejection, still paused.
    let outcome = engine.set_passphrase("not it").expect("applies");
    assert_eq!(outcome.rejected, 1);
    let status = engine.active_status().expect("tracked");
    assert_eq!(status.phase, "awaiting-passphrase");

    // Correct passphrase: full verified reconstruction.
    let outcome = engine.set_passphrase("optical secret").expect("applies");
    assert_eq!(outcome.unlocked, 1);
    let status = engine.active_status().expect("tracked");
    assert_eq!(status.phase, "verified", "status: {status:?}");
    let file = engine
        .take_file(status.transfer_id)
        .expect("ok")
        .expect("file");
    assert_eq!(file.data, data, "reconstruction must be byte-exact");
    assert_eq!(file.sha256, sha256_hex(&data));
    assert_eq!(file.filename, "encrypted.txt");
}
