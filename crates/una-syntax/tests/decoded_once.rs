//! A guard on the single decoding of project files (#2252, #2292, #2312): no caller decodes a
//! decoded text, `.text()` of a `SourceFile`, a second time, which would read a lone `\r`
//! before a `\r\n` as one more line end and measure the size bound on the decoded text.
//!
//! The type system holds most of it: a reader of `ita.una`, `ita.lock`, `itaty.una` or
//! `mirrors.una` takes a `SourceFile`, a `&String`, a `&Cow<str>` or a `&'static str` (a
//! literal), and no shorter-lived `&str`, so `parse_ita_lock(file.text())` does not compile,
//! nor `let t = file.text(); parse_ita_lock(t)`, under any import of the reader (the
//! `compile_fail` examples of `una_syntax::ita::Text`). What a type cannot refuse is a decoded
//! text wrapped again by hand: `Text::Written(..)`, `SourceFile::new(..)` or `SourceDb::add(..)`
//! of `.text()`, directly or through a variable bound to it. This test reads the sources of the
//! workspace's crates and of the fuzz targets and refuses those.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

/// The calls that decode or wrap a text as written, and the readers themselves.
const READERS: &[&str] = &[
    "parse_ita_manifest(",
    "parse_ita_lock(",
    "parse_workspace(",
    "ita::parse_manifest(",
    "ita::parse_lock(",
    "ita::lines(",
    "ita::format(",
    "mirrors::parse(",
    "Written(",
    "Text::from(",
    "SourceFile::new(",
    ".add(",
];

/// The names that `source` binds to a decoded text, `let name = <..>.text();` (or with
/// `.as_bytes()`, `.to_owned()` or `.to_string()` after it), each with the byte range of its
/// scope: from the binding to the first line indented less than it, which closes its block.
fn decoded_names(source: &str) -> Vec<(&str, std::ops::Range<usize>)> {
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        starts.push((offset, line));
        offset += line.len();
    }
    let mut names = Vec::new();
    for (index, &(start, line)) in starts.iter().enumerate() {
        let Some(rest) = line.trim_start().strip_prefix("let ") else {
            continue;
        };
        let rest = rest.strip_prefix("mut ").unwrap_or(rest);
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let name = name.split(':').next().unwrap_or("").trim();
        let value = value.trim().trim_end_matches(';');
        let decoded = [
            ".text()",
            ".text().as_bytes()",
            ".text().to_owned()",
            ".text().to_string()",
        ]
        .iter()
        .any(|end| value.ends_with(end));
        if !decoded
            || name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            continue;
        }
        let end = starts[index + 1..]
            .iter()
            .find(|(_, next)| !next.trim().is_empty() && indent(next) < indent(line))
            .map_or(source.len(), |&(next, _)| next);
        names.push((name, start..end));
    }
    names
}

/// Whether `argument` names `name` as a whole word.
fn names(argument: &str, name: &str) -> bool {
    argument.match_indices(name).any(|(at, _)| {
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let before = argument.as_bytes().get(at.wrapping_sub(1)).copied();
        let after = argument.as_bytes().get(at + name.len()).copied();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

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

/// The 1-based lines of `source` where a decoded text is read or decoded again.
fn offenders(source: &str) -> Vec<usize> {
    let decoded = decoded_names(source);
    let mut lines = Vec::new();
    for reader in READERS {
        for (at, _) in source.match_indices(reader) {
            // `parse_workspace(` also ends `ita::parse_workspace(`; a definition is not a call.
            let before = &source[..at];
            if before.ends_with("fn ") || before.ends_with("pub fn ") {
                continue;
            }
            let line_start = before.rfind('\n').map_or(0, |at| at + 1);
            if source[line_start..at].trim_start().starts_with("//") {
                continue;
            }
            let argument = argument(source, at + reader.len());
            let through = decoded
                .iter()
                .any(|(name, scope)| scope.contains(&at) && names(argument, name));
            if argument.contains(".text()") || through {
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
        offenders("let m = mirrors::parse(Text::Written(file.text()));\n"),
        [1]
    );
    assert_eq!(offenders("let m = parse(Written(file.text()));\n"), [1]);
    // Through a variable, and decoded again into a file or a database.
    assert_eq!(
        offenders("let text = file.text();\nlet (m, _) = parse_manifest(Text::Written(text));\n"),
        [2]
    );
    assert_eq!(
        offenders("let text = file.text();\nlet id = sources.add(\"ita.una\", text.as_bytes());\n"),
        [2]
    );
    assert_eq!(
        offenders("let again = SourceFile::new(\"ita.una\", file.text().as_bytes());\n"),
        [1]
    );
    assert!(offenders("let (lock, _) = parse_ita_lock(sources.file(id));\n").is_empty());
    assert!(
        offenders("let (m, _) = parse_manifest(package.sources.file(id).text());\n").is_empty()
    );
    assert!(offenders("pub fn parse_workspace(text: &str) { text.text() }\n").is_empty());
    // A name that only contains a decoded name is another name.
    assert!(
        offenders("let t = file.text();\nlet id = sources.add(\"a\", tt.as_bytes());\n").is_empty()
    );
    // Out of its block, the name is another binding; a comment is not a call.
    assert!(
        offenders("fn a() {\n    let text = file.text();\n}\nfn b(text: &str) {\n    let id = sources.add(\"a\", text.as_bytes());\n}\n")
            .is_empty()
    );
    assert!(offenders("/// parse_ita_lock(file.text())\n").is_empty());
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
