//! Writes into a package directory that never follow a symbolic link planted in it out of the
//! package (#1160): the files that `una fmt`, `una fix` and `una build` write, and the entries
//! of the proof cache. A cloned repository can commit a link at any name the toolchain writes,
//! so every such write goes through here.
//!
//! - A file the toolchain owns, such as a proof cache entry, is written at its name without
//!   ever following a link there ([`write_new_name`]).
//! - A file is replaced through a fresh temporary file, created exclusively under a name unique
//!   to the write, then synced (unless the caller opts out) and renamed onto its destination
//!   ([`write_atomically`]).
//! - A link at the destination is written through only when it resolves inside the package
//!   root; the directory written in must resolve inside it too.
//! - A directory chain is created one level at a time, and a link in it that leads outside the
//!   root is refused ([`create_dir_inside`]).
//! - The output of an external program, such as the linker, is checked before it starts
//!   ([`checked_output`]), written into a fresh private directory ([`private_dir`]), and moved
//!   into place under the same rules as a write ([`install`]).

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
    let (directory, name) = checked_destination(&root, target, create)?;
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

/// Replaces whatever is at the name `target` with a new file holding `bytes`, through a fresh
/// temporary file and a rename, and never follows a symbolic link at that name: a link there,
/// even one that resolves inside `root`, is itself replaced, and the file it leads to is left
/// as it was. The directory written in must resolve inside `root`. The new file is created under
/// the umask; `sync` is as for [`write_atomically`].
///
/// # Errors
///
/// Any error of the file system, or the refusal of a directory outside `root`.
pub fn write_new_name(root: &Path, target: &Path, bytes: &[u8], sync: bool) -> io::Result<()> {
    let root = canonical_root(root)?;
    let directory = std::fs::canonicalize(directory_of(target))?;
    if !directory.starts_with(&root) {
        return Err(outside(target, &directory));
    }
    let name = file_name(target, target)?;
    let destination = directory.join(&name);
    let (temporary, file) = fresh_temporary(&directory, &name, false)?;
    let written = fill_and_rename(file, &temporary, &destination, None, bytes, sync);
    if written.is_err() {
        // The name was created by this write alone, so removing it removes no one else's file.
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The canonical directory and the file name that a write of `target` under the canonical
/// `root` lands on: a symbolic link at `target` is followed only when the file it resolves to
/// is inside `root`, and the directory must resolve inside `root`. `create` allows a target
/// that does not exist yet.
fn checked_destination(
    root: &Path,
    target: &Path,
    create: bool,
) -> io::Result<(PathBuf, OsString)> {
    let destination = match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let resolved = std::fs::canonicalize(target)?;
            if !resolved.starts_with(root) {
                return Err(outside(target, &resolved));
            }
            resolved
        }
        Ok(_) => target.to_path_buf(),
        Err(error) if create && error.kind() == io::ErrorKind::NotFound => target.to_path_buf(),
        Err(error) => return Err(error),
    };
    let directory = std::fs::canonicalize(directory_of(&destination))?;
    if !directory.starts_with(root) {
        return Err(outside(target, &directory));
    }
    let name = file_name(target, &destination)?;
    Ok((directory, name))
}

/// A private directory for an external program to write into, from [`private_dir`]. It is
/// removed, with everything in it, when this value is dropped, on success and failure alike.
#[derive(Debug)]
pub struct PrivateDir {
    /// The canonical path of the directory.
    path: PathBuf,
}

impl PrivateDir {
    /// The canonical path of the directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        // The directory was created by this process alone; `remove_dir_all` removes a link
        // inside it, never what the link leads to.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// How old a private directory left by an earlier process must be before [`private_dir`]
/// removes it: one hour, far longer than any link (#1359).
pub const STALE_PRIVATE_DIR: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Creates a fresh private directory next to `near`, for an external program, such as the
/// linker, to write its output in instead of at `near` itself. The directory of `near` must
/// resolve inside `root`. The new directory has a name unique to this process and call, is
/// created with `mkdir`, which fails when anything, a link included, already has that name,
/// and has mode 0700 on Unix. Move the program's output into place with [`install`].
///
/// A process ended by a signal, such as an interrupt, never drops its [`PrivateDir`], so the
/// directory stays. Once the new one exists, the private directories that earlier processes
/// left in the same directory are removed ([`sweep_stale_private_dirs`]): only those named as
/// this function names them, owned by the same user, and older than [`STALE_PRIVATE_DIR`].
///
/// # Errors
///
/// Any error of the file system, or the refusal of a directory outside `root` with
/// [`io::ErrorKind::PermissionDenied`].
pub fn private_dir(root: &Path, near: &Path) -> io::Result<PrivateDir> {
    let root = canonical_root(root)?;
    let directory = std::fs::canonicalize(directory_of(near))?;
    if !directory.starts_with(&root) {
        return Err(outside(near, &directory));
    }
    let name = file_name(near, near)?;
    let mut last = None;
    for _ in 0..16 {
        let candidate = directory.join(unique_name(&name));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&candidate) {
            Ok(()) => {
                sweep_stale_private_dirs(
                    &directory,
                    &candidate,
                    STALE_PRIVATE_DIR,
                    std::time::SystemTime::now(),
                );
                return Ok(PrivateDir { path: candidate });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no fresh temporary name")))
}

/// Removes from `directory` the private directories that earlier processes left there, best
/// effort (#1359): each entry named `.<name>.<process>.<serial>.una-tmp`, as [`private_dir`]
/// names them, that is a directory and not a link, owned by the owner of `fresh` (the private
/// directory this process just created, so the same user), and last modified more than
/// `older_than` before `now`. A directory of a build still running is younger than that, and a
/// file, such as the temporary file of a write, is never touched. `remove_dir_all` removes a
/// link inside, never what it leads to. Nothing happens off Unix, where the owner cannot be
/// read. O(entries of `directory`).
pub fn sweep_stale_private_dirs(
    directory: &Path,
    fresh: &Path,
    older_than: std::time::Duration,
    now: std::time::SystemTime,
) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(owner) = std::fs::symlink_metadata(fresh).map(|metadata| metadata.uid()) else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == fresh || !is_private_dir_name(&entry.file_name()) {
                continue;
            }
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let stale = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > older_than);
            if metadata.file_type().is_dir() && metadata.uid() == owner && stale {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (directory, fresh, older_than, now);
}

/// Whether `name` is one that [`unique_name`] makes: `.<name>.<process>.<serial>.una-tmp`, with
/// a nonempty name and decimal process and serial numbers.
fn is_private_dir_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(inner) = name
        .strip_prefix('.')
        .and_then(|rest| rest.strip_suffix(".una-tmp"))
    else {
        return false;
    };
    let mut parts = inner.rsplitn(3, '.');
    let digits = |part: Option<&str>| {
        part.is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    };
    digits(parts.next())
        && digits(parts.next())
        && parts.next().is_some_and(|rest| !rest.is_empty())
}

/// Moves the file `produced`, which an external program wrote inside `root` (in a
/// [`PrivateDir`]), onto `target`, under the rules of [`checked_output`] for the destination.
///
/// `produced` must be a regular file, not a link, in a directory that resolves inside `root`.
/// It is opened, checked again through the open file (the same device and inode, still a
/// regular file), synced to disk, and renamed onto `target`, whose directory must resolve
/// inside `root`. A link at `target` is refused, wherever it leads, even one planted after the
/// program ran: a build output is not a source, so it is never written through. The file keeps
/// the permissions the program gave it.
///
/// # Errors
///
/// Any error of the file system (the rename needs `produced` and the destination on the same
/// file system, as a private directory next to `target` is), the refusal of a source that is
/// not a regular file inside `root`, of a link at `target` or of a destination outside `root`,
/// with [`io::ErrorKind::PermissionDenied`].
pub fn install(root: &Path, produced: &Path, target: &Path) -> io::Result<()> {
    let root = canonical_root(root)?;
    let named = std::fs::symlink_metadata(produced)?;
    if !named.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("`{}` is not a regular file", produced.display()),
        ));
    }
    let source_directory = std::fs::canonicalize(directory_of(produced))?;
    if !source_directory.starts_with(&root) {
        return Err(outside(produced, &source_directory));
    }
    let source = source_directory.join(file_name(produced, produced)?);
    let file = File::open(&source)?;
    let opened = file.metadata()?;
    if !opened.is_file() || !same_file(&named, &opened) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("`{}` changed while it was installed", produced.display()),
        ));
    }
    file.sync_all()?;
    drop(file);
    // A build output is not a source: a link at its name is refused, even one inside `root`.
    let destination = checked_output(&root, target)?;
    std::fs::rename(&source, destination)
}

/// Whether `first` and `second` are the metadata of the same file.
fn same_file(first: &std::fs::Metadata, second: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        first.dev() == second.dev() && first.ino() == second.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (first, second);
        true
    }
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
    let mut last = None;
    for _ in 0..16 {
        let temporary = directory.join(unique_name(name));
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

/// A name for a temporary file or directory of a write of `name`, unique to this process and
/// call: `.<name>.<process>.<serial>.una-tmp`.
fn unique_name(name: &OsStr) -> OsString {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut unique = OsString::from(".");
    unique.push(name);
    unique.push(format!(".{}.{serial}.una-tmp", std::process::id()));
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch directory holding `package/` and `outside/`, canonical.
    fn scratch(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("una-safe-write-{name}-{}", std::process::id()));
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
        assert!(write_atomically(&package, &target, b"one", false, true).is_err());
        write_atomically(&package, &target, b"one", true, true).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"one");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, Permissions::from_mode(0o640)).unwrap();
            write_atomically(&package, &target, b"two", false, false).unwrap();
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
            let error = write_atomically(&package, &target, b"x", true, true).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            let error = checked_output(&package, &target).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        }
        let error = create_dir_inside(&package, &package.join("out/una")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(!outside.join("una").exists());
        assert!(create_dir_inside(&package, &package.join("../escape")).is_err());
        assert!(create_dir_inside(&package, &outside.join("elsewhere")).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"kept");
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        // A link that stays inside is written through onto its file.
        std::fs::write(package.join("real.txt"), b"old").unwrap();
        std::os::unix::fs::symlink(package.join("real.txt"), package.join("inner")).unwrap();
        write_atomically(&package, &package.join("inner"), b"new", false, true).unwrap();
        assert_eq!(std::fs::read(package.join("real.txt")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directories_are_created_inside_and_an_output_is_placed_in_them() {
        let (dir, package, _) = scratch("dirs");
        let made = create_dir_inside(&package, &package.join("target/una")).unwrap();
        assert_eq!(made, package.join("target/una"));
        assert_eq!(
            create_dir_inside(&package, &package.join("target/una")).unwrap(),
            made
        );
        let output = checked_output(&package, &made.join("app")).unwrap();
        assert_eq!(output, made.join("app"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn stale_private_directories_of_the_same_user_are_swept_and_nothing_else() {
        // #1359: a build ended by a signal leaves its private directory, with the runtime copy
        // in it. The next private directory in the same place removes it once it is older
        // than an hour, and leaves a young one (a build still running), a file of the same
        // name shape, a link, and any other entry alone.
        let (dir, package, outside) = scratch("sweep");
        let made = create_dir_inside(&package, &package.join("target/una")).unwrap();
        let hour_and_more = STALE_PRIVATE_DIR + std::time::Duration::from_secs(60);
        let old = std::time::SystemTime::now() - hour_and_more;
        let age = |path: &Path| {
            File::open(path).unwrap().set_modified(old).unwrap();
        };
        let stale = made.join(".app.4242.0.una-tmp");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("libuna_rt.a"), b"runtime").unwrap();
        age(&stale);
        let young = made.join(".app.4243.0.una-tmp");
        std::fs::create_dir(&young).unwrap();
        let file = made.join(".app.4244.0.una-tmp");
        std::fs::write(&file, b"a temporary file").unwrap();
        age(&file);
        let other = made.join(".app.backup");
        std::fs::create_dir(&other).unwrap();
        age(&other);
        let link = made.join(".app.4245.0.una-tmp");
        std::fs::create_dir(outside.join("kept")).unwrap();
        std::fs::write(outside.join("kept/file"), b"kept").unwrap();
        std::os::unix::fs::symlink(outside.join("kept"), &link).unwrap();
        let fresh = private_dir(&package, &made.join("app")).unwrap();
        assert!(!stale.exists());
        for kept in [&young, &file, &other, fresh.path()] {
            assert!(kept.exists(), "{kept:?}");
        }
        assert!(std::fs::symlink_metadata(&link).is_ok());
        assert_eq!(std::fs::read(outside.join("kept/file")).unwrap(), b"kept");
        // An older time limit than the directory's age keeps it; the shape of the name counts.
        let again = made.join(".app.4246.1.una-tmp");
        std::fs::create_dir(&again).unwrap();
        age(&again);
        sweep_stale_private_dirs(
            &made,
            fresh.path(),
            hour_and_more + std::time::Duration::from_secs(3600),
            std::time::SystemTime::now(),
        );
        assert!(again.exists());
        sweep_stale_private_dirs(
            &made,
            fresh.path(),
            STALE_PRIVATE_DIR,
            std::time::SystemTime::now(),
        );
        assert!(!again.exists());
        for (name, shaped) in [
            (".app.1.2.una-tmp", true),
            (".a.b.1.2.una-tmp", true),
            ("app.1.2.una-tmp", false),
            (".1.2.una-tmp", false),
            (".app.x.2.una-tmp", false),
            (".app.1..una-tmp", false),
            (".app.1.2.una-tmp.keep", false),
        ] {
            assert_eq!(is_private_dir_name(OsStr::new(name)), shaped, "{name}");
        }
        drop(fresh);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_output_is_installed_from_a_private_directory_never_through_a_swapped_link() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, package, outside) = scratch("install");
        let made = create_dir_inside(&package, &package.join("target/una")).unwrap();
        let target = made.join("app");
        let victim = outside.join("victim");
        std::fs::write(&victim, b"kept").unwrap();
        let checked = checked_output(&package, &target).unwrap();
        let private = private_dir(&package, &checked).unwrap();
        let mode = std::fs::metadata(private.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        assert!(private.path().starts_with(&made));
        assert_ne!(private.path(), made.as_path());
        let produced = private.path().join("app");
        std::fs::write(&produced, b"linked").unwrap();
        std::fs::set_permissions(&produced, Permissions::from_mode(0o755)).unwrap();
        // A link swapped in at the final name after the check, as a concurrent process could
        // plant it while the linker runs: refused, and the file it leads to is kept.
        std::os::unix::fs::symlink(&victim, &target).unwrap();
        let error = install(&package, &produced, &target).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&victim).unwrap(), b"kept");
        // Without the link, the file is moved into place with the linker's permissions.
        std::fs::remove_file(&target).unwrap();
        install(&package, &produced, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"linked");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        // A link at the target is refused even when it leads inside the root: a build output
        // is never written through.
        let again = private.path().join("again");
        std::fs::write(&again, b"linked again").unwrap();
        let inner = made.join("inner");
        std::os::unix::fs::symlink(&target, &inner).unwrap();
        let error = install(&package, &again, &inner).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&target).unwrap(), b"linked");
        std::fs::remove_file(&inner).unwrap();
        // A source that is a link is not installed.
        let fake = private.path().join("fake");
        std::os::unix::fs::symlink(&victim, &fake).unwrap();
        assert!(install(&package, &fake, &made.join("other")).is_err());
        // The private directory goes away with its value, and only the executable is left.
        let path = private.path().to_path_buf();
        drop(private);
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(&made).unwrap().count(), 1);
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_new_name_replaces_a_link_and_keeps_its_target() {
        let (dir, package, outside) = scratch("new-name");
        let inside = package.join("kept.txt");
        std::fs::write(&inside, b"inside").unwrap();
        let victim = outside.join("victim");
        std::fs::write(&victim, b"outside").unwrap();
        for (name, to) in [("a.json", &inside), ("b.json", &victim)] {
            let target = package.join(name);
            std::os::unix::fs::symlink(to, &target).unwrap();
            write_new_name(&package, &target, b"entry", false).unwrap();
            assert!(std::fs::symlink_metadata(&target).unwrap().is_file());
            assert_eq!(std::fs::read(&target).unwrap(), b"entry");
        }
        assert_eq!(std::fs::read(&inside).unwrap(), b"inside");
        assert_eq!(std::fs::read(&victim).unwrap(), b"outside");
        // A directory outside the root is refused.
        std::os::unix::fs::symlink(&outside, package.join("out")).unwrap();
        let error = write_new_name(&package, &package.join("out/x"), b"x", false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
