#[cfg(feature = "capability-fs")]
use std::ffi::OsString;
use std::fs::{self, OpenOptions as FsOpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "capability-fs")]
use cap_std::fs::{Dir, OpenOptions as CapOpenOptions};
#[cfg(feature = "capability-fs")]
use std::path::Component;

use super::{WriteOptions, XlsxWriteError, XlsxWriteErrorCode};
use crate::XlsxDocumentKind;

const DETAIL_OUTPUT_KIND: &str = "destination extension does not match the document package kind";
const DETAIL_TEMPORARY_FILE: &str = "could not allocate a sibling temporary output file";
#[cfg(feature = "capability-fs")]
const DETAIL_CAPABILITY_DESTINATION: &str =
    "capability-relative destination must be exactly one file name";
const MAX_TEMPORARY_ATTEMPTS: u64 = 100;
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write_bytes_to_path(
    bytes: &[u8],
    kind: XlsxDocumentKind,
    destination: &Path,
    options: WriteOptions,
) -> Result<(), XlsxWriteError> {
    validate_destination_kind(kind, destination)?;
    if destination.exists() && !options.replace_existing() {
        return Err(XlsxWriteError::new(XlsxWriteErrorCode::DestinationExists));
    }
    let (mut temporary, temporary_path) = create_temporary_sibling(destination)?;
    if let Err(error) = temporary
        .write_all(bytes)
        .and_then(|()| temporary.flush())
        .and_then(|()| temporary.sync_all())
    {
        drop(temporary);
        let _ = fs::remove_file(&temporary_path);
        return Err(io_error(error));
    }
    drop(temporary);
    if let Err(error) = persist_temporary(&temporary_path, destination, options.replace_existing())
    {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    sync_parent(destination)?;
    Ok(())
}

#[cfg(feature = "capability-fs")]
pub(crate) fn write_bytes_to_directory(
    bytes: &[u8],
    kind: XlsxDocumentKind,
    directory: &Dir,
    destination: &Path,
    options: WriteOptions,
) -> Result<(), XlsxWriteError> {
    validate_capability_destination(destination)?;
    validate_destination_kind(kind, destination)?;
    let (mut temporary, temporary_path) = create_temporary_in_directory(directory, destination)?;
    if let Err(error) = temporary
        .write_all(bytes)
        .and_then(|()| temporary.flush())
        .and_then(|()| temporary.sync_all())
    {
        drop(temporary);
        let _ = directory.remove_file(&temporary_path);
        return Err(io_error(error));
    }
    drop(temporary);
    if let Err(error) =
        persist_temporary_in_directory(directory, &temporary_path, destination, options)
    {
        let _ = directory.remove_file(&temporary_path);
        return Err(error);
    }
    sync_directory(directory)?;
    Ok(())
}

pub(crate) fn validate_destination_kind(
    kind: XlsxDocumentKind,
    path: &Path,
) -> Result<(), XlsxWriteError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    let valid = matches!(
        (kind, extension.as_deref()),
        (XlsxDocumentKind::Xlsx, Some("xlsx")) | (XlsxDocumentKind::Xlsm, Some("xlsm"))
    );
    if valid {
        Ok(())
    } else {
        Err(XlsxWriteError::new(XlsxWriteErrorCode::OutputKindMismatch)
            .with_detail(DETAIL_OUTPUT_KIND))
    }
}

#[cfg(feature = "capability-fs")]
fn validate_capability_destination(destination: &Path) -> Result<(), XlsxWriteError> {
    let mut components = destination.components();
    let valid =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(XlsxWriteError::new(XlsxWriteErrorCode::Io).with_detail(DETAIL_CAPABILITY_DESTINATION))
    }
}

fn create_temporary_sibling(destination: &Path) -> Result<(fs::File, PathBuf), XlsxWriteError> {
    let parent = parent_directory(destination);
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            XlsxWriteError::new(XlsxWriteErrorCode::Io).with_detail(DETAIL_TEMPORARY_FILE)
        })?;
    for _ in 0..MAX_TEMPORARY_ATTEMPTS {
        let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{name}.cellrune-{}-{counter}.tmp",
            std::process::id()
        ));
        match FsOpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((file, candidate)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Err(XlsxWriteError::new(XlsxWriteErrorCode::Io).with_detail(DETAIL_TEMPORARY_FILE))
}

#[cfg(feature = "capability-fs")]
fn create_temporary_in_directory(
    directory: &Dir,
    destination: &Path,
) -> Result<(cap_std::fs::File, PathBuf), XlsxWriteError> {
    for _ in 0..MAX_TEMPORARY_ATTEMPTS {
        let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut candidate = OsString::from(".");
        candidate.push(destination.as_os_str());
        candidate.push(format!(".cellrune-{}-{counter}.tmp", std::process::id()));
        let candidate = PathBuf::from(candidate);
        let mut options = CapOpenOptions::new();
        options.write(true).create_new(true);
        match directory.open_with(&candidate, &options) {
            Ok(file) => return Ok((file, candidate)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Err(XlsxWriteError::new(XlsxWriteErrorCode::Io).with_detail(DETAIL_TEMPORARY_FILE))
}

fn persist_temporary(
    temporary: &Path,
    destination: &Path,
    replace_existing: bool,
) -> Result<(), XlsxWriteError> {
    if replace_existing {
        return fs::rename(temporary, destination).map_err(|error| {
            let code = if destination.exists() {
                XlsxWriteErrorCode::AtomicReplaceFailed
            } else {
                XlsxWriteErrorCode::Io
            };
            XlsxWriteError::new(code).with_cause(error)
        });
    }
    match fs::hard_link(temporary, destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::DestinationExists).with_cause(error)
            );
        }
        Err(_) => return reserve_and_rename(temporary, destination),
    }
    let _ = fs::remove_file(temporary);
    Ok(())
}

/// Installs the temporary file when the destination filesystem cannot create a
/// hard link. exFAT, FAT32, and many SMB and FUSE mounts reject `link` outright,
/// so the preferred exclusive-link install is unavailable there.
///
/// Creating the destination with `create_new` reserves it in one atomic step, so
/// the no-clobber guarantee keeps holding without a time-of-check window. The
/// rename then replaces a reservation this call owns, never a caller's file.
fn reserve_and_rename(temporary: &Path, destination: &Path) -> Result<(), XlsxWriteError> {
    FsOpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::AlreadyExists {
                XlsxWriteErrorCode::DestinationExists
            } else {
                XlsxWriteErrorCode::Io
            };
            XlsxWriteError::new(code).with_cause(error)
        })?;
    fs::rename(temporary, destination).map_err(|error| {
        let _ = fs::remove_file(destination);
        XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
    })
}

#[cfg(feature = "capability-fs")]
fn persist_temporary_in_directory(
    directory: &Dir,
    temporary: &Path,
    destination: &Path,
    options: WriteOptions,
) -> Result<(), XlsxWriteError> {
    if options.replace_existing() {
        return directory
            .rename(temporary, directory, destination)
            .map_err(|error| {
                XlsxWriteError::new(XlsxWriteErrorCode::AtomicReplaceFailed).with_cause(error)
            });
    }
    match directory.hard_link(temporary, directory, destination) {
        Ok(()) => {}
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                || directory.symlink_metadata(destination).is_ok() =>
        {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::DestinationExists).with_cause(error)
            );
        }
        Err(_) => return reserve_and_rename_in_directory(directory, temporary, destination),
    }
    let _ = directory.remove_file(temporary);
    Ok(())
}

/// Capability-relative counterpart of [`reserve_and_rename`]. Every step stays
/// bound to the directory capability, so the fallback cannot widen the authority
/// the caller granted.
#[cfg(feature = "capability-fs")]
fn reserve_and_rename_in_directory(
    directory: &Dir,
    temporary: &Path,
    destination: &Path,
) -> Result<(), XlsxWriteError> {
    let mut reservation = CapOpenOptions::new();
    reservation.write(true).create_new(true);
    directory
        .open_with(destination, &reservation)
        .map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::AlreadyExists {
                XlsxWriteErrorCode::DestinationExists
            } else {
                XlsxWriteErrorCode::Io
            };
            XlsxWriteError::new(code).with_cause(error)
        })?;
    directory
        .rename(temporary, directory, destination)
        .map_err(|error| {
            let _ = directory.remove_file(destination);
            XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
        })
}

#[cfg(unix)]
fn sync_parent(destination: &Path) -> Result<(), XlsxWriteError> {
    let parent = parent_directory(destination);
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
}

#[cfg(not(unix))]
fn sync_parent(_destination: &Path) -> Result<(), XlsxWriteError> {
    Ok(())
}

#[cfg(all(feature = "capability-fs", unix))]
fn sync_directory(directory: &Dir) -> Result<(), XlsxWriteError> {
    // Linux directory capabilities may use O_PATH, which cannot be synced.
    // Reopen "." through the capability to obtain a readable directory handle.
    directory
        .open(Path::new("."))
        .and_then(|file| file.sync_all())
        .map_err(io_error)
}

#[cfg(all(feature = "capability-fs", not(unix)))]
fn sync_directory(_directory: &Dir) -> Result<(), XlsxWriteError> {
    Ok(())
}

fn io_error(error: std::io::Error) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
}

fn parent_directory(destination: &Path) -> &Path {
    destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    #[cfg(unix)]
    use super::sync_parent;
    #[cfg(unix)]
    use super::write_bytes_to_path;
    use super::{persist_temporary, reserve_and_rename};
    #[cfg(any(unix, feature = "capability-fs"))]
    use crate::XlsxDocumentKind;
    #[cfg(any(unix, feature = "capability-fs"))]
    use crate::xlsx::write::WriteOptions;
    use crate::xlsx::write::XlsxWriteErrorCode;

    #[cfg(feature = "capability-fs")]
    use cap_std::ambient_authority;
    #[cfg(feature = "capability-fs")]
    use cap_std::fs::Dir;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "cellrune-path-output-{name}-{}-{}",
                std::process::id(),
                super::TEMPORARY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(unix)]
    #[test]
    fn relative_filename_syncs_the_current_directory() {
        sync_parent(Path::new("output.xlsx"))
            .expect("a bare relative file name must resolve its parent to the current directory");
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_replace_install_succeeds_without_an_existing_destination() {
        let directory = TestDirectory::new("new-destination");
        let temporary = directory.path().join("temporary");
        let destination = directory.path().join("output.xlsx");
        std::fs::write(&temporary, b"new").expect("write temporary");

        persist_temporary(&temporary, &destination, false).expect("install destination");

        assert_eq!(
            std::fs::read(&destination).expect("read destination"),
            b"new"
        );
        assert!(!temporary.exists());
    }

    #[test]
    fn no_replace_install_cannot_clobber_a_late_destination() {
        let directory = TestDirectory::new("late-destination");
        let temporary = directory.path().join("temporary");
        let destination = directory.path().join("output.xlsx");
        std::fs::write(&temporary, b"new").expect("write temporary");
        std::fs::write(&destination, b"existing").expect("write destination");

        let error = persist_temporary(&temporary, &destination, false)
            .expect_err("existing destination must be retained");

        assert_eq!(error.code(), XlsxWriteErrorCode::DestinationExists);
        assert_eq!(
            std::fs::read(&destination).expect("read destination"),
            b"existing"
        );
        assert_eq!(std::fs::read(&temporary).expect("read temporary"), b"new");
    }

    #[test]
    fn link_free_install_writes_the_destination() {
        let directory = TestDirectory::new("link-free-new");
        let temporary = directory.path().join("temporary");
        let destination = directory.path().join("output.xlsx");
        std::fs::write(&temporary, b"new").expect("write temporary");

        reserve_and_rename(&temporary, &destination).expect("install destination without a link");

        assert_eq!(
            std::fs::read(&destination).expect("read destination"),
            b"new"
        );
        assert!(!temporary.exists());
    }

    #[test]
    fn link_free_install_cannot_clobber_an_existing_destination() {
        let directory = TestDirectory::new("link-free-existing");
        let temporary = directory.path().join("temporary");
        let destination = directory.path().join("output.xlsx");
        std::fs::write(&temporary, b"new").expect("write temporary");
        std::fs::write(&destination, b"existing").expect("write destination");

        let error = reserve_and_rename(&temporary, &destination)
            .expect_err("existing destination must be retained");

        assert_eq!(error.code(), XlsxWriteErrorCode::DestinationExists);
        assert_eq!(
            std::fs::read(&destination).expect("read destination"),
            b"existing"
        );
        assert_eq!(std::fs::read(&temporary).expect("read temporary"), b"new");
    }

    #[test]
    fn explicit_replacement_atomically_replaces_the_destination() {
        let directory = TestDirectory::new("replacement");
        let temporary = directory.path().join("temporary");
        let destination = directory.path().join("output.xlsx");
        std::fs::write(&temporary, b"new").expect("write temporary");
        std::fs::write(&destination, b"existing").expect("write destination");

        persist_temporary(&temporary, &destination, true).expect("replace destination");

        assert_eq!(
            std::fs::read(&destination).expect("read destination"),
            b"new"
        );
        assert!(!temporary.exists());
    }

    #[cfg(unix)]
    #[test]
    fn no_replace_write_rejects_a_dangling_destination_symlink() {
        let directory = TestDirectory::new("dangling-symlink");
        let destination = directory.path().join("output.xlsx");
        std::os::unix::fs::symlink(directory.path().join("missing"), &destination)
            .expect("create dangling symlink");

        let error = write_bytes_to_path(
            b"new",
            XlsxDocumentKind::Xlsx,
            &destination,
            WriteOptions::default(),
        )
        .expect_err("dangling destination entry must not be replaced");

        assert_eq!(error.code(), XlsxWriteErrorCode::DestinationExists);
        assert_eq!(
            std::fs::read_link(&destination).expect("read symlink"),
            directory.path().join("missing")
        );
    }

    #[cfg(feature = "capability-fs")]
    #[test]
    fn capability_write_preserves_replace_policy() {
        let directory = TestDirectory::new("capability-replace-policy");
        let capability = Dir::open_ambient_dir(directory.path(), ambient_authority())
            .expect("open directory capability");
        std::fs::write(directory.path().join("output.xlsx"), b"existing")
            .expect("write existing destination");

        let error = super::write_bytes_to_directory(
            b"new",
            XlsxDocumentKind::Xlsx,
            &capability,
            Path::new("output.xlsx"),
            WriteOptions::default(),
        )
        .expect_err("no-replace write must retain an existing destination");
        assert_eq!(error.code(), XlsxWriteErrorCode::DestinationExists);
        assert_eq!(
            std::fs::read(directory.path().join("output.xlsx")).expect("read retained destination"),
            b"existing"
        );

        super::write_bytes_to_directory(
            b"new",
            XlsxDocumentKind::Xlsx,
            &capability,
            Path::new("output.xlsx"),
            WriteOptions::default().with_replace_existing(true),
        )
        .expect("explicit replacement must succeed");
        assert_eq!(
            std::fs::read(directory.path().join("output.xlsx")).expect("read replaced destination"),
            b"new"
        );
    }

    #[cfg(all(feature = "capability-fs", unix))]
    #[test]
    fn capability_write_stays_bound_to_the_open_parent_directory() {
        let directory = TestDirectory::new("capability-parent-swap");
        let live_parent = directory.path().join("live");
        let parked_parent = directory.path().join("parked");
        let outside_parent = directory.path().join("outside");
        std::fs::create_dir(&live_parent).expect("create live parent");
        std::fs::create_dir(&outside_parent).expect("create outside parent");
        let capability = Dir::open_ambient_dir(&live_parent, ambient_authority())
            .expect("open parent capability");

        std::fs::rename(&live_parent, &parked_parent).expect("park original parent");
        std::os::unix::fs::symlink(&outside_parent, &live_parent)
            .expect("replace ambient path with outside symlink");

        super::write_bytes_to_directory(
            b"new",
            XlsxDocumentKind::Xlsx,
            &capability,
            Path::new("output.xlsx"),
            WriteOptions::default(),
        )
        .expect("capability-bound write must succeed");

        assert_eq!(
            std::fs::read(parked_parent.join("output.xlsx")).expect("read bound output"),
            b"new"
        );
        assert!(
            !outside_parent.join("output.xlsx").exists(),
            "ambient path replacement must not redirect the write"
        );
    }
}
