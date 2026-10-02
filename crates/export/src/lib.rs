//! Bounded, row-streaming PNG export contracts.

use std::fmt;
use std::io::{self, Write};

use image_to_binary_core::{Cancellation, ContractError, Generation};

const MAX_ROW_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BROWSER_CHUNK_BYTES: usize = 1024 * 1024;

/// Legacy bounded byte output plan retained for text and non-streaming callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportPlan {
    format: image_to_binary_core::OutputFormat,
    max_bytes: u64,
}

impl ExportPlan {
    /// Creates a bounded output plan from resource limits.
    #[must_use]
    pub const fn new(
        format: image_to_binary_core::OutputFormat,
        limits: image_to_binary_core::ResourceLimits,
    ) -> Self {
        Self {
            format,
            max_bytes: limits.max_output_bytes(),
        }
    }

    /// Returns the requested output format.
    #[must_use]
    pub const fn format(self) -> image_to_binary_core::OutputFormat {
        self.format
    }

    /// Wraps bytes after enforcing the configured output ceiling.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] when bytes exceed
    /// the configured ceiling.
    pub fn finish(self, bytes: Vec<u8>) -> Result<image_to_binary_core::Output, ContractError> {
        image_to_binary_core::Output::new(self.format, bytes, self.max_bytes)
    }
}

/// Errors returned while encoding a streamed PNG.
#[derive(Debug)]
pub enum ExportError {
    /// A contract value or cancellation state was rejected.
    Contract(ContractError),
    /// The row provider rejected a row.
    Row(Box<ExportError>),
    /// The PNG encoder or output sink failed.
    Png(png::EncodingError),
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::Row(error) => write!(formatter, "row provider failed: {error}"),
            Self::Png(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            Self::Row(error) => Some(error),
            Self::Png(error) => Some(error),
        }
    }
}

impl From<ContractError> for ExportError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

/// Checked dimensions and output ceiling for one PNG stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PngExportPlan {
    width: u32,
    height: u32,
    max_encoded_bytes: u64,
}

impl PngExportPlan {
    /// Creates a plan for a non-empty RGBA8 image and bounded encoded output.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidDimensions`] for zero dimensions or
    /// overflowing row arithmetic, and [`ContractError::InvalidSettings`] for
    /// a zero output ceiling.
    pub fn new(width: u32, height: u32, max_encoded_bytes: u64) -> Result<Self, ExportError> {
        let row_bytes = u64::from(width)
            .checked_mul(4)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if width == 0 || height == 0 || width > 0x7fff_ffff {
            return Err(ContractError::InvalidDimensions.into());
        }
        if row_bytes > MAX_ROW_BYTES {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        usize::try_from(row_bytes).map_err(|_| ContractError::ResourceLimitExceeded)?;
        if max_encoded_bytes == 0 {
            return Err(ContractError::InvalidSettings("output limit").into());
        }
        Ok(Self {
            width,
            height,
            max_encoded_bytes,
        })
    }

    /// Returns the output width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the output height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Returns the encoded output ceiling.
    #[must_use]
    pub const fn max_encoded_bytes(self) -> u64 {
        self.max_encoded_bytes
    }

    /// Encodes rows directly into a caller-owned sink without retaining a full frame.
    ///
    /// The provider is called once per row with a zero-based row index and must
    /// return exactly `width * 4` RGBA8 bytes. Cancellation is checked before
    /// every row and before final PNG finalization. Sinks can contain partial
    /// PNG output when this returns an error; callers must discard it. The
    /// encoder uses `Fast` compression and `NoFilter` for deterministic,
    /// benchmark-stable export settings.
    ///
    /// # Errors
    ///
    /// Returns a contract, row-provider, PNG, sink, or cancellation error.
    pub fn encode<W, F>(
        self,
        sink: &mut W,
        cancellation: &Cancellation,
        generation: Generation,
        mut rows: F,
    ) -> Result<(), ExportError>
    where
        W: Write,
        F: FnMut(u32) -> Result<Vec<u8>, ExportError>,
    {
        cancellation.check(generation)?;
        let row_length = usize::try_from(u64::from(self.width) * 4)
            .map_err(|_| ContractError::ArithmeticOverflow)?;
        let mut counted = CountedWriter::new(sink, self.max_encoded_bytes);
        let result = encode_png_rows(
            &mut counted,
            self.width,
            self.height,
            row_length,
            cancellation,
            generation,
            &mut rows,
        );
        if counted.limit_reached {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        result
    }
}

fn encode_png_rows<W, F>(
    counted: &mut CountedWriter<'_, W>,
    width: u32,
    height: u32,
    row_length: usize,
    cancellation: &Cancellation,
    generation: Generation,
    rows: &mut F,
) -> Result<(), ExportError>
where
    W: Write,
    F: FnMut(u32) -> Result<Vec<u8>, ExportError>,
{
    let mut encoder = png::Encoder::new(counted, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::NoFilter);
    let mut writer = encoder.write_header().map_err(ExportError::Png)?;
    let mut stream = writer.stream_writer().map_err(ExportError::Png)?;
    for row in 0..height {
        cancellation.check(generation)?;
        let bytes = rows(row).map_err(|error| ExportError::Row(Box::new(error)))?;
        if bytes.len() != row_length {
            return Err(ContractError::InvalidDimensions.into());
        }
        stream
            .write_all(&bytes)
            .map_err(|error| ExportError::Png(png::EncodingError::IoError(error)))?;
    }
    cancellation.check(generation)?;
    let stream_result = stream.finish();
    let writer_result = writer.finish();
    if let Err(error) = stream_result {
        return Err(ExportError::Png(error));
    }
    writer_result.map_err(ExportError::Png)
}

struct CountedWriter<'a, W> {
    sink: &'a mut W,
    bytes: u64,
    limit: u64,
    limit_reached: bool,
}

impl<'a, W> CountedWriter<'a, W> {
    fn new(sink: &'a mut W, limit: u64) -> Self {
        Self {
            sink,
            bytes: 0,
            limit,
            limit_reached: false,
        }
    }
}

impl<W: Write> Write for CountedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let remaining = self.limit.saturating_sub(self.bytes);
        if remaining == 0 {
            self.limit_reached = true;
            return Err(io::Error::other("encoded PNG output limit exceeded"));
        }
        let allowed = usize::try_from(remaining)
            .ok()
            .map_or(bytes.len(), |value| value.min(bytes.len()));
        let written = self.sink.write(&bytes[..allowed])?;
        if written > allowed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sink reported more bytes than requested",
            ));
        }
        self.bytes = self
            .bytes
            .checked_add(
                u64::try_from(written)
                    .map_err(|_| io::Error::other("encoded byte count overflow"))?,
            )
            .ok_or_else(|| io::Error::other("encoded byte count overflow"))?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }
}

/// Bounded browser-facing chunk sink retaining encoded chunks for Blob creation.
#[derive(Debug)]
pub struct BrowserChunkSink {
    chunks: Vec<Vec<u8>>,
    current: Vec<u8>,
    chunk_size: usize,
    bytes: u64,
    limit: u64,
}

impl BrowserChunkSink {
    /// Creates a chunk sink with positive chunk and output limits.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidSettings`] for a zero chunk size or
    /// output ceiling.
    pub fn new(chunk_size: usize, max_encoded_bytes: u64) -> Result<Self, ExportError> {
        if chunk_size == 0 || chunk_size > MAX_BROWSER_CHUNK_BYTES {
            return Err(ContractError::InvalidSettings("chunk size").into());
        }
        if max_encoded_bytes == 0 {
            return Err(ContractError::InvalidSettings("output limit").into());
        }
        let capacity = chunk_size.min(usize::try_from(max_encoded_bytes).unwrap_or(usize::MAX));
        let mut current = Vec::new();
        current
            .try_reserve_exact(capacity)
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        Ok(Self {
            chunks: Vec::new(),
            current,
            chunk_size,
            bytes: 0,
            limit: max_encoded_bytes,
        })
    }

    /// Borrows chunks currently retained by this sink.
    pub fn chunks(&self) -> impl Iterator<Item = &[u8]> + '_ {
        self.chunks
            .iter()
            .map(Vec::as_slice)
            .chain((!self.current.is_empty()).then_some(self.current.as_slice()))
    }

    /// Returns owned chunks after a successful export. The final partial chunk
    /// is retained for the browser Blob path; this is bounded encoded payload
    /// storage, not a constant-memory claim.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] if retaining the
    /// final chunk cannot reserve vector capacity.
    pub fn into_chunks(mut self) -> Result<Vec<Vec<u8>>, ExportError> {
        if !self.current.is_empty() {
            self.chunks
                .try_reserve(1)
                .map_err(|_| ContractError::ResourceLimitExceeded)?;
            self.chunks.push(self.current);
        }
        Ok(self.chunks)
    }
}

impl Write for BrowserChunkSink {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let requested = bytes.len();
        let remaining = self.limit.saturating_sub(self.bytes);
        if u64::try_from(requested).unwrap_or(u64::MAX) > remaining {
            return Err(io::Error::other("encoded PNG output limit exceeded"));
        }
        while !bytes.is_empty() {
            let available = self.chunk_size - self.current.len();
            let take = available.min(bytes.len());
            self.current.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.current.len() == self.chunk_size {
                self.chunks
                    .try_reserve(1)
                    .map_err(|_| io::Error::other("browser chunk allocation failed"))?;
                self.chunks.push(std::mem::take(&mut self.current));
                self.current
                    .try_reserve_exact(self.chunk_size)
                    .map_err(|_| io::Error::other("browser chunk allocation failed"))?;
            }
        }
        self.bytes = self
            .bytes
            .checked_add(
                u64::try_from(requested)
                    .map_err(|_| io::Error::other("encoded byte count overflow"))?,
            )
            .ok_or_else(|| io::Error::other("encoded byte count overflow"))?;
        Ok(requested)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_core::{OutputFormat, ResourceLimits};
    use std::io::Cursor;

    fn plan() -> PngExportPlan {
        PngExportPlan::new(3, 2, 64 * 1024).unwrap()
    }

    fn rows(row: u32) -> Vec<u8> {
        vec![u8::try_from(row + 1).unwrap(); 3 * 4]
    }

    #[derive(Debug)]
    struct TestSink {
        bytes: Vec<u8>,
        max_write: usize,
        fail_after: Option<usize>,
        fail_flush: bool,
    }

    impl TestSink {
        fn short() -> Self {
            Self {
                bytes: Vec::new(),
                max_write: 1,
                fail_after: None,
                fail_flush: false,
            }
        }

        fn failing(fail_after: Option<usize>, fail_flush: bool) -> Self {
            Self {
                bytes: Vec::new(),
                max_write: usize::MAX,
                fail_after,
                fail_flush,
            }
        }
    }

    impl Write for TestSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self
                .fail_after
                .is_some_and(|limit| self.bytes.len() >= limit)
            {
                return Err(io::Error::other("test sink failure"));
            }
            let amount = self.max_write.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..amount]);
            Ok(amount)
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("test flush failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn legacy_output_plan_remains_bounded() {
        let limits = ResourceLimits::new(1, 2).unwrap();
        let export = ExportPlan::new(OutputFormat::Text, limits);
        assert_eq!(export.format(), OutputFormat::Text);
        assert!(export.finish(vec![1, 2]).is_ok());
        assert!(export.finish(vec![1, 2, 3]).is_err());
    }

    #[test]
    fn native_and_browser_streams_are_identical_and_decode() {
        let cancellation = Cancellation::new(Generation::new(1));
        let mut native = Cursor::new(Vec::new());
        plan()
            .encode(&mut native, &cancellation, Generation::new(1), |row| {
                Ok(rows(row))
            })
            .unwrap();
        let mut browser = BrowserChunkSink::new(7, 64 * 1024).unwrap();
        plan()
            .encode(&mut browser, &cancellation, Generation::new(1), |row| {
                Ok(rows(row))
            })
            .unwrap();
        let chunks = browser.into_chunks().unwrap();
        let browser_bytes: Vec<u8> = chunks.into_iter().flatten().collect();
        assert_eq!(native.get_ref(), &browser_bytes);
        let png_decoder = png::Decoder::new(Cursor::new(native.into_inner()));
        let mut reader = png_decoder.read_info().unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), (3, 2));
    }

    #[test]
    fn rows_are_checked_and_cancellation_stops_before_next_row() {
        let cancellation = Cancellation::new(Generation::new(2));
        let mut output = Vec::new();
        let result = plan().encode(&mut output, &cancellation, Generation::new(2), |row| {
            if row == 1 {
                cancellation.cancel();
            }
            Ok(rows(row))
        });
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::Cancelled))
        ));

        let mut output = Vec::new();
        let result = plan().encode(
            &mut output,
            &Cancellation::new(Generation::new(3)),
            Generation::new(3),
            |_| Ok(vec![0; 3]),
        );
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::InvalidDimensions))
        ));
    }

    #[test]
    fn output_limit_is_typed_and_plan_rejects_invalid_values() {
        assert!(PngExportPlan::new(0, 1, 1).is_err());
        assert!(PngExportPlan::new(1, 1, 0).is_err());
        assert!(PngExportPlan::new(0x8000_0000, 1, 1).is_err());
        let mut output = Vec::new();
        let result = PngExportPlan::new(3, 2, 8).unwrap().encode(
            &mut output,
            &Cancellation::new(Generation::new(4)),
            Generation::new(4),
            |row| Ok(rows(row)),
        );
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    #[test]
    fn short_writes_still_produce_a_valid_png() {
        let mut sink = TestSink::short();
        plan()
            .encode(
                &mut sink,
                &Cancellation::new(Generation::new(5)),
                Generation::new(5),
                |row| Ok(rows(row)),
            )
            .unwrap();
        let mut reader = png::Decoder::new(Cursor::new(sink.bytes))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        assert_eq!(reader.next_frame(&mut decoded).unwrap().width, 3);
    }

    #[test]
    fn sink_and_final_flush_errors_are_preserved() {
        for fail_after in [Some(0), Some(40)] {
            let mut sink = TestSink::failing(fail_after, false);
            let result = plan().encode(
                &mut sink,
                &Cancellation::new(Generation::new(6)),
                Generation::new(6),
                |row| Ok(rows(row)),
            );
            assert!(
                matches!(result, Err(ExportError::Png(png::EncodingError::IoError(error))) if error.to_string() == "test sink failure")
            );
        }
        let mut sink = TestSink::failing(None, true);
        let result = plan().encode(
            &mut sink,
            &Cancellation::new(Generation::new(7)),
            Generation::new(7),
            |row| Ok(rows(row)),
        );
        assert!(
            matches!(result, Err(ExportError::Png(png::EncodingError::IoError(error))) if error.to_string() == "test flush failure")
        );
    }

    #[test]
    fn provider_and_generation_failures_do_not_return_output() {
        let mut output = Vec::new();
        let result = plan().encode(
            &mut output,
            &Cancellation::new(Generation::new(8)),
            Generation::new(8),
            |_| Err(ExportError::Contract(ContractError::InvalidSettings("row"))),
        );
        assert!(matches!(result, Err(ExportError::Row(_))));
        let mut output = Vec::new();
        let result = plan().encode(
            &mut output,
            &Cancellation::new(Generation::new(8)),
            Generation::new(9),
            |_| Ok(rows(0)),
        );
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::StaleGeneration))
        ));
    }

    #[test]
    fn native_file_sink_roundtrips_and_cleans_up() {
        let mut path = std::env::temp_dir();
        path.push(format!("image-to-binary-export-{}.png", std::process::id()));
        let mut file = std::fs::File::create_new(&path).unwrap();
        plan()
            .encode(
                &mut file,
                &Cancellation::new(Generation::new(10)),
                Generation::new(10),
                |row| Ok(rows(row)),
            )
            .unwrap();
        drop(file);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(png::Decoder::new(Cursor::new(bytes)).read_info().is_ok());
    }

    #[test]
    fn browser_chunks_split_partial_and_exact_boundaries_without_collecting_accessor() {
        let mut sink = BrowserChunkSink::new(3, 16).unwrap();
        sink.write_all(&[1, 2, 3, 4, 5]).unwrap();
        let chunks: Vec<&[u8]> = sink.chunks().collect();
        assert_eq!(chunks, vec![&[1, 2, 3][..], &[4, 5][..]]);
        let owned = sink.into_chunks().unwrap();
        assert_eq!(owned, vec![vec![1, 2, 3], vec![4, 5]]);
    }

    #[test]
    fn browser_chunk_limits_reject_invalid_and_oversized_writes() {
        assert!(BrowserChunkSink::new(0, 1).is_err());
        assert!(BrowserChunkSink::new(MAX_BROWSER_CHUNK_BYTES + 1, 1).is_err());
        assert!(BrowserChunkSink::new(1, 0).is_err());
        let mut sink = BrowserChunkSink::new(4, 3).unwrap();
        assert!(sink.write(&[0; 4]).is_err());
        let mut exact = BrowserChunkSink::new(4, 4).unwrap();
        exact.write_all(&[0; 4]).unwrap();
        assert_eq!(exact.chunks().next(), Some(&[0, 0, 0, 0][..]));
    }

    #[test]
    fn browser_empty_write_and_into_chunks_are_safe() {
        let sink = BrowserChunkSink::new(4, 4).unwrap();
        assert!(sink.chunks().next().is_none());
        assert!(sink.into_chunks().unwrap().is_empty());
        let mut sink = BrowserChunkSink::new(4, 4).unwrap();
        sink.write_all(&[]).unwrap();
        assert!(sink.into_chunks().unwrap().is_empty());
    }

    #[test]
    fn row_stride_boundary_is_checked_before_encoding() {
        assert!(PngExportPlan::new(16_777_216, 1, 1).is_ok());
        assert!(PngExportPlan::new(16_777_217, 1, 1).is_err());
    }

    #[test]
    fn png_has_header_idat_and_iend_markers() {
        let mut output = Vec::new();
        plan()
            .encode(
                &mut output,
                &Cancellation::new(Generation::new(11)),
                Generation::new(11),
                |row| Ok(rows(row)),
            )
            .unwrap();
        assert_eq!(&output[..8], b"\x89PNG\r\n\x1a\n");
        assert!(output.windows(4).any(|part| part == b"IDAT"));
        assert_eq!(&output[output.len() - 8..output.len() - 4], b"IEND");
    }

    #[derive(Debug)]
    struct IendFailSink;

    impl Write for IendFailSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.windows(4).any(|part| part == b"IEND") {
                return Err(io::Error::other("IEND sink failure"));
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn iend_sink_failure_preserves_original_error() {
        let mut sink = IendFailSink;
        let result = plan().encode(
            &mut sink,
            &Cancellation::new(Generation::new(12)),
            Generation::new(12),
            |row| Ok(rows(row)),
        );
        assert!(
            matches!(result, Err(ExportError::Png(png::EncodingError::IoError(error))) if error.to_string() == "IEND sink failure")
        );
    }

    #[test]
    fn cancellation_on_last_provider_row_is_reported_before_finalize() {
        let cancellation = Cancellation::new(Generation::new(13));
        let mut output = Vec::new();
        let result = plan().encode(&mut output, &cancellation, Generation::new(13), |row| {
            if row == 1 {
                cancellation.cancel();
            }
            Ok(rows(row))
        });
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::Cancelled))
        ));
    }
}
