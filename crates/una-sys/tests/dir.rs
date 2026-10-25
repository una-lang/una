//! The descriptor-relative file operations of `una_sys::dir` (#1094), each through its public
//! wrapper.

#![allow(clippy::unwrap_used)]

use std::io;

use std::io::Write;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::PathBuf;
use una_sys::dir::*;

fn scratch(name: &str) -> PathBuf {
    let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let dir = base.join(format!("una-sys-dir-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const BAD: [&str; 6] = ["", ".", "..", "a/b", "/a", "a\0b"];

fn invalid<T: std::fmt::Debug>(result: io::Result<T>) -> bool {
    result.unwrap_err().kind() == io::ErrorKind::InvalidInput
}

#[test]
fn a_handle_opens_a_directory_and_refuses_a_link_or_a_file() {
    let dir = scratch("open");
    std::fs::create_dir(dir.join("real")).unwrap();
    std::fs::write(dir.join("file"), b"x").unwrap();
    symlink(dir.join("real"), dir.join("link")).unwrap();
    let handle = DirHandle::open(&dir.join("real")).unwrap();
    assert!(handle.metadata().unwrap().is_dir());
    handle.set_mode(0o750).unwrap();
    assert_eq!(
        std::fs::metadata(dir.join("real")).unwrap().mode() & 0o777,
        0o750
    );
    // A file system that cannot sync a directory says so; any other error fails.
    if let Err(error) = handle.fsync() {
        let kind = error.kind();
        assert!(matches!(
            kind,
            io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
        ));
    }
    assert!(DirHandle::open(&dir.join("link")).is_err());
    assert!(DirHandle::open(&dir.join("file")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_is_created_in_the_handles_directory_even_after_its_path_moves() {
    let dir = scratch("create");
    std::fs::create_dir(dir.join("fresh")).unwrap();
    let handle = DirHandle::open(&dir.join("fresh")).unwrap();
    // The fresh directory's path is swapped for a link to another directory.
    std::fs::create_dir(dir.join("away")).unwrap();
    std::fs::rename(dir.join("fresh"), dir.join("moved")).unwrap();
    symlink(dir.join("away"), dir.join("fresh")).unwrap();
    let mut file = create_file_at(&handle, "a", 0o600).unwrap();
    file.write_all(b"bytes").unwrap();
    assert_eq!(std::fs::read(dir.join("moved/a")).unwrap(), b"bytes");
    assert_eq!(
        std::fs::metadata(dir.join("moved/a")).unwrap().mode() & 0o777,
        0o600
    );
    assert!(!dir.join("away/a").exists());
    // A name already there, or a dangling link at it, is refused, and nothing is followed.
    assert_eq!(
        create_file_at(&handle, "a", 0o600).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    symlink(dir.join("aim"), dir.join("moved/dangling")).unwrap();
    assert!(create_file_at(&handle, "dangling", 0o600).is_err());
    assert!(!dir.join("aim").exists());
    for name in BAD {
        assert!(invalid(create_file_at(&handle, name, 0o600)), "{name:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_is_made_and_opened_relative_to_a_handle() {
    let dir = scratch("mkdir");
    let handle = DirHandle::open(&dir).unwrap();
    mkdir_at(&handle, "sub", 0o700).unwrap();
    assert_eq!(
        std::fs::metadata(dir.join("sub")).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(
        mkdir_at(&handle, "sub", 0o700).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let sub = open_dir_at(&handle, "sub").unwrap();
    create_file_at(&sub, "f", 0o600).unwrap();
    assert!(dir.join("sub/f").is_file());
    // A link or a file at the name is not opened as a directory.
    symlink(dir.join("sub"), dir.join("link")).unwrap();
    assert!(open_dir_at(&handle, "link").is_err());
    assert!(open_dir_at(&sub, "f").is_err());
    assert!(mkdir_at(&handle, "link", 0o700).is_err());
    for name in BAD {
        assert!(invalid(mkdir_at(&handle, name, 0o700)), "{name:?}");
        assert!(invalid(open_dir_at(&handle, name)), "{name:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_this_process_creates_is_owned_by_its_effective_user() {
    let dir = scratch("uid");
    let handle = DirHandle::open(&dir).unwrap();
    mkdir_at(&handle, "mine", 0o700).unwrap();
    let mine = open_dir_at(&handle, "mine").unwrap();
    assert_eq!(mine.metadata().unwrap().uid(), effective_uid());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rename_never_replaces_and_a_replace_never_follows() {
    let dir = scratch("rename");
    let handle = DirHandle::open(&dir).unwrap();
    mkdir_at(&handle, "fresh", 0o700).unwrap();
    mkdir_at(&handle, "taken", 0o700).unwrap();
    rename_at(&handle, "fresh", &handle, "placed").unwrap();
    assert!(dir.join("placed").is_dir() && !dir.join("fresh").exists());
    // Anything at the target refuses the rename: an empty directory, or a link.
    assert_eq!(
        rename_at(&handle, "placed", &handle, "taken")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    symlink(dir.join("nowhere"), dir.join("dangling")).unwrap();
    assert!(rename_at(&handle, "placed", &handle, "dangling").is_err());
    assert!(dir.join("placed").is_dir());
    // A replace takes the place of a file, and of a link, which is not followed.
    let placed = open_dir_at(&handle, "placed").unwrap();
    create_file_at(&placed, "one", 0o600)
        .unwrap()
        .write_all(b"1")
        .unwrap();
    replace_at(&placed, "one", &handle, "dangling").unwrap();
    assert_eq!(std::fs::read(dir.join("dangling")).unwrap(), b"1");
    assert!(!dir.join("nowhere").exists());
    for name in BAD {
        assert!(invalid(rename_at(&handle, name, &handle, "x")), "{name:?}");
        assert!(
            invalid(rename_at(&handle, "placed", &handle, name)),
            "{name:?}"
        );
        assert!(invalid(replace_at(&handle, name, &handle, "x")), "{name:?}");
        assert!(
            invalid(replace_at(&handle, "placed", &handle, name)),
            "{name:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_removal_is_relative_to_the_handle_and_never_follows_a_link() {
    let dir = scratch("unlink");
    let handle = DirHandle::open(&dir).unwrap();
    mkdir_at(&handle, "sub", 0o700).unwrap();
    let sub = open_dir_at(&handle, "sub").unwrap();
    create_file_at(&sub, "f", 0o600).unwrap();
    // A directory that is not empty, or a file named as a directory, is not removed.
    assert!(unlink_at(&handle, "sub", true).is_err());
    assert!(unlink_at(&sub, "f", true).is_err());
    assert!(dir.join("sub/f").is_file());
    // The handle still reaches its directory after the path moves.
    std::fs::rename(dir.join("sub"), dir.join("moved")).unwrap();
    unlink_at(&sub, "f", false).unwrap();
    assert!(!dir.join("moved/f").exists());
    // A link is removed itself; its target stays.
    symlink(dir.join("moved"), dir.join("link")).unwrap();
    assert!(unlink_at(&handle, "link", true).is_err());
    unlink_at(&handle, "link", false).unwrap();
    assert!(dir.join("moved").is_dir() && std::fs::symlink_metadata(dir.join("link")).is_err());
    unlink_at(&handle, "moved", true).unwrap();
    assert!(!dir.join("moved").exists());
    for name in BAD {
        assert!(invalid(unlink_at(&handle, name, false)), "{name:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
