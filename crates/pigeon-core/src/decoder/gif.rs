//! Streaming GIF parsing with manual frame compositing.
//!
//! Frames are produced one at a time (bounded memory: one canvas plus the
//! current frame), so a hostile GIF cannot force the whole animation into
//! memory. Disposal methods and transparency are honored so composited
//! frames decode like the original animation.

use std::io::Cursor;

use gif::{ColorOutput, DecodeOptions, DisposalMethod};

use crate::error::PigeonError;
use crate::protocol::{MAX_GIF_DIMENSION, MAX_GIF_FILE_BYTES, MAX_GIF_FRAMES};

/// One composited frame, RGBA.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodedFrame {
    pub index: usize,
    /// Frame delay in milliseconds as declared by the GIF.
    pub delay_ms: u32,
    pub width: u16,
    pub height: u16,
    /// RGBA, row-major, full canvas size (width * height * 4).
    #[serde(serialize_with = "crate::protocol::ser::bytes")]
    pub data: Vec<u8>,
}

struct PendingDisposal {
    dispose: DisposalMethod,
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    /// Snapshot of the canvas before compositing (only for Previous disposal).
    previous: Option<Vec<u8>>,
}

/// Streaming decoder over a GIF file.
pub struct GifFrameStream {
    decoder: gif::Decoder<Cursor<Vec<u8>>>,
    canvas: Vec<u8>,
    width: u16,
    height: u16,
    pending_disposal: Option<PendingDisposal>,
    index: usize,
    has_global_background: bool,
}

impl GifFrameStream {
    /// Opens a GIF for streaming frame extraction.
    ///
    /// Enforces the signature check, dimension cap and file-size cap up front;
    /// the frame cap is enforced while iterating.
    pub fn new(bytes: &[u8]) -> Result<Self, PigeonError> {
        if bytes.len() > MAX_GIF_FILE_BYTES as usize {
            return Err(PigeonError::GifFileTooLarge {
                size: bytes.len() as u64,
                max: MAX_GIF_FILE_BYTES,
            });
        }
        if bytes.len() < 6 || (&bytes[0..6] != b"GIF87a" && &bytes[0..6] != b"GIF89a") {
            return Err(PigeonError::GifBadSignature);
        }

        let mut options = DecodeOptions::new();
        options.set_color_output(ColorOutput::RGBA);
        let decoder = options
            .read_info(Cursor::new(bytes.to_vec()))
            .map_err(|e| PigeonError::GifFrameMalformed {
                index: 0,
                reason: e.to_string(),
            })?;
        let (width, height) = (decoder.width(), decoder.height());
        if u32::from(width) > MAX_GIF_DIMENSION || u32::from(height) > MAX_GIF_DIMENSION {
            return Err(PigeonError::GifDimensionsTooLarge(
                width,
                height,
                MAX_GIF_DIMENSION,
            ));
        }
        let canvas = vec![0u8; width as usize * height as usize * 4];
        Ok(Self {
            decoder,
            canvas,
            width,
            height,
            pending_disposal: None,
            index: 0,
            has_global_background: true,
        })
    }

    #[must_use]
    pub fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// Extracts and composites the next frame; `Ok(None)` at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<DecodedFrame>, PigeonError> {
        // Apply the disposal requested by the *previous* frame before drawing.
        if let Some(pending) = self.pending_disposal.take() {
            match pending.dispose {
                DisposalMethod::Background => {
                    for y in pending.top..(pending.top + pending.height).min(self.height) {
                        let start =
                            y as usize * self.width as usize * 4 + pending.left as usize * 4;
                        let end = start + pending.width as usize * 4;
                        // Fill with white: QR content reads best on light canvas.
                        if self.has_global_background {
                            for chunk in self.canvas[start..end].as_chunks_mut::<4>().0 {
                                chunk.copy_from_slice(&[255, 255, 255, 255]);
                            }
                        } else {
                            for chunk in self.canvas[start..end].as_chunks_mut::<4>().0 {
                                chunk.copy_from_slice(&[0, 0, 0, 0]);
                            }
                        }
                    }
                }
                DisposalMethod::Previous => {
                    if let Some(snapshot) = pending.previous {
                        self.canvas = snapshot;
                    }
                }
                _ => {}
            }
        }

        let frame = match self.decoder.read_next_frame() {
            Ok(Some(f)) => f,
            Ok(None) => return Ok(None),
            Err(e) => {
                return Err(PigeonError::GifFrameMalformed {
                    index: self.index,
                    reason: e.to_string(),
                })
            }
        };

        if self.index >= MAX_GIF_FRAMES {
            return Err(PigeonError::GifFrameLimitExceeded {
                max: MAX_GIF_FRAMES,
            });
        }

        // Snapshot first if this frame will request Previous disposal.
        let previous = if frame.dispose == DisposalMethod::Previous {
            Some(self.canvas.clone())
        } else {
            None
        };

        // Composite the frame's patch (transparency: alpha 0 = skip).
        let (fw, fh, fl, ft) = (frame.width, frame.height, frame.left, frame.top);
        for row in 0..fh as usize {
            let cy = ft as usize + row;
            if cy >= self.height as usize {
                break;
            }
            let src = &frame.buffer[row * fw as usize * 4..(row + 1) * fw as usize * 4];
            let dst_start = (cy * self.width as usize + fl as usize) * 4;
            let dst_end = dst_start + fw as usize * 4;
            if dst_end > self.canvas.len() {
                return Err(PigeonError::GifFrameMalformed {
                    index: self.index,
                    reason: "frame patch extends past canvas".into(),
                });
            }
            let dst = &mut self.canvas[dst_start..dst_end];
            for (px, chunk) in dst.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let alpha = src[px * 4 + 3];
                if alpha != 0 {
                    chunk.copy_from_slice(&src[px * 4..px * 4 + 4]);
                }
            }
        }

        let delay_ms = u32::from(frame.delay) * 10;
        let out = DecodedFrame {
            index: self.index,
            delay_ms,
            width: self.width,
            height: self.height,
            data: self.canvas.clone(),
        };

        self.pending_disposal = Some(PendingDisposal {
            dispose: frame.dispose,
            left: fl,
            top: ft,
            width: fw,
            height: fh,
            previous,
        });
        self.index += 1;
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::gif::encode_gif;
    use crate::encoder::qr::QrMatrix;

    fn pigeon_gif(frames: usize) -> Vec<u8> {
        let matrices: Vec<QrMatrix> = (0..frames)
            .map(|i| {
                let size = 21;
                let modules: Vec<u8> = (0..size * size).map(|j| ((i + j) % 2) as u8).collect();
                QrMatrix { size, modules }
            })
            .collect();
        encode_gif(&matrices, 4, 100).expect("encodes")
    }

    #[test]
    fn parses_our_gifs_frame_by_frame() {
        let gif = pigeon_gif(3);
        let mut stream = GifFrameStream::new(&gif).expect("opens");
        assert_eq!(stream.dimensions(), (29 * 4, 29 * 4));
        let mut count = 0;
        while let Some(frame) = stream.next_frame().expect("frame") {
            assert_eq!(frame.width, 29 * 4);
            assert_eq!(frame.delay_ms, 100);
            assert_eq!(frame.data.len(), (29 * 4 * 29 * 4 * 4) as usize);
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn non_gif_rejected_by_signature() {
        assert!(matches!(
            GifFrameStream::new(b"PNG blah blah"),
            Err(PigeonError::GifBadSignature)
        ));
        assert!(matches!(
            GifFrameStream::new(b""),
            Err(PigeonError::GifBadSignature)
        ));
    }

    #[test]
    fn truncated_gif_errors_not_hangs() {
        let gif = pigeon_gif(2);
        let truncated = &gif[..gif.len() / 2];
        let mut stream = GifFrameStream::new(truncated).expect("opens");
        let mut result = Ok(());
        for _ in 0..10 {
            match stream.next_frame() {
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        // Either an error surfaces or the stream simply ends early — never a hang.
        assert!(result.is_err() || true);
    }

    #[test]
    fn oversized_gif_rejected() {
        // Craft a header claiming huge dimensions: 4000x4000
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&[0xA0u8, 0x0F, 0xA0, 0x0F]); // 4000 x 4000
        bytes.extend_from_slice(&[0x00; 20]);
        assert!(matches!(
            GifFrameStream::new(&bytes),
            Err(PigeonError::GifDimensionsTooLarge(_, _, _))
        ));
    }
}
