//! Bounded, row-streaming PNG export contracts.

use std::fmt;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

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
    /// Shared browser chunk storage is unavailable after a poisoned lock.
    Sink(&'static str),
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::Row(error) => write!(formatter, "row provider failed: {error}"),
            Self::Png(error) => error.fmt(formatter),
            Self::Sink(reason) => write!(formatter, "chunk sink failed: {reason}"),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            Self::Row(error) => Some(error),
            Self::Png(error) => Some(error),
            Self::Sink(_) => None,
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
        let row_length = row_byte_length(self.width)?;
        let flags = Arc::new(WriteFlags::default());
        let counted = CountedWriter::new(sink, self.max_encoded_bytes, Arc::clone(&flags));
        let result = encode_png_rows(
            counted,
            self.width,
            self.height,
            row_length,
            cancellation,
            generation,
            &mut rows,
        );
        if flags.resource_limit.load(Ordering::Acquire) {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        result
    }

    /// Starts an owned incremental encoder that writes the PNG header to `sink`.
    ///
    /// The encoder uses the same settings as [`PngExportPlan::encode`], so equal
    /// RGBA8 rows produce equal bytes on both paths. Trailer write failures are
    /// reported by [`PngStreamEncoder::finish`] through the shared failure flags.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Contract`] when the header exceeds the output
    /// ceiling and [`ExportError::Png`] for other header write failures.
    pub fn stream_encoder<W: Write + 'static>(
        self,
        sink: W,
    ) -> Result<PngStreamEncoder<W>, ExportError> {
        let row_length = row_byte_length(self.width)?;
        let flags = Arc::new(WriteFlags::default());
        let counted = CountedWriter::new(sink, self.max_encoded_bytes, Arc::clone(&flags));
        let writer = configured_encoder(counted, self.width, self.height)
            .write_header()
            .map_err(|error| png_error(&flags, error))?;
        let stream = writer
            .into_stream_writer()
            .map_err(|error| png_error(&flags, error))?;
        Ok(PngStreamEncoder {
            stream,
            flags,
            height: self.height,
            row_length,
            rows_written: 0,
        })
    }
}

fn row_byte_length(width: u32) -> Result<usize, ExportError> {
    let bytes = u64::from(width)
        .checked_mul(4)
        .ok_or(ContractError::ArithmeticOverflow)?;
    Ok(usize::try_from(bytes).map_err(|_| ContractError::ArithmeticOverflow)?)
}

/* Shared PNG settings: the borrowed and owned encoders must agree byte-for-byte. */
fn configured_encoder<W: Write>(sink: W, width: u32, height: u32) -> png::Encoder<'static, W> {
    let mut encoder = png::Encoder::new(sink, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::NoFilter);
    encoder
}

fn png_error(flags: &WriteFlags, error: png::EncodingError) -> ExportError {
    if flags.resource_limit.load(Ordering::Acquire) {
        ExportError::Contract(ContractError::ResourceLimitExceeded)
    } else {
        ExportError::Png(error)
    }
}

fn encode_png_rows<W, F>(
    counted: CountedWriter<&mut W>,
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
    let mut writer = configured_encoder(counted, width, height)
        .write_header()
        .map_err(ExportError::Png)?;
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

/* Shared failure flags: png's owned stream writer drops its inner writer on
finish and discards trailer errors, so the encoder must read these flags. */
#[derive(Debug, Default)]
struct WriteFlags {
    resource_limit: AtomicBool,
    failed: AtomicBool,
}

/* Owned sinks are flushed when the counting writer drops, which happens inside
png's StreamWriter::finish; png never flushes the sink itself on that path. */
struct CountedWriter<S: Write> {
    sink: S,
    bytes: u64,
    limit: u64,
    flags: Arc<WriteFlags>,
}

impl<S: Write> CountedWriter<S> {
    fn new(sink: S, limit: u64, flags: Arc<WriteFlags>) -> Self {
        Self {
            sink,
            bytes: 0,
            limit,
            flags,
        }
    }

    fn write_counted(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let remaining = self.limit.saturating_sub(self.bytes);
        if remaining == 0 {
            self.flags.resource_limit.store(true, Ordering::Release);
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
}

impl<S: Write> CountedWriter<S> {
    fn record_sink_error(&self, error: &io::Error) {
        self.flags.failed.store(true, Ordering::Release);
        let storage_ceiling = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<ContractError>())
            .is_some_and(|contract| *contract == ContractError::ResourceLimitExceeded);
        if storage_ceiling {
            self.flags.resource_limit.store(true, Ordering::Release);
        }
    }
}

impl<S: Write> Write for CountedWriter<S> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let result = self.write_counted(bytes);
        if let Err(error) = &result {
            self.record_sink_error(error);
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.sink.flush();
        if let Err(error) = &result {
            self.record_sink_error(error);
        }
        result
    }
}

impl<S: Write> Drop for CountedWriter<S> {
    fn drop(&mut self) {
        if let Err(error) = self.sink.flush() {
            self.record_sink_error(&error);
        }
    }
}

/// Incremental RGBA8 PNG encoder that owns its sink and accepts complete rows
/// across many calls.
///
/// Invariants: every accepted write is a whole number of rows, the row total
/// never exceeds the plan height, and the encoded byte count never exceeds the
/// plan ceiling. Dropping an unfinished encoder discards the stream without
/// reporting an error; callers must also drop or discard any bytes already
/// written to the sink.
pub struct PngStreamEncoder<W: Write + 'static> {
    stream: png::StreamWriter<'static, CountedWriter<W>>,
    flags: Arc<WriteFlags>,
    height: u32,
    row_length: usize,
    rows_written: u32,
}

impl<W: Write + 'static> PngStreamEncoder<W> {
    /// Writes one or more complete RGBA8 rows.
    ///
    /// The slice length must be a whole multiple of `width * 4`. Rows beyond the
    /// plan height are rejected before any byte is written.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidDimensions`] for a partial row,
    /// [`ContractError::InvalidProgress`] when the rows exceed the plan height,
    /// [`ContractError::ResourceLimitExceeded`] when the encoded ceiling or the
    /// sink's own storage ceiling is reached, or [`ExportError::Png`] for other
    /// sink failures.
    pub fn write_rows(&mut self, rows: &[u8]) -> Result<(), ExportError> {
        if rows.len() % self.row_length != 0 {
            return Err(ContractError::InvalidDimensions.into());
        }
        let count = u32::try_from(rows.len() / self.row_length)
            .map_err(|_| ContractError::ArithmeticOverflow)?;
        let rows_written = self
            .rows_written
            .checked_add(count)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if rows_written > self.height {
            return Err(ContractError::InvalidProgress.into());
        }
        self.stream
            .write_all(rows)
            .map_err(|error| png_error(&self.flags, png::EncodingError::IoError(error)))?;
        self.rows_written = rows_written;
        Ok(())
    }

    /// Finalizes the PNG after every planned row has been written.
    ///
    /// Finalization releases the owned sink, which flushes it; a flush failure
    /// is returned here, so a buffered sink cannot report success after losing
    /// data.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] when rows are missing,
    /// [`ContractError::ResourceLimitExceeded`] when the trailer exceeds the
    /// ceiling or the sink's storage ceiling is reached, or [`ExportError::Png`]
    /// when finalization, a trailer write, or the final flush fails.
    pub fn finish(self) -> Result<(), ExportError> {
        let Self {
            stream,
            flags,
            height,
            rows_written,
            ..
        } = self;
        if rows_written != height {
            return Err(ContractError::InvalidProgress.into());
        }
        let result = stream.finish();
        if flags.resource_limit.load(Ordering::Acquire) {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        result.map_err(|error| png_error(&flags, error))?;
        if flags.failed.load(Ordering::Acquire) {
            return Err(ExportError::Png(png::EncodingError::IoError(
                io::Error::other("PNG trailer could not be written"),
            )));
        }
        Ok(())
    }
}

impl<W: Write + 'static> std::fmt::Debug for PngStreamEncoder<W> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PngStreamEncoder")
            .field("height", &self.height)
            .field("rows_written", &self.rows_written)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct PendingChunks {
    chunk_size: usize,
    max_pending_bytes: u64,
    pending_bytes: u64,
    completed: Vec<Vec<u8>>,
    current: Vec<u8>,
    closed: bool,
}

impl PendingChunks {
    fn push(&mut self, mut bytes: &[u8]) -> Result<(), ContractError> {
        if self.closed {
            return Err(ContractError::InvalidSettings("closed chunk sink"));
        }
        let requested =
            u64::try_from(bytes.len()).map_err(|_| ContractError::ArithmeticOverflow)?;
        let pending = self
            .pending_bytes
            .checked_add(requested)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if pending > self.max_pending_bytes {
            return Err(ContractError::ResourceLimitExceeded);
        }
        while !bytes.is_empty() {
            let take = (self.chunk_size - self.current.len()).min(bytes.len());
            self.current.extend_from_slice(&bytes[..take]);
            self.pending_bytes = self
                .pending_bytes
                .checked_add(u64::try_from(take).map_err(|_| ContractError::ArithmeticOverflow)?)
                .ok_or(ContractError::ArithmeticOverflow)?;
            bytes = &bytes[take..];
            if self.current.len() == self.chunk_size {
                let next = allocate_chunk(self.chunk_size)?;
                self.completed
                    .try_reserve(1)
                    .map_err(|_| ContractError::ResourceLimitExceeded)?;
                let full = std::mem::replace(&mut self.current, next);
                self.completed.push(full);
            }
        }
        Ok(())
    }
}

fn allocate_chunk(chunk_size: usize) -> Result<Vec<u8>, ContractError> {
    let mut chunk = Vec::new();
    chunk
        .try_reserve_exact(chunk_size)
        .map_err(|_| ContractError::ResourceLimitExceeded)?;
    Ok(chunk)
}

fn lock_chunks(
    shared: &Mutex<PendingChunks>,
) -> Result<MutexGuard<'_, PendingChunks>, ExportError> {
    shared
        .lock()
        .map_err(|_| ExportError::Sink("pending chunk storage lock was poisoned"))
}

/// Write half of a drainable browser chunk channel.
///
/// `Send` so native adapters may move it across threads. Its pending storage is
/// shared only with the paired [`BrowserChunkDrain`].
#[derive(Debug)]
pub struct BrowserChunkWriter {
    shared: Arc<Mutex<PendingChunks>>,
}

impl Write for BrowserChunkWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self
            .shared
            .lock()
            .map_err(|_| io::Error::other("pending chunk storage lock was poisoned"))?;
        state.push(bytes).map_err(io::Error::other)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Drain half of a browser chunk channel.
///
/// Completed chunks leave storage exactly once through [`Self::drain`]. The
/// final partial chunk is returned only by [`Self::take_remaining`], which
/// closes the channel.
#[derive(Debug)]
pub struct BrowserChunkDrain {
    shared: Arc<Mutex<PendingChunks>>,
}

impl BrowserChunkDrain {
    /// Moves completed chunks out of storage and releases their pending bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Sink`] when the shared storage lock was poisoned.
    pub fn drain(&self) -> Result<Vec<Vec<u8>>, ExportError> {
        let mut state = lock_chunks(&self.shared)?;
        let drained = std::mem::take(&mut state.completed);
        for chunk in &drained {
            let released = u64::try_from(chunk.len())
                .map_err(|_| ExportError::Contract(ContractError::ArithmeticOverflow))?;
            state.pending_bytes = state
                .pending_bytes
                .checked_sub(released)
                .ok_or(ExportError::Contract(ContractError::ArithmeticOverflow))?;
        }
        Ok(drained)
    }

    /// Closes the channel and returns every undrained chunk, including the
    /// final partial chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Sink`] when the shared storage lock was poisoned,
    /// or a resource-limit contract error when the result vector cannot grow.
    pub fn take_remaining(self) -> Result<Vec<Vec<u8>>, ExportError> {
        let mut state = lock_chunks(&self.shared)?;
        let mut chunks = std::mem::take(&mut state.completed);
        if !state.current.is_empty() {
            chunks
                .try_reserve(1)
                .map_err(|_| ExportError::Contract(ContractError::ResourceLimitExceeded))?;
            chunks.push(std::mem::take(&mut state.current));
        }
        state.pending_bytes = 0;
        state.closed = true;
        Ok(chunks)
    }
}

/// Creates a writer and drain pair for browser-facing PNG chunks.
///
/// Completed chunks hold exactly `chunk_size` bytes. `max_pending_bytes` bounds
/// completed chunks plus the partial chunk until they are drained; a write that
/// would exceed it is rejected whole.
///
/// # Errors
///
/// Returns [`ContractError::InvalidSettings`] for a zero or oversized chunk
/// size, or a pending bound smaller than one chunk, and
/// [`ContractError::ResourceLimitExceeded`] when the first chunk cannot be
/// reserved.
pub fn browser_chunk_channel(
    chunk_size: usize,
    max_pending_bytes: u64,
) -> Result<(BrowserChunkWriter, BrowserChunkDrain), ExportError> {
    if chunk_size == 0 || chunk_size > MAX_BROWSER_CHUNK_BYTES {
        return Err(ContractError::InvalidSettings("chunk size").into());
    }
    let chunk_bytes = u64::try_from(chunk_size).map_err(|_| ContractError::ArithmeticOverflow)?;
    if max_pending_bytes < chunk_bytes {
        return Err(ContractError::InvalidSettings("pending limit").into());
    }
    let shared = Arc::new(Mutex::new(PendingChunks {
        chunk_size,
        max_pending_bytes,
        pending_bytes: 0,
        completed: Vec::new(),
        current: allocate_chunk(chunk_size)?,
        closed: false,
    }));
    Ok((
        BrowserChunkWriter {
            shared: Arc::clone(&shared),
        },
        BrowserChunkDrain { shared },
    ))
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

    fn pattern_row(row: u32, row_length: usize) -> Vec<u8> {
        (0..row_length)
            .map(|index| {
                let value = u64::from(row) * 31 + u64::try_from(index).unwrap() * 7;
                u8::try_from(value % 251).unwrap()
            })
            .collect()
    }

    fn decoded_pixels(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(Cursor::new(bytes.to_vec()))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        (info.width, info.height, decoded)
    }

    #[derive(Debug)]
    struct BufferedSink {
        buffer: Vec<u8>,
        target: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for BufferedSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.buffer.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            let mut target = self
                .target
                .lock()
                .map_err(|_| io::Error::other("target lock poisoned"))?;
            target.append(&mut self.buffer);
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FlushFailSink;

    impl Write for FlushFailSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("test flush failure"))
        }
    }

    #[test]
    fn stream_finish_flushes_buffered_sink_into_target() {
        let plan = PngExportPlan::new(17, 11, 1 << 20).unwrap();
        let row_length = 17 * 4;
        let mut native = Vec::new();
        plan.encode(
            &mut native,
            &Cancellation::new(Generation::new(3)),
            Generation::new(3),
            |row| Ok(pattern_row(row, row_length)),
        )
        .unwrap();
        let target = Arc::new(Mutex::new(Vec::new()));
        let mut encoder = plan
            .stream_encoder(BufferedSink {
                buffer: Vec::new(),
                target: Arc::clone(&target),
            })
            .unwrap();
        for row in 0..11 {
            encoder.write_rows(&pattern_row(row, row_length)).unwrap();
        }
        encoder.finish().unwrap();
        assert_eq!(*target.lock().unwrap(), native);
    }

    #[test]
    fn stream_finish_reports_flush_failure() {
        let plan = PngExportPlan::new(3, 2, 64 * 1024).unwrap();
        let mut encoder = plan.stream_encoder(FlushFailSink).unwrap();
        encoder.write_rows(&[1; 24]).unwrap();
        assert!(matches!(encoder.finish(), Err(ExportError::Png(_))));
    }

    #[test]
    fn sink_storage_ceiling_is_typed_as_resource_limit() {
        let plan = PngExportPlan::new(64, 64, 1 << 20).unwrap();
        let (writer, _drain) = browser_chunk_channel(64, 100).unwrap();
        let mut encoder = plan.stream_encoder(writer).unwrap();
        let result = (0..64).try_for_each(|row| encoder.write_rows(&pattern_row(row, 256)));
        let result = result.and_then(|()| encoder.finish());
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    fn assert_send<T: Send>() {}

    #[test]
    fn stream_encoder_is_send_and_matches_encode_bytes() {
        assert_send::<PngStreamEncoder<BrowserChunkWriter>>();
        assert_send::<BrowserChunkDrain>();
        let plan = PngExportPlan::new(17, 11, 1 << 20).unwrap();
        let row_length = 17 * 4;
        let mut native = Vec::new();
        plan.encode(
            &mut native,
            &Cancellation::new(Generation::new(1)),
            Generation::new(1),
            |row| Ok(pattern_row(row, row_length)),
        )
        .unwrap();
        let (writer, drain) = browser_chunk_channel(7, 1 << 20).unwrap();
        let mut encoder = plan.stream_encoder(writer).unwrap();
        let mut pixels = Vec::new();
        for row in 0..11 {
            let bytes = pattern_row(row, row_length);
            encoder.write_rows(&bytes).unwrap();
            pixels.extend_from_slice(&bytes);
        }
        encoder.finish().unwrap();
        let streamed: Vec<u8> = drain
            .take_remaining()
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(streamed, native);
        assert_eq!(decoded_pixels(&streamed), (17, 11, pixels));
    }

    #[test]
    fn stream_batches_drain_completed_chunks_once() {
        let plan = PngExportPlan::new(17, 11, 1 << 20).unwrap();
        let row_length = 17 * 4;
        let mut native = Vec::new();
        plan.encode(
            &mut native,
            &Cancellation::new(Generation::new(2)),
            Generation::new(2),
            |row| Ok(pattern_row(row, row_length)),
        )
        .unwrap();
        let (writer, drain) = browser_chunk_channel(16, 1 << 20).unwrap();
        let mut encoder = plan.stream_encoder(writer).unwrap();
        let mut collected = Vec::new();
        for (start, count) in [(0_u32, 3_u32), (3, 3), (6, 3), (9, 2)] {
            let mut bytes = Vec::new();
            for row in start..start + count {
                bytes.extend_from_slice(&pattern_row(row, row_length));
            }
            encoder.write_rows(&bytes).unwrap();
            let drained = drain.drain().unwrap();
            assert!(drain.drain().unwrap().is_empty());
            assert!(drained.iter().all(|chunk| chunk.len() == 16));
            collected.extend(drained);
        }
        encoder.finish().unwrap();
        collected.extend(drain.take_remaining().unwrap());
        let streamed: Vec<u8> = collected.into_iter().flatten().collect();
        assert_eq!(streamed, native);
    }

    #[test]
    fn stream_encoder_validates_rows_and_finalization() {
        let plan = PngExportPlan::new(3, 2, 64 * 1024).unwrap();
        let (writer, _drain) = browser_chunk_channel(64, 1 << 16).unwrap();
        let mut encoder = plan.stream_encoder(writer).unwrap();
        assert!(matches!(
            encoder.write_rows(&[0; 11]),
            Err(ExportError::Contract(ContractError::InvalidDimensions))
        ));
        encoder.write_rows(&[]).unwrap();
        encoder.write_rows(&[7; 12]).unwrap();
        assert!(matches!(
            encoder.write_rows(&[0; 24]),
            Err(ExportError::Contract(ContractError::InvalidProgress))
        ));
        encoder.write_rows(&[9; 12]).unwrap();
        encoder.finish().unwrap();

        let (writer, _drain) = browser_chunk_channel(64, 1 << 16).unwrap();
        let mut early = plan.stream_encoder(writer).unwrap();
        early.write_rows(&[1; 12]).unwrap();
        assert!(matches!(
            early.finish(),
            Err(ExportError::Contract(ContractError::InvalidProgress))
        ));
    }

    #[test]
    fn stream_encoder_enforces_encoded_limit() {
        let plan = PngExportPlan::new(64, 64, 200).unwrap();
        let (writer, _drain) = browser_chunk_channel(64, 1 << 16).unwrap();
        let mut encoder = plan.stream_encoder(writer).unwrap();
        let result = (0..64).try_for_each(|row| encoder.write_rows(&pattern_row(row, 256)));
        let result = result.and_then(|()| encoder.finish());
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    #[test]
    fn stream_encoder_reports_sink_and_trailer_failures() {
        let plan = PngExportPlan::new(3, 2, 64 * 1024).unwrap();
        let mut encoder = plan
            .stream_encoder(TestSink::failing(Some(40), false))
            .unwrap();
        let result = match encoder.write_rows(&[1; 24]) {
            Ok(()) => encoder.finish(),
            Err(error) => Err(error),
        };
        assert!(matches!(result, Err(ExportError::Png(_))));

        let mut encoder = plan.stream_encoder(IendFailSink).unwrap();
        encoder.write_rows(&[1; 24]).unwrap();
        assert!(matches!(
            encoder.finish(),
            Err(ExportError::Png(png::EncodingError::IoError(_)))
        ));
    }

    #[test]
    fn browser_channel_drains_once_and_bounds_pending_bytes() {
        let (mut writer, drain) = browser_chunk_channel(3, 6).unwrap();
        writer.write_all(&[1, 2, 3, 4, 5]).unwrap();
        assert_eq!(drain.drain().unwrap(), vec![vec![1, 2, 3]]);
        assert!(drain.drain().unwrap().is_empty());
        writer.write_all(&[6, 7, 8, 9]).unwrap();
        assert!(writer.write(&[10]).is_err());
        assert_eq!(drain.drain().unwrap(), vec![vec![4, 5, 6], vec![7, 8, 9]]);
        writer.write_all(&[10]).unwrap();
        assert_eq!(drain.take_remaining().unwrap(), vec![vec![10]]);
        assert!(writer.write(&[11]).is_err());
    }

    #[test]
    fn browser_channel_rejects_invalid_settings_and_poisoned_locks() {
        assert!(browser_chunk_channel(0, 8).is_err());
        assert!(browser_chunk_channel(MAX_BROWSER_CHUNK_BYTES + 1, u64::MAX).is_err());
        assert!(browser_chunk_channel(4, 3).is_err());

        let (mut writer, drain) = browser_chunk_channel(4, 16).unwrap();
        let shared = Arc::clone(&writer.shared);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = shared.lock().unwrap();
            panic!("poison the chunk storage lock");
        }));
        assert!(writer.write(&[1]).is_err());
        assert!(matches!(drain.drain(), Err(ExportError::Sink(_))));
    }

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
