//! The Core IR that `una core --json` exports keeps a plain symbolic label `ℓ_T` on a position of
//! type `T` only (#440): everywhere else a symbol of a type parameter is starred, `ℓ*_T`. The
//! formal model's reading of `ℓ_T` needs it (`LTy.InPlace`, `spec/07 §7` rule 8), and the Lean
//! re-checker reads this export, so the checker and the model cannot drift apart.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every package and single-file case of the corpus: the conformance cases and the examples.
fn corpus() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if dir.join("una.toml").is_file() {
            out.push(dir.to_path_buf());
            return;
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "una") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&root().join("tests/conformance"), &mut out);
    walk(&root().join("examples"), &mut out);
    out
}

/// A symbol of a type parameter that is not starred.
fn plain(symbol: &str) -> bool {
    !symbol.ends_with('*') && !symbol.starts_with('?')
}

/// The plain symbols of a label object `{level, categories, symbols}`.
fn plain_symbols(label: &Value) -> Vec<String> {
    label["symbols"]
        .as_array()
        .map(|symbols| {
            symbols
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| plain(s))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Walks a node, recording each label with a plain symbol off its own type parameter.
fn walk(node: &Value, path: &str, out: &mut Vec<String>) {
    match node {
        Value::Object(fields) => {
            let own = (fields.get("kind").and_then(Value::as_str) == Some("Base"))
                .then(|| &fields["base"])
                .filter(|base| base["kind"] == "Param")
                .and_then(|base| base["value"].as_str());
            for (key, value) in fields {
                let here = format!("{path}.{key}");
                if key == "label" && value.get("symbols").is_some() {
                    for symbol in plain_symbols(value) {
                        if own != Some(symbol.as_str()) {
                            out.push(format!("{here}: plain `{symbol}`"));
                        }
                    }
                } else if key == "labels" {
                    for (index, label) in value.as_array().into_iter().flatten().enumerate() {
                        for symbol in plain_symbols(label) {
                            out.push(format!("{here}[{index}]: plain `{symbol}`"));
                        }
                    }
                } else {
                    walk(value, &here, out);
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk(item, &format!("{path}[{index}]"), out);
            }
        }
        _ => {}
    }
}

#[test]
fn core_exports_a_plain_symbolic_label_only_on_its_own_type_parameter() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in corpus() {
        let output = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .args(["--json", "core"])
            .arg(&case)
            .current_dir(root())
            .output()
            .unwrap();
        let Ok(document) = serde_json::from_slice::<Value>(&output.stdout) else {
            continue;
        };
        checked += 1;
        let mut found = Vec::new();
        walk(&document, "", &mut found);
        for finding in found {
            failures.push(format!("{}: {finding}", case.display()));
        }
    }
    assert!(checked > 500, "only {checked} cases exported Core IR");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
