//! The guides (`docs/guide/`, #178): every `una` block of `una-for-agents.md` and of
//! `syntax-reference.md` is a whole module, checked as the only module of a package of edition
//! `0.2`, or of `0.1` when a leading `-- edition 0.1` line says so. It checks clean at tier 1 in
//! canonical form (`una fmt --check`), or yields only the code of its leading
//! `-- fails with U####` line. Blocks introduced by file paths form a package. Each `json` block
//! parses and names a schema the CLI prints. Each `una` or `./` line of a `sh` block exits with
//! 0, or with the code of its `# exit N`, from the repository root, the directory of its
//! `# in <dir>`, or a fresh copy of the directory of a `# copy <dir>` earlier in the block.
//! Tier 1 needs Z3 at the version of `verification.lock` on the path.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::Value;

/// The manifest of a package of `edition`.
fn manifest(edition: &str) -> String {
    format!("[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n")
}

/// The guides whose blocks are checked.
const GUIDES: [&str; 2] = [
    "docs/guide/una-for-agents.md",
    "docs/guide/syntax-reference.md",
];

/// The schemas the CLI prints (`spec/13 §1, §2, §6`, ADRs 0029, 0032 and 0034).
const SCHEMAS: [&str; 10] = [
    "una.diagnostics/1",
    "una.diagnostic/1",
    "una.obligations/1",
    "una.test/1",
    "una.run/1",
    "una.contract/1",
    "una.trace/1",
    "una.report/2",
    "una.build/1",
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
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
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

/// The leading `--` lines of a `una` block: the code it fails with, and its edition.
fn directives(body: &str) -> (Option<&str>, &str) {
    let mut fails = None;
    let mut edition = "0.2";
    for line in body.lines().take_while(|line| line.starts_with("--")) {
        if let Some(code) = line.strip_prefix("-- fails with ") {
            fails = Some(code.trim());
        } else if let Some(found) = line.strip_prefix("-- edition ") {
            edition = found.trim();
        }
    }
    (fails, edition)
}

/// The path under `src/` of the module that `body` declares.
fn module_file(body: &str) -> PathBuf {
    let header = body
        .lines()
        .find_map(|line| {
            line.strip_prefix("module ")
                .or_else(|| line.strip_prefix("trusted module "))
        })
        .expect("a `una` block declares its module");
    let mut path: PathBuf = header.trim().split('.').collect();
    path.set_extension("una");
    Path::new("src").join(path)
}

/// Writes the lock of the repository into `dir`, and `una.toml` of `edition` unless the
/// package has its own.
fn complete(dir: &Path, edition: &str) {
    if !dir.join("una.toml").exists() {
        std::fs::write(dir.join("una.toml"), manifest(edition)).unwrap();
    }
    std::fs::copy(
        repository().join("verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
}

/// Whether `una fmt --check` accepts the package at `dir`.
fn canonical(dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["fmt", "--check", dir.to_str().unwrap()])
        .output()
        .unwrap()
}

/// Copies the directory `from` into `to`.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            if entry.file_name() != "target" && entry.file_name() != ".una" {
                copy_dir(&entry.path(), &target);
            }
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Builds the runtime library next to the `una` binary, once, for the `una build` lines:
/// `cargo test` builds `una-rt`'s static library only when asked (#203).
fn runtime() {
    static BUILT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "-p", "una-rt"])
            .current_dir(repository())
            .status()
            .unwrap();
        assert!(status.success(), "cargo build -p una-rt failed");
    });
}

/// Runs each command line of a `sh` block, as the module documentation says.
fn run_commands(block: &Block, scratch_base: usize) {
    let mut copy: Option<PathBuf> = None;
    for line in block.body.lines() {
        let (words, comment) = line.split_once('#').unwrap_or((line, ""));
        let words = words.trim();
        let mut code = 0;
        let mut dir = copy.clone().unwrap_or_else(repository);
        for part in comment.split(';').map(str::trim) {
            if let Some(n) = part.strip_prefix("exit ") {
                code = n.parse().unwrap();
            } else if let Some(d) = part.strip_prefix("in ") {
                dir = repository().join(d);
            } else if let Some(d) = part.strip_prefix("copy ") {
                let fresh = scratch(scratch_base + block.line).join("copy");
                copy_dir(&repository().join(d), &fresh);
                copy = Some(fresh.clone());
                dir = fresh;
            }
        }
        if words.starts_with("una build") {
            runtime();
        }
        let mut parts = words.split_whitespace();
        let program = match parts.next() {
            Some("una") => PathBuf::from(env!("CARGO_BIN_EXE_una")),
            Some(local) if local.starts_with("./") => dir.join(local),
            _ => continue,
        };
        let output = Command::new(&program)
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .args(parts)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`{line}` (block at line {}): {}",
            block.line,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Checks every block of the guide at `path`, and returns how many modules, documents,
/// commands and packages it holds.
fn check_guide(path: &str, scratch_base: usize) -> [usize; 4] {
    let text = std::fs::read_to_string(repository().join(path)).unwrap();
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
            ["una", "json", "sh", "text", "ebnf"].contains(&block.lang.as_str()),
            "{path}, block at line {at}: a `{}` block outside a package is not checked",
            block.lang
        );
        match block.lang.as_str() {
            "una" => {
                let (fails, edition) = directives(&block.body);
                let dir = scratch(scratch_base + at);
                let file = dir.join(module_file(&block.body));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(&file, &block.body).unwrap();
                complete(&dir, edition);
                let (_, found) = check(&dir);
                match fails {
                    Some(code) => assert!(
                        !found.is_empty() && found.iter().all(|f| f.starts_with(code)),
                        "{path}, block at line {at}: expected only {code}, found {found:?}"
                    ),
                    None => {
                        assert_eq!(found, Vec::<String>::new(), "{path}, block at line {at}");
                        let output = canonical(&dir);
                        assert!(
                            output.status.success(),
                            "{path}, block at line {at} is not canonical: {}",
                            String::from_utf8_lossy(&output.stdout)
                        );
                    }
                }
                checked[0] += 1;
            }
            "json" => {
                let document: Value = serde_json::from_str(&block.body)
                    .map_err(|e| format!("{path}, block at line {at}: {e}"))
                    .unwrap();
                let schema = document["schema"].as_str().unwrap_or("");
                assert!(
                    SCHEMAS.contains(&schema),
                    "{path}, block at line {at}: {schema}"
                );
                checked[1] += 1;
            }
            "sh" => {
                run_commands(block, scratch_base);
                checked[2] += 1;
            }
            _ => {}
        }
    }
    for files in &packages {
        let at = files[0].line;
        let dir = scratch(scratch_base + at);
        for file in files {
            let path = dir.join(file.path.as_deref().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &file.body).unwrap();
        }
        complete(&dir, "0.2");
        let (output, found) = check(&dir);
        assert_eq!(found, Vec::<String>::new(), "{path}, package at line {at}");
        assert_eq!(
            output.status.code(),
            Some(0),
            "{path}, package at line {at}"
        );
        let output = canonical(&dir);
        assert!(
            output.status.success(),
            "{path}, package at line {at} is not canonical: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        checked[3] += 1;
    }
    checked
}

#[test]
fn every_block_of_the_agent_guide_is_checked() {
    let checked = check_guide(GUIDES[0], 0);
    // Every kind of block is present: modules, documents, commands and packages.
    assert!(checked.iter().all(|n| *n > 0), "{checked:?}");
}

#[test]
fn every_block_of_the_syntax_reference_is_checked() {
    let checked = check_guide(GUIDES[1], 200_000);
    assert!(checked[0] > 0, "{checked:?}");
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
