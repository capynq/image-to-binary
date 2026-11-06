//! Deterministic common-codec decoding into an owned RGBA8 source.

#[cfg(feature = "heif")]
mod heif;

use std::fmt;
use std::io::Cursor;

use image::{metadata::Orientation, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use image_to_binary_core::{ContractError, Dimensions, ResourceLimits, SourceMetadata};

/// ISO-BMFF image families decoded through the optional native codec path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFormat {
    /// HEIF or HEIC still images, decoded from the primary item.
    Heif,
    /// AVIF still images, decoded from the primary item.
    Avif,
}

impl fmt::Display for NativeFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Heif => "HEIF/HEIC",
            Self::Avif => "AVIF",
        })
    }
}

const AVIF_BRANDS: [&[u8]; 2] = [b"avif", b"avis"];
const HEIF_BRANDS: [&[u8]; 10] = [
    b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs", b"mif1", b"msf1",
];

/// Returns the `ftyp` payload when `bytes` begins with a well-formed box.
///
/// Rejects non-`ftyp` boxes, sizes shorter than their header, and declared
/// lengths beyond the available bytes, so a truncated file is never classified.
fn ftyp_payload(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.get(4..8)? != b"ftyp" {
        return None;
    }
    let declared = u32::from_be_bytes(bytes.get(0..4)?.try_into().ok()?);
    let (header, length) = match declared {
        0 => return None,
        1 => {
            let large = u64::from_be_bytes(bytes.get(8..16)?.try_into().ok()?);
            (16, usize::try_from(large).ok()?)
        }
        size => (8, usize::try_from(size).ok()?),
    };
    if length < header || length > bytes.len() {
        return None;
    }
    bytes.get(header..length)
}

/// Upper bound on compatible brands examined per `ftyp`; real files list a handful.
const MAX_FTYP_COMPATIBLE_BRANDS: usize = 64;

/// Classifies an ISO-BMFF file by its `ftyp` major and compatible brands.
///
/// AVIF brands are checked before HEIF brands because AVIF files commonly
/// also list `mif1`. At most `MAX_FTYP_COMPATIBLE_BRANDS` compatible brands
/// are examined, so a large declared box cannot widen the scan. Returns
/// `None` for anything that is not a recognized `ftyp` box, including
/// zero-size, truncated, or malformed headers, so the caller can fall through
/// to the common decoders.
fn native_format(bytes: &[u8]) -> Option<NativeFormat> {
    let payload = ftyp_payload(bytes)?;
    let major = payload.get(0..4)?;
    let compatible = payload.get(8..)?;
    let brands = || {
        std::iter::once(major).chain(compatible.chunks_exact(4).take(MAX_FTYP_COMPATIBLE_BRANDS))
    };
    if brands().any(|brand| AVIF_BRANDS.contains(&brand)) {
        Some(NativeFormat::Avif)
    } else if brands().any(|brand| HEIF_BRANDS.contains(&brand)) {
        Some(NativeFormat::Heif)
    } else {
        None
    }
}

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

    /// Returns the exact normalized RGBA8 byte length for `dimensions`.
    ///
    /// The pixel ceiling and decoded-byte budget are checked before the caller
    /// allocates or copies pixels.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] above either limit, or
    /// an arithmetic error when the length is not representable.
    pub fn rgba_len(self, dimensions: Dimensions) -> Result<usize, ContractError> {
        self.resource.validate(dimensions)?;
        let bytes = dimensions
            .pixel_count()?
            .checked_mul(4)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if bytes > self.max_decoded_bytes {
            return Err(ContractError::ResourceLimitExceeded);
        }
        usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)
    }
}

/// An image decoded once and normalized to owned, orientation-corrected RGBA8.
#[derive(Debug, Eq, PartialEq)]
pub struct DecodedSource {
    metadata: SourceMetadata,
    pixels: Vec<u8>,
}

impl DecodedSource {
    /// Wraps already orientation-normalized, row-major RGBA8 pixels.
    ///
    /// Takes ownership of `pixels` without copying.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] above the limits, or
    /// [`ContractError::InvalidDimensions`] when `pixels.len()` is not exactly
    /// four bytes per pixel.
    pub fn from_rgba(
        dimensions: Dimensions,
        pixels: Vec<u8>,
        limits: DecodeLimits,
    ) -> Result<Self, ContractError> {
        if limits.rgba_len(dimensions)? != pixels.len() {
            return Err(ContractError::InvalidDimensions);
        }
        Ok(Self {
            metadata: SourceMetadata::new(dimensions, 4)?,
            pixels,
        })
    }

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
    /// The input is a HEIF/AVIF file but this build cannot decode it: the
    /// `heif` feature is off, or the linked libheif has no matching decoder.
    /// Callers may route such input to a lazily loaded decoder.
    CodecUnavailable(NativeFormat),
    /// A core dimension or allocation contract failed.
    Contract(ContractError),
    /// The decoder reported that a configured dimension or allocation limit was exceeded.
    ResourceLimit(String),
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
            Self::CodecUnavailable(format) => write!(
                formatter,
                "{format} decoding is not available in this build; use a PNG, JPEG, WebP, BMP, TIFF, or GIF source"
            ),
            Self::Contract(error) => error.fmt(formatter),
            Self::ResourceLimit(diagnostic) => formatter.write_str(diagnostic),
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
        match error {
            image::ImageError::Limits(limit) => Self::ResourceLimit(limit.to_string()),
            other => Self::Image(other),
        }
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
    if let Some(format) = native_format(bytes) {
        return decode_native(bytes, format, limits);
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

#[cfg(not(feature = "heif"))]
fn decode_native(
    _bytes: &[u8],
    format: NativeFormat,
    _limits: DecodeLimits,
) -> Result<DecodedSource, DecodeError> {
    Err(DecodeError::CodecUnavailable(format))
}

#[cfg(feature = "heif")]
use heif::decode_native;

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::codecs::gif::GifEncoder;
    use image::error::{
        ImageFormatHint, LimitError, LimitErrorKind, ParameterError, ParameterErrorKind,
        UnsupportedError, UnsupportedErrorKind,
    };
    use image::{
        Delay, DynamicImage, Frame, ImageBuffer, ImageError, ImageFormat, Luma, LumaA, Rgba,
    };
    use tiff::encoder::colortype::RGB8;
    use tiff::encoder::TiffEncoder as RawTiffEncoder;

    use super::*;

    const HEIC: &[u8] = include_bytes!("../tests/fixtures/heif/sample.heic");
    const AVIF: &[u8] = include_bytes!("../tests/fixtures/heif/sample.avif");

    fn ftyp(size: u32, major: [u8; 4], compatible: &[[u8; 4]]) -> Vec<u8> {
        let mut bytes = size.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(&major);
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        for brand in compatible {
            bytes.extend_from_slice(brand);
        }
        bytes
    }

    fn sized_ftyp(major: [u8; 4], compatible: &[[u8; 4]]) -> Vec<u8> {
        let size = u32::try_from(16 + 4 * compatible.len()).unwrap();
        ftyp(size, major, compatible)
    }

    #[test]
    fn recognizes_avif_and_heif_brands() {
        let cases: [(Vec<u8>, Option<NativeFormat>); 14] = [
            (sized_ftyp(*b"avif", &[]), Some(NativeFormat::Avif)),
            (sized_ftyp(*b"avis", &[]), Some(NativeFormat::Avif)),
            (
                sized_ftyp(*b"mif1", &[*b"avif", *b"miaf"]),
                Some(NativeFormat::Avif),
            ),
            (sized_ftyp(*b"mif1", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"msf1", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"heic", &[*b"mif1"]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"heix", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"hevc", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"hevx", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"heim", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"heis", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"hevm", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"hevs", &[]), Some(NativeFormat::Heif)),
            (sized_ftyp(*b"isom", &[*b"mp41"]), None),
        ];
        for (bytes, expected) in cases {
            assert_eq!(native_format(&bytes), expected);
        }
        assert_eq!(native_format(HEIC), Some(NativeFormat::Heif));
        assert_eq!(native_format(AVIF), Some(NativeFormat::Avif));
    }

    #[test]
    fn rejects_truncated_short_and_non_ftyp_input() {
        let full = sized_ftyp(*b"avif", &[]);
        assert_eq!(native_format(&full[..full.len() - 1]), None);
        assert_eq!(native_format(&full[..7]), None);
        assert_eq!(native_format(&ftyp(4, *b"avif", &[])), None);
        assert_eq!(native_format(&ftyp(7, *b"avif", &[])), None);
        assert_eq!(native_format(&ftyp(64, *b"avif", &[])), None);
        let mut moov = sized_ftyp(*b"avif", &[]);
        moov[4..8].copy_from_slice(b"moov");
        assert_eq!(native_format(&moov), None);
        let mut large = vec![0, 0, 0, 1];
        large.extend_from_slice(b"ftyp");
        large.extend_from_slice(&8_u64.to_be_bytes()[..4]);
        assert_eq!(native_format(&large), None);
        assert_eq!(native_format(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(native_format(&[]), None);
    }

    #[test]
    fn zero_size_ftyp_is_not_recognized() {
        let bytes = ftyp(0, *b"avif", &[]);
        assert_eq!(native_format(&bytes), None);
        let mut smuggled = ftyp(0, *b"isom", &[*b"isom"]);
        smuggled.extend_from_slice(b"heic");
        assert_eq!(native_format(&smuggled), None);
        let mut exact = vec![0, 0, 0, 0];
        exact.extend_from_slice(b"ftyp");
        exact.extend_from_slice(b"isom");
        exact.extend_from_slice(&[0, 0, 0, 0]);
        exact.extend_from_slice(b"heic");
        assert_eq!(native_format(&exact), None);
    }

    #[test]
    fn large_declared_ftyp_scans_only_the_brand_cap() {
        let filler = [*b"isom"; MAX_FTYP_COMPATIBLE_BRANDS];
        let mut past_cap = Vec::from(filler);
        past_cap.push(*b"heic");
        let past = sized_ftyp(*b"isom", &past_cap);
        assert_eq!(native_format(&past), None);

        let mut within_cap = Vec::from(filler);
        within_cap[MAX_FTYP_COMPATIBLE_BRANDS - 1] = *b"heic";
        let within = sized_ftyp(*b"isom", &within_cap);
        assert_eq!(native_format(&within), Some(NativeFormat::Heif));
    }

    #[test]
    fn fixtures_classify_after_brand_bounds() {
        assert_eq!(native_format(HEIC), Some(NativeFormat::Heif));
        assert_eq!(native_format(AVIF), Some(NativeFormat::Avif));
    }

    #[cfg(not(feature = "heif"))]
    #[test]
    fn default_build_reports_codec_unavailable_for_fixtures() {
        assert!(matches!(
            decode(HEIC, DecodeLimits::default()),
            Err(DecodeError::CodecUnavailable(NativeFormat::Heif))
        ));
        assert!(matches!(
            decode(AVIF, DecodeLimits::default()),
            Err(DecodeError::CodecUnavailable(NativeFormat::Avif))
        ));
        let message = DecodeError::CodecUnavailable(NativeFormat::Avif).to_string();
        assert!(message.starts_with("AVIF decoding is not available"));
    }

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
        assert!(matches!(
            decode(&[137, 80, 78, 71, 13, 10, 26, 10], DecodeLimits::default()),
            Err(DecodeError::Image(_))
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
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded)
                | DecodeError::ResourceLimit(_))
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
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded)
                | DecodeError::ResourceLimit(_))
        ));
    }

    #[test]
    fn image_limit_errors_classify_as_resource_limits() {
        for kind in [
            LimitErrorKind::DimensionError,
            LimitErrorKind::InsufficientMemory,
        ] {
            let source = ImageError::Limits(LimitError::from_kind(kind));
            let expected = source.to_string();
            let error = DecodeError::from(source);
            assert!(matches!(&error, DecodeError::ResourceLimit(text) if *text == expected));
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn non_limit_image_errors_stay_image_errors() {
        let parameter = ImageError::Parameter(ParameterError::from_kind(
            ParameterErrorKind::DimensionMismatch,
        ));
        assert!(matches!(
            DecodeError::from(parameter),
            DecodeError::Image(ImageError::Parameter(_))
        ));
        let unsupported = ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormatHint::Unknown,
            UnsupportedErrorKind::GenericFeature(String::from("feature")),
        ));
        assert!(matches!(
            DecodeError::from(unsupported),
            DecodeError::Image(ImageError::Unsupported(_))
        ));
    }

    #[test]
    fn from_rgba_takes_exact_pixels_and_rejects_bad_lengths_and_limits() {
        let dimensions = Dimensions::new(2, 1).unwrap();
        let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let source =
            DecodedSource::from_rgba(dimensions, pixels.clone(), DecodeLimits::default()).unwrap();
        assert_eq!(source.metadata().dimensions(), dimensions);
        assert_eq!(source.metadata().channels(), 4);
        assert_eq!(source.pixels(), pixels);
        for length in [0, 7, 9] {
            assert_eq!(
                DecodedSource::from_rgba(dimensions, vec![0; length], DecodeLimits::default()),
                Err(ContractError::InvalidDimensions)
            );
        }
        let small_pixels =
            DecodeLimits::new(ResourceLimits::new(1, 1024).unwrap(), 1024, 1024).unwrap();
        assert_eq!(
            DecodedSource::from_rgba(dimensions, vec![0; 8], small_pixels),
            Err(ContractError::ResourceLimitExceeded)
        );
        let small_bytes = DecodeLimits::new(ResourceLimits::default(), 1024, 7).unwrap();
        assert_eq!(
            DecodedSource::from_rgba(dimensions, vec![0; 8], small_bytes),
            Err(ContractError::ResourceLimitExceeded)
        );
    }

    #[test]
    fn rgba_len_enforces_the_fifty_megapixel_ceiling_without_allocating() {
        let limits = DecodeLimits::default();
        assert_eq!(
            limits.rgba_len(Dimensions::new(10_000, 5_000).unwrap()),
            Ok(200_000_000)
        );
        assert_eq!(
            limits.rgba_len(Dimensions::new(10_000, 5_001).unwrap()),
            Err(ContractError::ResourceLimitExceeded)
        );
        assert_eq!(
            limits.rgba_len(Dimensions::new(u32::MAX, u32::MAX).unwrap()),
            Err(ContractError::ResourceLimitExceeded)
        );
    }
}
