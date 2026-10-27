//! The guard of #1160: the non-test sources of `una-cli`, `una-verify`, `una-codegen` and
//! `una-project` write into a package directory only through `una_diag::safe_write`, which
//! never follows a link planted there out of the package, or, for the package store of
//! `una ita`, through `una-project`'s `tree` module, whose sites the allowlist names. A raw
//! write ([`RAW_WRITES`]: `fs::write`, `File::create`,
//! `.create(true)`, `create_dir_all`, `create_dir`, `OpenOptions::new`, `DirBuilder`,
//! `fs::rename`, `fs::copy` or `hard_link`) fails this test unless the allowlist below names
//! it, with its exact count and reason. The helper itself, `crates/una-diag/src/safe_write.rs`,
//! is not read.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The calls that write without the checks of `una_diag::safe_write`.
const RAW_WRITES: [&str; 10] = [
    "fs::write(",
    "File::create(",
    ".create(true)",
    "create_dir_all(",
    "OpenOptions::new(",
    "fs::rename(",
    "fs::copy(",
    "hard_link(",
    "DirBuilder",
    "create_dir(",
];

/// The crates whose sources are read.
const CRATES: [&str; 4] = ["una-cli", "una-verify", "una-codegen", "una-project"];

/// The raw writes that remain, by file and call, with how many there are and why each is safe.
const ALLOWED: [(&str, &str, usize, &str); 13] = [
    (
        "crates/una-cli/src/build.rs",
        "create_dir_all(",
        1,
        "the absolute directory that `una build --out` names on the command line: the user chose \
         it, not the package, and the files in it are still written through \
         `una_diag::safe_write`",
    ),
    (
        "crates/una-cli/src/proof_key.rs",
        "DirBuilder",
        2,
        "the directory of the user's proof cache key under HOME (or `UNA_PROOF_KEY`), outside \
         any package, created with mode 0700",
    ),
    (
        "crates/una-cli/src/proof_key.rs",
        "OpenOptions::new(",
        2,
        "the staged key file and the effective-user probe in the temporary directory, both \
         created exclusively (`create_new`) with mode 0600, outside any package",
    ),
    (
        "crates/una-cli/src/proof_key.rs",
        "hard_link(",
        1,
        "the staged key linked to its name, which fails when anything is at that name (#282)",
    ),
    (
        "crates/una-project/src/tree.rs",
        "DirBuilder",
        2,
        "the package store of `una ita` (ADR 0122 rules 14 and 30), under Itaoca or the vendor \
         directory: the fresh private directory, created with `mkdir` and mode 0700, which \
         fails when anything is at its name, a link included",
    ),
    (
        "crates/una-project/src/tree.rs",
        "OpenOptions::new(",
        3,
        "the package store of `una ita` (ADR 0122 rules 14 and 30), under Itaoca or the vendor \
         directory: a package file opened for reading with `O_NOFOLLOW`, a file created in \
         the fresh private directory with `create_new` and `O_NOFOLLOW`, mode 0600, and the \
         floor lock of a registry in Itaoca (rule 39), opened with `O_NOFOLLOW`",
    ),
    (
        "crates/una-project/src/tree.rs",
        ".create(true)",
        1,
        "the floor lock of a registry in Itaoca (rule 39), outside any package: an empty file \
         for an advisory lock, never truncated or written, mode 0600, opened with `O_NOFOLLOW` \
         so a link at its name is refused",
    ),
    (
        "crates/una-project/src/tree.rs",
        "create_dir(",
        2,
        "the package store of `una ita` (ADR 0122 rules 14 and 30), under Itaoca or the vendor \
         directory: `mkdir` of the fresh directory where the platform has no mode, and of \
         each shared directory, each checked afterward to be a directory and not a link",
    ),
    (
        "crates/una-project/src/tree.rs",
        "create_dir_all(",
        1,
        "the package store of `una ita` (ADR 0122 rules 14 and 30), under Itaoca or the vendor \
         directory: the store's base, which the user names and which must not be a link; \
         every directory below it is checked not to be one",
    ),
    (
        "crates/una-project/src/tree.rs",
        "fs::rename(",
        2,
        "the package store of `una ita` (ADR 0122 rules 14 and 30), under Itaoca or the vendor \
         directory: the fresh directory, or its one file, renamed into place after its \
         identity is checked by device and inode; a rename replaces a link, never follows it",
    ),
    (
        "crates/una-project/src/tree.rs",
        "hard_link(",
        1,
        "the package store of `una ita` (ADR 0122 rules 14 and 30), under Itaoca or the vendor \
         directory: a stored provenance linked to its name, which fails when anything is \
         there, so it is never replaced",
    ),
    (
        "crates/una-cli/src/runtime.rs",
        "File::create(",
        1,
        "the trace file of `una run --trace`, streamed as the run goes (ADR 0117 rule 10): the \
         path the user names on the command line, not one the package chooses, as for `una build \
         --out`",
    ),
    (
        "crates/una-cli/src/runtime.rs",
        "OpenOptions::new(",
        1,
        "`write_file`, a program's own write under its granted file-system capability \
         (ADR 0038 D9 rule 3): a new file is created with `create_new`, never through a link",
    ),
];

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The `.rs` files under `dir`, in a stable order.
fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// The lines of `text` outside its test modules, with their numbers: a module that
/// `#[cfg(test)]` marks at the top level ends at the first line that is `}` alone, as rustfmt
/// lays it out.
fn non_test_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let module = lines.get(index + 1).is_some_and(|next| {
            next.starts_with("mod ") && next.ends_with(" {") && !next.contains(';')
        });
        if line == "#[cfg(test)]" && module {
            index += 2;
            while index < lines.len() && lines[index] != "}" {
                index += 1;
            }
            index += 1;
            continue;
        }
        kept.push((index + 1, line));
        index += 1;
    }
    kept
}

#[test]
fn package_writes_go_through_the_safe_helper_1160() {
    let root = root();
    let mut found: BTreeMap<(String, &str), Vec<String>> = BTreeMap::new();
    for name in CRATES {
        let mut files = Vec::new();
        sources(&root.join("crates").join(name).join("src"), &mut files);
        assert!(!files.is_empty(), "no sources in {name}");
        for file in files {
            let relative = file
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&file).unwrap();
            for (number, line) in non_test_lines(&text) {
                let code = line.split("//").next().unwrap_or_default();
                for call in RAW_WRITES {
                    if code.contains(call) {
                        found
                            .entry((relative.clone(), call))
                            .or_default()
                            .push(format!("{relative}:{number}: {}", line.trim()));
                    }
                }
            }
        }
    }
    let mut problems = Vec::new();
    for ((file, call), sites) in &found {
        let allowed = ALLOWED
            .iter()
            .find(|(path, allowed_call, _, _)| *path == file.as_str() && allowed_call == call);
        match allowed {
            Some((_, _, count, _)) if *count == sites.len() => {}
            Some((_, _, count, _)) => problems.push(format!(
                "{file}: {} raw `{call}` where the allowlist names {count}:\n  {}",
                sites.len(),
                sites.join("\n  ")
            )),
            None => problems.push(format!(
                "a raw write outside `una_diag::safe_write`:\n  {}",
                sites.join("\n  ")
            )),
        }
    }
    for (path, call, _, reason) in ALLOWED {
        assert!(!reason.is_empty());
        if !found.contains_key(&(path.to_owned(), call)) {
            problems.push(format!(
                "the allowlist names `{call}` in {path}, which no longer has it"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn test_modules_are_skipped_and_the_rest_is_kept() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() {\n    }\n}\nfn c() {}\n";
    let kept: Vec<&str> = non_test_lines(text)
        .into_iter()
        .map(|(_, line)| line)
        .collect();
    assert_eq!(kept, ["fn a() {}", "fn c() {}"]);
}
