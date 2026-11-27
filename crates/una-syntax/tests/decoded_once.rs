//! A guard on the single decoding of project files (#2252, #2292): no caller passes a decoded
//! text, `.text()` of a `SourceFile`, to a reader of `ita.una`, `ita.lock`, `itaty.una` or
//! `mirrors.una`. Such a call compiles, since a `&str` is a text as written, and decodes the
//! file a second time, which reads a lone `\r` before a `\r\n` as one more line end and
//! measures the size bound on the decoded text. A caller passes the `SourceFile` itself.
//!
//! The guard reads the sources of the workspace's crates and of the fuzz targets, and refuses
//! every call of a reader whose argument holds `.text()`. The TOML readers of `una.toml` and
//! `verification.lock` (`una_project::parse_manifest` and `parse_lock`, called unqualified)
//! never decode through these readers and are not named here.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

/// The calls of the project-file readers, as they are written: each qualified form that names
/// them alone, and the entries that make a text as written.
const READERS: &[&str] = &[
    "parse_ita_manifest(",
    "parse_ita_lock(",
    "parse_workspace(",
    "ita::parse_manifest(",
    "ita::parse_lock(",
    "ita::lines(",
    "ita::format(",
    "mirrors::parse(",
    "Text::Written(",
    "Text::from(",
];

/// The argument list of the call whose `(` ends at `open` (exclusive), up to its matching
/// `)`, across lines. Parentheses inside text literals are rare in these calls and only make
/// the argument longer, which never hides a `.text()`.
fn argument(source: &str, open: usize) -> &str {
    let mut depth = 1usize;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open..open + offset];
                }
            }
            _ => {}
        }
    }
    &source[open..]
}

/// The 1-based lines of `source` where a reader is called on a decoded text.
fn offenders(source: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    for reader in READERS {
        for (at, _) in source.match_indices(reader) {
            // `parse_workspace(` also ends `ita::parse_workspace(`; a definition is not a call.
            let before = &source[..at];
            if before.ends_with("fn ") || before.ends_with("pub fn ") {
                continue;
            }
            if argument(source, at + reader.len()).contains(".text()") {
                lines.push(before.matches('\n').count() + 1);
            }
        }
    }
    lines.sort_unstable();
    lines.dedup();
    lines
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_guard_finds_a_decoded_text_passed_to_a_reader() {
    assert_eq!(
        offenders("let (lock, _) = parse_ita_lock(sources.file(id).text());\n"),
        [1]
    );
    assert_eq!(
        offenders("x\nlet read = ita::parse_manifest(\n    package.sources.file(id).text(),\n);\n"),
        [2]
    );
    assert_eq!(
        offenders("let m = mirrors::parse(&Text::Written(file.text()));\n"),
        [1]
    );
    assert!(offenders("let (lock, _) = parse_ita_lock(sources.file(id));\n").is_empty());
    assert!(
        offenders("let (m, _) = parse_manifest(package.sources.file(id).text());\n").is_empty()
    );
    assert!(offenders("pub fn parse_workspace(text: &str) { text.text() }\n").is_empty());
}

#[test]
fn no_reader_of_a_project_file_is_passed_a_decoded_text() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for crate_dir in std::fs::read_dir(root.join("crates")).unwrap() {
        rust_files(&crate_dir.unwrap().path().join("src"), &mut files);
    }
    rust_files(&root.join("fuzz/fuzz_targets"), &mut files);
    files.sort();
    assert!(
        files.len() > 100,
        "the sources were not found under {}",
        root.display()
    );
    let mut found = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for line in offenders(&source) {
            found.push(format!("{}:{line}", file.display()));
        }
    }
    assert!(
        found.is_empty(),
        "a project-file reader is passed `.text()`, which decodes the file twice; pass the \
         `SourceFile` itself (#2252, #2292):\n{}",
        found.join("\n")
    );
}
