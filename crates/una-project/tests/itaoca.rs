//! Itaoca, the store of fetched packages (ADR 0122 rule 30): stored once by digest, read-only,
//! and verified on every load.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;

use pretty_assertions::assert_eq;
use una_project::content::{File, archive, digest_of};
use una_project::itaoca::Itaoca;

fn store(name: &str) -> Itaoca {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("itaoca")
        .join(name);
    if root.exists() {
        make_writable(&root);
        std::fs::remove_dir_all(&root).unwrap();
    }
    Itaoca { root }
}

fn make_writable(dir: &std::path::Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        // The entry itself, never what a link names: a link whose target is gone has no
        // metadata to follow, and a link to a directory is not walked (#2369).
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            make_writable(&path);
        } else {
            let mut permissions = metadata.permissions();
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
    assert_eq!(loaded.files().len(), 2);
    assert_eq!(loaded.digest(), digest);
    // Stored files are read-only.
    let dir = itaoca.package_dir(&digest).unwrap();
    assert!(
        std::fs::metadata(dir.join("src/a/b.una"))
            .unwrap()
            .permissions()
            .readonly()
    );
    // Storing it again verifies what is there.
    assert_eq!(itaoca.store(&bytes).unwrap(), digest);
}

#[test]
fn a_store_tampered_with_does_not_load() {
    let itaoca = store("tampered");
    let digest = itaoca.store(&archive(&mut package()).unwrap()).unwrap();
    let dir = itaoca.package_dir(&digest).unwrap();
    make_writable(&dir);
    std::fs::write(
        dir.join("src/a/b.una"),
        "module a.b\n\npub evil() -> Int = 1\n",
    )
    .unwrap();
    assert!(
        itaoca
            .load(&digest)
            .unwrap_err()
            .contains("were changed after they were written")
    );
    // An extra file, or a link, refuses it too.
    std::fs::write(dir.join("src/a/b.una"), "module a.b\n").unwrap();
    assert!(itaoca.load(&digest).is_ok());
    std::fs::write(dir.join("src/extra.una"), "module extra\n").unwrap();
    assert!(itaoca.load(&digest).is_err());
    std::fs::remove_file(dir.join("src/extra.una")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("ita.una"), dir.join("src/link.una")).unwrap();
        assert!(itaoca.load(&digest).unwrap_err().contains("symbolic link"));
        std::fs::remove_file(dir.join("src/link.una")).unwrap();
        // A hard link to a file outside the store refuses it too.
        let outside = dir.parent().unwrap().join("outside.una");
        std::fs::write(&outside, "module outside\n").unwrap();
        std::fs::hard_link(&outside, dir.join("src/hard.una")).unwrap();
        assert!(itaoca.load(&digest).unwrap_err().contains("hard link"));
        std::fs::remove_file(dir.join("src/hard.una")).unwrap();
        std::fs::remove_file(&outside).unwrap();
        assert!(itaoca.load(&digest).is_ok());
        // So does the package directory itself replaced by a link to a copy of it.
        let copy = dir.parent().unwrap().join("copy");
        std::fs::rename(&dir, &copy).unwrap();
        std::os::unix::fs::symlink(&copy, &dir).unwrap();
        assert!(itaoca.load(&digest).unwrap_err().contains("symbolic link"));
        std::fs::remove_file(&dir).unwrap();
        std::fs::rename(&copy, &dir).unwrap();
        assert!(itaoca.load(&digest).is_ok());
    }
}

#[test]
fn storing_never_writes_through_a_link() {
    let itaoca = store("through-link");
    #[cfg(unix)]
    {
        // `packages` is a link to a directory elsewhere: nothing is written there.
        let elsewhere = itaoca.root.with_file_name("through-link-elsewhere");
        let _ = std::fs::remove_dir_all(&elsewhere);
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(&itaoca.root).unwrap();
        std::os::unix::fs::symlink(&elsewhere, itaoca.root.join("packages")).unwrap();
        let refusal = itaoca.store(&archive(&mut package()).unwrap()).unwrap_err();
        assert!(refusal.contains("symbolic link"), "{refusal}");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    }
}

#[cfg(unix)]
#[test]
fn a_store_root_that_is_a_link_is_refused() {
    let itaoca = store("root-link");
    let real = itaoca.root.with_file_name("root-link-real");
    if real.exists() {
        make_writable(&real);
        std::fs::remove_dir_all(&real).unwrap();
    }
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, &itaoca.root).unwrap();
    let bytes = archive(&mut package()).unwrap();
    let refusal = itaoca.store(&bytes).unwrap_err();
    assert!(refusal.contains("symbolic link"), "{refusal}");
    assert_eq!(std::fs::read_dir(&real).unwrap().count(), 0);
    // A package stored in the real directory does not load through the link either.
    let direct = Itaoca { root: real.clone() };
    let digest = direct.store(&bytes).unwrap();
    assert!(itaoca.load(&digest).unwrap_err().contains("symbolic link"));
    assert!(itaoca.contains(&digest).is_err());
    std::fs::remove_file(&itaoca.root).unwrap();
}

#[test]
fn storing_leaves_no_fresh_directory_behind() {
    let itaoca = store("no-leftover");
    let digest = itaoca.store(&archive(&mut package()).unwrap()).unwrap();
    let parent = itaoca.package_dir(&digest).unwrap();
    let names: Vec<String> = std::fs::read_dir(parent.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [digest.strip_prefix("sha256:").unwrap()]);
}

#[test]
fn a_digest_that_is_not_one_names_no_directory() {
    let itaoca = store("names");
    assert!(itaoca.package_dir("sha256:../../etc").is_none());
    assert!(itaoca.package_dir("md5:00").is_none());
    assert!(itaoca.load("sha256:xyz").is_err());
}
