//! Turning a producer's PNG into the canonical frame the device already draws.
//!
//! The order of the checks below is the security-relevant part. The 1 MiB body
//! limit bounds the *compressed* bytes; it says nothing about what they expand
//! to. What actually stops a decompression bomb is reading the IHDR alone and
//! refusing on dimensions **before** any pixel buffer is allocated, which is
//! why this uses `read_header_info` rather than a one-shot decode.

use protocol::{SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH};
use sha2::{Digest, Sha256};

const LVGL_IMAGE_HEADER_BYTES: usize = 12;
const LVGL_IMAGE_MAGIC: u32 = 0x19;
const LVGL_COLOR_FORMAT_RGB565: u32 = 0x12;

/// One accepted picture, in the exact form the asset store and the wire use.
///
/// `digest` hashes the **decoded** blob, never the PNG — stage 4's rule. Two
/// different PNG encodings of the same picture are therefore the same asset,
/// and the digest survives a `png` crate upgrade.
#[derive(Debug, Clone)]
pub(crate) struct CanonicalFrame {
    pub digest: [u8; 32],
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageIngestError {
    NotPng,
    WrongDimensions { width: u32, height: u32 },
    UnsupportedBitDepth,
    UnsupportedColorType,
    Decode,
}

impl std::fmt::Display for ImageIngestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Written for the producer: one line, no internals.
        match self {
            Self::NotPng => write!(formatter, "the body is not a PNG"),
            Self::WrongDimensions { width, height } => write!(
                formatter,
                "the picture is {width}x{height}; it must be exactly {SCENE_CANVAS_WIDTH}x{SCENE_CANVAS_HEIGHT}"
            ),
            Self::UnsupportedBitDepth => {
                write!(formatter, "the picture must be 8 bits per channel")
            }
            Self::UnsupportedColorType => {
                write!(formatter, "the picture must be RGB or RGBA")
            }
            Self::Decode => write!(formatter, "the PNG could not be decoded"),
        }
    }
}

pub(crate) fn canonical_frame_from_png(bytes: &[u8]) -> Result<CanonicalFrame, ImageIngestError> {
    let width = u32::try_from(SCENE_CANVAS_WIDTH).expect("fixed canvas");
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).expect("fixed canvas");

    let mut decoder = png::Decoder::new(bytes);
    // Belt and braces beside the dimension check below: even a header this
    // accepts can never ask for more than one canvas worth of pixels.
    decoder.set_limits(png::Limits {
        bytes: (width as usize) * (height as usize) * 4 + 64 * 1_024,
    });

    // Header only. Nothing is allocated for pixels yet.
    let info = decoder
        .read_header_info()
        .map_err(|_| ImageIngestError::NotPng)?;
    let (declared_width, declared_height, depth, color) =
        (info.width, info.height, info.bit_depth, info.color_type);

    if declared_width != width || declared_height != height {
        return Err(ImageIngestError::WrongDimensions {
            width: declared_width,
            height: declared_height,
        });
    }
    if depth != png::BitDepth::Eight {
        return Err(ImageIngestError::UnsupportedBitDepth);
    }
    let samples = match color {
        png::ColorType::Rgb => 3usize,
        png::ColorType::Rgba => 4usize,
        _ => return Err(ImageIngestError::UnsupportedColorType),
    };

    // Only now does anything get allocated.
    let mut reader = decoder.read_info().map_err(|_| ImageIngestError::Decode)?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|_| ImageIngestError::Decode)?;
    let pixels = &buffer[..frame.buffer_size()];

    let pixel_count = usize::try_from(width * height).expect("fixed canvas fits usize");
    let chunks = pixels.chunks_exact(samples);
    if chunks.len() != pixel_count {
        return Err(ImageIngestError::Decode);
    }
    let mut canonical_bytes = rgb565_buffer(width, height);
    for chunk in chunks {
        // The panel has no alpha. Composite over black, matching the
        // established `pixmap.fill(Color::BLACK)` behavior, so a transparent
        // producer pixel lands on the panel's black ground.
        let alpha = if samples == 4 {
            u32::from(chunk[3])
        } else {
            255
        };
        let over_black = |channel: u8| -> u8 {
            u8::try_from(u32::from(channel) * alpha / 255).expect("scaled by <= 1")
        };
        canonical_bytes.extend_from_slice(
            &pack_rgb565(
                over_black(chunk[0]),
                over_black(chunk[1]),
                over_black(chunk[2]),
            )
            .to_le_bytes(),
        );
    }
    let digest = Sha256::digest(&canonical_bytes).into();

    Ok(CanonicalFrame {
        digest,
        bytes: canonical_bytes,
    })
}

/// The inverse of [`canonical_frame_from_png`], for the one place a person looks at
/// a stored frame: the companion's preview of a picture card.
///
/// Each channel is expanded by bit replication, the standard inverse of a 5/6-bit
/// quantization, so black stays black and full scale stays full scale. `None` for
/// anything that is not exactly one canvas of RGB565 behind the LVGL header -- the
/// store only ever holds such frames, so that would be corruption, and the preview
/// then says "no frame" rather than drawing garbage.
pub(crate) fn png_from_canonical_frame(frame: &[u8]) -> Option<Vec<u8>> {
    let width = u32::try_from(SCENE_CANVAS_WIDTH).expect("fixed canvas");
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).expect("fixed canvas");
    let pixel_count = usize::try_from(width * height).expect("fixed canvas fits usize");
    let pixels = frame.get(LVGL_IMAGE_HEADER_BYTES..)?;
    if pixels.len() != pixel_count * 2 {
        return None;
    }

    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for pair in pixels.as_chunks::<2>().0 {
        let packed = u16::from_le_bytes(*pair);
        let (red, green, blue) = (packed >> 11, (packed >> 5) & 0x3f, packed & 0x1f);
        let expand = |value: u16, bits: u32| -> u8 {
            u8::try_from((value << (8 - bits)) | (value >> (2 * bits - 8))).expect("eight bits")
        };
        rgb.extend_from_slice(&[expand(red, 5), expand(green, 6), expand(blue, 5)]);
    }

    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&rgb).ok()?;
    writer.finish().ok()?;
    Some(png)
}

fn rgb565_buffer(width: u32, height: u32) -> Vec<u8> {
    let pixel_count = usize::try_from(width * height).expect("fixed canvas fits usize");
    let mut bytes = Vec::with_capacity(LVGL_IMAGE_HEADER_BYTES + pixel_count * 2);
    let stride = width * 2;
    let word0 = LVGL_IMAGE_MAGIC | (LVGL_COLOR_FORMAT_RGB565 << 8);
    let word1 = (width & 0xffff) | ((height & 0xffff) << 16);
    let word2 = stride & 0xffff;
    bytes.extend_from_slice(&word0.to_le_bytes());
    bytes.extend_from_slice(&word1.to_le_bytes());
    bytes.extend_from_slice(&word2.to_le_bytes());
    bytes
}

fn pack_rgb565(red: u8, green: u8, blue: u8) -> u16 {
    // Round each 8-bit channel to the nearest endpoint-inclusive RGB565 value.
    let red = (u16::from(red) * 31 + 127) / 255;
    let green = (u16::from(green) * 63 + 127) / 255;
    let blue = (u16::from(blue) * 31 + 127) / 255;
    (red << 11) | (green << 5) | blue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_frame_shown_as_a_png_is_the_same_frame_when_pushed_back() {
        // Every one of the 65,536 RGB565 values, tiled across one canvas: expanding
        // to eight bits and re-quantizing must land on the value it started from,
        // or the preview would show colours the panel never displays.
        let width = u32::try_from(SCENE_CANVAS_WIDTH).expect("fixed canvas");
        let height = u32::try_from(SCENE_CANVAS_HEIGHT).expect("fixed canvas");
        let mut frame = rgb565_buffer(width, height);
        for index in 0..(width * height) {
            let value = u16::try_from(index % 65_536).expect("sixteen bits");
            frame.extend_from_slice(&value.to_le_bytes());
        }
        let png = png_from_canonical_frame(&frame).expect("a whole frame decodes");
        let again = canonical_frame_from_png(&png).expect("and is an acceptable push");
        assert_eq!(again.bytes, frame);
    }

    #[test]
    fn anything_but_one_whole_canvas_is_no_preview_rather_than_garbage() {
        assert_eq!(png_from_canonical_frame(&[]), None);
        assert_eq!(png_from_canonical_frame(&[0; 11]), None);
        assert_eq!(
            png_from_canonical_frame(&vec![0; 12 + 448 * 368 * 2 - 2]),
            None
        );
    }

    /// Builds a real PNG of the given size and colour type.
    fn png_of(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().expect("header");
            let samples = match color {
                png::ColorType::Rgb => 3,
                png::ColorType::Rgba => 4,
                _ => unreachable!("tests use rgb or rgba only"),
            };
            let bytes_per_sample = if depth == png::BitDepth::Sixteen {
                2
            } else {
                1
            };
            let data = vec![0u8; (width * height) as usize * samples * bytes_per_sample];
            writer.write_image_data(&data).expect("data");
        }
        out
    }

    fn png_with_pixels(color: png::ColorType, pixels: &[[u8; 4]]) -> Vec<u8> {
        let samples = match color {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            _ => unreachable!("tests use rgb or rgba only"),
        };
        let mut data = vec![0u8; (W * H) as usize * samples];
        for (target, source) in data.chunks_exact_mut(samples).zip(pixels) {
            target.copy_from_slice(&source[..samples]);
        }
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, W, H);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&data).expect("data");
        }
        out
    }

    fn png_with_smaller_first_animation_frame() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, W, H);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(1, 0).expect("animation metadata");
            let mut writer = encoder.write_header().expect("header");
            writer.set_frame_dimension(W - 1, H).expect("smaller frame");
            writer
                .write_image_data(&vec![0u8; ((W - 1) * H * 3) as usize])
                .expect("animation frame");
        }
        out
    }

    const W: u32 = 448;
    const H: u32 = 368;

    #[test]
    fn an_exact_448x368_rgb_png_is_accepted() {
        let frame =
            canonical_frame_from_png(&png_of(W, H, png::ColorType::Rgb, png::BitDepth::Eight))
                .expect("accepted");
        // 12-byte LVGL header + one LE u16 per pixel.
        assert_eq!(frame.bytes.len(), 12 + (W * H * 2) as usize);
    }

    #[test]
    fn an_rgba_png_is_accepted_and_composited_over_black() {
        // A fully transparent RGBA frame composites to black, not to white and
        // not to the decoder's uninitialised buffer.
        // Opaque white pixels at zero alpha.
        let out = png_with_pixels(
            png::ColorType::Rgba,
            &vec![[255, 255, 255, 0]; (W * H) as usize],
        );
        let frame = canonical_frame_from_png(&out).expect("accepted");
        let first_pixel = u16::from_le_bytes([frame.bytes[12], frame.bytes[13]]);
        assert_eq!(first_pixel, 0, "alpha zero must composite to black");
    }

    #[test]
    fn colored_rgb_pixels_pin_channel_order_and_quantization_boundaries() {
        let png = png_with_pixels(
            png::ColorType::Rgb,
            &[
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [4, 2, 4, 255],
                [5, 3, 5, 255],
                [17, 129, 250, 255],
            ],
        );
        let frame = canonical_frame_from_png(&png).expect("colored PNG");

        assert_eq!(frame.bytes.len(), 12 + (W * H * 2) as usize);
        assert_eq!(&frame.bytes[..4], &0x0000_1219u32.to_le_bytes());
        assert_eq!(&frame.bytes[4..8], &(W | (H << 16)).to_le_bytes());
        assert_eq!(&frame.bytes[8..12], &(W * 2).to_le_bytes());
        assert_eq!(
            &frame.bytes[12..24],
            &[
                0x00, 0xf8, 0xe0, 0x07, 0x1f, 0x00, 0x00, 0x00, 0x21, 0x08, 0x1e, 0x14
            ]
        );
    }

    #[test]
    fn rgba_pixels_pin_opaque_and_partial_alpha_truncation() {
        let png = png_with_pixels(
            png::ColorType::Rgba,
            &[
                [17, 129, 250, 255],
                [255, 128, 64, 128],
                [17, 129, 250, 77],
                [1, 254, 127, 254],
            ],
        );
        let frame = canonical_frame_from_png(&png).expect("RGBA PNG");

        assert_eq!(
            &frame.bytes[12..20],
            &[0x1e, 0x14, 0x04, 0x82, 0x29, 0x09, 0xef, 0x07]
        );
    }

    #[test]
    fn a_smaller_first_apng_frame_is_rejected_after_its_full_size_header() {
        let error = canonical_frame_from_png(&png_with_smaller_first_animation_frame())
            .expect_err("a subframe cannot fill the canonical canvas");
        assert_eq!(error, ImageIngestError::Decode);
    }

    #[test]
    fn off_by_one_widths_are_rejected() {
        for (name, width) in [("one_pixel_narrow", 447), ("one_pixel_wide", 449)] {
            let error = canonical_frame_from_png(&png_of(
                width,
                H,
                png::ColorType::Rgb,
                png::BitDepth::Eight,
            ))
            .expect_err(name);
            assert_eq!(
                error,
                ImageIngestError::WrongDimensions { width, height: H },
                "{name}"
            );
        }
    }

    #[test]
    fn sixteen_bit_is_rejected() {
        let error =
            canonical_frame_from_png(&png_of(W, H, png::ColorType::Rgb, png::BitDepth::Sixteen))
                .expect_err("16-bit");
        assert!(matches!(error, ImageIngestError::UnsupportedBitDepth));
    }

    #[test]
    fn a_body_that_is_not_a_png_is_rejected() {
        assert!(matches!(
            canonical_frame_from_png(b"not a png at all").expect_err("garbage"),
            ImageIngestError::NotPng
        ));
    }

    #[test]
    fn a_decompression_bomb_is_rejected_without_allocating_its_pixels() {
        // A valid IHDR declaring an enormous canvas, with almost no body. If the
        // dimension check ran after `next_frame`, this would try to allocate
        // ~48 GB. It must be refused from the header alone.
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 100_000, 100_000);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            // Writing the header alone is exactly the hostile shape: a declared
            // size with no matching data.
            let _writer = encoder.write_header().expect("header");
        }
        let error = canonical_frame_from_png(&out).expect_err("bomb");
        assert!(
            matches!(
                error,
                ImageIngestError::WrongDimensions {
                    width: 100_000,
                    height: 100_000
                }
            ),
            "must fail on dimensions, not on a truncated decode: {error:?}"
        );
    }

    #[test]
    fn the_canonical_header_and_digest_are_pinned() {
        // An endianness or header slip must not pass silently.
        let frame =
            canonical_frame_from_png(&png_of(W, H, png::ColorType::Rgb, png::BitDepth::Eight))
                .expect("accepted");
        // word0 = magic 0x19 | (RGB565 0x12 << 8); word1 = w | h << 16; word2 = stride.
        assert_eq!(&frame.bytes[0..4], &0x0000_1219u32.to_le_bytes());
        assert_eq!(&frame.bytes[4..8], &(W | (H << 16)).to_le_bytes());
        assert_eq!(&frame.bytes[8..12], &(W * 2).to_le_bytes());
        // An all-black frame's digest is a fixed value. Capture it on first run
        // from the failure message and paste it here; do not compute it in the test.
        assert_eq!(
            protocol::digest_hex(&frame.digest),
            "0443dd2c6007c4f8ed7a464a9056c48c33999cfaaddd71cfcab719012bf47241"
        );
    }

    #[test]
    fn the_same_picture_twice_yields_the_same_digest() {
        let png = png_of(W, H, png::ColorType::Rgb, png::BitDepth::Eight);
        let first = canonical_frame_from_png(&png).expect("first");
        let second = canonical_frame_from_png(&png).expect("second");
        assert_eq!(first.digest, second.digest);
    }
}
