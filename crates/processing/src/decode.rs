//! Deterministic common-codec decoding into an owned RGBA8 source.

use std::fmt;
use std::io::Cursor;

use image::{metadata::Orientation, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use image_to_binary_core::{ContractError, Dimensions, ResourceLimits, SourceMetadata};

/// Limits applied before allocating decoded pixels.
///
/// The decoded budget accounts for native decoder bytes, normalized RGBA8,
/// and an orientation working copy when needed. Encoded input bytes are
/// governed separately and are excluded from that working estimate. Decoder
/// allocation limits are best-effort because codec implementations may not
/// support every limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    resource: ResourceLimits,
    max_encoded_bytes: u64,
    max_decoded_bytes: u64,
}

impl DecodeLimits {
    /// Creates limits for encoded input and normalized RGBA output.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidSettings`] when either byte limit is zero.
    pub const fn new(
        resource: ResourceLimits,
        max_encoded_bytes: u64,
        max_decoded_bytes: u64,
    ) -> Result<Self, ContractError> {
        if max_encoded_bytes == 0 {
            return Err(ContractError::InvalidSettings("encoded input limit"));
        }
        if max_decoded_bytes == 0 {
            return Err(ContractError::InvalidSettings("decoded memory limit"));
        }
        Ok(Self {
            resource,
            max_encoded_bytes,
            max_decoded_bytes,
        })
    }

    /// Returns the default 50 MP, 256 MiB encoded and 1 GiB decoded limits.
    #[must_use]
    pub const fn default() -> Self {
        Self {
            resource: ResourceLimits::default(),
            max_encoded_bytes: 256 * 1024 * 1024,
            max_decoded_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// An image decoded once and normalized to owned, orientation-corrected RGBA8.
#[derive(Debug, Eq, PartialEq)]
pub struct DecodedSource {
    metadata: SourceMetadata,
    pixels: Vec<u8>,
}

impl DecodedSource {
    /// Returns normalized source metadata.
    #[must_use]
    pub const fn metadata(&self) -> SourceMetadata {
        self.metadata
    }

    /// Returns normalized RGBA8 pixels in row-major order.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Errors produced while detecting, limiting, or decoding a common image.
#[derive(Debug)]
pub enum DecodeError {
    /// The input exceeds the configured encoded-byte limit.
    EncodedInputTooLarge,
    /// The input has no supported magic format.
    UnsupportedFormat,
    /// A core dimension or allocation contract failed.
    Contract(ContractError),
    /// The image crate rejected malformed or unsupported image data.
    Image(image::ImageError),
    /// The image reader could not inspect its input.
    Io(std::io::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EncodedInputTooLarge => formatter.write_str("encoded input exceeds limit"),
            Self::UnsupportedFormat => formatter.write_str("unsupported image format"),
            Self::Contract(error) => error.fmt(formatter),
            Self::Image(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<ContractError> for DecodeError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

impl From<image::ImageError> for DecodeError {
    fn from(error: image::ImageError) -> Self {
        Self::Image(error)
    }
}

/// Decodes a supported common image and returns an owned normalized RGBA8 source.
///
/// # Errors
///
/// Returns [`DecodeError`] when format detection, dimensions, resource limits,
/// or image decoding fails.
pub fn decode(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedSource, DecodeError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits.max_encoded_bytes {
        return Err(DecodeError::EncodedInputTooLarge);
    }

    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(DecodeError::Io)?;
    let mut header_limits = Limits::default();
    let dimension_ceiling = u32::try_from(limits.resource.max_pixels().min(u64::from(u32::MAX)))
        .map_err(|_| DecodeError::Contract(ContractError::ArithmeticOverflow))?;
    header_limits.max_image_width = Some(dimension_ceiling);
    header_limits.max_image_height = Some(dimension_ceiling);
    header_limits.max_alloc = Some(limits.max_decoded_bytes);
    reader.limits(header_limits);
    let format = reader.format().ok_or(DecodeError::UnsupportedFormat)?;
    if !matches!(
        format,
        ImageFormat::Bmp
            | ImageFormat::Gif
            | ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::Tiff
            | ImageFormat::WebP
    ) {
        return Err(DecodeError::UnsupportedFormat);
    }
    let (width, height) = reader.into_dimensions()?;
    let dimensions = Dimensions::new(width, height)?;
    limits.resource.validate(dimensions)?;
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(DecodeError::Io)?;
    let mut decoder_limits = Limits::default();
    decoder_limits.max_image_width = Some(dimensions.width());
    decoder_limits.max_image_height = Some(dimensions.height());
    decoder_limits.max_alloc = Some(limits.max_decoded_bytes);
    reader.limits(decoder_limits);
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let native_bytes = decoder
        .total_bytes()
        .max(dimensions.pixel_count()?.saturating_mul(4));
    let rgba_bytes = dimensions
        .pixel_count()?
        .checked_mul(4)
        .ok_or(ContractError::ArithmeticOverflow)?;
    let orientation_duplicate = if orientation == Orientation::NoTransforms {
        0
    } else {
        native_bytes
    };
    let working_bytes = native_bytes
        .checked_add(rgba_bytes)
        .and_then(|bytes| bytes.checked_add(orientation_duplicate))
        .ok_or(ContractError::ArithmeticOverflow)?;
    if working_bytes > limits.max_decoded_bytes {
        return Err(DecodeError::Contract(ContractError::ResourceLimitExceeded));
    }
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let rgba = image.into_rgba8();
    let normalized_dimensions = Dimensions::new(rgba.width(), rgba.height())?;
    let metadata = SourceMetadata::new(normalized_dimensions, 4)?;
    Ok(DecodedSource {
        metadata,
        pixels: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::codecs::gif::GifEncoder;
    use image::{Delay, DynamicImage, Frame, ImageBuffer, ImageFormat, Luma, LumaA, Rgba};
    use tiff::encoder::colortype::RGB8;
    use tiff::encoder::TiffEncoder as RawTiffEncoder;

    use super::*;

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([10, 20, 30, 40])
            } else {
                Rgba([200, 210, 220, 230])
            }
        }));
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, format).unwrap();
        output.into_inner()
    }

    fn exif_oriented_jpeg() -> Vec<u8> {
        let mut jpeg = encoded(ImageFormat::Jpeg);
        let exif = [
            b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1,
            0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let segment_length = u8::try_from(exif.len() + 2).unwrap();
        let mut segment = vec![0xff, 0xe1, 0, segment_length];
        segment.extend_from_slice(&exif);
        jpeg.splice(2..2, segment);
        jpeg
    }

    fn animated_gif() -> Vec<u8> {
        let first = ImageBuffer::from_pixel(2, 1, Rgba([255, 0, 0, 255]));
        let second = ImageBuffer::from_pixel(2, 1, Rgba([0, 0, 255, 255]));
        let mut output = Cursor::new(Vec::new());
        GifEncoder::new(&mut output)
            .encode_frames([
                Frame::from_parts(first, 0, 0, Delay::from_numer_denom_ms(1, 1)),
                Frame::from_parts(second, 0, 0, Delay::from_numer_denom_ms(1, 1)),
            ])
            .unwrap();
        output.into_inner()
    }

    fn multipage_tiff() -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        let mut encoder = RawTiffEncoder::new(&mut output).unwrap();
        encoder
            .write_image::<RGB8>(2, 1, &[255, 0, 0, 255, 0, 0])
            .unwrap();
        encoder
            .write_image::<RGB8>(2, 1, &[0, 0, 255, 0, 0, 255])
            .unwrap();
        output.into_inner()
    }

    #[test]
    fn common_codecs_decode_to_deterministic_rgba() {
        for format in [
            ImageFormat::Bmp,
            ImageFormat::Gif,
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::Tiff,
            ImageFormat::WebP,
        ] {
            let result = decode(&encoded(format), DecodeLimits::default()).unwrap();
            assert_eq!(
                result.metadata().dimensions(),
                Dimensions::new(2, 1).unwrap()
            );
            assert_eq!(result.pixels().len(), 8);
        }
    }

    #[test]
    fn malformed_and_oversized_inputs_fail_before_decode() {
        assert!(matches!(
            decode(b"not-an-image", DecodeLimits::default()),
            Err(DecodeError::UnsupportedFormat | DecodeError::Image(_))
        ));
        let limits = DecodeLimits::new(ResourceLimits::default(), 2, 1024 * 1024 * 1024).unwrap();
        assert!(matches!(
            decode(&[0; 3], limits),
            Err(DecodeError::EncodedInputTooLarge)
        ));
    }

    #[test]
    fn pixel_limit_rejects_header_before_allocation() {
        let limits = DecodeLimits::new(
            ResourceLimits::new(1, 256 * 1024 * 1024).unwrap(),
            256 * 1024 * 1024,
            1024 * 1024 * 1024,
        )
        .unwrap();
        assert!(matches!(
            decode(&encoded(ImageFormat::Png), limits),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded) | DecodeError::Image(_))
        ));
    }

    #[test]
    fn exif_orientation_swaps_normalized_dimensions() {
        let result = decode(&exif_oriented_jpeg(), DecodeLimits::default()).unwrap();
        assert_eq!(
            result.metadata().dimensions(),
            Dimensions::new(1, 2).unwrap()
        );
    }

    #[test]
    fn gif_and_tiff_use_first_image_only() {
        let gif = decode(&animated_gif(), DecodeLimits::default()).unwrap();
        assert_eq!(&gif.pixels()[..4], &[255, 0, 0, 255]);
        let tiff = decode(&multipage_tiff(), DecodeLimits::default()).unwrap();
        assert_eq!(&tiff.pixels()[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn alpha_grayscale_and_sixteen_bit_inputs_normalize_to_rgba8() {
        let alpha = DynamicImage::ImageLumaA8(ImageBuffer::from_pixel(1, 1, LumaA([12, 34])));
        let gray = DynamicImage::ImageLuma8(ImageBuffer::from_pixel(1, 1, Luma([56])));
        let wide = DynamicImage::ImageLuma16(ImageBuffer::from_pixel(1, 1, Luma([0x1234])));
        for (image, expected) in [
            (alpha, [12, 12, 12, 34]),
            (gray, [56, 56, 56, 255]),
            (wide, [0x12, 0x12, 0x12, 255]),
        ] {
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, ImageFormat::Png).unwrap();
            let decoded = decode(bytes.get_ref(), DecodeLimits::default()).unwrap();
            assert_eq!(decoded.pixels(), expected);
        }
    }

    #[test]
    fn recognized_malformed_payload_and_working_budget_fail() {
        assert!(matches!(
            decode(&[137, 80, 78, 71, 13, 10, 26, 10], DecodeLimits::default()),
            Err(DecodeError::Image(_))
        ));
        let limits = DecodeLimits::new(ResourceLimits::default(), 256 * 1024 * 1024, 4).unwrap();
        assert!(matches!(
            decode(&encoded(ImageFormat::Png), limits),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded) | DecodeError::Image(_))
        ));
    }
}
