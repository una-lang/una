//! The case lists of the tests that run one test per case (#1652): the runnable programs of the
//! native differential (`native.rs`), the targets of the determinism test (`cli.rs`) and the
//! corpus of the Core IR export test (`core_symbols.rs`). The build script of
//! `crates/una-test-cases` includes this file to generate one test per entry, and each test file
//! includes it again to check that the generated list is the one on disk, so that a case that is
//! added, removed or renamed without a rebuild fails a test rather than being skipped.
//!
//! Std only: a build script includes it. Every list is in path order and holds paths relative to
//! the repository root `root`.

// Each includer uses its own subset of these helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// A runnable program of the native differential: its path relative to the repository root,
/// and the grants it runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The package directory or single file, relative to the repository root.
    pub path: String,
    /// The `--grant` arguments of its runs.
    pub grants: Vec<String>,
}

/// The entries of `dir` that are not hidden, sorted; none when it cannot be read.
pub fn sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    entries.retain(|e| {
        !e.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    });
    entries.sort();
    entries
}

/// The files with `extension` under `dir`, recursively, in path order.
pub fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    if dir.is_file() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in sorted(dir) {
        if entry.is_dir() {
            out.extend(files(&entry, extension));
        } else if entry.extension().is_some_and(|e| e == extension) {
            out.push(entry);
        }
    }
    out
}

/// `path` relative to `root`, as text.
fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The programs of the suites that may run (`native.rs`): the examples, the conformance cases
/// that expect output or grant a capability, and the adversarial programs of the reviews that
/// have `pub main`.
pub fn native_programs(root: &Path) -> Vec<Program> {
    let mut programs = Vec::new();
    for entry in sorted(&root.join("examples")) {
        programs.push(Program {
            path: relative(&entry, root),
            grants: Vec::new(),
        });
    }
    for case in files(&root.join("tests/conformance"), "una") {
        let text = std::fs::read_to_string(&case).unwrap_or_default();
        let directives: Vec<&str> = text.lines().take_while(|l| l.starts_with("-- ")).collect();
        if !directives
            .iter()
            .any(|d| d.starts_with("-- expect: run-output") || d.starts_with("-- grant: "))
        {
            continue;
        }
        let grants = directives
            .iter()
            .filter_map(|d| d.strip_prefix("-- grant: "))
            .map(|g| (*g).to_owned())
            .collect();
        // A case of several modules is the directory above its `src`, described by `una.toml`
        // or, from edition 0.4, by `ita.una` (ADR 0122).
        let path = match case
            .ancestors()
            .find(|a| a.join("una.toml").is_file() || a.join("ita.una").is_file())
        {
            Some(package) if package.starts_with(root.join("tests/conformance")) => {
                package.to_path_buf()
            }
            _ => case,
        };
        programs.push(Program {
            path: relative(&path, root),
            grants,
        });
    }
    for review in sorted(&root.join("docs/reviews")) {
        let mut dirs = vec![review.join("adversarial")];
        dirs.extend(sorted(&review).into_iter().map(|d| d.join("adversarial")));
        for dir in dirs.into_iter().filter(|d| d.is_dir()) {
            for program in sorted(&dir) {
                let text: String = files(&program, "una")
                    .iter()
                    .chain(std::iter::once(&program))
                    .filter_map(|f| std::fs::read_to_string(f).ok())
                    .collect();
                if !text.contains("pub main(") {
                    continue;
                }
                // A single file has no `[run] grants`: it runs with the grants that need no
                // path.
                let grants = if program.is_file() {
                    vec!["console".to_owned(), "clock".to_owned(), "env".to_owned()]
                } else {
                    Vec::new()
                };
                programs.push(Program {
                    path: relative(&program, root),
                    grants,
                });
            }
        }
    }
    programs.sort_by(|a, b| a.path.cmp(&b.path));
    programs.dedup_by(|a, b| a.path == b.path);
    programs
}

/// The areas whose cases the determinism test checks (`compiler/testing.md §4`): every area but
/// `verification`.
pub const DETERMINISM_AREAS: [&str; 8] = [
    "syntax",
    "fmt",
    "names",
    "project",
    "types",
    "authority",
    "security",
    "runtime",
];

/// The targets of the determinism test (`cli.rs`): every example, then every entry of the
/// areas of [`DETERMINISM_AREAS`], in that order, each sorted.
pub fn determinism_targets(root: &Path) -> Vec<String> {
    let names = |dir: &Path| -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|r| {
                r.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| !name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    };
    let mut targets: Vec<String> = names(&root.join("examples"))
        .into_iter()
        .map(|name| format!("examples/{name}"))
        .collect();
    for area in DETERMINISM_AREAS {
        targets.extend(
            names(&root.join("tests/conformance").join(area))
                .into_iter()
                .map(|name| format!("tests/conformance/{area}/{name}")),
        );
    }
    targets
}

/// The corpus of the Core IR export test (`core_symbols.rs`): every package (a directory with
/// `una.toml`) and every other `.una` file under `tests/conformance`, then under `examples`.
pub fn core_corpus(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        if dir.join("una.toml").is_file() {
            out.push(relative(dir, root));
            return;
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "una") {
                out.push(relative(&path, root));
            }
        }
    }
    let mut out = Vec::new();
    walk(&root.join("tests/conformance"), root, &mut out);
    walk(&root.join("examples"), root, &mut out);
    out
}
