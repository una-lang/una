//! Reading and writing a package's files with no link followed (ADR 0122 rules 27 and 30):
//! symbolic and hard links, `..`, absolute paths, hidden and foreign files, device names,
//! case-colliding names and oversize files each refuse the package.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use una_project::content::{MAX_BYTES, digest_of};
use una_project::tree::{locate, make_dirs, read_file, read_package, write_fresh, write_new_file};

/// A fresh directory named `name`, with a package of two files in `pkg/`.
fn fresh(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("tree")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pkg/src")).unwrap();
    std::fs::write(
        dir.join("pkg/ita.una"),
        "ita demo\n  version 0.1.0\n  edition 0.4\n",
    )
    .unwrap();
    std::fs::write(dir.join("pkg/src/demo.una"), "module demo\n").unwrap();
    dir
}

fn read(base: &Path) -> Result<usize, String> {
    let dir = locate(base, &["pkg"])?.ok_or("missing")?;
    read_package(&dir).map(|files| files.len())
}

#[test]
fn a_package_of_regular_files_reads_with_its_digest() {
    let base = fresh("plain");
    let dir = locate(&base, &["pkg"]).unwrap().unwrap();
    let mut files = read_package(&dir).unwrap();
    assert_eq!(files.len(), 2);
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["ita.una", "src/demo.una"]);
    assert!(digest_of(&mut files).unwrap().starts_with("sha256:"));
    // A missing component is absent, not an error.
    assert_eq!(locate(&base, &["nothing", "here"]).unwrap(), None);
    assert_eq!(locate(&base.join("nothing"), &["pkg"]).unwrap(), None);
}

#[test]
fn components_that_leave_the_base_are_refused() {
    let base = fresh("components");
    for part in ["..", ".", "", "a/b", "/abs", "a\\b", "a\0b"] {
        assert!(locate(&base, &[part]).is_err(), "{part:?}");
        assert!(make_dirs(&base, &[part]).is_err(), "{part:?}");
    }
    // Written paths are rule 27's, checked before anything is written.
    for path in [
        "../escape.una",
        "/abs.una",
        "src/../x.una",
        "src/.hidden.una",
    ] {
        let target = base.join(format!("w-{}", path.len()));
        assert!(
            write_fresh(&target, &[(path, b"x".as_slice())], false).is_err(),
            "{path}"
        );
    }
    assert!(!base.join("escape.una").exists());
}

#[cfg(unix)]
#[test]
fn links_anywhere_refuse_the_package() {
    use std::os::unix::fs::symlink;
    // A file that is a symbolic link.
    let base = fresh("file-link");
    std::fs::write(base.join("secret.una"), "module secret\n").unwrap();
    symlink(base.join("secret.una"), base.join("pkg/src/link.una")).unwrap();
    assert!(read(&base).unwrap_err().contains("symbolic link"));
    // A directory that is a symbolic link.
    let base = fresh("dir-link");
    std::fs::create_dir_all(base.join("elsewhere")).unwrap();
    std::fs::write(base.join("elsewhere/x.una"), "module x\n").unwrap();
    symlink(base.join("elsewhere"), base.join("pkg/src/sub")).unwrap();
    assert!(read(&base).unwrap_err().contains("symbolic link"));
    // The package directory, or a component above it, that is a link.
    let base = fresh("component-link");
    std::fs::rename(base.join("pkg"), base.join("real")).unwrap();
    symlink(base.join("real"), base.join("pkg")).unwrap();
    assert!(
        locate(&base, &["pkg"])
            .unwrap_err()
            .contains("symbolic link")
    );
    std::fs::create_dir_all(base.join("deep")).unwrap();
    symlink(base.join("real"), base.join("deep/link")).unwrap();
    assert!(locate(&base, &["deep", "link", "src"]).is_err());
    // A hard link to a file outside.
    let base = fresh("hard-link");
    std::fs::write(base.join("outside.una"), "module outside\n").unwrap();
    std::fs::hard_link(base.join("outside.una"), base.join("pkg/src/hard.una")).unwrap();
    assert!(read(&base).unwrap_err().contains("hard link"));
}

#[test]
fn hidden_foreign_and_colliding_entries_refuse_the_package() {
    let base = fresh("hidden");
    std::fs::write(base.join("pkg/src/.DS_Store"), "x").unwrap();
    assert!(read(&base).is_err());
    let base = fresh("foreign");
    std::fs::write(base.join("pkg/src/lib.c"), "int x;").unwrap();
    assert!(
        read(&base)
            .unwrap_err()
            .contains("not a file a package holds")
    );
    let base = fresh("device-name");
    std::fs::write(base.join("pkg/src/con.una"), "module con\n").unwrap();
    assert!(read(&base).unwrap_err().contains("device name"));
    // Two names equal ignoring case, where the file system keeps both.
    let base = fresh("case");
    std::fs::write(base.join("pkg/src/Demo.una"), "module Demo\n").unwrap();
    if std::fs::read_dir(base.join("pkg/src")).unwrap().count() == 2 {
        let dir = locate(&base, &["pkg"]).unwrap().unwrap();
        let mut files = read_package(&dir).unwrap();
        assert!(digest_of(&mut files).unwrap_err().contains("ignoring case"));
    }
}

#[test]
fn an_oversize_file_is_refused_before_it_is_read() {
    let base = fresh("oversize");
    // Sparse: its size is past the bound, and no byte of it is written or read.
    let file = std::fs::File::create(base.join("pkg/src/big.una")).unwrap();
    file.set_len(MAX_BYTES + 1).unwrap();
    assert!(read(&base).unwrap_err().contains("more than"));
    assert!(read_file(&base.join("pkg/src/big.una"), 1 << 20).is_err());
}

#[test]
fn writing_is_exclusive_and_never_through_a_link() {
    let base = fresh("write");
    let target = base.join("out");
    write_fresh(
        &target,
        &[
            ("ita.una", b"ita x\n".as_slice()),
            ("src/a/b.una", b"module a.b\n".as_slice()),
        ],
        true,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("src/a/b.una")).unwrap(),
        "module a.b\n"
    );
    assert!(
        std::fs::metadata(target.join("ita.una"))
            .unwrap()
            .permissions()
            .readonly()
    );
    // The directory must be fresh, and a file is never written over.
    assert!(write_fresh(&target, &[("ita.una", b"x".as_slice())], false).is_err());
    assert!(write_new_file(&target.join("ita.una"), b"x", false).is_err());
    #[cfg(unix)]
    {
        // A dangling link where the file goes: the link is not followed, nothing is created.
        let aim = base.join("aim.una");
        std::os::unix::fs::symlink(&aim, base.join("dangling.una")).unwrap();
        assert!(write_new_file(&base.join("dangling.una"), b"x", false).is_err());
        assert!(!aim.exists());
        // A link among the directories to make.
        std::fs::create_dir_all(base.join("away")).unwrap();
        std::os::unix::fs::symlink(base.join("away"), base.join("made")).unwrap();
        assert!(make_dirs(&base, &["made", "deeper"]).is_err());
        assert!(!base.join("away/deeper").exists());
    }
    let made = make_dirs(&base, &["new", "deeper"]).unwrap();
    assert!(made.is_dir());
}
