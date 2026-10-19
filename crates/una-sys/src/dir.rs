//! Directory handles, and creation and renaming relative to them (#1094): the writes of
//! vendoring and of the Itaoca store create every file and directory relative to the
//! descriptor of a fresh private directory, and rename it relative to its parent's, so a path
//! swapped for a link while they write redirects nothing.
//!
//! Why not std: the standard library opens, creates and renames by path only; it has no
//! `openat`, `mkdirat`, `renameat`, `renameat2`, `renameatx_np` or `geteuid`. What std does
//! have is used: opening the first directory by path, and reading, protecting and syncing
//! through an open descriptor. Every name is one path component, refused before any call when
//! it is empty, `.`, `..`, or holds `/` or a NUL byte. The `libc` crate gives the declarations
//! only (ADR 0116 Appendix A). No call falls back to a weaker one: a file system that cannot
//! rename without replacing refuses the rename.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs::{File, Metadata, OpenOptions, Permissions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// A directory open as a descriptor, with `O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`: what the
/// functions of this module create, open and rename relative to.
#[derive(Debug)]
pub struct DirHandle(File);

/// `O_NOFOLLOW_ANY` on macOS, which refuses a link at any component of a path; none on Linux.
#[cfg(target_os = "macos")]
const NO_FOLLOW_ANY: libc::c_int = libc::O_NOFOLLOW_ANY;
#[cfg(target_os = "linux")]
const NO_FOLLOW_ANY: libc::c_int = 0;

impl DirHandle {
    /// Opens the directory at `path`, refusing a link at its last component, and on macOS at
    /// any component. Through std, which adds `O_CLOEXEC` itself.
    pub fn open(path: &Path) -> io::Result<DirHandle> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | NO_FOLLOW_ANY)
            .open(path)
            .map(DirHandle)
    }

    /// The directory's metadata, through its descriptor (`fstat`).
    pub fn metadata(&self) -> io::Result<Metadata> {
        self.0.metadata()
    }

    /// Sets the directory's permission bits through its descriptor (`fchmod`).
    pub fn set_mode(&self, mode: u32) -> io::Result<()> {
        self.0.set_permissions(Permissions::from_mode(mode))
    }

    /// Syncs the directory's entries to storage, through its descriptor (std's `sync_all`:
    /// `fsync`, and `F_FULLFSYNC` on macOS).
    pub fn fsync(&self) -> io::Result<()> {
        self.0.sync_all()
    }
}

/// `name` as a C string, when it is one path component: not empty, not `.` or `..`, and
/// without `/` or a NUL byte.
fn component(name: &str) -> io::Result<CString> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a name relative to a directory is one path component",
        ));
    }
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL byte"))
}

/// The descriptor `fd` that `openat` returned, owned, or the error it set.
fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a descriptor that `openat` just opened, which nothing else owns; the
    // `OwnedFd` closes it once.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    Ok(fd)
}

/// `0` as success, any other result as the error the call set.
fn done(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Creates the file `name` in `dir`, write only, with `mode` (less the umask): `openat` with
/// `O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC`, so anything already at the name, a link
/// included, refuses it. Why not std: std creates by path only.
pub fn create_file_at(dir: &DirHandle, name: &str, mode: u32) -> io::Result<File> {
    let name = component(name)?;
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `dir` holds an open descriptor for the length of the call and `name` is a valid
    // C string it outlives; the mode is passed as the `c_uint` the variadic argument promotes
    // to. Nothing else is read or written.
    let fd = unsafe {
        libc::openat(
            dir.0.as_raw_fd(),
            name.as_ptr(),
            flags,
            mode as libc::c_uint,
        )
    };
    owned(fd).map(File::from)
}

/// Creates the directory `name` in `dir` with `mode` (less the umask): `mkdirat`, which
/// refuses a name already there, a link included. Why not std: std creates by path only.
pub fn mkdir_at(dir: &DirHandle, name: &str, mode: u32) -> io::Result<()> {
    let name = component(name)?;
    // SAFETY: `dir` holds an open descriptor and `name` is a valid C string, both alive for
    // the call, which reads nothing else.
    let result = unsafe { libc::mkdirat(dir.0.as_raw_fd(), name.as_ptr(), mode as libc::mode_t) };
    done(result)
}

/// Opens the directory `name` in `dir`: `openat` with `O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`,
/// so a link or anything but a directory at the name refuses it. Why not std: std opens by
/// path only.
pub fn open_dir_at(dir: &DirHandle, name: &str) -> io::Result<DirHandle> {
    let name = component(name)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `dir` holds an open descriptor and `name` is a valid C string, both alive for
    // the call; no mode is passed, since `O_CREAT` is not set.
    let fd = unsafe { libc::openat(dir.0.as_raw_fd(), name.as_ptr(), flags) };
    owned(fd).map(|fd| DirHandle(File::from(fd)))
}

/// Renames `from` in `from_dir` to `to` in `to_dir`, refusing (`AlreadyExists`) when anything
/// is at `to`: `renameat2` with `RENAME_NOREPLACE` on Linux, `renameatx_np` with `RENAME_EXCL`
/// on macOS, atomic either way. A file system that does not support the flag refuses the
/// rename; nothing falls back to a rename that replaces. Why not std: std renames by path
/// only, and always replaces.
pub fn rename_at(from_dir: &DirHandle, from: &str, to_dir: &DirHandle, to: &str) -> io::Result<()> {
    let (from, to) = (component(from)?, component(to)?);
    let (from_fd, to_fd) = (from_dir.0.as_raw_fd(), to_dir.0.as_raw_fd());
    // SAFETY: both handles hold open descriptors and both names are valid C strings, all alive
    // for the call, which reads nothing else.
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            from_fd,
            from.as_ptr(),
            to_fd,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    // SAFETY: as above.
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            from_fd,
            from.as_ptr(),
            to_fd,
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    done(result)
}

/// Renames `from` in `from_dir` to `to` in `to_dir`, atomically replacing what is at `to`:
/// `renameat`, which replaces the entry itself and never follows a link there. For one file
/// that takes the place of an older one, never for a directory. Why not std: std renames by
/// path only.
pub fn replace_at(
    from_dir: &DirHandle,
    from: &str,
    to_dir: &DirHandle,
    to: &str,
) -> io::Result<()> {
    let (from, to) = (component(from)?, component(to)?);
    let (from_fd, to_fd) = (from_dir.0.as_raw_fd(), to_dir.0.as_raw_fd());
    // SAFETY: both handles hold open descriptors and both names are valid C strings, all alive
    // for the call, which reads nothing else.
    let result = unsafe { libc::renameat(from_fd, from.as_ptr(), to_fd, to.as_ptr()) };
    done(result)
}

/// The effective user id of the process, which owns every file and directory it creates: a
/// fresh directory opened after `mkdir_at` that another user owns was put in its place. Why
/// not std: std has no `geteuid`.
pub fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes no argument, touches no memory and always succeeds.
    unsafe { libc::geteuid() }
}
