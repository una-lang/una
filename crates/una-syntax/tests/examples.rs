//! Every example under `examples/` lexes and parses without diagnostics, and its syntax tree
//! matches the reviewed snapshot (M1 acceptance, approved M1 plan §4).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use una_syntax::{Interner, KeywordSet, SourceDb, dump, lex_in, parse};

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Every `.una` file under `examples/`, sorted, with its path relative to `examples/`.
fn example_files() -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "una") {
                files.push(path);
            }
        }
    }
    let root = examples_root();
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (relative, path)
        })
        .collect()
}

/// The keywords of the edition of the example package that holds `name`, a path relative to
/// `examples/`: its `una.toml` says (`spec/13 §7`).
fn keywords(name: &str) -> KeywordSet {
    let package = name.split('/').next().unwrap_or("");
    let manifest =
        fs::read_to_string(examples_root().join(package).join("una.toml")).unwrap_or_default();
    if manifest.contains("edition = \"0.2\"") {
        KeywordSet::V0_2
    } else {
        KeywordSet::V0_1
    }
}

#[test]
fn every_example_parses_without_diagnostics() {
    let files = example_files();
    assert!(files.len() >= 6, "expected the hello and events examples");
    for (name, path) in files {
        let mut db = SourceDb::new();
        let id = db.add(name.clone(), &fs::read(&path).unwrap());
        let mut interner = Interner::new();
        let lexed = lex_in(db.file(id), id, &mut interner, keywords(&name));
        let parsed = parse(&lexed, id, &interner);
        assert!(lexed.errors.is_empty(), "{name}: {:?}", lexed.errors);
        assert!(parsed.errors.is_empty(), "{name}: {:?}", parsed.errors);
        assert!(parsed.file.module.is_some(), "{name}: no module header");
    }
}

#[test]
fn example_syntax_trees_match_their_snapshots() {
    for (name, path) in example_files() {
        let mut db = SourceDb::new();
        let id = db.add(name.clone(), &fs::read(&path).unwrap());
        let mut interner = Interner::new();
        let lexed = lex_in(db.file(id), id, &mut interner, keywords(&name));
        let parsed = parse(&lexed, id, &interner);
        let snapshot = name.trim_end_matches(".una").replace('/', "__");
        insta::assert_snapshot!(snapshot, dump(&parsed.file, &interner, db.file(id).text()));
    }
}
