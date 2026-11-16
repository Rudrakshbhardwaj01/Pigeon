//! Reproducible benchmarks for the Pigeon engine.
//!
//! Run in release mode (debug builds are 50-100x slower for RaptorQ):
//!
//! ```sh
//! cargo test -p pigeon-core --release --test bench -- --ignored --nocapture
//! ```
//!
//! Every number printed here is a real measurement on the current machine;
//! see docs/BENCHMARKS.md for recorded results.

use std::time::Instant;

use pigeon_core::decoder::gif::GifFrameStream;
use pigeon_core::decoder::{DecoderEngine, DecodeLimits};
use pigeon_core::encoder::gif::{encode_gif, QUIET_MODULES};
use pigeon_core::encoder::encode_qr_matrices;
use pigeon_core::encoder::{encode_file, CompressionMode, EncodeConfig};
use pigeon_core::integrity::sha256_hex;

use rqrr::PreparedImage;

fn xorshift_data(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

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

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn bench_encode_pipeline_1mb() {
    let data = xorshift_data(1024 * 1024, 0xABCD);
    let cfg = EncodeConfig {
        filename: "bench-1mb.bin".into(),
        mime_type: Some("application/octet-stream".into()),
        compression: CompressionMode::Off,
        overhead_percent: 30,
        ..Default::default()
    };

    let t0 = Instant::now();
    let transfer = encode_file(&data, cfg).expect("encodes");
    let fountain_ms = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let matrices = encode_qr_matrices(&transfer, &EncodeConfig::default()).expect("qr");
    let qr_ms = t1.elapsed().as_millis();

    let modules = matrices[0].size + 2 * QUIET_MODULES;
    let scale = pigeon_core::encoder::gif::scale_for_target_px(800, modules);
    let t2 = Instant::now();
    let gif = encode_gif(&matrices, scale, transfer.stats.frame_delay_ms).expect("gif");
    let gif_ms = t2.elapsed().as_millis();

    println!("== encode 1 MiB (incompressible, overhead 30%, V{}-{}) ==", transfer.stats.qr_version, transfer.stats.qr_ecl);
    println!("  fountain encode : {fountain_ms} ms ({} frames)", transfer.stats.total_frames);
    println!("  QR generation   : {qr_ms} ms ({:.1} frames/ms)", transfer.stats.total_frames as f64 / qr_ms as f64);
    println!("  GIF assembly    : {gif_ms} ms ({}x{} px frames)", matrices[0].size + 2 * QUIET_MODULES, matrices[0].size + 2 * QUIET_MODULES);
    println!("  GIF size        : {} KiB ({:.2} KiB/frame)", gif.len() / 1024, gif.len() as f64 / 1024.0 / transfer.stats.total_frames as f64);
}

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn bench_decode_pipeline_1mb() {
    let data = xorshift_data(1024 * 1024, 0xABCD);
    let cfg = EncodeConfig {
        filename: "bench-1mb.bin".into(),
        compression: CompressionMode::Off,
        overhead_percent: 30,
        ..Default::default()
    };
    let transfer = encode_file(&data, cfg).expect("encodes");
    let matrices = encode_qr_matrices(&transfer, &EncodeConfig::default()).expect("qr");
    let modules = matrices[0].size + 2 * QUIET_MODULES;
    let scale = pigeon_core::encoder::gif::scale_for_target_px(800, modules);
    let gif = encode_gif(&matrices, scale, transfer.stats.frame_delay_ms).expect("gif");

    // GIF parse + QR decode rate (with 20% of frames dropped)
    let mut engine = DecoderEngine::new(DecodeLimits::default());
    let mut stream = GifFrameStream::new(&gif).expect("opens");
    let t = Instant::now();
    let (mut seen, mut decoded) = (0usize, 0usize);
    while let Some(frame) = stream.next_frame().expect("frame") {
        seen += 1;
        if seen % 5 == 0 {
            continue; // 20% frame loss
        }
        if let Some(bytes) = decode_qr_rgba(&frame.data, frame.width as usize, frame.height as usize) {
            decoded += 1;
            engine.ingest_packet(&bytes).expect("ingest");
        }
    }
    let elapsed = t.elapsed().as_millis();
    println!("== decode 1 MiB transfer (20% frame loss) ==");
    println!("  frames parsed+QR-decoded : {seen} parsed, {decoded} decoded in {elapsed} ms");
    println!("  throughput               : {:.1} frames/ms", seen as f64 / elapsed as f64);
    println!("  effective data rate      : {:.0} KiB/s at 4 fps capture", data.len() as f64 / 1024.0 / (seen as f64 * 0.25));
    let status = engine.active_status().expect("tracked");
    assert_eq!(status.phase, "verified");
    println!("  reconstruction           : verified, sha256 {}", &sha256_hex(&data)[0..8]);
}

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn bench_qr_capacity_table() {
    use pigeon_core::encoder::qr::{probe_byte_capacity, version_from_u8, QrEcl};
    println!("== byte-mode capacity per QR version (bytes) ==");
    print!("  version: ");
    for v in [10u8, 15, 20, 21, 23, 25, 26, 29, 33, 40] {
        print!("V{v:>3} ");
    }
    println!();
    print!("  ECL L  : ");
    for v in [10u8, 15, 20, 21, 23, 25, 26, 29, 33, 40] {
        print!("{:>4} ", probe_byte_capacity(version_from_u8(v).unwrap(), QrEcl::L));
    }
    println!();
    print!("  ECL M  : ");
    for v in [10u8, 15, 20, 21, 23, 25, 26, 29, 33, 40] {
        print!("{:>4} ", probe_byte_capacity(version_from_u8(v).unwrap(), QrEcl::M));
    }
    println!();
}
