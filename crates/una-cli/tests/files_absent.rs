//! `Files` below a directory that does not exist (`spec/14 §3` rule 2, #1459): the reference
//! interpreter answers `NotFound` itself, and the world's file operations are given no path.
//! A recording world, the machine's file system otherwise, keeps every path a read or a write
//! is given; the native runtime's own test of the same cases is in `una-rt` (`host.rs`), and
//! `tests/native.rs` compares the two engines' outcomes on the disk.

// Tests may unwrap freely (compiler/conventions.md §1). Links need Unix.
#![cfg(unix)]
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};

use una_interp::{Absence, FileFault, Grant, Host, Limits, NotText, World};

/// The machine's file system, as the world of `una run` reaches it, keeping the printed lines
/// and each path that a file operation is given.
#[derive(Default)]
struct Recording {
    lines: Vec<String>,
    operations: Vec<(&'static str, PathBuf)>,
}

impl World for Recording {
    fn print(&mut self, line: &str) {
        self.lines.push(line.to_owned());
    }
    fn now(&mut self) -> i64 {
        0
    }
    fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
        Ok(None)
    }
    fn read_file(&mut self, path: &Path) -> Result<String, FileFault> {
        self.operations.push(("read", path.to_path_buf()));
        std::fs::read_to_string(path).map_err(fault)
    }
    fn write_file(&mut self, path: &Path, content: &str) -> Result<(), FileFault> {
        self.operations.push(("write", path.to_path_buf()));
        let exists = std::fs::symlink_metadata(path).is_ok();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(exists)
            .create_new(!exists)
            .open(path)
            .map_err(fault)?;
        file.write_all(content.as_bytes()).map_err(fault)
    }
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }
    fn is_link(&mut self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    }
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        una_interp::disk_step(directory, name)
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        una_interp::disk_absent(path)
    }
}

fn fault(error: std::io::Error) -> FileFault {
    if error.kind() == std::io::ErrorKind::NotFound {
        FileFault::NotFound
    } else {
        FileFault::Io
    }
}

/// A program that performs each `(operation, path)` of `cases` in order and prints
/// `<index> <operation>: <outcome>`.
fn program(cases: &[(&str, &str)]) -> String {
    let mut program = String::from(
        "module main\n\ndescribe(failure: FileError) -> Text\n=\n  when failure\n    is NotFound -> \"not found\"\n    is Denied -> \"denied\"\n    is Io -> \"io\"\n\nshow(path: Text) -> Text\n  uses @files: Files[read]\n=\n  let content = attempt @files.read(path)\n  when content\n    is Ok -> \"read\"\n    is Err(failure) -> describe(failure)\n\nsave(path: Text) -> Text\n  uses @files: Files[write]\n=\n  let saved = attempt @files.write(path, content: \"entry\")\n  when saved\n    is Ok -> \"written\"\n    is Err(failure) -> describe(failure)\n\npub main() -> Unit\n  uses @console: Console[print]\n  uses @files: Files[read, write]\n=\n",
    );
    for (index, (operation, path)) in cases.iter().enumerate() {
        let call = if *operation == "read" { "show" } else { "save" };
        program.push_str(&format!(
            "  let r{index} = {call}(\"{path}\")\n  @console.print(\"{index} {operation}: {{r{index}}}\")\n"
        ));
    }
    program
}

/// Runs the program of `cases` in the interpreter with a `Files` grant, read and write, on
/// each of `prefixes` under `dir`, in order, and returns the world it ran in.
fn run(dir: &Path, prefixes: &[&str], cases: &[(&str, &str)]) -> Recording {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"absent\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n",
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), program(cases)).unwrap();
    let checked = una_cli::check(dir, una_cli::Tier::Zero).unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mut grants = vec![Grant::Console];
    grants.extend(prefixes.iter().map(|prefix| Grant::Files {
        prefix: dir.join(prefix),
        read: true,
        write: true,
    }));
    let host = Host {
        grants,
        keys: BTreeMap::new(),
        virtual_time: false,
        cost_clock: false,
    };
    let mut world = Recording::default();
    let run = una_interp::run_main(
        &checked.program,
        "main",
        &mut world,
        &host,
        Limits::depth(10_000),
    )
    .unwrap();
    assert!(matches!(run.completion, una_interp::Completion::Ok(_)));
    world
}

/// A fresh directory with `box/note.txt`, `box/sub`, the link `box/in_dir` to `sub` and the
/// link `boxlink` to `box`.
fn tree(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("files_absent")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("box/sub")).unwrap();
    std::fs::write(dir.join("box/note.txt"), "inside\n").unwrap();
    std::os::unix::fs::symlink("sub", dir.join("box/in_dir")).unwrap();
    std::os::unix::fs::symlink("box", dir.join("boxlink")).unwrap();
    dir
}

#[test]
fn an_absent_directory_fails_the_operation_without_a_path_1459() {
    // Every operation of `Files`, a read and a write, below a directory that does not exist:
    // `NotFound`, and `Io` below a component that is not a directory, as before #1459; the
    // world's file operations are never called, where before they were given the path as the
    // program wrote it.
    let dir = tree("absent");
    let paths = [
        "missing/new.txt",
        "missing/deeper/new.txt",
        "in_dir/missing/new.txt",
        "sub/../missing/a/b/new.txt",
        "note.txt/x/new.txt",
    ];
    let cases: Vec<(&str, &str)> = paths
        .iter()
        .flat_map(|path| [("read", *path), ("write", *path)])
        .collect();
    for prefix in ["box", "boxlink"] {
        let world = run(&dir, &[prefix], &cases);
        let expected: Vec<String> = cases
            .iter()
            .enumerate()
            .map(|(index, (operation, path))| {
                let outcome = if path.starts_with("note.txt") {
                    "io"
                } else {
                    "not found"
                };
                format!("{index} {operation}: {outcome}")
            })
            .collect();
        assert_eq!(world.lines, expected, "{prefix}");
        assert_eq!(world.operations, Vec::new(), "{prefix}");
    }
    assert!(!dir.join("box/missing").exists());
    // A grant prefix that does not exist has no canonical directory: every path under it is
    // `NotFound`, with no path given either. The first grant that does not deny decides, as
    // before, so the grant on `box` after it is not reached.
    let cases = [
        ("read", "note.txt"),
        ("write", "new.txt"),
        ("write", "sub/new.txt"),
        ("read", "../box/note.txt"),
    ];
    let world = run(&dir, &["nothing/deeper", "box"], &cases);
    assert_eq!(
        world.lines,
        [
            "0 read: not found",
            "1 write: not found",
            "2 write: not found",
            "3 read: denied"
        ]
    );
    assert_eq!(world.operations, Vec::new());
    assert!(!dir.join("nothing").exists());
    assert!(!dir.join("box/new.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_final_component_is_opened_in_its_canonical_directory_1459() {
    // Unchanged by #1459: only the final component is missing, its directory resolves, so the
    // operation is given the canonical directory joined with it. Through a prefix that is a
    // link and a directory that is a link, a write creates the file and a read of a file that
    // is not there fails with `NotFound` at the operation.
    let dir = tree("leaf");
    let cases = [
        ("read", "in_dir/gone.txt"),
        ("write", "in_dir/new.txt"),
        ("read", "in_dir/new.txt"),
        ("write", "fresh.txt"),
    ];
    let world = run(&dir, &["boxlink"], &cases);
    assert_eq!(
        world.lines,
        [
            "0 read: not found",
            "1 write: written",
            "2 read: read",
            "3 write: written"
        ]
    );
    let canonical = std::fs::canonicalize(dir.join("box")).unwrap();
    assert_eq!(
        world.operations,
        [
            ("read", canonical.join("sub/gone.txt")),
            ("write", canonical.join("sub/new.txt")),
            ("read", canonical.join("sub/new.txt")),
            ("write", canonical.join("fresh.txt")),
        ]
    );
    for (_, path) in &world.operations {
        assert!(path.starts_with(&canonical), "{}", path.display());
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("box/sub/new.txt")).unwrap(),
        "entry"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
