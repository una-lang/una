//! The pin of the Z3 built into `una` (ADR 0124 rule 1, #1206): `supply-chain/z3.lock` names the
//! release tag's commit, its tree, the digests of the source tarball and its mirror, and
//! `supply-chain/z3/` holds the recipe. The pin must agree with `verification.lock`, which names
//! the solver version a package is checked with, and the recipe must touch only the shell front
//! end, which is what `shell-library.patch` is for. The test reads files and runs nothing.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The `key = "value"` lines of a lock, up to its first table header.
fn settings(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .take_while(|line| !line.starts_with('['))
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            Some((key.trim().to_string(), value.to_string()))
        })
        .collect()
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[test]
fn the_z3_pin_agrees_with_verification_lock_and_names_its_source() {
    let lock = settings(&read("supply-chain/z3.lock"));
    let solver = settings(&read("verification.lock"));
    assert_eq!(solver["solver"], "z3");
    let version = &lock["version"];
    assert_eq!(
        version, &solver["solver_version"],
        "z3.lock and verification.lock pin one version"
    );
    assert_eq!(lock["tag"], format!("z3-{version}"));
    assert!(is_hex(&lock["commit"], 40), "commit is a git hash");
    assert!(is_hex(&lock["tree"], 40), "tree is a git tree hash");
    assert_ne!(lock["commit"], lock["tree"]);
    assert_eq!(lock["source"], format!("z3-{version}.tar.gz"));
    assert!(is_hex(&lock["source_sha256"], 64), "the tarball's digest");
    assert!(
        is_hex(&lock["source_tar_sha256"], 64),
        "the uncompressed tar's digest"
    );
    assert_ne!(lock["source_sha256"], lock["source_tar_sha256"]);
    // The mirror is a release asset of this repository, named by the version, so a tarball for
    // another version can never be read as this one (the digest is the security boundary).
    assert_eq!(
        lock["mirror"],
        format!(
            "https://github.com/una-lang/una/releases/download/mirror/z3-{version}/z3-{version}.tar.gz"
        )
    );
    // Only the installer still downloads the Z3 project's own binaries, until step 1 ends.
    assert!(
        read("supply-chain/z3.lock").contains("[library]"),
        "the per-target library digests"
    );
}

#[test]
fn the_recipe_builds_a_static_library_without_gmp_and_patches_only_the_shell() {
    let options: Vec<String> = read("supply-chain/z3/cmake-options.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    for required in [
        "CMAKE_BUILD_TYPE=Release",
        "Z3_BUILD_LIBZ3_SHARED=OFF",
        "Z3_USE_LIB_GMP=OFF",
        "Z3_INCLUDE_GIT_HASH=OFF",
        "Z3_INCLUDE_GIT_DESCRIBE=OFF",
        "Z3_BUILD_PYTHON_BINDINGS=OFF",
        "Z3_ENABLE_EXAMPLE_TARGETS=OFF",
        "Z3_BUILD_DOCUMENTATION=OFF",
    ] {
        assert!(
            options.iter().any(|o| o == required),
            "cmake-options.txt lacks {required}"
        );
    }
    for option in &options {
        let (key, value) = option.split_once('=').expect("an option is KEY=VALUE");
        assert!(!key.is_empty() && !value.is_empty(), "{option}");
        assert!(!key.contains(char::is_whitespace), "{option}");
        assert!(
            !(key.contains("BINDINGS") && value == "ON"),
            "{option}: no bindings are built"
        );
        assert_ne!(key, "Z3_USE_LIB_GMP", "{option}");
    }
    // The patch adds the library target and the renamed entry, and touches only src/shell/.
    let patch = read("supply-chain/z3/shell-library.patch");
    let files: Vec<&str> = patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .collect();
    assert_eq!(files, ["src/shell/CMakeLists.txt", "src/shell/main.cpp"]);
    assert!(
        patch.contains("add_library(shell_lib STATIC"),
        "the library target"
    );
    assert!(patch.contains("OUTPUT_NAME z3shell"), "libz3shell.a");
    assert!(
        patch.contains("extern \"C\" int una_z3_shell_main(int argc, char ** argv)"),
        "the renamed entry"
    );
    assert!(
        patch.contains("#ifdef UNA_Z3_SHELL_LIBRARY"),
        "the entry is renamed only for the library"
    );
    assert!(
        !patch
            .lines()
            .any(|line| line.starts_with('-') && !line.starts_with("---")),
        "the patch removes nothing"
    );
}

#[test]
fn the_build_script_reads_the_lock_and_the_recipe() {
    let script = read("scripts/build-z3.sh");
    for needed in [
        "supply-chain/z3.lock",
        "cmake-options.txt",
        "shell-library.patch",
        "--reproducible",
        "libz3shell.a",
        "-ffile-prefix-map=",
    ] {
        assert!(
            script.contains(needed),
            "scripts/build-z3.sh does not mention {needed}"
        );
    }
    // The digest is checked before anything is extracted or compiled (rule 1).
    let check = script.find("SHA-256 mismatch for $source").unwrap();
    let extract = script.find("tar -xzf").unwrap();
    assert!(
        check < extract,
        "the digest check comes before the extraction"
    );
    assert!(!script.contains("-ffp-contract"), "not adopted (rule 1)");
    let exec = Path::new(&root()).join("scripts/build-z3.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            fs::metadata(exec).unwrap().permissions().mode() & 0o111 != 0,
            "executable"
        );
    }
}
