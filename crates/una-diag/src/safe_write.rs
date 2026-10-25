//! Writes into a package directory that never follow a symbolic link planted in it out of the
//! package (#1160): the files that `una fmt`, `una fix` and `una build` write, and the entries
//! of the proof cache. A cloned repository can commit a link at any name the toolchain writes,
//! so every such write goes through here.
//!
//! - A file is replaced through a fresh temporary file, created exclusively under a name unique
//!   to the write, then synced (unless the caller opts out) and renamed onto its destination
//!   ([`write_atomically`]).
//! - A link at the destination is written through only when it resolves inside the package
//!   root; the directory written in must resolve inside it too.
//! - A directory chain is created one level at a time, and a link in it that leads outside the
//!   root is refused ([`create_dir_inside`]).
//! - The output of an external program, such as the linker, is checked before it starts
//!   ([`checked_output`]).

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

/// Replaces the file `target` with `bytes` through a fresh temporary file and a rename, so an
/// interrupted write never leaves a truncated file. `create` allows a target that does not
/// exist yet.
///
/// `sync` flushes the temporary file to disk (`fsync`, a full flush on macOS) before the rename,
/// so a crash never leaves a torn file under the destination's name. A best-effort write whose
/// torn result is detected and redone may pass `false`: the proof cache does, since an entry
/// torn by a crash fails its HMAC tag and the query is solved again.
///
/// The temporary file is created next to the destination with `create_new` (`O_CREAT|O_EXCL`,
/// which never follows a symbolic link at its name), under a name unique to this process and
/// write, with mode 0600 on Unix until it takes the destination's permissions (a new file is
/// created under the umask). Anything already at that name, a planted link included, is never
/// written through.
///
/// A symbolic link at `target` is written through onto the file it resolves to only when that
/// file, and the directory it lives in, resolve inside `root`. Otherwise the write is refused
/// with [`io::ErrorKind::PermissionDenied`], naming the link and where it leads.
///
/// # Errors
///
/// Any error of the file system, or the refusal of a destination outside `root`.
pub fn write_atomically(
    root: &Path,
    target: &Path,
    bytes: &[u8],
    create: bool,
    sync: bool,
) -> io::Result<()> {
    let root = canonical_root(root)?;
    let destination = match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let resolved = std::fs::canonicalize(target)?;
            if !resolved.starts_with(&root) {
                return Err(outside(target, &resolved));
            }
            resolved
        }
        Ok(_) => target.to_path_buf(),
        Err(error) if create && error.kind() == io::ErrorKind::NotFound => target.to_path_buf(),
        Err(error) => return Err(error),
    };
    let directory = std::fs::canonicalize(directory_of(&destination))?;
    if !directory.starts_with(&root) {
        return Err(outside(target, &directory));
    }
    let name = file_name(target, &destination)?;
    let destination = directory.join(&name);
    let permissions = std::fs::metadata(&destination)
        .ok()
        .map(|metadata| metadata.permissions());
    let (temporary, file) = fresh_temporary(&directory, &name, permissions.is_some())?;
    let written = fill_and_rename(file, &temporary, &destination, permissions, bytes, sync);
    if written.is_err() {
        // The name was created by this write alone, so removing it removes no one else's file.
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Creates the directory `dir`, which names a directory under `root`, one level at a time, and
/// returns its canonical path. A level that exists already is kept; one that is a symbolic link
/// is followed only when it resolves to a directory inside `root`, and refused otherwise with
/// [`io::ErrorKind::PermissionDenied`].
///
/// # Errors
///
/// Any error of the file system, a `dir` that is not under `root`, or the refusal of a level
/// that leads outside `root`.
pub fn create_dir_inside(root: &Path, dir: &Path) -> io::Result<PathBuf> {
    let base = canonical_root(root)?;
    let relative = dir.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("`{}` is not under `{}`", dir.display(), root.display()),
        )
    })?;
    let mut current = base.clone();
    for component in relative.components() {
        let name = match component {
            Component::Normal(name) => name,
            Component::CurDir => continue,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("`{}` leaves `{}`", dir.display(), root.display()),
                ));
            }
        };
        let next = current.join(name);
        match std::fs::create_dir(&next) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        current = existing_level(&base, &next)?;
    }
    Ok(current)
}

/// The canonical path of the existing level `next` of a directory chain under `root`.
fn existing_level(root: &Path, next: &Path) -> io::Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(next)?;
    let resolved = if metadata.file_type().is_symlink() {
        let resolved = std::fs::canonicalize(next)?;
        if !resolved.starts_with(root) {
            return Err(outside(next, &resolved));
        }
        resolved
    } else {
        next.to_path_buf()
    };
    if std::fs::metadata(&resolved)?.is_dir() {
        Ok(resolved)
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("`{}` exists and is not a directory", next.display()),
        ))
    }
}

/// The path an external program, such as the linker, is given to write `output` at, checked
/// before it starts: `output` must not be a symbolic link, and its directory must resolve
/// inside `root`. The path returned names the file in the canonical directory, so a link in the
/// directory chain cannot be swapped between the check and the write.
///
/// # Errors
///
/// Any error of the file system, or the refusal of a link or of a directory outside `root`,
/// with [`io::ErrorKind::PermissionDenied`].
pub fn checked_output(root: &Path, output: &Path) -> io::Result<PathBuf> {
    let root = canonical_root(root)?;
    match std::fs::symlink_metadata(output) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "`{}` is a symbolic link; it is not written through",
                    output.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let directory = std::fs::canonicalize(directory_of(output))?;
    if !directory.starts_with(&root) {
        return Err(outside(output, &directory));
    }
    Ok(directory.join(file_name(output, output)?))
}

/// The canonical path of `root`, where an empty path is the current directory.
fn canonical_root(root: &Path) -> io::Result<PathBuf> {
    if root.as_os_str().is_empty() {
        std::fs::canonicalize(".")
    } else {
        std::fs::canonicalize(root)
    }
}

/// The directory of `path`: its parent, or the current directory.
fn directory_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// The file name of `destination`, or an error naming `target`.
fn file_name(target: &Path, destination: &Path) -> io::Result<OsString> {
    destination
        .file_name()
        .map(OsStr::to_os_string)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`{}` names no file", target.display()),
            )
        })
}

/// Writes `bytes` into the fresh `file`, gives it `permissions` (or those of a new file), syncs
/// it when `sync` is set and renames it from `temporary` onto `destination`.
fn fill_and_rename(
    mut file: File,
    temporary: &Path,
    destination: &Path,
    permissions: Option<Permissions>,
    bytes: &[u8],
    sync: bool,
) -> io::Result<()> {
    file.write_all(bytes)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    if sync {
        file.sync_all()?;
    }
    drop(file);
    std::fs::rename(temporary, destination)
}

/// The error for a write whose destination resolves outside the package directory.
fn outside(target: &Path, resolved: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "`{}` leads to `{}`, outside the package; it is not written through",
            target.display(),
            resolved.display()
        ),
    )
}

/// Creates a new temporary file in `directory` for a write of `name`: exclusively, so nothing
/// already at the chosen name is opened, retrying a few unique names before giving up. A
/// `private` one, for a replacement, is created with mode 0600 on Unix.
fn fresh_temporary(directory: &Path, name: &OsStr, private: bool) -> io::Result<(PathBuf, File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let process = std::process::id();
    let mut last = None;
    for _ in 0..16 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut file_name = OsString::from(".");
        file_name.push(name);
        file_name.push(format!(".{process}.{serial}.una-tmp"));
        let temporary = directory.join(file_name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // A replacement starts private and then takes the destination's permissions; a new
            // file is created as `std::fs::write` would create it, under the umask.
            options.mode(if private { 0o600 } else { 0o666 });
        }
        #[cfg(not(unix))]
        let _ = private;
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no fresh temporary name")))
}
