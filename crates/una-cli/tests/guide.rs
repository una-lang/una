//! The guides (`docs/guide/`, #178): every `una` block of `una-for-agents.md` and of
//! `syntax-reference.md` is a whole module, checked as the only module of a package of edition
//! `0.2`, or of `0.1` when a leading `-- edition 0.1` line says so. It checks clean at tier 1 in
//! canonical form (`una fmt --check`), or yields only the code of its leading
//! `-- fails with U####` line. Blocks introduced by file paths form a package. Each `json` block
//! parses and names a schema the CLI prints. Each `una` or `./` line of a `sh` block exits with
//! 0, or with the code of its `# exit N`, from the repository root, the directory of its
//! `# in <dir>`, or a fresh copy of the directory of a `# copy <dir>` earlier in the block.
//! Each example package's `README.md` holds only such `sh` blocks and the output of `una run`
//! (#586). Every repository path that a guide writes in inline code names a file or directory
//! of the repository (#651).
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
const GUIDES: [&str; 4] = [
    "docs/guide/una-for-agents.md",
    "docs/guide/syntax-reference.md",
    "docs/guide/getting-started.md",
    "docs/guide/una-compact.md",
];

/// The schemas the CLI prints (`spec/13 §1, §2, §6`, ADRs 0029, 0032 and 0034).
const SCHEMAS: [&str; 12] = [
    "una.diagnostics/1",
    "una.fix/1",
    "una.diagnostic/1",
    "una.obligations/1",
    "una.test/1",
    "una.run/1",
    "una.contract/1",
    "una.trace/1",
    "una.report/2",
    "una.build/1",
    "una.explain/1",
    "una.version/1",
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

/// Writes `una.toml` of `edition` and the lock of the repository into `dir`, each unless the
/// package has its own: a page may show a lock, such as one naming another solver version.
fn complete(dir: &Path, edition: &str) {
    if !dir.join("una.toml").exists() {
        std::fs::write(dir.join("una.toml"), manifest(edition)).unwrap();
    }
    if !dir.join("verification.lock").exists() {
        std::fs::copy(
            repository().join("verification.lock"),
            dir.join("verification.lock"),
        )
        .unwrap();
    }
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
        // A package may show a failure, named by a leading `-- fails with` line of one of its
        // modules.
        let fails = files
            .iter()
            .filter(|file| file.lang == "una")
            .find_map(|file| directives(&file.body).0);
        if let Some(code) = fails {
            assert!(
                !found.is_empty() && found.iter().all(|f| f.starts_with(code)),
                "{path}, package at line {at}: expected only {code}, found {found:?}"
            );
            checked[3] += 1;
            continue;
        }
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

#[test]
fn every_block_of_the_getting_started_tutorial_is_checked() {
    // #245: the tutorial's packages, from the first to the finished program, and its commands,
    // which run in fresh copies of `examples/wordfreq`.
    let checked = check_guide(GUIDES[2], 400_000);
    assert!(checked[2] > 0 && checked[3] >= 4, "{checked:?}");
    // The finished program, the tutorial's last package, is `examples/wordfreq`, which the
    // tests of the examples check and run in both engines.
    let text = std::fs::read_to_string(repository().join(GUIDES[2])).unwrap();
    let blocks = blocks(&text);
    let last = blocks.iter().filter_map(|b| b.package).max().unwrap();
    for block in blocks.iter().filter(|b| b.package == Some(last)) {
        let path = block.path.as_deref().unwrap();
        assert_eq!(
            std::fs::read_to_string(repository().join("examples/wordfreq").join(path)).unwrap(),
            block.body,
            "`{path}` of the tutorial's last package differs from examples/wordfreq"
        );
    }
}

/// The most characters `una-compact.md` may hold. The model's tokenizer (Claude Opus 5.5,
/// measured on 2026-10-08 as the difference in input tokens between two identical sessions,
/// one of which reads the file) counted 1,940 tokens for its 4,506 characters, 2.32 per token;
/// 4,550 characters stay under its budget of 2,000 tokens (#430).
const COMPACT_MAX_CHARS: usize = 4_550;

#[test]
fn the_compact_reference_is_checked_and_within_its_token_budget() {
    // #430: every block of the compact reference is checked, as in the other guides, and its
    // size stays under the budget that agents load into their context.
    let checked = check_guide(GUIDES[3], 600_000);
    assert!(checked[0] > 0, "{checked:?}");
    let text = std::fs::read_to_string(repository().join(GUIDES[3])).unwrap();
    let chars = text.chars().count();
    assert!(
        chars <= COMPACT_MAX_CHARS,
        "una-compact.md holds {chars} characters, over {COMPACT_MAX_CHARS} (about 2,000 tokens)"
    );
}

#[test]
fn the_policy_example_of_the_information_flow_spec_checks_in_the_events_package() {
    // #569: `spec/07 §5`'s `events.keys` module, put in place of the `events` example's own,
    // checks clean at tier 1 in canonical form, so the spec's example cannot drift into one
    // that fails (it once omitted `fails KeyError`, U3007).
    let spec =
        std::fs::read_to_string(repository().join("docs/spec/07-information-flow.md")).unwrap();
    let block = blocks(&spec)
        .into_iter()
        .find(|b| b.lang == "una" && b.body.starts_with("trusted module events.keys"))
        .expect("spec/07 has the events.keys policy example");
    let dir = scratch(block.line).join("events");
    copy_dir(&repository().join("examples/events"), &dir);
    std::fs::write(dir.join("src/events/keys.una"), &block.body).unwrap();
    let (output, codes) = check(&dir);
    assert_eq!(
        codes,
        Vec::<String>::new(),
        "spec/07 §5 at line {}",
        block.line
    );
    assert!(output.status.success());
    assert!(
        canonical(&dir).status.success(),
        "spec/07 §5 is in canonical form"
    );
}

/// The parts of a construct or configuration page (#585), in order.
const PARTS: [&str; 7] = [
    "## What it does",
    "## Forms",
    "## Examples",
    "## Edge cases",
    "## Diagnostics",
    "## Edition",
    "## Normative source",
];

/// Every `.md` file under `dir`, sorted.
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(markdown_files(&path));
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// The specification sections `text` cites: `spec/NN`, and `spec/NN §M` for each section of a
/// citation such as `spec/06 §3, §5`.
fn spec_citations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("spec/") {
        rest = &rest[at + 5..];
        let chapter: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if chapter.len() != 2 {
            continue;
        }
        let mut tail = &rest[2..];
        let mut sections = Vec::new();
        while let Some(after) = tail.strip_prefix(" §").or_else(|| tail.strip_prefix(", §")) {
            let section: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let section = section.trim_end_matches('.').to_owned();
            if section.is_empty() {
                break;
            }
            tail = &after[section.len()..];
            sections.push(section);
        }
        if sections.is_empty() {
            out.push(format!("spec/{chapter}"));
        }
        for section in sections {
            out.push(format!("spec/{chapter} §{section}"));
        }
    }
    out
}

#[test]
fn every_construct_page_follows_the_template_and_its_blocks_check() {
    // The construct and configuration pages (#585): front matter that names the page's
    // diagnostic codes and every specification section its text cites, the seven parts of the template in order, a Diagnostics part that lists
    // exactly those codes, each with its explain text, a failing block only with one of them,
    // and every block checked as the guides' are.
    let root = repository();
    let mut pages = Vec::new();
    for dir in ["docs/guide/language", "docs/guide/configure"] {
        let dir = root.join(dir);
        if dir.is_dir() {
            pages.extend(markdown_files(&dir));
        }
    }
    assert!(!pages.is_empty(), "no construct pages");
    for (index, page) in pages.iter().enumerate() {
        let relative = page
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let text = std::fs::read_to_string(page).unwrap();
        let front = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(front, _)| front)
            .unwrap_or_else(|| panic!("{relative}: no front matter"));
        let mut codes: Vec<&str> = front
            .lines()
            .find_map(|line| line.strip_prefix("codes: "))
            .unwrap_or_else(|| panic!("{relative}: the front matter has no `codes`"))
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .collect();
        codes.sort_unstable();
        let listed_spec = front
            .lines()
            .find_map(|line| line.strip_prefix("spec: "))
            .unwrap_or_else(|| panic!("{relative}: the front matter has no `spec`"));
        let body = text
            .split_once("\n---\n")
            .map_or(text.as_str(), |(_, body)| body);
        for cited in spec_citations(body) {
            let quoted = format!("\"{cited}\"");
            let chapter_only = !cited.contains('§');
            assert!(
                listed_spec.contains(&quoted)
                    || (chapter_only && listed_spec.contains(&format!("\"{cited} §"))),
                "{relative}: {cited} is cited but missing from the front matter's `spec`"
            );
        }
        let mut from = 0;
        for part in PARTS {
            let at = text[from..]
                .find(&format!("\n{part}\n"))
                .unwrap_or_else(|| panic!("{relative}: `{part}` is missing or out of order"));
            from += at + 1;
        }
        let diagnostics = text
            .split("\n## Diagnostics\n")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .unwrap();
        let mut listed: Vec<&str> = diagnostics
            .lines()
            .filter_map(|line| line.strip_prefix("- ["))
            .filter_map(|line| line.split(']').next())
            .collect();
        listed.sort_unstable();
        assert_eq!(
            listed, codes,
            "{relative}: the Diagnostics part and the front matter's codes"
        );
        for code in &codes {
            assert!(
                root.join(format!("crates/una-explain/explain/{code}.md"))
                    .is_file(),
                "{relative}: {code} has no explain text"
            );
        }
        let page_blocks = blocks(&text);
        for block in page_blocks.iter().filter(|block| block.lang == "una") {
            if let (Some(code), _) = directives(&block.body) {
                assert!(
                    codes.contains(&code),
                    "{relative}, block at line {}: {code} is not among the page's codes",
                    block.line
                );
            }
        }
        let checked = check_guide(&relative, 1_000_000 + index * 10_000);
        assert!(
            checked[0] + checked[3] > 0,
            "{relative}: no checked Una code"
        );
    }
}

#[test]
fn every_example_has_a_readme_whose_commands_run() {
    // #586: each package of `examples/` has a `README.md`. Its code is only `sh` blocks, whose
    // commands run as the guides' do, and the `text` block after them, which is what `una run`
    // prints. Its `Edition` part names the edition of the package's `una.toml`, and each of
    // its relative links leads to a file of the repository.
    let root = repository();
    let mut names: Vec<String> = std::fs::read_dir(root.join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_str().unwrap().to_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no example packages");
    for (index, name) in names.iter().enumerate() {
        let package = root.join("examples").join(name);
        let relative = format!("examples/{name}/README.md");
        let text = std::fs::read_to_string(package.join("README.md"))
            .unwrap_or_else(|_| panic!("{relative} is missing"));
        let readme_blocks = blocks(&text);
        for block in &readme_blocks {
            assert!(
                ["sh", "text"].contains(&block.lang.as_str()),
                "{relative}, block at line {}: a `{}` block is not checked",
                block.line,
                block.lang
            );
        }
        let checked = check_guide(&relative, 2_000_000 + index * 10_000);
        assert!(checked[2] > 0, "{relative}: no commands");

        let printed = readme_blocks
            .iter()
            .find(|block| block.lang == "text")
            .unwrap_or_else(|| panic!("{relative}: no `text` block of what `una run` prints"));
        // `[package]` requires `name` and `edition` (`spec/13 §7`).
        let manifest = std::fs::read_to_string(package.join("una.toml")).unwrap();
        let key = |wanted: &str| -> String {
            manifest
                .lines()
                .filter_map(|line| line.split_once('='))
                .find(|(key, _)| key.trim() == wanted)
                .map(|(_, value)| value.trim().trim_matches('"').to_owned())
                .unwrap_or_else(|| panic!("{relative}: its una.toml has no `{wanted}`"))
        };
        let edition = key("edition");

        // `una run` and the native executable print the `text` block.
        let copy = scratch(2_000_000 + index * 10_000 + 9_999).join(name);
        copy_dir(&package, &copy);
        let una = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_una"))
                .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                .args(args)
                .current_dir(&copy)
                .output()
                .unwrap()
        };
        let output = una(&["run", "."]);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            printed.body,
            "{relative}: what `una run` prints"
        );
        runtime();
        let built = una(&["build", "."]);
        assert!(
            built.status.success(),
            "{relative}: `una build .`: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        let output = Command::new(copy.join("target/una").join(key("name")))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .current_dir(&copy)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            printed.body,
            "{relative}: what the native executable prints"
        );
        let part = text
            .split("\n## Edition\n")
            .nth(1)
            .unwrap_or_else(|| panic!("{relative}: no `## Edition` part"));
        assert!(
            part.trim_start()
                .starts_with(&format!("Edition `{edition}`")),
            "{relative}: the Edition part does not name edition {edition}"
        );

        let mut rest = text.as_str();
        let mut links = 0;
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            let target = rest.split(')').next().unwrap();
            let file = target.split('#').next().unwrap();
            if file.is_empty() || file.contains("://") {
                continue;
            }
            assert!(
                package.join(file).is_file(),
                "{relative}: the link to {target} leads to no file"
            );
            links += 1;
        }
        assert!(
            text.contains("](../../docs/guide/language/"),
            "{relative}: no link to a construct page"
        );
        assert!(links > 0, "{relative}: no links");
    }
}

/// The repository paths that `text` writes in inline code outside fenced blocks (#651): a code
/// span without spaces or placeholders (`<`, `>`, `*`, `{`, `}`, `$`, `|`) whose first segment
/// is one of `top`, the repository's top-level directories, as in `images/logo.png` or
/// `examples/hello/`. A `#` fragment or a `:line` suffix is dropped.
fn repository_paths(text: &str, top: &[String]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fence = false;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        for (i, span) in line.split('`').enumerate() {
            let span = span.trim();
            if i % 2 == 0
                || !span.contains('/')
                || span.contains(char::is_whitespace)
                || span.contains(['<', '>', '*', '{', '}', '$', '|'])
            {
                continue;
            }
            let path = span.split(['#', ':']).next().unwrap();
            if top
                .iter()
                .any(|t| path.split('/').next() == Some(t.as_str()))
            {
                out.push((n + 1, path.to_string()));
            }
        }
    }
    out
}

#[test]
fn every_repository_path_in_the_guides_exists() {
    let root = repository();
    let mut top: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.') && name != "target")
        .collect();
    top.sort();

    // The case of #651: a missing file in inline code is found; a placeholder, a sentence, a
    // path outside the repository's directories and a fenced block are not paths to check.
    let case = "Install `images/missing-0.2.0.vsix`, see `examples/hello/`.\n\
                Not paths: `examples/<name>/`, `a / b`, `src/app.una`.\n\
                ```sh\nls `images/inside-a-fence`\n```\n";
    let found = repository_paths(case, &["examples".into(), "images".into()]);
    assert_eq!(
        found,
        vec![
            (1, "images/missing-0.2.0.vsix".to_string()),
            (1, "examples/hello/".to_string())
        ]
    );

    let mut checked = 0;
    for file in markdown_files(&root.join("docs/guide")) {
        let text = std::fs::read_to_string(&file).unwrap();
        let relative = file.strip_prefix(&root).unwrap().display().to_string();
        for (line, path) in repository_paths(&text, &top) {
            assert!(
                root.join(&path).exists(),
                "{relative}:{line}: `{path}` names no file or directory of the repository"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no repository path found in docs/guide");
}
