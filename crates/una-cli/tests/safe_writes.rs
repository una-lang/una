//! The guard of #1160: the non-test sources of `una-cli`, `una-verify` and `una-codegen` write
//! into a package directory only through `una_diag::safe_write`, which never follows a link
//! planted there out of the package. A raw write (`fs::write`, `File::create`, `.create(true)`
//! or `create_dir_all`) fails this test unless the allowlist below names it with its reason.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The calls that write without the checks of `una_diag::safe_write`.
const RAW_WRITES: [&str; 4] = [
    "fs::write(",
    "File::create(",
    ".create(true)",
    "create_dir_all(",
];

/// The crates whose sources are read.
const CRATES: [&str; 3] = ["una-cli", "una-verify", "una-codegen"];

/// The raw writes that remain, by file and call, with how many there are and why each is safe.
const ALLOWED: [(&str, &str, usize, &str); 1] = [(
    "crates/una-cli/src/build.rs",
    "create_dir_all(",
    1,
    "the directory that `una build --out` names on the command line: the user chose it, not \
     the package, and the files in it are still written through `una_diag::safe_write`",
)];

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
