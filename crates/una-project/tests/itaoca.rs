//! Itaoca, the store of fetched packages (ADR 0122 rule 30): stored once by digest, read-only,
//! and verified on every load.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;

use pretty_assertions::assert_eq;
use una_project::content::{File, archive, digest_of};
use una_project::itaoca::Itaoca;

fn store(name: &str) -> Itaoca {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("itaoca").join(name);
    if root.exists() {
        make_writable(&root);
        std::fs::remove_dir_all(&root).unwrap();
    }
    Itaoca { root }
}

fn make_writable(dir: &std::path::Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            make_writable(&path);
        } else {
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
    }
}

fn package() -> Vec<File> {
    vec![
        File {
            path: "ita.una".to_owned(),
            bytes: b"ita demo\n  version 0.1.0\n  edition 0.4\n".to_vec(),
        },
        File {
            path: "src/a/b.una".to_owned(),
            bytes: b"module a.b\n".to_vec(),
        },
    ]
}

#[test]
fn a_package_is_stored_once_and_loads_verified() {
    let itaoca = store("once");
    let bytes = archive(&mut package()).unwrap();
    let digest = itaoca.store(&bytes).unwrap();
    assert_eq!(digest, digest_of(&mut package()).unwrap());
    let loaded = itaoca.load(&digest).unwrap();
    assert_eq!(loaded.len(), 2);
    // Stored files are read-only.
    let dir = itaoca.package_dir(&digest).unwrap();
    assert!(std::fs::metadata(dir.join("src/a/b.una")).unwrap().permissions().readonly());
    // Storing it again verifies what is there.
    assert_eq!(itaoca.store(&bytes).unwrap(), digest);
}

#[test]
fn a_store_tampered_with_does_not_load() {
    let itaoca = store("tampered");
    let digest = itaoca.store(&archive(&mut package()).unwrap()).unwrap();
    let dir = itaoca.package_dir(&digest).unwrap();
    make_writable(&dir);
    std::fs::write(dir.join("src/a/b.una"), "module a.b\n\npub evil() -> Int = 1\n").unwrap();
    assert!(itaoca.load(&digest).unwrap_err().contains("changed after it was fetched"));
    // An extra file, or a link, refuses it too.
    std::fs::write(dir.join("src/a/b.una"), "module a.b\n").unwrap();
    assert!(itaoca.load(&digest).is_ok());
    std::fs::write(dir.join("src/extra.una"), "module extra\n").unwrap();
    assert!(itaoca.load(&digest).is_err());
    std::fs::remove_file(dir.join("src/extra.una")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("ita.una"), dir.join("src/link.una")).unwrap();
        assert!(itaoca.load(&digest).unwrap_err().contains("not a regular file"));
    }
}

#[test]
fn a_digest_that_is_not_one_names_no_directory() {
    let itaoca = store("names");
    assert!(itaoca.package_dir("sha256:../../etc").is_none());
    assert!(itaoca.package_dir("md5:00").is_none());
    assert!(itaoca.load("sha256:xyz").is_err());
}
