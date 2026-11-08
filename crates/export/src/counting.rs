//! Byte-ceiling and failure-flag plumbing shared by the PNG and GIF stream encoders.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use image_to_binary_core::ContractError;

/* Shared failure flags: png's owned stream writer drops its inner writer on
finish and discards trailer errors, so the encoder must read these flags. */
#[derive(Debug, Default)]
pub(crate) struct WriteFlags {
    pub(crate) resource_limit: AtomicBool,
    pub(crate) failed: AtomicBool,
}

/* Owned sinks are flushed when the counting writer drops, which happens inside
png's StreamWriter::finish; png never flushes the sink itself on that path. */
pub(crate) struct CountedWriter<S: Write> {
    sink: S,
    bytes: u64,
    limit: u64,
    flags: Arc<WriteFlags>,
}

impl<S: Write> CountedWriter<S> {
    pub(crate) fn new(sink: S, limit: u64, flags: Arc<WriteFlags>) -> Self {
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
