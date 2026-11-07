//! Atomic file sink. Bytes go to a hidden temporary sibling of the destination and reach it only
//! through [`AtomicFileSink::commit`], so a cancelled or failed write never leaves a partial file
//! and never damages an existing destination.
//!
//! Contract:
//! - A symbolic link is resolved once, when the sink is created. The temporary file sits beside
//!   the resolved file and the rename replaces that file, so the link itself survives. A link
//!   whose target is missing fails with `NotFound` before any temporary file exists.
//! - The temporary file is created with mode `0600` on unix, so its content is private while it
//!   is written. Commit gives it the permissions of an existing destination, or `0644` for a new
//!   file, before the rename.
//! - Temporary names are `.<name>.<pid>-<sequence>.tmp`. A crash can leave one behind. The sink
//!   never removes files it did not create, and because the file is `0600` an orphan is readable
//!   only by its owner.
//! - After the rename the parent directory is synced on unix on a best-effort basis. A failure
//!   there is ignored, because the rename has already published the file.

#![cfg_attr(not(test), allow(dead_code))]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique-name attempts before creation gives up. Collisions are rare, so the bound only exists
/// to keep a pathological directory from looping forever.
pub(crate) const TEMP_ATTEMPTS: u32 = 32;
const DEFAULT_SUGGESTED_NAME: &str = "image.png";
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;
#[cfg(unix)]
const NEW_FILE_MODE: u32 = 0o644;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Storage behind a sink: anything writable that can also be synced to stable storage.
trait SinkFile: Write + Send {
    fn sync(&self) -> io::Result<()>;
}

impl SinkFile for File {
    fn sync(&self) -> io::Result<()> {
        self.sync_all()
    }
}

/// Writes to a temporary sibling and renames it over the destination on [`commit`](Self::commit).
///
/// Invariants: the destination is replaced only by a successful commit, and the temporary file
/// is removed on every path that does not commit, because `Drop` owns its cleanup.
pub(crate) struct AtomicFileSink {
    target: PathBuf,
    temp: PathBuf,
    file: Option<Box<dyn SinkFile>>,
    committed: bool,
}

impl AtomicFileSink {
    /// Creates a unique private temporary file beside the file that `target` names. A symbolic
    /// link is followed first.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a link whose target is missing, `InvalidInput` when the resolved
    /// target has no file name, `AlreadyExists` when no unused temporary name is found within
    /// [`TEMP_ATTEMPTS`], and any error from creating the file.
    pub(crate) fn create(target: &Path) -> io::Result<Self> {
        Self::create_with_suffixes(target, |_| next_suffix())
    }

    /// Flushes, syncs, sets the final permissions and renames the temporary file over the
    /// destination.
    ///
    /// # Errors
    ///
    /// Returns the first failing flush, sync, permission change or rename. The destination is
    /// untouched on error, and the temporary file is removed when the sink is dropped.
    pub(crate) fn commit(mut self) -> io::Result<()> {
        let mut file = self
            .file
            .take()
            .ok_or_else(|| io::Error::other("the sink is already closed"))?;
        file.flush()?;
        file.sync()?;
        drop(file);
        prepare_permissions(&self.temp, &self.target)?;
        fs::rename(&self.temp, &self.target)?;
        self.committed = true;
        sync_parent(&self.target);
        Ok(())
    }

    fn create_with_suffixes(
        target: &Path,
        mut suffix: impl FnMut(u32) -> String,
    ) -> io::Result<Self> {
        let destination = resolve_destination(target)?;
        let file_name = destination.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "the target has no file name")
        })?;
        let directory = parent_directory(&destination);
        for attempt in 0..TEMP_ATTEMPTS {
            let temp = directory.join(format!(
                ".{}.{}.tmp",
                file_name.to_string_lossy(),
                suffix(attempt)
            ));
            match create_private(&temp) {
                Ok(file) => {
                    return Ok(Self::from_parts(destination, temp, Box::new(file)));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no unused temporary name was available",
        ))
    }

    fn from_parts(target: PathBuf, temp: PathBuf, file: Box<dyn SinkFile>) -> Self {
        Self {
            target,
            temp,
            file: Some(file),
            committed: false,
        }
    }

    fn open_file(&mut self) -> io::Result<&mut Box<dyn SinkFile>> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("the sink is already closed"))
    }
}

impl Write for AtomicFileSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.open_file()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.open_file()?.flush()
    }
}

impl Drop for AtomicFileSink {
    fn drop(&mut self) {
        if !self.committed {
            self.file = None;
            let _ = fs::remove_file(&self.temp);
        }
    }
}

/// Resolves a destination to the file it names. A symbolic link is followed to its final file,
/// and any other path is returned unchanged.
fn resolve_destination(target: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(target).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    "the chosen link does not lead to a writable file",
                )
            })
        }
        Ok(_) => Ok(target.to_path_buf()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(target.to_path_buf()),
        Err(error) => Err(error),
    }
}

fn parent_directory(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(PRIVATE_MODE);
    }
    options.open(path)
}

/// Gives the temporary file the destination's permissions when it exists, or the default mode
/// for a new file. Any other metadata error stops the commit.
fn prepare_permissions(temp: &Path, destination: &Path) -> io::Result<()> {
    match fs::metadata(destination) {
        Ok(existing) => fs::set_permissions(temp, existing.permissions()),
        #[cfg(unix)]
        Err(error) if error.kind() == io::ErrorKind::NotFound => set_new_file_permissions(temp),
        #[cfg(not(unix))]
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn set_new_file_permissions(temp: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(temp, fs::Permissions::from_mode(NEW_FILE_MODE))
}

#[cfg(unix)]
fn sync_parent(target: &Path) {
    if let Ok(directory) = File::open(parent_directory(target)) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent(_target: &Path) {}

/// Keeps only the final component of a page-supplied save name. The hint is never trusted as a
/// path: both separator styles are treated as boundaries, and `.`, `..`, empty, and NUL-bearing
/// names fall back to the default.
#[must_use]
pub(crate) fn suggested_file_name(hint: Option<&str>) -> String {
    hint.and_then(|hint| hint.rsplit(['/', '\\']).next())
        .filter(|name| !name.is_empty() && *name != "." && *name != ".." && !name.contains('\0'))
        .map_or_else(|| DEFAULT_SUGGESTED_NAME.to_owned(), str::to_owned)
}

fn next_suffix() -> String {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{sequence:x}", process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_support::TestDir;
    use std::io::ErrorKind;

    /// Writer whose write or sync fails on demand, to exercise the error paths of a real sink.
    struct ScriptedFile {
        fail_write: bool,
        fail_sync: bool,
    }

    impl Write for ScriptedFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.fail_write {
                return Err(io::Error::new(
                    ErrorKind::StorageFull,
                    "scripted write failure",
                ));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SinkFile for ScriptedFile {
        fn sync(&self) -> io::Result<()> {
            if self.fail_sync {
                return Err(io::Error::other("scripted sync failure"));
            }
            Ok(())
        }
    }

    fn assert_write_static<T: Write + 'static>() {}

    fn entry_count(dir: &Path) -> usize {
        fs::read_dir(dir).unwrap().count()
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn sink_is_a_static_writer() {
        assert_write_static::<AtomicFileSink>();
    }

    #[test]
    fn commit_replaces_the_target_and_leaves_no_temporary_file() {
        let dir = TestDir::new("sink-commit");
        let target = dir.file("out.png", b"old");
        let mut sink = AtomicFileSink::create(&target).unwrap();
        sink.write_all(b"new").unwrap();
        sink.commit().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(entry_count(dir.path()), 1);
    }

    #[cfg(unix)]
    #[test]
    fn the_temporary_file_is_private_before_commit() {
        let dir = TestDir::new("sink-private");
        let target = dir.path().join("out.png");
        let mut sink = AtomicFileSink::create(&target).unwrap();
        sink.write_all(b"secret pixels").unwrap();
        assert_eq!(mode_of(&sink.temp), 0o600);
        sink.commit().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn commit_keeps_the_permissions_of_an_existing_target() {
        let dir = TestDir::new("sink-keep-mode");
        let target = dir.file("out.png", b"old");
        set_mode(&target, 0o640);
        let mut sink = AtomicFileSink::create(&target).unwrap();
        sink.write_all(b"new").unwrap();
        sink.commit().unwrap();
        assert_eq!(mode_of(&target), 0o640);
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn commit_gives_a_new_target_mode_0644() {
        let dir = TestDir::new("sink-new-mode");
        let target = dir.path().join("fresh.png");
        let mut sink = AtomicFileSink::create(&target).unwrap();
        sink.write_all(b"new").unwrap();
        sink.commit().unwrap();
        assert_eq!(mode_of(&target), 0o644);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_destination_keeps_the_link_and_updates_the_file_it_names() {
        let dir = TestDir::new("sink-symlink");
        let real = dir.file("real.png", b"old");
        let link = dir.path().join("link.png");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let mut sink = AtomicFileSink::create(&link).unwrap();
        assert_eq!(
            sink.temp.parent(),
            Some(fs::canonicalize(real.parent().unwrap()).unwrap().as_path())
        );
        sink.write_all(b"new").unwrap();
        sink.commit().unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");
        assert_eq!(entry_count(dir.path()), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_destination_is_a_typed_error_and_creates_nothing() {
        let dir = TestDir::new("sink-dangling");
        let link = dir.path().join("dangling.png");
        std::os::unix::fs::symlink(dir.path().join("missing.png"), &link).unwrap();
        let error = AtomicFileSink::create(&link).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(entry_count(dir.path()), 1);
    }

    #[test]
    fn a_failed_rename_removes_the_temporary_file_and_leaves_no_entry_behind() {
        let dir = TestDir::new("sink-rename-fail");
        let blocked = dir.path().join("out-dir");
        fs::create_dir(&blocked).unwrap();
        let mut sink = AtomicFileSink::create(&blocked).unwrap();
        sink.write_all(b"data").unwrap();
        assert!(sink.commit().is_err());
        assert_eq!(entry_count(dir.path()), 1);
        assert!(blocked.is_dir());
    }

    #[test]
    fn dropping_without_commit_leaves_no_temporary_file_and_no_target() {
        let dir = TestDir::new("sink-drop");
        let target = dir.path().join("out.png");
        let mut sink = AtomicFileSink::create(&target).unwrap();
        sink.write_all(b"partial").unwrap();
        drop(sink);
        assert_eq!(entry_count(dir.path()), 0);
        assert!(!target.exists());
    }

    #[test]
    fn dropping_without_commit_keeps_an_existing_target_unchanged() {
        let dir = TestDir::new("sink-keep");
        let target = dir.file("out.png", b"old");
        let mut sink = AtomicFileSink::create(&target).unwrap();
        sink.write_all(b"new and never committed").unwrap();
        drop(sink);
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(entry_count(dir.path()), 1);
    }

    #[test]
    fn write_failure_removes_the_temporary_file_and_keeps_the_target() {
        let dir = TestDir::new("sink-write-fail");
        let target = dir.file("out.png", b"old");
        let temp = dir.file(".out.png.scripted.tmp", b"");
        let mut sink = AtomicFileSink::from_parts(
            target.clone(),
            temp.clone(),
            Box::new(ScriptedFile {
                fail_write: true,
                fail_sync: false,
            }),
        );
        let error = sink.write_all(b"data").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::StorageFull);
        drop(sink);
        assert!(!temp.exists());
        assert_eq!(fs::read(&target).unwrap(), b"old");
    }

    #[test]
    fn sync_failure_fails_commit_and_keeps_the_target() {
        let dir = TestDir::new("sink-sync-fail");
        let target = dir.file("out.png", b"old");
        let temp = dir.file(".out.png.scripted.tmp", b"");
        let mut sink = AtomicFileSink::from_parts(
            target.clone(),
            temp.clone(),
            Box::new(ScriptedFile {
                fail_write: false,
                fail_sync: true,
            }),
        );
        sink.write_all(b"new").unwrap();
        assert!(sink.commit().is_err());
        assert!(!temp.exists());
        assert_eq!(fs::read(&target).unwrap(), b"old");
    }

    #[test]
    fn temporary_name_collision_is_retried_with_the_next_name() {
        let dir = TestDir::new("sink-collide");
        let target = dir.path().join("out.png");
        dir.file(".out.png.a.tmp", b"someone else");
        let sink = AtomicFileSink::create_with_suffixes(&target, |attempt| {
            if attempt == 0 {
                "a".to_owned()
            } else {
                "b".to_owned()
            }
        })
        .unwrap();
        assert!(sink.temp.ends_with(".out.png.b.tmp"));
        assert_eq!(
            fs::read(dir.path().join(".out.png.a.tmp")).unwrap(),
            b"someone else"
        );
    }

    #[test]
    fn exhausted_temporary_names_fail_without_touching_existing_files() {
        let dir = TestDir::new("sink-exhaust");
        let target = dir.path().join("out.png");
        dir.file(".out.png.a.tmp", b"someone else");
        let error = AtomicFileSink::create_with_suffixes(&target, |_| "a".to_owned())
            .err()
            .unwrap();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(dir.path().join(".out.png.a.tmp")).unwrap(),
            b"someone else"
        );
    }

    #[test]
    fn target_without_a_file_name_is_rejected() {
        let error = AtomicFileSink::create(Path::new("/")).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn suggested_name_keeps_only_the_final_component() {
        assert_eq!(suggested_file_name(Some("../../x.png")), "x.png");
        assert_eq!(suggested_file_name(Some("/abs/dir/y.png")), "y.png");
        assert_eq!(suggested_file_name(Some("..\\..\\z.png")), "z.png");
        assert_eq!(suggested_file_name(Some("plain.png")), "plain.png");
        assert_eq!(suggested_file_name(Some("..")), DEFAULT_SUGGESTED_NAME);
        assert_eq!(suggested_file_name(Some("a/..")), DEFAULT_SUGGESTED_NAME);
        assert_eq!(suggested_file_name(Some("")), DEFAULT_SUGGESTED_NAME);
        assert_eq!(suggested_file_name(None), DEFAULT_SUGGESTED_NAME);
    }
}
