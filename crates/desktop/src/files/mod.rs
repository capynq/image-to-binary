//! Native file boundary. Pages hold opaque [`FileId`] handles; the paths they stand for stay in
//! the private [`FileTable`]. Dialogs and drops create handles, reads and sinks resolve them
//! inside Rust, and no path reaches IPC or an error message.

pub(crate) mod dialog;
pub(crate) mod dropped;
pub(crate) mod read;
pub(crate) mod sink;
mod table;

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{ErrorCode, IpcError};
use crate::limits::MAX_ENCODED_INPUT_BYTES;

pub(crate) use table::{FileId, FileTable};

const FALLBACK_NAME: &str = "file";
/// Same wording and code the browser uses for an empty encoded import.
const EMPTY_SOURCE_MESSAGE: &str = "Encoded image bytes must not be empty.";

/// Unicode general category Cf (format characters), as sorted inclusive ranges. Generated from
/// the Unicode 17 tables that `\p{Cf}` uses in the browser's download-name rule.
const FORMAT_RANGES: &[(u32, u32)] = &[
    (0xAD, 0xAD),
    (0x600, 0x605),
    (0x61C, 0x61C),
    (0x6DD, 0x6DD),
    (0x70F, 0x70F),
    (0x890, 0x891),
    (0x8E2, 0x8E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Page-visible description of a selected source. It carries a display name, never a path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileRef {
    id: FileId,
    name: String,
    byte_length: u64,
}

/// Page-visible description of a chosen destination. It carries a display name, never a path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SinkRef {
    id: FileId,
    name: String,
}

/// Validates a chosen source and stores its path. Validation runs first, so a rejected file never
/// replaces the stored source. An empty file is rejected with the browser's message.
///
/// # Errors
///
/// Returns the errors of [`read::regular_file_length`], `invalid_argument` for a zero-length file,
/// and the errors of [`FileTable::insert_source`].
pub(crate) fn source_ref(table: &FileTable, path: PathBuf) -> Result<FileRef, IpcError> {
    let byte_length = read::regular_file_length(&path, MAX_ENCODED_INPUT_BYTES)?;
    if byte_length == 0 {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            EMPTY_SOURCE_MESSAGE,
        ));
    }
    let name = display_name(&path);
    let id = table.insert_source(path)?;
    Ok(FileRef {
        id,
        name,
        byte_length,
    })
}

/// Stores a chosen destination path. The path must name a file, not a bare root.
///
/// # Errors
///
/// Returns `invalid_argument` when the path has no file name, and the errors of
/// [`FileTable::insert_sink`].
pub(crate) fn sink_ref(table: &FileTable, path: PathBuf) -> Result<SinkRef, IpcError> {
    if path.file_name().is_none() {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            "the chosen location has no file name",
        ));
    }
    let name = display_name(&path);
    let id = table.insert_sink(path)?;
    Ok(SinkRef { id, name })
}

/// Returns the final component of a path as lossy UTF-8 with control and format characters
/// replaced by `_`, so a bidi override cannot disguise the name. Only the display name changes;
/// the stored path keeps its exact bytes.
pub(crate) fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || FALLBACK_NAME.to_owned(),
        |name| {
            name.to_string_lossy()
                .chars()
                .map(|character| {
                    if character.is_control() || is_format(character) {
                        '_'
                    } else {
                        character
                    }
                })
                .collect()
        },
    )
}

fn is_format(character: char) -> bool {
    let code = u32::from(character);
    let index = FORMAT_RANGES.partition_point(|&(_, end)| end < code);
    FORMAT_RANGES
        .get(index)
        .is_some_and(|&(start, _)| start <= code)
}

#[cfg(test)]
impl FileRef {
    pub(crate) fn id(&self) -> FileId {
        self.id
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn byte_length(&self) -> u64 {
        self.byte_length
    }
}

#[cfg(test)]
impl SinkRef {
    pub(crate) fn id(&self) -> FileId {
        self.id
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    /// Temporary directory removed on drop. The label becomes part of the path, so tests can
    /// plant a marker and assert that it never leaks into an error.
    pub(crate) struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        pub(crate) fn new(label: &str) -> Self {
            let unique = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "i2b-desktop-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }

        pub(crate) fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::dropped::{drop_outcome, DropOutcome};
    use crate::files::test_support::TestDir;

    const MARKER: &str = "SECRET-PATH-MARKER";

    #[test]
    fn source_ref_reports_name_and_length_and_stores_the_path() {
        let dir = TestDir::new("mod-source");
        let file = dir.file("photo.bmp", b"hello");
        let table = FileTable::default();
        let chosen = source_ref(&table, file.clone()).unwrap();
        assert_eq!(chosen.name(), "photo.bmp");
        assert_eq!(chosen.byte_length(), 5);
        assert_eq!(table.source_path(chosen.id()).unwrap(), file);
    }

    #[test]
    fn a_rejected_source_consumes_no_handle() {
        let dir = TestDir::new("mod-reject");
        let table = FileTable::default();
        assert!(source_ref(&table, dir.path().to_path_buf()).is_err());
        assert!(table.insert_sink(PathBuf::from("probe")).is_ok());
    }

    #[test]
    fn errors_and_payloads_never_carry_the_path() {
        let dir = TestDir::new(MARKER);
        let table = FileTable::default();
        let directory = dir.path().to_path_buf();
        let missing = dir.path().join("missing.png");
        let large = dir.file("large.png", &[0; 9]);

        let mut errors = vec![
            source_ref(&table, directory.clone()).unwrap_err(),
            source_ref(&table, missing).unwrap_err(),
            read::regular_file_length(&large, 4).unwrap_err(),
            read::read_regular_file(&directory, 4).unwrap_err(),
        ];
        let gone = table.insert_source(large.clone()).unwrap();
        table.release(gone);
        errors.push(table.source_path(gone).unwrap_err());

        for error in &errors {
            assert!(!error.to_string().contains(MARKER), "{error}");
            let json = serde_json::to_string(error).unwrap();
            assert!(!json.contains(MARKER), "{json}");
        }

        let outcome = drop_outcome(&[directory], &table);
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(!json.contains(MARKER), "{json}");
        if let DropOutcome::Rejected { error } = outcome {
            assert!(!error.message().contains(MARKER));
        }
    }

    #[test]
    fn an_empty_source_is_rejected_with_the_browser_rule_and_keeps_the_previous_source() {
        let dir = TestDir::new("mod-empty");
        let good = dir.file("good.png", b"x");
        let empty = dir.file("empty.png", b"");
        let table = FileTable::default();
        let kept = source_ref(&table, good.clone()).unwrap();

        let error = source_ref(&table, empty).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert_eq!(error.message(), "Encoded image bytes must not be empty.");
        assert_eq!(table.source_path(kept.id()).unwrap(), good);
    }

    #[test]
    fn display_name_replaces_bidi_overrides_and_controls_but_not_the_stored_path() {
        let dir = TestDir::new("mod-rtl");
        let name = "photo\u{202E}gnp.png";
        let file = dir.file(name, b"abc");
        let table = FileTable::default();
        let chosen = source_ref(&table, file.clone()).unwrap();
        assert_eq!(chosen.name(), "photo_gnp.png");
        assert_eq!(table.source_path(chosen.id()).unwrap(), file);
    }

    #[test]
    fn display_name_replaces_controls_and_format_characters_at_range_edges() {
        let raw = PathBuf::from("a\u{0007}b\u{200B}c\u{FEFF}d\u{E0020}e\u{009F}f\u{00AD}g.png");
        assert_eq!(display_name(&raw), "a_b_c_d_e_f_g.png");
        assert_eq!(
            display_name(Path::new("keep-\u{00A0}é-漢.png")),
            "keep-\u{00A0}é-漢.png"
        );
    }

    #[test]
    fn sink_ref_stores_a_named_destination() {
        let dir = TestDir::new("mod-sink");
        let destination = dir.path().join("export.png");
        let table = FileTable::default();
        let chosen = sink_ref(&table, destination.clone()).unwrap();
        assert_eq!(chosen.name(), "export.png");
        assert_eq!(table.sink_path(chosen.id()).unwrap(), destination);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_their_exact_bytes_and_a_lossy_display_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let raw = PathBuf::from(OsStr::from_bytes(b"/dropped/photo-\xff.png"));
        assert_eq!(display_name(&raw), "photo-\u{FFFD}.png");
        let table = FileTable::default();
        let id = table.insert_source(raw.clone()).unwrap();
        let stored = table.source_path(id).unwrap();
        assert_eq!(stored.as_os_str().as_bytes(), raw.as_os_str().as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_file_is_read_through_its_exact_path_where_the_filesystem_allows_it() {
        use std::ffi::OsStr;
        use std::fs;
        use std::os::unix::ffi::OsStrExt;

        let dir = TestDir::new("mod-non-utf8-disk");
        let file = dir.path().join(OsStr::from_bytes(b"photo-\xff.png"));
        if let Err(error) = fs::write(&file, b"data") {
            eprintln!(
                "skipping on-disk non-UTF-8 check: the filesystem refused the name ({error})"
            );
            return;
        }
        let table = FileTable::default();
        let chosen = source_ref(&table, file.clone()).unwrap();
        assert_eq!(chosen.name(), "photo-\u{FFFD}.png");
        assert_eq!(chosen.byte_length(), 4);
        assert_eq!(table.source_path(chosen.id()).unwrap(), file);
    }
}
