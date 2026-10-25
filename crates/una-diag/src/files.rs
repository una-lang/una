//! Writes under a package directory that a planted symbolic link cannot redirect (#1160).
//!
//! A cloned repository chooses every name under it, links included, so each write the
//! toolchain makes under a package goes through this module:
//!
//! - [`write_atomically`] writes bytes through a fresh temporary file, created exclusively
//!   under a name unique to the write, then synced and renamed onto the destination. A link at
//!   the destination is written through only when its file and that file's directory resolve
//!   inside the package root.
//! - [`create_directories`] creates directories under the root one component at a time,
//!   refusing any component that resolves outside it.
//! - [`reserve`] gives an external program, such as the system linker, a fresh name of its own
//!   to write, in a private directory created next to the destination, inside the root, and
//!   renames the file onto the destination once the program has written it.
//!
//! The module only uses the standard library, and prints nothing.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

/// Writes `bytes` to `target` through a fresh temporary file and a rename, so a reader never
/// sees a partial file and an interrupted write never leaves a truncated one. `target` must
/// exist unless `create` is true.
///
/// The temporary file is created next to the destination with `create_new` (`O_CREAT|O_EXCL`,
/// which never follows a symbolic link at its name), under a name unique to this process and
/// write. A replacement starts with mode 0600 on Unix and takes the destination's permissions
/// before the rename; a new file is created with mode 0666 under the umask, as
/// `std::fs::write` creates it. Anything already at the temporary name, a planted link
/// included, is never written through.
///
/// A symbolic link at `target` is written through onto the file it resolves to only when that
/// file, and the directory it lives in, resolve inside `root`. Otherwise, as when the directory
/// of `target` itself resolves outside `root`, the write is refused with
/// [`io::ErrorKind::PermissionDenied`], naming where the path leads.
///
/// # Errors
///
/// The error of the first step that fails, or the refusal above. A temporary file created by
/// the write is removed when a later step fails.
pub fn write_atomically(root: &Path, target: &Path, bytes: &[u8], create: bool) -> io::Result<()> {
    let root = std::fs::canonicalize(root)?;
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
    let (directory, name) = checked_directory(&root, target, &destination)?;
    let destination = directory.join(&name);
    let permissions = std::fs::metadata(&destination)
        .ok()
        .map(|metadata| metadata.permissions());
    let (temporary, file) = fresh_temporary(&directory, &name, permissions.is_some())?;
    let written = fill_and_rename(file, &temporary, &destination, permissions, bytes);
    if written.is_err() {
        // The name was created by this write alone, so removing it removes no one else's file.
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Creates the directory `relative` under `root`, with its parents, and returns its canonical
/// path. Each component is created inside the canonical directory before it, checked with
/// `canonicalize`, so a component that exists as a symbolic link, or as anything else that
/// resolves outside `root`, is refused before anything is created through it.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidInput`] when `relative` is not a plain relative path (an absolute
/// path, or one with `..`), [`io::ErrorKind::PermissionDenied`] when a component resolves
/// outside `root`, [`io::ErrorKind::AlreadyExists`] when one is not a directory, or the error
/// of creating or resolving a component.
pub fn create_directories(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let root = std::fs::canonicalize(root)?;
    let mut current = root.clone();
    for component in relative.components() {
        let name = match component {
            Component::Normal(name) => name,
            Component::CurDir => continue,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "`{}` is not a plain path under the package",
                        relative.display()
                    ),
                ));
            }
        };
        let next = current.join(name);
        match std::fs::create_dir(&next) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let resolved = std::fs::canonicalize(&next)?;
        if !resolved.starts_with(&root) {
            return Err(outside(&next, &resolved));
        }
        if !std::fs::metadata(&resolved)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("`{}` is not a directory", next.display()),
            ));
        }
        current = resolved;
    }
    Ok(current)
}

/// A fresh name reserved for an external program to write, from [`reserve`]: a name with the
/// destination's file name, in a private directory this process created exclusively next to
/// the destination, inside the root. [`Reserved::commit`] renames the file onto the
/// destination; the directory, with anything left in it, is removed when the reservation is
/// dropped, committed or not.
#[derive(Debug)]
pub struct Reserved {
    /// The private directory, until it is removed.
    directory: Option<PathBuf>,
    /// The file the external program writes, in `directory`.
    path: PathBuf,
    /// The destination, in its canonical directory.
    destination: PathBuf,
}

impl Reserved {
    /// The path the external program writes. It has the destination's file name, which a
    /// program such as a linker may record in what it writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The private directory of the reservation, where the program's other transient files,
    /// such as an object file a linker reads, can be written too.
    #[must_use]
    pub fn directory(&self) -> &Path {
        self.directory.as_deref().unwrap_or(&self.path)
    }

    /// Renames the written file onto the destination. The rename replaces whatever is at the
    /// destination's name, a link included, and never writes through it.
    ///
    /// # Errors
    ///
    /// The error of the rename.
    pub fn commit(self) -> io::Result<()> {
        std::fs::rename(&self.path, &self.destination)
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        if let Some(directory) = self.directory.take() {
            // The directory was created by this process alone; `remove_dir_all` does not
            // follow a link inside it.
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

/// Reserves a fresh name for an external program, such as the system linker, to write
/// `target` through: the directory of `target` must resolve inside `root`, and a symbolic
/// link at `target` is refused. The program writes a file of the same name in a private
/// directory (mode 0700 on Unix) created exclusively next to `target`, which nothing else
/// names, and its permissions are those the program gives it.
///
/// # Errors
///
/// [`io::ErrorKind::PermissionDenied`] for a link at `target` or a directory outside `root`,
/// or the error of resolving the directory or creating the private one.
pub fn reserve(root: &Path, target: &Path) -> io::Result<Reserved> {
    let root = std::fs::canonicalize(root)?;
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "`{}` is a symbolic link; it is not written through",
                    target.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let (directory, name) = checked_directory(&root, target, target)?;
    let private = fresh_directory(&directory, &name)?;
    Ok(Reserved {
        path: private.join(&name),
        directory: Some(private),
        destination: directory.join(name),
    })
}

/// The canonical directory of `destination`, checked to be inside `root`, and its file name;
/// `target` is the path the caller named, for the messages.
fn checked_directory(
    root: &Path,
    target: &Path,
    destination: &Path,
) -> io::Result<(PathBuf, OsString)> {
    let directory = match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let directory = std::fs::canonicalize(&directory)?;
    if !directory.starts_with(root) {
        return Err(outside(target, &directory));
    }
    let Some(name) = destination.file_name().map(OsStr::to_os_string) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("`{}` names no file", target.display()),
        ));
    };
    Ok((directory, name))
}

/// Writes `bytes` into the fresh `file`, gives it `permissions` (or those of a new file), syncs
/// it and renames it from `temporary` onto `destination`.
fn fill_and_rename(
    mut file: File,
    temporary: &Path,
    destination: &Path,
    permissions: Option<std::fs::Permissions>,
    bytes: &[u8],
) -> io::Result<()> {
    file.write_all(bytes)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.sync_all()?;
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
    let mut last = None;
    for _ in 0..16 {
        let temporary = directory.join(unique_name(name));
        let mut options = std::fs::OpenOptions::new();
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

/// Creates a new private directory in `directory` for a write of `name`, exclusively (a
/// directory is never created through a link at its name), with mode 0700 on Unix.
fn fresh_directory(directory: &Path, name: &OsStr) -> io::Result<PathBuf> {
    let mut last = None;
    for _ in 0..16 {
        let candidate = directory.join(unique_name(name));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no fresh temporary name")))
}

/// A name for a temporary file or directory of a write of `name`, unique to this process and
/// write: `.<name>.<process>.<serial>.una-tmp`.
fn unique_name(name: &OsStr) -> OsString {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut file_name = OsString::from(".");
    file_name.push(name);
    file_name.push(format!(".{}.{serial}.una-tmp", std::process::id()));
    file_name
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch directory holding `package/` and `outside/`, canonical.
    fn scratch(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("una-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("package")).unwrap();
        std::fs::create_dir_all(dir.join("outside")).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        (dir.clone(), dir.join("package"), dir.join("outside"))
    }

    #[test]
    fn a_new_file_is_written_and_a_replacement_keeps_its_permissions() {
        let (dir, package, _) = scratch("write");
        let target = package.join("a.txt");
        assert!(write_atomically(&package, &target, b"one", false).is_err());
        write_atomically(&package, &target, b"one", true).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"one");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
            write_atomically(&package, &target, b"two", false).unwrap();
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o640);
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(&package).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn links_leading_outside_the_root_are_refused() {
        let (dir, package, outside) = scratch("links");
        let victim = outside.join("victim");
        std::fs::write(&victim, b"kept").unwrap();
        // A link at the target, and a directory that is a link.
        std::os::unix::fs::symlink(&victim, package.join("a.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, package.join("out")).unwrap();
        for target in [package.join("a.txt"), package.join("out/victim")] {
            let error = write_atomically(&package, &target, b"x", true).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert!(reserve(&package, &target).is_err());
        }
        let error = create_directories(&package, Path::new("out/una")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(!outside.join("una").exists());
        assert!(create_directories(&package, Path::new("../escape")).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"kept");
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        // A link that stays inside is written through onto its file.
        std::fs::write(package.join("real.txt"), b"old").unwrap();
        std::os::unix::fs::symlink(package.join("real.txt"), package.join("inner")).unwrap();
        write_atomically(&package, &package.join("inner"), b"new", false).unwrap();
        assert_eq!(std::fs::read(package.join("real.txt")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directories_are_created_inside_and_a_reserved_name_is_renamed_or_removed() {
        let (dir, package, _) = scratch("reserve");
        let made = create_directories(&package, Path::new("target/una")).unwrap();
        assert_eq!(made, package.join("target/una"));
        assert_eq!(
            create_directories(&package, Path::new("target/una")).unwrap(),
            made
        );
        let target = made.join("app");
        let reserved = reserve(&package, &target).unwrap();
        assert_eq!(reserved.path().file_name(), target.file_name());
        assert_eq!(reserved.path().parent(), Some(reserved.directory()));
        std::fs::write(reserved.path(), b"linked").unwrap();
        std::fs::write(reserved.directory().join("app.o"), b"object").unwrap();
        reserved.commit().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"linked");
        let dropped = reserve(&package, &target).unwrap();
        let private = dropped.directory().to_path_buf();
        assert!(private.is_dir());
        drop(dropped);
        assert!(!private.exists());
        // Only the executable is left: the private directories are gone.
        assert_eq!(std::fs::read_dir(&made).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
