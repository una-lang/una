//! The Core IR that `una core --json` exports keeps a plain symbolic label `ℓ_T` on a position of
//! type `T` only (#440): everywhere else a symbol of a type parameter is starred, `ℓ*_T`. The
//! formal model's reading of `ℓ_T` needs it (`LTy.InPlace`, `spec/07 §7` rule 8), and the Lean
//! re-checker reads this export, so the checker and the model cannot drift apart.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

#[path = "common/case_lists.rs"]
mod case_lists;
#[path = "common/hermetic.rs"]
mod hermetic;

use std::process::Command;

use serde_json::Value;

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

/// Walks a node, recording each label with a plain symbol off its own type parameter, and
/// counting the plain symbols in place.
fn walk(node: &Value, path: &str, out: &mut Vec<String>, in_place: &mut usize) {
    match node {
        Value::Object(fields) => {
            let own = (fields.get("kind").and_then(Value::as_str) == Some("base"))
                .then(|| &fields["base"])
                .filter(|base| base["kind"] == "param")
                .and_then(|base| base["value"].as_str());
            for (key, value) in fields {
                let here = format!("{path}.{key}");
                if key == "label" && value.get("symbols").is_some() {
                    for symbol in plain_symbols(value) {
                        if own == Some(symbol.as_str()) {
                            *in_place += 1;
                        } else {
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
                    walk(value, &here, out, in_place);
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk(item, &format!("{path}[{index}]"), out, in_place);
            }
        }
        _ => {}
    }
}

/// What `una --json core` prints for `case`, a package or single file by path relative to the
/// repository root.
enum Export {
    /// A document, with the labels that hold a plain symbol off its own type parameter, the
    /// number of plain symbolic labels in place, and the edition it names.
    Walked(Vec<String>, usize, Option<String>),
    /// A document nested deeper than the parser's recursion limit of 128, which the walk
    /// passes over, as it did before #1652.
    TooDeep,
    /// No document at all.
    Nothing(std::process::Output),
}

fn exports(case: &str) -> Export {
    // A sealed copy of the case, or of the package that holds it, so that a module file of a
    // package keeps the package's edition and modules: no manifest above the checkout changes
    // the edition of its single files (#1560).
    let package = case_lists::package_of(case, &hermetic::repository());
    let copy = hermetic::checkout_case(&[package.as_deref().unwrap_or(case)]);
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "core"])
        .arg(copy.root.join(case))
        .current_dir(&copy.root)
        .output()
        .unwrap();
    match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(document) => {
            let mut found = Vec::new();
            let mut in_place = 0;
            walk(&document, "", &mut found, &mut in_place);
            let edition = document["edition"].as_str().map(str::to_owned);
            Export::Walked(found, in_place, edition)
        }
        Err(error)
            if output.stdout.starts_with(b"{\"schema\":")
                && error.to_string().starts_with("recursion limit exceeded") =>
        {
            Export::TooDeep
        }
        Err(_) => Export::Nothing(output),
    }
}

/// The case `case` exports Core IR with a plain symbolic label only on its own type parameter.
/// Before #1652, one test over the whole corpus passed over a case whose output did not parse,
/// so long as more than 500 parsed; each case holds on its own now that it prints a document,
/// which parses unless it is nested too deep for the parser.
fn core_exports_a_plain_symbolic_label_only_on_its_own_type_parameter(case: &str) {
    match exports(case) {
        Export::Walked(found, _, edition) => {
            // The edition the case has in place: the one `una` gives it in its package, read
            // from the checkout's own manifest, or 0.1 for a single file outside every package
            // (ADR 0021), so a copy that lost its package fails here.
            let repository = hermetic::repository();
            let expected = match case_lists::package_of(case, &repository) {
                Some(_) => {
                    let output = Command::new(env!("CARGO_BIN_EXE_una"))
                        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                        .env_remove("UNA_PROOF_KEY")
                        .args(["--json", "core", case])
                        .current_dir(&repository)
                        .output()
                        .unwrap();
                    String::from_utf8_lossy(&output.stdout)
                        .split_once("\"edition\":\"")
                        .and_then(|(_, rest)| rest.split_once('"'))
                        .map(|(edition, _)| edition.to_owned())
                }
                None => Some("0.1".to_owned()),
            };
            if edition.is_some() || expected.is_some() {
                assert_eq!(edition, expected, "{case}: the edition of its Core IR");
            }
            let failures: Vec<String> = found
                .into_iter()
                .map(|finding| format!("{case}: {finding}"))
                .collect();
            assert!(failures.is_empty(), "{}", failures.join("\n"));
        }
        Export::TooDeep => {}
        Export::Nothing(output) => {
            panic!("{case}: `una --json core` printed no document: {output:?}")
        }
    }
}

/// One test per package or single file of the corpus (#1652), `core_ir::<name>`.
macro_rules! core_tests {
    ($($name:ident => $path:literal [],)*) => {
        /// The Core IR of every case keeps a plain symbolic label on its own type parameter only.
        mod core_ir {
            $(
                #[test]
                fn $name() {
                    super::core_exports_a_plain_symbolic_label_only_on_its_own_type_parameter($path);
                }
            )*
            /// The generated corpus, in order.
            pub const CORPUS: &[&str] = &[$($path),*];
        }
    };
}

una_test_cases::core_corpus!(core_tests);

/// The generated tests are the packages and single files on disk, more than 500 of them: a case
/// added, removed or renamed without a rebuild of `una-test-cases` fails here, never goes
/// untested.
#[test]
fn the_core_corpus_is_the_packages_and_files_on_disk() {
    let repository = hermetic::repository();
    assert!(repository.join("examples").is_dir(), "examples is missing");
    let on_disk = case_lists::core_corpus(&repository);
    assert!(on_disk.len() > 500, "only {} cases", on_disk.len());
    assert_eq!(core_ir::CORPUS, on_disk.as_slice());
}

/// The corpus has generic bodies, so the walk must see `ℓ_T` in place, or it checks nothing:
/// the examples hold one, which the walk finds.
#[test]
fn the_walk_sees_a_plain_symbolic_label_in_place() {
    let in_place: usize = core_ir::CORPUS
        .iter()
        .filter(|case| case.starts_with("examples/"))
        .map(|case| match exports(case) {
            Export::Walked(_, in_place, _) => in_place,
            _ => 0,
        })
        .sum();
    assert!(in_place > 0, "no plain symbolic label found in place");
}
