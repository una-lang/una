//! The verified copy that `build.rs` links (#1925): the linker must read the bytes whose digest
//! was checked, from a directory that holds them alone. A library planted beside the given one
//! must never be in the link's search directory, and a change of the given file after the check
//! must never reach the link. The tests run the build script's own code (`stage.rs`) on
//! temporary directories; "the link" here is what the directives name, the search directory and
//! the `lib<name>.a` in it, which is what rustc and the linker read.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

#[path = "../stage.rs"]
#[allow(dead_code)]
mod stage;

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// A fresh directory for one test, under the target directory of the tests.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("una-z3-stage-{name}"));
    if dir.exists() {
        make_writable(&dir);
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The copy is read-only; a test that reuses a directory makes it writable first.
fn make_writable(dir: &Path) {
    for entry in walk(dir) {
        let mut permissions = fs::metadata(&entry).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&entry, permissions).unwrap();
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        }
        out.push(path);
    }
    out
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The given library: `<root>/cache/lib/libz3shell.a`, with some bytes.
fn library(root: &Path, bytes: &[u8]) -> PathBuf {
    let lib = root.join("cache/lib");
    fs::create_dir_all(&lib).unwrap();
    let path = lib.join("libz3shell.a");
    fs::write(&path, bytes).unwrap();
    path
}

/// The search directories and libraries the directives name.
fn link_inputs(directives: &[String]) -> (Vec<PathBuf>, Vec<String>) {
    let mut search = Vec::new();
    let mut libs = Vec::new();
    for directive in directives {
        if let Some(dir) = directive.strip_prefix("cargo:rustc-link-search=") {
            let dir = dir
                .strip_prefix("native=")
                .expect("a native search directory");
            search.push(PathBuf::from(dir));
        } else if let Some(lib) = directive.strip_prefix("cargo:rustc-link-lib=") {
            libs.push(lib.to_owned());
        } else {
            panic!("an unexpected directive: {directive}");
        }
    }
    (search, libs)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn a_library_beside_the_given_one_is_never_in_the_link_search() {
    let root = scratch("planted");
    let source = library(&root, b"!<arch>\nthe pinned bytes\n");
    let beside = source.parent().unwrap();
    // What code with write access to the cache could plant for `-lstdc++` (Linux) or `-lc++`
    // (macOS), which the linker looks up in each search directory before the toolchain's.
    for planted in [
        "libstdc++.a",
        "libstdc++.so",
        "libc++.a",
        "libc++.tbd",
        "libc++.dylib",
    ] {
        fs::write(beside.join(planted), b"planted").unwrap();
    }
    let want = sha256(b"!<arch>\nthe pinned bytes\n");
    let staged = stage::stage(&source, &root.join("out/z3-link"), Some(&want)).unwrap();
    for os in ["linux", "macos"] {
        let directives = stage::directives(&staged, Some(os)).unwrap();
        let (search, libs) = link_inputs(&directives);
        assert_eq!(
            search,
            [root.join("out/z3-link")],
            "{os}: one search directory"
        );
        for dir in &search {
            assert_ne!(
                dir, beside,
                "{os}: the given library's directory is never searched"
            );
            assert_eq!(names(dir), ["libz3shell.a"], "{os}: the copy alone");
        }
        assert_eq!(libs[0], "static=z3shell");
    }
}

#[test]
fn a_change_of_the_given_file_after_the_check_never_reaches_the_link() {
    let root = scratch("changed");
    let pinned = b"!<arch>\nthe pinned bytes\n";
    let source = library(&root, pinned);
    let staged = stage::stage(&source, &root.join("out/z3-link"), Some(&sha256(pinned))).unwrap();
    // The file changes after the check, before rustc and the linker read the library.
    fs::write(&source, b"!<arch>\nother bytes\n").unwrap();
    let directives = stage::directives(&staged, Some("linux")).unwrap();
    let (search, libs) = link_inputs(&directives);
    let name = libs[0].strip_prefix("static=").unwrap();
    let linked = fs::read(search[0].join(format!("lib{name}.a"))).unwrap();
    assert_eq!(
        sha256(&linked),
        sha256(pinned),
        "the link reads the checked bytes"
    );
    assert_eq!(staged.sha256, sha256(pinned));
    assert_eq!(staged.path(), search[0].join("libz3shell.a"));
    assert!(
        fs::metadata(staged.path())
            .unwrap()
            .permissions()
            .readonly(),
        "the copy is read-only"
    );
}

#[test]
fn what_an_earlier_build_left_in_the_directory_is_removed() {
    let root = scratch("stale");
    let source = library(&root, b"bytes");
    let dir = root.join("out/z3-link");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("libstdc++.a"), b"left").unwrap();
    fs::write(dir.join("libz3shell.a"), b"old copy").unwrap();
    make_writable(&dir);
    let staged = stage::stage(&source, &dir, None).unwrap();
    assert_eq!(names(&dir), ["libz3shell.a"]);
    assert_eq!(fs::read(staged.path()).unwrap(), b"bytes");
    // A second build over the read-only copy of the first.
    let again = stage::stage(&source, &dir, None).unwrap();
    assert_eq!(again.sha256, sha256(b"bytes"));
    assert_eq!(names(&dir), ["libz3shell.a"]);
}

#[test]
fn a_digest_mismatch_fails_and_names_the_given_file() {
    let root = scratch("mismatch");
    let source = library(&root, b"bytes");
    let want = "0".repeat(64);
    let err = stage::stage(&source, &root.join("out/z3-link"), Some(&want)).unwrap_err();
    assert_eq!(
        err,
        format!(
            "SHA-256 mismatch for {}: UNA_EMBED_Z3_SHA256 is {want}, the file is {}",
            source.display(),
            sha256(b"bytes")
        )
    );
}

#[test]
fn a_development_build_records_the_digest_without_a_check() {
    let root = scratch("dev");
    let source = library(&root, b"dev bytes");
    let staged = stage::stage(&source, &root.join("out/z3-link"), None).unwrap();
    assert_eq!(staged.sha256, sha256(b"dev bytes"));
}

#[test]
fn the_library_name_and_the_target_are_checked() {
    let root = scratch("names");
    for bad in ["z3shell.a", "libz3shell.so", "lib.a"] {
        let path = root.join(bad);
        fs::write(&path, b"x").unwrap();
        let err = stage::stage(&path, &root.join("out"), None).unwrap_err();
        assert!(err.contains("must be named lib<name>.a"), "{bad}: {err}");
    }
    let source = library(&root, b"x");
    let staged = stage::stage(&source, &root.join("out/z3-link"), None).unwrap();
    let macos = stage::directives(&staged, Some("macos")).unwrap();
    assert_eq!(macos[2], "cargo:rustc-link-lib=c++");
    let linux = stage::directives(&staged, Some("linux")).unwrap();
    assert_eq!(linux[2], "cargo:rustc-link-lib=static:-bundle=stdc++");
    let err = stage::directives(&staged, Some("windows")).unwrap_err();
    assert_eq!(
        err,
        "una-z3 links on macOS and Linux only, not Some(\"windows\")"
    );
}
