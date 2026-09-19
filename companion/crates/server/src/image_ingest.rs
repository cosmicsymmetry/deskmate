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

/// Shared with [`crate::face_render`], which rasterizes a server-authored face.
/// Both paths use the same canonical header and RGB565 quantization.
pub(crate) fn encode_rgb565(width: u32, height: u32, pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
    let mut bytes = rgb565_buffer(width, height);
    for pixel in pixmap.pixels() {
        bytes.extend_from_slice(
            &pack_rgb565(pixel.red(), pixel.green(), pixel.blue()).to_le_bytes(),
        );
    }
    bytes
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
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, W, H);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            // Opaque white pixels at zero alpha.
            let data: Vec<u8> = std::iter::repeat_n([255u8, 255, 255, 0], (W * H) as usize)
                .flatten()
                .collect();
            writer.write_image_data(&data).expect("data");
        }
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
    fn one_pixel_narrow_is_rejected() {
        let error =
            canonical_frame_from_png(&png_of(447, H, png::ColorType::Rgb, png::BitDepth::Eight))
                .expect_err("447 wide");
        assert!(matches!(
            error,
            ImageIngestError::WrongDimensions {
                width: 447,
                height: 368
            }
        ));
    }

    #[test]
    fn one_pixel_wide_is_rejected() {
        let error =
            canonical_frame_from_png(&png_of(449, H, png::ColorType::Rgb, png::BitDepth::Eight))
                .expect_err("449 wide");
        assert!(matches!(
            error,
            ImageIngestError::WrongDimensions { width: 449, .. }
        ));
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
