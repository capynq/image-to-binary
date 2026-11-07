//! Bounded reads of user-selected files. Non-regular files are rejected before any open, the
//! handle is rechecked after open, and the read itself is capped so a file that grows after its
//! length was checked still fails with a typed limit error.

#![cfg_attr(not(test), allow(dead_code))]

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

use crate::error::{ErrorCode, IpcError};

/// Returns the length of a regular file that fits within `limit` bytes. Only metadata is read,
/// so a FIFO or device is never opened. Symbolic links are followed: a link to a regular file is
/// accepted and its target is checked, while a link to a directory or FIFO is rejected.
///
/// # Errors
///
/// Returns `invalid_argument` for a non-regular file, `encoded_too_large` above `limit`, and
/// `decode_failed` when the metadata cannot be read. Messages never contain the path.
pub(crate) fn regular_file_length(path: &Path, limit: u64) -> Result<u64, IpcError> {
    let metadata = fs::metadata(path).map_err(|error| io_failure(&error))?;
    check_regular(&metadata)?;
    check_length(metadata.len(), limit)?;
    Ok(metadata.len())
}

/// Opens a regular file and checks the opened handle, because the path can be swapped between
/// the first check and the open. `open` is the only step that touches the file; it is a parameter
/// so tests can substitute a different object for the path.
pub(crate) fn open_regular_file_with(
    path: &Path,
    limit: u64,
    open: impl FnOnce(&Path) -> io::Result<File>,
) -> Result<(File, u64), IpcError> {
    regular_file_length(path, limit)?;
    let file = open(path).map_err(|error| io_failure(&error))?;
    let metadata = file.metadata().map_err(|error| io_failure(&error))?;
    check_regular(&metadata)?;
    check_length(metadata.len(), limit)?;
    Ok((file, metadata.len()))
}

/// Reads a whole regular file that fits within `limit` bytes.
///
/// # Errors
///
/// Returns the same errors as [`open_regular_file_with`] and [`read_capped`].
pub(crate) fn read_regular_file(path: &Path, limit: u64) -> Result<Vec<u8>, IpcError> {
    let (file, length) = open_regular_file_with(path, limit, open_for_read)?;
    read_capped(file, limit, length)
}

/// Opens a path for reading without waiting for a FIFO's writer. On unix `O_NONBLOCK` makes
/// `open` return at once, so a FIFO swapped in after the pre-check is refused by the handle check
/// instead of blocking. It does not change reads from a regular file.
fn open_for_read(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        File::open(path)
    }
}

/// Reads at most `limit` bytes from `reader`. The read goes through `take(limit + 1)`, so at most
/// one byte past the limit is ever pulled, and that byte is the signal that the input is too large.
/// `length_hint` only sizes the buffer and is clamped to `limit`.
///
/// # Errors
///
/// Returns `encoded_too_large` when more than `limit` bytes are available, and `decode_failed`
/// for an I/O error during the read.
pub(crate) fn read_capped<R: Read>(
    reader: R,
    limit: u64,
    length_hint: u64,
) -> Result<Vec<u8>, IpcError> {
    let ceiling = limit.checked_add(1).ok_or_else(|| {
        IpcError::new(
            ErrorCode::ArithmeticOverflow,
            "the input limit cannot be represented",
        )
    })?;
    let capacity = usize::try_from(length_hint.min(limit)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    reader
        .take(ceiling)
        .read_to_end(&mut bytes)
        .map_err(|error| io_failure(&error))?;
    check_length(u64::try_from(bytes.len()).unwrap_or(u64::MAX), limit)?;
    Ok(bytes)
}

fn check_regular(metadata: &Metadata) -> Result<(), IpcError> {
    if metadata.is_file() {
        return Ok(());
    }
    Err(IpcError::new(
        ErrorCode::InvalidArgument,
        "only regular files can be opened as images",
    ))
}

fn check_length(length: u64, limit: u64) -> Result<(), IpcError> {
    if length <= limit {
        return Ok(());
    }
    Err(IpcError::new(
        ErrorCode::EncodedTooLarge,
        format!("encoded input exceeds the {limit} byte limit"),
    ))
}

fn io_failure(error: &io::Error) -> IpcError {
    let kind = error.kind();
    IpcError::new(
        ErrorCode::DecodeFailed,
        format!("the file could not be read: {kind:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_support::TestDir;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    /// Counts the bytes pulled from the wrapped reader, so a test can see how far a read went.
    struct Counting<R> {
        inner: R,
        pulled: u64,
    }

    impl<R: Read> Read for Counting<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = self.inner.read(buf)?;
            self.pulled += u64::try_from(count).unwrap_or(0);
            Ok(count)
        }
    }

    /// Appends bytes to the file on its first read, modelling a file that grows mid-read.
    struct GrowOnFirstRead {
        inner: File,
        path: PathBuf,
        grown: bool,
    }

    impl Read for GrowOnFirstRead {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.grown {
                self.grown = true;
                OpenOptions::new()
                    .append(true)
                    .open(&self.path)?
                    .write_all(&[0; 16])?;
            }
            self.inner.read(buf)
        }
    }

    #[test]
    fn regular_file_is_read_in_full() {
        let dir = TestDir::new("read-regular");
        let file = dir.file("in.png", b"\x89PNG-data");
        assert_eq!(read_regular_file(&file, 1024).unwrap(), b"\x89PNG-data");
    }

    #[test]
    fn empty_file_reads_as_empty() {
        let dir = TestDir::new("read-empty");
        let file = dir.file("empty.png", b"");
        assert_eq!(read_regular_file(&file, 1024).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn exact_limit_passes_and_one_byte_over_fails() {
        let dir = TestDir::new("read-limit");
        let exact = dir.file("exact.bin", &[7; 8]);
        let over = dir.file("over.bin", &[7; 9]);
        assert_eq!(read_regular_file(&exact, 8).unwrap(), vec![7; 8]);
        let error = read_regular_file(&over, 8).unwrap_err();
        assert_eq!(error.code(), ErrorCode::EncodedTooLarge);
    }

    #[test]
    fn a_file_that_grows_past_the_limit_during_the_read_fails() {
        let dir = TestDir::new("read-grow");
        let file = dir.file("grow.bin", &[1; 8]);
        let mut reader = Counting {
            inner: GrowOnFirstRead {
                inner: File::open(&file).unwrap(),
                path: file.clone(),
                grown: false,
            },
            pulled: 0,
        };
        let error = read_capped(&mut reader, 8, 8).unwrap_err();
        assert_eq!(error.code(), ErrorCode::EncodedTooLarge);
        assert!(
            reader.pulled <= 9,
            "the read pulled {} bytes past a limit of 8, expected at most 9",
            reader.pulled
        );
    }

    #[test]
    fn an_unbounded_reader_is_stopped_at_one_byte_past_the_limit() {
        let mut source = Counting {
            inner: io::repeat(3).take(4 * 1024 * 1024),
            pulled: 0,
        };
        let error = read_capped(&mut source, 16, 0).unwrap_err();
        assert_eq!(error.code(), ErrorCode::EncodedTooLarge);
        assert_eq!(source.pulled, 17);
    }

    #[test]
    fn directory_is_rejected_before_it_is_read() {
        let dir = TestDir::new("read-directory");
        let error = read_regular_file(dir.path(), 1024).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn declared_length_above_the_limit_is_rejected_before_open() {
        let dir = TestDir::new("read-declared");
        let file = dir.file("big.bin", &[0; 9]);
        let error = open_regular_file_with(&file, 8, |_| {
            panic!("the file must not be opened when its length already exceeds the limit")
        })
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::EncodedTooLarge);
    }

    #[test]
    fn a_path_swapped_to_a_directory_after_the_check_is_rejected() {
        let dir = TestDir::new("read-swap");
        let file = dir.file("swap.png", b"regular at check time");
        let swapped = dir.path().to_path_buf();
        let error = open_regular_file_with(&file, 1024, move |_| File::open(&swapped)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }

    #[cfg(unix)]
    #[test]
    fn fifo_is_rejected_without_blocking_on_open() {
        let dir = TestDir::new("read-fifo");
        let fifo = dir.path().join("pipe.png");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success(), "mkfifo must create the test FIFO");

        let (sender, receiver) = channel();
        let target = fifo.clone();
        std::thread::spawn(move || {
            let _ = sender.send(read_regular_file(&target, 1024));
        });
        let outcome = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("a FIFO must be rejected without blocking the reader");
        assert_eq!(outcome.unwrap_err().code(), ErrorCode::InvalidArgument);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_swapped_in_after_the_check_is_refused_without_blocking_the_open() {
        let dir = TestDir::new("read-fifo-swap");
        let fifo = dir.path().join("pipe.png");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success(), "mkfifo must create the test FIFO");
        let regular = dir.file("regular.png", b"abc");

        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            let outcome = open_regular_file_with(&regular, 1024, move |_| open_for_read(&fifo));
            let _ = sender.send(outcome.map(|(_, length)| length));
        });
        let outcome = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the open must not block on a FIFO without a writer");
        assert_eq!(outcome.unwrap_err().code(), ErrorCode::InvalidArgument);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_a_regular_file_is_followed_and_symlink_to_a_directory_is_rejected() {
        let dir = TestDir::new("read-symlink");
        let target = dir.file("real.png", b"abc");
        let link = dir.path().join("link.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(read_regular_file(&link, 16).unwrap(), b"abc");

        let folder_link = dir.path().join("folder-link.png");
        std::os::unix::fs::symlink(dir.path(), &folder_link).unwrap();
        let error = read_regular_file(&folder_link, 16).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }
}
