//! A file named to `una fmt` or `una check` takes the edition of the package that holds it
//! (#1175): the nearest `ita.una` or `una.toml` above the file gives the edition, and nothing
//! else; a file with no manifest above it keeps edition 0.1 (ADR 0021).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The module in its canonical form of edition 0.4: the long header wraps (ADR 0125).
const WRAPPED: &str = "module s\n\npub kind Pair[A, B]\n  left: A\n  right: B\n\npub tagged[T: Eq, U](first_value: T, second_value: T, tag: U) -> Pair[List[U],\n  Int\n] = Pair(left: if first_value == second_value then [tag] else [], right: 1)\n";

/// The part of the same declaration that editions 0.1 to 0.3 write on one line.
const JOINED_HEADER: &str = "-> Pair[List[U], Int] = Pair(";

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// A fresh directory under the test's temporary directory.
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    root
}

fn codes(output: &Output) -> Vec<String> {
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned())
        .collect()
}

fn text_of(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_file_named_inside_an_edition_0_4_package_is_canonical_in_that_edition() {
    let root = scratch("named-file-0-4");
    std::fs::write(
        root.join("ita.una"),
        "ita s\n  version 0.1.0\n  edition 0.4\n",
    )
    .unwrap();
    let file = root.join("src/s.una");
    std::fs::write(&file, WRAPPED).unwrap();
    let file_arg = file.to_str().unwrap();

    let check = una(&root, &["fmt", "--check", "--json", file_arg]);
    assert_eq!(check.status.code(), Some(0), "{}", codes(&check).join(","));
    assert!(!codes(&check).contains(&"U7001".to_owned()));
    let checked = una(&root, &["check", "--json", file_arg]);
    assert!(
        !codes(&checked).contains(&"U7001".to_owned()),
        "{:?}",
        codes(&checked)
    );
    // The relative spelling reads the same manifest.
    let relative = una(&root, &["fmt", "--check", "src/s.una"]);
    assert_eq!(relative.status.code(), Some(0));

    let fmt = una(&root, &["fmt", file_arg]);
    assert_eq!(fmt.status.code(), Some(0));
    assert_eq!(text_of(&file), WRAPPED, "una fmt left the file unchanged");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_named_inside_an_edition_0_3_package_still_uses_edition_0_3() {
    let root = scratch("named-file-0-3");
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"s\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n",
    )
    .unwrap();
    let file = root.join("src/s.una");
    std::fs::write(&file, WRAPPED).unwrap();
    let file_arg = file.to_str().unwrap();

    // Edition 0.3 never wraps, so the wrapped layout is not canonical there.
    let check = una(&root, &["fmt", "--check", "--json", file_arg]);
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(codes(&check), ["U7001"]);
    let fmt = una(&root, &["fmt", file_arg]);
    assert_eq!(fmt.status.code(), Some(0));
    assert!(text_of(&file).contains(JOINED_HEADER), "{}", text_of(&file));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_loose_file_with_no_manifest_above_it_stays_in_edition_0_1() {
    let root = scratch("named-file-loose");
    let file = root.join("s.una");
    std::fs::write(&file, WRAPPED).unwrap();
    let file_arg = file.to_str().unwrap();

    let check = una(&root, &["fmt", "--check", "--json", file_arg]);
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(codes(&check), ["U7001"]);
    let fmt = una(&root, &["fmt", file_arg]);
    assert_eq!(fmt.status.code(), Some(0));
    assert!(text_of(&file).contains(JOINED_HEADER), "{}", text_of(&file));
    std::fs::remove_dir_all(&root).unwrap();
}
