//! Reading and writing a package's files with no link followed (ADR 0122 rules 27 and 30):
//! symbolic and hard links, `..`, absolute paths, hidden and foreign files, device names,
//! case-colliding names and oversize files each refuse the package; the bytes are verified once,
//! in memory, so a file changed between its listing and its digest fails the load; and writing
//! goes through a fresh private directory, never through a link.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use una_project::content::{File, MAX_BYTES, archive, digest_of};
use una_project::tree::{
    VerifiedTree, list, locate, make_dirs, read_file, read_verified, store_file, store_tree,
};

const MANIFEST: &str = "ita demo\n  version 0.1.0\n  edition 0.4\n";
const MODULE: &str = "module demo\n";

/// A fresh directory named `name`, with a package of two files in `pkg/`.
fn fresh(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("tree")
        .join(name);
    if dir.exists() {
        make_writable(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
    std::fs::create_dir_all(dir.join("pkg/src")).unwrap();
    std::fs::write(dir.join("pkg/ita.una"), MANIFEST).unwrap();
    std::fs::write(dir.join("pkg/src/demo.una"), MODULE).unwrap();
    dir
}

fn make_writable(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            make_writable(&path);
        } else if meta.is_file() {
            let mut permissions = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
    }
}

/// The files of the package [`fresh`] writes, and their digest.
fn package() -> (Vec<File>, String) {
    let mut files = vec![
        File {
            path: "ita.una".to_owned(),
            bytes: MANIFEST.as_bytes().to_vec(),
        },
        File {
            path: "src/demo.una".to_owned(),
            bytes: MODULE.as_bytes().to_vec(),
        },
    ];
    let digest = digest_of(&mut files).unwrap();
    (files, digest)
}

/// Lists `pkg/` below `base`: how many files, or why it is refused.
fn listed(base: &Path) -> Result<usize, String> {
    let dir = locate(base, &["pkg"])?.ok_or("missing")?;
    list(&dir).map(|listing| listing.len())
}

#[test]
fn a_package_of_regular_files_reads_verified() {
    let base = fresh("plain");
    let (files, digest) = package();
    let dir = locate(&base, &["pkg"]).unwrap().unwrap();
    let tree = read_verified(&dir, &digest).unwrap();
    assert_eq!(tree.files(), files.as_slice());
    assert_eq!(tree.digest(), digest);
    assert_eq!(tree.file("ita.una"), Some(MANIFEST.as_bytes()));
    // Another digest: nothing read is returned.
    let other = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    assert!(read_verified(&dir, other).unwrap_err().contains("changed"));
    // A missing component is absent, not an error.
    assert_eq!(locate(&base, &["nothing", "here"]).unwrap(), None);
    assert_eq!(locate(&base.join("nothing"), &["pkg"]).unwrap(), None);
}

#[test]
fn bytes_changed_between_the_listing_and_the_digest_fail_it() {
    let base = fresh("changed-after-listing");
    let (_, digest) = package();
    let dir = locate(&base, &["pkg"]).unwrap().unwrap();
    // The same length, other bytes, the same file: only the digest can tell.
    let listing = list(&dir).unwrap();
    std::fs::write(dir.join("src/demo.una"), "module evil\n").unwrap();
    let refusal = listing.verify(&digest).unwrap_err();
    assert!(refusal.contains("have the digest"), "{refusal}");
    // Another length: refused as changed.
    std::fs::write(dir.join("src/demo.una"), MODULE).unwrap();
    let listing = list(&dir).unwrap();
    std::fs::write(
        dir.join("src/demo.una"),
        "module demo\n\npub x() -> Int = 1\n",
    )
    .unwrap();
    assert!(listing.verify(&digest).is_err());
    // A file replaced by another one, with the original bytes: another inode, refused. The new
    // file exists before the old one goes, so its inode cannot be the old one's.
    std::fs::write(dir.join("src/demo.una"), MODULE).unwrap();
    let listing = list(&dir).unwrap();
    std::fs::write(base.join("demo.una"), MODULE).unwrap();
    std::fs::rename(base.join("demo.una"), dir.join("src/demo.una")).unwrap();
    assert!(listing.verify(&digest).unwrap_err().contains("changed"));
    // Untouched, it verifies.
    assert!(list(&dir).unwrap().verify(&digest).is_ok());
}

#[cfg(unix)]
#[test]
fn a_file_replaced_by_a_link_after_the_listing_is_refused() {
    use std::os::unix::fs::symlink;
    let base = fresh("link-after-listing");
    let (_, digest) = package();
    let dir = locate(&base, &["pkg"]).unwrap().unwrap();
    // A link to a file with the very same bytes: still refused, never followed.
    std::fs::write(base.join("same.una"), MODULE).unwrap();
    let listing = list(&dir).unwrap();
    std::fs::remove_file(dir.join("src/demo.una")).unwrap();
    symlink(base.join("same.una"), dir.join("src/demo.una")).unwrap();
    assert!(listing.verify(&digest).is_err());
    // A directory above it replaced by a link to a copy: the file is another entry, refused.
    std::fs::remove_file(dir.join("src/demo.una")).unwrap();
    std::fs::write(dir.join("src/demo.una"), MODULE).unwrap();
    let listing = list(&dir).unwrap();
    std::fs::create_dir_all(base.join("copy")).unwrap();
    std::fs::write(base.join("copy/demo.una"), MODULE).unwrap();
    std::fs::rename(dir.join("src"), base.join("src-real")).unwrap();
    symlink(base.join("copy"), dir.join("src")).unwrap();
    assert!(listing.verify(&digest).is_err());
    // And a listing taken now refuses the link outright.
    assert!(list(&dir).unwrap_err().contains("symbolic link"));
}

#[test]
fn components_that_leave_the_base_are_refused() {
    let base = fresh("components");
    for part in ["..", ".", "", "a/b", "/abs", "a\\b", "a\0b"] {
        assert!(locate(&base, &[part]).is_err(), "{part:?}");
        assert!(make_dirs(&base, &[part]).is_err(), "{part:?}");
    }
    // The name a fresh directory or file is moved to is one component too.
    let (mut files, _) = package();
    let tree = VerifiedTree::from_archive(&archive(&mut files).unwrap()).unwrap();
    let parent = make_dirs(&base, &["out"]).unwrap();
    for name in ["..", "a/b", ""] {
        assert!(store_tree(&parent, name, &tree, false).is_err(), "{name:?}");
        assert!(store_file(&parent, name, b"x", false).is_err(), "{name:?}");
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
    assert!(listed(&base).unwrap_err().contains("symbolic link"));
    // A directory that is a symbolic link.
    let base = fresh("dir-link");
    std::fs::create_dir_all(base.join("elsewhere")).unwrap();
    std::fs::write(base.join("elsewhere/x.una"), "module x\n").unwrap();
    symlink(base.join("elsewhere"), base.join("pkg/src/sub")).unwrap();
    assert!(listed(&base).unwrap_err().contains("symbolic link"));
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
    assert!(listed(&base).unwrap_err().contains("hard link"));
}

#[test]
fn hidden_foreign_and_colliding_entries_refuse_the_package() {
    let base = fresh("hidden");
    std::fs::write(base.join("pkg/src/.DS_Store"), "x").unwrap();
    assert!(listed(&base).is_err());
    let base = fresh("foreign");
    std::fs::write(base.join("pkg/src/lib.c"), "int x;").unwrap();
    assert!(
        listed(&base)
            .unwrap_err()
            .contains("not a file a package holds")
    );
    let base = fresh("device-name");
    std::fs::write(base.join("pkg/src/con.una"), "module con\n").unwrap();
    assert!(listed(&base).unwrap_err().contains("device name"));
    // Two names equal ignoring case, where the file system keeps both.
    let base = fresh("case");
    std::fs::write(base.join("pkg/src/Demo.una"), "module Demo\n").unwrap();
    if std::fs::read_dir(base.join("pkg/src")).unwrap().count() == 2 {
        let dir = locate(&base, &["pkg"]).unwrap().unwrap();
        let (_, digest) = package();
        assert!(
            list(&dir)
                .unwrap()
                .verify(&digest)
                .unwrap_err()
                .contains("ignoring case")
        );
    }
}

#[test]
fn an_oversize_file_is_refused_before_it_is_read() {
    let base = fresh("oversize");
    // Sparse: its size is past the bound, and no byte of it is written or read.
    let file = std::fs::File::create(base.join("pkg/src/big.una")).unwrap();
    file.set_len(MAX_BYTES + 1).unwrap();
    assert!(listed(&base).unwrap_err().contains("more than"));
    assert!(read_file(&base.join("pkg/src/big.una"), 1 << 20).is_err());
}

#[test]
fn writing_goes_through_a_fresh_private_directory() {
    let base = fresh("write");
    let mut files = vec![
        File {
            path: "ita.una".to_owned(),
            bytes: b"ita x\n".to_vec(),
        },
        File {
            path: "src/a/b.una".to_owned(),
            bytes: b"module a.b\n".to_vec(),
        },
    ];
    let digest = digest_of(&mut files).unwrap();
    let tree = VerifiedTree::from_archive(&archive(&mut files).unwrap()).unwrap();
    let parent = make_dirs(&base, &["store"]).unwrap();
    store_tree(&parent, "out", &tree, true).unwrap();
    let target = parent.path().join("out");
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().mode() & 0o777;
        assert_eq!(mode(&target.join("ita.una")), 0o444);
        assert_eq!(mode(&target), 0o755);
        assert_eq!(mode(&target.join("src/a")), 0o755);
    }
    // What was written reads back verified, and nothing else is left in the parent.
    assert_eq!(read_verified(&target, &digest).unwrap(), tree);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
    // A directory already there is never written over.
    assert!(store_tree(&parent, "out", &tree, false).is_err());
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
    // Nor is an empty one: the rename refuses anything at the name.
    std::fs::create_dir(parent.path().join("empty")).unwrap();
    assert!(store_tree(&parent, "empty", &tree, false).is_err());
    assert_eq!(
        std::fs::read_dir(parent.path().join("empty"))
            .unwrap()
            .count(),
        0
    );
    std::fs::remove_dir(parent.path().join("empty")).unwrap();
    // A single file replaces the name atomically.
    store_file(&parent, "doc.provenance", b"one", false).unwrap();
    store_file(&parent, "doc.provenance", b"two", false).unwrap();
    assert_eq!(
        std::fs::read(parent.path().join("doc.provenance")).unwrap(),
        b"two"
    );
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 2);
    #[cfg(unix)]
    {
        // A dangling link where the file goes: the link is replaced, not followed.
        let aim = base.join("aim.provenance");
        std::os::unix::fs::symlink(&aim, parent.path().join("dangling.provenance")).unwrap();
        store_file(&parent, "dangling.provenance", b"x", false).unwrap();
        assert!(!aim.exists());
        assert!(
            std::fs::symlink_metadata(parent.path().join("dangling.provenance"))
                .unwrap()
                .is_file()
        );
        // A link among the directories to make.
        std::fs::create_dir_all(base.join("away")).unwrap();
        std::os::unix::fs::symlink(base.join("away"), base.join("made")).unwrap();
        assert!(make_dirs(&base, &["made", "deeper"]).is_err());
        assert!(!base.join("away/deeper").exists());
        // A parent that is a link is never opened, so nothing is written through it.
        assert!(make_dirs(&base, &["made"]).is_err());
        assert_eq!(std::fs::read_dir(base.join("away")).unwrap().count(), 0);
    }
    let made = make_dirs(&base, &["new", "deeper"]).unwrap();
    assert!(made.path().is_dir());
}
