//! Animated GIF assembly from QR matrices.
//!
//! Every frame is a full-frame two-colour bitmap (no inter-frame deltas, no
//! transparency) so any frame remains independently QR-decodable even when a
//! messaging platform extracts or recompresses individual frames.

use std::borrow::Cow;

use gif::{DisposalMethod, Encoder, Frame, Repeat};

use crate::error::PigeonError;

use super::qr::QrMatrix;

/// Quiet zone in QR modules on each side (spec recommends 4).
pub const QUIET_MODULES: usize = 4;

/// GIF delay is in hundredths of a second; keep at least 20 ms so players
/// do not fast-forward past frames.
const MIN_DELAY_CENTISECONDS: u16 = 2;

/// Pixel scale (px per module) chosen for a target frame size.
#[must_use]
pub fn scale_for_target_px(target_px: u32, modules_incl_quiet: usize) -> u16 {
    let target = target_px.max(64) as usize;
    let scale = target / modules_incl_quiet.max(1);
    scale.clamp(2, 16) as u16
}

/// Renders one QR matrix into a palette-index bitmap including the quiet zone.
///
/// Returns (width, height); index 0 = white, 1 = black.
pub fn render_frame(matrix: &QrMatrix, scale: u16, out: &mut Vec<u8>) -> (u16, u16) {
    let scale = scale.max(1) as usize;
    let total = (matrix.size + 2 * QUIET_MODULES) * scale;
    let w = total as u16;
    out.clear();
    out.resize(total * total, 0); // white background

    let offset = QUIET_MODULES * scale;
    for (my, row) in matrix.modules.chunks(matrix.size).enumerate() {
        for (mx, &dark) in row.iter().enumerate() {
            if dark == 1 {
                let y0 = offset + my * scale;
                let x0 = offset + mx * scale;
                for y in y0..y0 + scale {
                    let row_start = y * total + x0;
                    out[row_start..row_start + scale].fill(1);
                }
            }
        }
    }
    (w, w)
}

/// Assembles an animated GIF from QR matrices.
///
/// Frames share the global two-colour palette; `delay_ms` is converted to GIF
/// centiseconds (minimum 20 ms).
pub fn encode_gif(
    matrices: &[QrMatrix],
    scale: u16,
    delay_ms: u32,
) -> Result<Vec<u8>, PigeonError> {
    if matrices.is_empty() {
        return Err(PigeonError::InvalidConfig("no frames to encode".into()));
    }
    let delay_cs = u16::try_from(delay_ms / 10)
        .unwrap_or(1000)
        .max(MIN_DELAY_CENTISECONDS);

    // Two-colour global palette: index 0 = white, index 1 = black.
    let palette: [u8; 6] = [0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00];

    let mut bitmap: Vec<u8> = Vec::new();
    let (w, h) = render_frame(&matrices[0], scale, &mut bitmap);
    if u32::from(w) > crate::protocol::MAX_GIF_DIMENSION
        || u32::from(h) > crate::protocol::MAX_GIF_DIMENSION
    {
        return Err(PigeonError::GifDimensionsTooLarge(
            w,
            h,
            crate::protocol::MAX_GIF_DIMENSION,
        ));
    }

    let mut writer = Vec::new();
    {
        let mut enc = Encoder::new(&mut writer, w, h, &palette)
            .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))?;
        enc.set_repeat(Repeat::Infinite)
            .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))?;

        for matrix in matrices {
            render_frame(matrix, scale, &mut bitmap);
            let frame = Frame {
                width: w,
                height: h,
                delay: delay_cs,
                dispose: DisposalMethod::Keep,
                transparent: None,
                needs_user_input: false,
                top: 0,
                left: 0,
                interlaced: false,
                palette: None,
                buffer: Cow::Borrowed(&bitmap),
            };
            enc.write_frame(&frame)
                .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))?;
        }
    }
    Ok(writer)
}

/// Streaming GIF writer driven by a QR-matrix iterator: exactly one matrix is
/// in memory at any time plus the growing output buffer.
pub fn encode_gif_streaming<S>(
    first: QrMatrix,
    session: &mut S,
    scale: u16,
    delay_ms: u32,
) -> Result<Vec<u8>, PigeonError>
where
    S: Iterator<Item = Result<QrMatrix, PigeonError>>,
{
    let delay_cs = u16::try_from(delay_ms / 10)
        .unwrap_or(1000)
        .max(MIN_DELAY_CENTISECONDS);
    let palette: [u8; 6] = [0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00];

    let mut bitmap: Vec<u8> = Vec::new();
    let (w, h) = render_frame(&first, scale, &mut bitmap);
    if u32::from(w) > crate::protocol::MAX_GIF_DIMENSION
        || u32::from(h) > crate::protocol::MAX_GIF_DIMENSION
    {
        return Err(PigeonError::GifDimensionsTooLarge(
            w,
            h,
            crate::protocol::MAX_GIF_DIMENSION,
        ));
    }

    let mut writer = Vec::new();
    {
        let mut enc = Encoder::new(&mut writer, w, h, &palette)
            .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))?;
        enc.set_repeat(Repeat::Infinite)
            .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))?;

        let write_current = |enc: &mut Encoder<&mut Vec<u8>>, bitmap: &[u8]| {
            let frame = Frame {
                width: w,
                height: h,
                delay: delay_cs,
                dispose: DisposalMethod::Keep,
                transparent: None,
                needs_user_input: false,
                top: 0,
                left: 0,
                interlaced: false,
                palette: None,
                buffer: Cow::Borrowed(bitmap),
            };
            enc.write_frame(&frame)
                .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))
        };
        write_current(&mut enc, &bitmap)?;

        for matrix_result in session {
            render_frame(&matrix_result?, scale, &mut bitmap);
            write_current(&mut enc, &bitmap)?;
        }
    }
    Ok(writer)
}

/// Incremental GIF writer driven one QR matrix at a time.
///
/// Lets a caller (e.g. a Web Worker) interleave QR generation with GIF
/// writing and report progress between frames; exactly one rendered bitmap
/// is in memory at a time.
pub struct GifBuilder {
    encoder: Option<Encoder<Vec<u8>>>,
    scale: u16,
    width: u16,
    height: u16,
    delay_cs: u16,
    bitmap: Vec<u8>,
    frames_written: usize,
}

impl GifBuilder {
    /// Starts a GIF sized for `first`'s matrix at `scale` px per module.
    /// The first frame is written immediately (it fixes the canvas size).
    pub fn start(first: &QrMatrix, scale: u16, delay_ms: u32) -> Result<Self, PigeonError> {
        let delay_cs = u16::try_from(delay_ms / 10)
            .unwrap_or(1000)
            .max(MIN_DELAY_CENTISECONDS);
        let palette: [u8; 6] = [0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00];
        let mut bitmap = Vec::new();
        let (w, h) = render_frame(first, scale, &mut bitmap);
        if u32::from(w) > crate::protocol::MAX_GIF_DIMENSION
            || u32::from(h) > crate::protocol::MAX_GIF_DIMENSION
        {
            return Err(PigeonError::GifDimensionsTooLarge(
                w,
                h,
                crate::protocol::MAX_GIF_DIMENSION,
            ));
        }
        let mut encoder = Encoder::new(Vec::new(), w, h, &palette)
            .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))?;
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))?;
        write_matrix_frame(&mut encoder, &bitmap, w, h, delay_cs)?;
        Ok(Self {
            encoder: Some(encoder),
            scale,
            width: w,
            height: h,
            delay_cs,
            bitmap,
            frames_written: 1,
        })
    }

    /// Renders and appends the next frame.
    pub fn write_matrix(&mut self, matrix: &QrMatrix) -> Result<(), PigeonError> {
        render_frame(matrix, self.scale, &mut self.bitmap);
        let encoder = self
            .encoder
            .as_mut()
            .ok_or_else(|| PigeonError::GifEncodeFailed("builder already finished".into()))?;
        write_matrix_frame(
            encoder,
            &self.bitmap,
            self.width,
            self.height,
            self.delay_cs,
        )?;
        self.frames_written += 1;
        Ok(())
    }

    /// Number of frames written so far.
    #[must_use]
    pub fn frames_written(&self) -> usize {
        self.frames_written
    }

    /// Completes the file and returns the GIF bytes.
    pub fn finish(mut self) -> Result<Vec<u8>, PigeonError> {
        let encoder = self
            .encoder
            .take()
            .ok_or_else(|| PigeonError::GifEncodeFailed("builder already finished".into()))?;
        encoder
            .into_inner()
            .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))
    }
}

fn write_matrix_frame(
    encoder: &mut Encoder<Vec<u8>>,
    bitmap: &[u8],
    width: u16,
    height: u16,
    delay_cs: u16,
) -> Result<(), PigeonError> {
    let frame = Frame {
        width,
        height,
        delay: delay_cs,
        dispose: DisposalMethod::Keep,
        transparent: None,
        needs_user_input: false,
        top: 0,
        left: 0,
        interlaced: false,
        palette: None,
        buffer: Cow::Borrowed(bitmap),
    };
    encoder
        .write_frame(&frame)
        .map_err(|e| PigeonError::GifEncodeFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn simple_matrix() -> QrMatrix {
        let size = 21;
        let modules: Vec<u8> = (0..size * size).map(|i| (i % 2) as u8).collect();
        QrMatrix { size, modules }
    }

    #[test]
    fn render_includes_quiet_zone() {
        let m = simple_matrix();
        let mut buf = Vec::new();
        let (w, h) = render_frame(&m, 4, &mut buf);
        assert_eq!(w, (21 + 8) * 4);
        assert_eq!(h, w);
        assert_eq!(buf.len(), w as usize * h as usize);
        // corners are quiet-zone white (index 0)
        assert_eq!(buf[0], 0);
        // center region has black pixels
        assert!(buf.contains(&1));
    }

    #[test]
    fn gif_encodes_and_reparses() {
        let matrices = vec![simple_matrix(), simple_matrix()];
        let bytes = encode_gif(&matrices, 4, 250).expect("encodes");
        assert!(bytes.starts_with(b"GIF89a"));

        let mut dec = gif::DecodeOptions::new();
        dec.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = dec.read_info(Cursor::new(&bytes[..])).expect("parses back");
        let (w, _h) = (decoder.width(), decoder.height());
        assert_eq!(w, (21 + 8) * 4);
        let mut frames = 0;
        while let Some(frame) = decoder.read_next_frame().expect("frame") {
            frames += 1;
            assert_eq!(frame.width, w);
            assert_eq!(frame.delay, 25); // 250 ms -> 25 centiseconds
        }
        assert_eq!(frames, 2);
    }

    #[test]
    fn gif_min_delay_enforced() {
        let matrices = vec![simple_matrix()];
        let bytes = encode_gif(&matrices, 4, 5).expect("encodes");
        let mut dec = gif::DecodeOptions::new();
        dec.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = dec.read_info(Cursor::new(&bytes[..])).expect("parses");
        let frame = decoder.read_next_frame().expect("frame").expect("some");
        assert!(frame.delay >= MIN_DELAY_CENTISECONDS);
    }

    #[test]
    fn empty_matrix_list_rejected() {
        assert!(encode_gif(&[], 4, 250).is_err());
    }

    #[test]
    fn gif_builder_matches_one_shot() {
        let matrices = vec![simple_matrix(), simple_matrix(), simple_matrix()];
        let one_shot = encode_gif(&matrices, 4, 250).expect("one-shot");

        let mut builder = GifBuilder::start(&matrices[0], 4, 250).expect("starts");
        for m in &matrices[1..] {
            builder.write_matrix(m).expect("writes");
        }
        assert_eq!(builder.frames_written(), 3);
        let incremental = builder.finish().expect("finishes");
        assert_eq!(one_shot, incremental);
    }

    #[test]
    fn scale_for_target_clamps() {
        assert_eq!(scale_for_target_px(800, 100), 8);
        assert_eq!(scale_for_target_px(4000, 100), 16);
        assert_eq!(scale_for_target_px(50, 100), 2);
    }
}
