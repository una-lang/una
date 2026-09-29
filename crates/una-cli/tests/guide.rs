//! The guide for agents (`plan/milestones-v0.1.md` M10: "Every snippet is tested", DECISION
//! D-128): each `una` block of `docs/guide/una-for-agents.md` is a whole module that checks clean
//! at tier 1, or yields only the code its first line names; blocks introduced by file paths form
//! a package; each `json` block parses and names a schema the CLI prints; each `una` line of a
//! `sh` block exits with 0, or with the code of its `# exit N`, from the repository root or the
//! directory of its `# in <dir>`. Tier 1 needs Z3 at the version of `verification.lock` on the
//! path.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::Value;

const MANIFEST: &str = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// The schemas the CLI prints (`spec/13 §1, §2, §6`, ADRs 0029, 0032 and 0034).
const SCHEMAS: [&str; 9] = [
    "una.diagnostics/1",
    "una.diagnostic/1",
    "una.obligations/1",
    "una.test/1",
    "una.run/1",
    "una.contract/1",
    "una.trace/1",
    "una.report/1",
    "una.explain/1",
];

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fenced block of the guide.
struct Block {
    /// The language after the opening fence.
    lang: String,
    body: String,
    /// The line of the opening fence.
    line: usize,
    /// The file path on the line before the block, for a block of a package.
    path: Option<String>,
    /// The package the block belongs to, numbered in order.
    package: Option<usize>,
}

/// The blocks of `text`. A line holding only a path in backticks names the file of the next
/// block; path blocks separated only by blank lines form one package.
fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut packages = 0;
    let mut separated = true;
    let mut lines = text.lines().enumerate();
    while let Some((number, line)) = lines.next() {
        if let Some(lang) = line.strip_prefix("```") {
            let mut body = String::new();
            for (_, inner) in lines.by_ref() {
                if inner.starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            let package = path.as_ref().map(|_| packages);
            out.push(Block {
                lang: lang.to_owned(),
                body,
                line: number + 1,
                path: path.take(),
                package,
            });
            if package.is_none() {
                separated = true;
            }
        } else if line.len() > 2
            && line.starts_with('`')
            && line.ends_with('`')
            && !line.contains(' ')
        {
            if separated {
                packages += 1;
                separated = false;
            }
            path = Some(line.trim_matches('`').to_owned());
        } else if !line.trim().is_empty() {
            separated = true;
            path = None;
        }
    }
    out
}

/// A fresh directory for the block at `line`.
fn scratch(line: usize) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("guide")
        .join(line.to_string());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn check(target: &Path) -> (Output, Vec<String>) {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["--json", "check", target.to_str().unwrap()])
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| {
            format!(
                "{}: {}",
                target.display(),
                String::from_utf8_lossy(&output.stderr)
            )
        })
        .unwrap();
    let codes = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            format!(
                "{} {}:{} {}",
                d["code"].as_str().unwrap(),
                d["primary"]["start"]["line"],
                d["primary"]["start"]["col"],
                d["message"].as_str().unwrap()
            )
        })
        .collect();
    (output, codes)
}

#[test]
fn every_block_of_the_guide_is_checked() {
    let text = std::fs::read_to_string(repository().join("docs/guide/una-for-agents.md")).unwrap();
    let blocks = blocks(&text);
    let mut checked = [0; 4];
    let mut packages: Vec<Vec<&Block>> = Vec::new();
    for block in &blocks {
        if let Some(package) = block.package {
            if packages.len() < package {
                packages.resize(package, Vec::new());
            }
            packages[package - 1].push(block);
            continue;
        }
        let at = block.line;
        assert!(
            ["una", "json", "sh"].contains(&block.lang.as_str()),
            "block at line {at}: a `{}` block outside a package is not checked",
            block.lang
        );
        match block.lang.as_str() {
            "una" => {
                let dir = scratch(at);
                let file = dir.join("snippet.una");
                std::fs::write(&file, &block.body).unwrap();
                let (_, found) = check(&file);
                let expected = block
                    .body
                    .lines()
                    .next()
                    .and_then(|first| first.strip_prefix("-- fails with "));
                match expected {
                    Some(code) => assert!(
                        !found.is_empty() && found.iter().all(|f| f.starts_with(code)),
                        "block at line {at}: expected only {code}, found {found:?}"
                    ),
                    None => assert_eq!(found, Vec::<String>::new(), "block at line {at}"),
                }
                checked[0] += 1;
            }
            "json" => {
                let document: Value = serde_json::from_str(&block.body)
                    .map_err(|e| format!("block at line {at}: {e}"))
                    .unwrap();
                let schema = document["schema"].as_str().unwrap_or("");
                assert!(SCHEMAS.contains(&schema), "block at line {at}: {schema}");
                checked[1] += 1;
            }
            "sh" => {
                for command in block.body.lines().filter(|l| l.starts_with("una ")) {
                    let (words, comment) = command.split_once('#').unwrap_or((command, ""));
                    let mut code = 0;
                    let mut dir = repository();
                    for part in comment.split(';').map(str::trim) {
                        if let Some(n) = part.strip_prefix("exit ") {
                            code = n.parse().unwrap();
                        } else if let Some(d) = part.strip_prefix("in ") {
                            dir = repository().join(d);
                        }
                    }
                    let args: Vec<&str> = words.split_whitespace().skip(1).collect();
                    let output = Command::new(env!("CARGO_BIN_EXE_una"))
                        .args(&args)
                        .current_dir(&dir)
                        .output()
                        .unwrap();
                    assert_eq!(
                        output.status.code(),
                        Some(code),
                        "`{command}`: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                checked[2] += 1;
            }
            _ => {}
        }
    }
    for files in &packages {
        let at = files[0].line;
        let dir = scratch(at);
        for file in files {
            let path = dir.join(file.path.as_deref().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &file.body).unwrap();
        }
        if !dir.join("una.toml").exists() {
            std::fs::write(dir.join("una.toml"), MANIFEST).unwrap();
        }
        std::fs::copy(
            repository().join("verification.lock"),
            dir.join("verification.lock"),
        )
        .unwrap();
        let (output, found) = check(&dir);
        assert_eq!(found, Vec::<String>::new(), "package at line {at}");
        assert_eq!(output.status.code(), Some(0), "package at line {at}");
        checked[3] += 1;
    }
    // Every kind of block is present: modules, documents, commands and packages.
    assert!(checked.iter().all(|n| *n > 0), "{checked:?}");
}

/// Every `.una` file under `dir`, read.
fn una_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(una_files(&path));
        } else if path.extension().is_some_and(|e| e == "una") {
            out.push(std::fs::read_to_string(&path).unwrap());
        }
    }
    out
}

#[test]
fn the_readme_shows_only_checked_code() {
    // `compiler/testing.md §6`: every `una` block of `README.md` is copied from a file that the
    // other tests check (the examples, the demo programs, the guide), or checks clean on its
    // own; every `json` block parses and names a schema the CLI prints.
    let readme = std::fs::read_to_string(repository().join("README.md")).unwrap();
    let mut checked = una_files(&repository().join("examples"));
    checked.extend(una_files(&repository().join("tests/demo")));
    checked
        .push(std::fs::read_to_string(repository().join("docs/guide/una-for-agents.md")).unwrap());
    let mut blocks_seen = 0;
    for block in blocks(&readme) {
        let at = block.line;
        match block.lang.as_str() {
            "una" if checked.iter().any(|source| source.contains(&block.body)) => {}
            "una" => {
                let file = scratch(100_000 + at).join("readme.una");
                std::fs::write(&file, &block.body).unwrap();
                let (_, found) = check(&file);
                assert_eq!(found, Vec::<String>::new(), "readme block at line {at}");
            }
            "json" => {
                let document: Value = serde_json::from_str(&block.body)
                    .map_err(|e| format!("readme block at line {at}: {e}"))
                    .unwrap();
                let schema = document["schema"].as_str().unwrap_or("");
                assert!(
                    SCHEMAS.contains(&schema),
                    "readme block at line {at}: {schema}"
                );
            }
            _ => continue,
        }
        blocks_seen += 1;
    }
    assert!(blocks_seen > 0, "the readme shows no code");
}
