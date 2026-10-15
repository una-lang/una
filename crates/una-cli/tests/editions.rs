//! Editions are cumulative (`spec/13 §7`): every edition `0.2` package of the conformance suite,
//! checked as edition `0.3`, reports byte-identical `una --json check` output and exit status.
//! The rules that target edition `0.3` change some of them: a construct such a rule adds, which
//! edition `0.2` rejects as a syntax error, an error such a rule adds, or an error such a rule
//! removes under the amended preview rule (ADR 0103 rule 9). Each such package is
//! listed in `EXPECTED_DIFFERENCES` with the exact result of both editions, and a listed package
//! that does not differ fails the test, so the list stays exact.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// An edition `0.2` package whose output in edition `0.3` differs by design, because of a rule
/// that targets edition `0.3` (`spec/13 §7`).
enum Difference {
    /// The package writes a construct that the rule adds and edition `0.2` rejects, as a syntax
    /// error or an unknown table of `una.toml`. The construct only narrows what a program may do, so erasing it gives a program
    /// that edition `0.2` accepts: the twin.
    Syntax {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// The one diagnostic of edition `0.2`: its code and `file:line:col`.
        edition_0_2: (&'static str, &'static str),
        /// The `effect_condition` obligations of edition `0.3`, each `status at file:line:col`,
        /// in order; the check itself reports nothing.
        edition_0_3: &'static [&'static str],
        /// The same package with the construct erased, relative to `tests/conformance`.
        twin: &'static str,
        /// The file of the package, relative to it, and the text, written there exactly once,
        /// whose removal gives the twin; every other file is the twin's.
        erased: (&'static str, &'static str),
    },
    /// The rule only adds errors: the diagnostics document of edition `0.3` is that of edition
    /// `0.2` with exactly these diagnostics added, each a code and `file:line:col`.
    Stricter {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// The added diagnostics.
        edition_0_3: &'static [(&'static str, &'static str)],
    },
    /// The rule only removes errors, as ADR 0103 rule 9 lets edition `0.3` accept refined type
    /// arguments and the facts they give (`spec/13 §7`): the diagnostics document of edition
    /// `0.2` is that of edition `0.3` with exactly these diagnostics added, each a code and
    /// `file:line:col`.
    Looser {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// The diagnostics of edition `0.2` that edition `0.3` does not report.
        edition_0_2: &'static [(&'static str, &'static str)],
    },
}

impl Difference {
    /// The package, relative to `tests/conformance`.
    fn case(&self) -> &'static str {
        match self {
            Difference::Syntax { case, .. }
            | Difference::Stricter { case, .. }
            | Difference::Looser { case, .. } => case,
        }
    }

    /// The edition `0.2` packages the entry names.
    fn packages(&self) -> Vec<&'static str> {
        match self {
            Difference::Syntax { case, twin, .. } => vec![*case, *twin],
            Difference::Stricter { case, .. } | Difference::Looser { case, .. } => vec![*case],
        }
    }
}

/// The edition `0.2` packages whose output in edition `0.3` differs by design, with the exact
/// result of each edition.
const EXPECTED_DIFFERENCES: &[Difference] = &[
    // ADR 0104 rule 1: a unit declaration, a parse error before edition 0.3.
    Difference::Syntax {
        case: "types/units_edition_0_2",
        edition_0_2: ("U0101", "src/m.una:6:13"),
        edition_0_3: &[],
        twin: "types/units_edition_0_2_erased",
        erased: ("src/m.una", "pub measure Meter\n\n"),
    },
    // ADR 0092 rule 1: a condition on an operation of a `uses`.
    Difference::Syntax {
        case: "authority/op_condition_edition_0_2",
        edition_0_2: ("U0101", "src/main.una:10:24"),
        edition_0_3: &["proven at src/main.una:13:5"],
        twin: "authority/op_condition_edition_0_2_erased",
        erased: ("src/main.una", " where not dry_run"),
    },
    // ADR 0092 rule 8: a lock of `[contracts]`, an unknown table before edition 0.3.
    Difference::Syntax {
        case: "authority/contracts_edition_0_2",
        edition_0_2: ("U7002", "una.toml:6:2"),
        edition_0_3: &[],
        twin: "authority/contracts_edition_0_2_erased",
        erased: (
            "una.toml",
            "\n[contracts]\n\"main.update\" = [\"@catalog: Catalog[price]\"]\n",
        ),
    },
    // ADR 0093 rule 4: a narrowed rebinding, `with @x = @y[ops]`.
    Difference::Syntax {
        case: "authority/narrowed_rebinding_edition_0_2",
        edition_0_2: ("U0101", "src/main.una:20:39"),
        edition_0_3: &[],
        twin: "authority/narrowed_rebinding_edition_0_2_erased",
        erased: ("src/main.una", "[write, read]"),
    },
    // ADR 0093 rule 1: the `[boundaries]` table of `una.toml`.
    Difference::Syntax {
        case: "project/boundaries_edition_0_2",
        edition_0_2: ("U7002", "una.toml:6:2"),
        edition_0_3: &[],
        twin: "project/boundaries_edition_0_2_erased",
        erased: (
            "una.toml",
            "\n[boundaries]\n\"gallery.thumbnailer\" = [\"Files[read]\"]\n",
        ),
    },
    // ADR 0100 rule 2: `attempt` of a call that cannot fail.
    Difference::Stricter {
        case: "authority/dead_attempt_edition_0_2",
        edition_0_3: &[("U3026", "src/main.una:8:42")],
    },
    // ADR 0100 rule 1: an `if error` clause on an expression that cannot fail.
    Difference::Stricter {
        case: "authority/dead_clause_edition_0_2",
        edition_0_3: &[
            ("U3025", "src/main.una:22:5"),
            ("U3025", "src/main.una:24:5"),
            ("U3025", "src/main.una:31:5"),
            ("U3025", "src/main.una:37:5"),
        ],
    },
    // ADR 0100 rule 2: `attempt` of a `fails of` call given a function that cannot fail.
    Difference::Stricter {
        case: "runtime/fails_of_relay",
        edition_0_3: &[
            ("U3026", "src/relay.una:81:12"),
            ("U3026", "src/relay.una:83:12"),
        ],
    },
    // ADR 0103 rule 1: a refined type argument, U2002 before edition 0.3.
    Difference::Looser {
        case: "verification/refined_elements_0_2",
        edition_0_2: &[("U2002", "src/edition_0_2.una:8:22")],
    },
    // ADR 0103 rule 6: `[p]` is a `List[Pos]` in edition 0.3, so `first`'s element carries `Pos`.
    Difference::Looser {
        case: "verification/refined_elements_0_2_facts",
        edition_0_2: &[("U5001", "src/edition_0_2_facts.una:14:25")],
    },
];

const EDITION_0_2: &str = "edition = \"0.2\"";
const EDITION_0_3: &str = "edition = \"0.3\"";

/// The start of every `una --json check` document (`spec/13 §5`).
const DOCUMENT: &[u8] = b"{\"schema\":\"una.diagnostics/1\"";

/// The start of every `una --json obligations` document.
const OBLIGATIONS: &[u8] = b"{\"schema\":\"una.obligations/1\"";

/// An edition `0.2` package that uses fixed-width integers, so it checks differently as edition
/// `0.1`: the positive control that the copied manifest is the one read.
const CONTROL: &str = "types/fixed_width_methods";

fn conformance() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}

/// Every package directory under `dir`, in a stable order.
fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    for entry in entries {
        if entry.join("una.toml").is_file() {
            out.push(entry);
        } else {
            packages(&entry, out);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The exit status and output of `una --json check .` on a fresh copy of `package` at `work`,
/// with its manifest's edition line replaced by `edition`.
fn check_as(package: &Path, work: &Path, edition: &str) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    run_as(package, work, edition, "check", DOCUMENT)
}

/// The exit status and output of `una --json <command> .` on a fresh copy of `package` at
/// `work`, with its manifest's edition line replaced by `edition`; stdout starts with `document`.
fn run_as(
    package: &Path,
    work: &Path,
    edition: &str,
    command: &str,
    document: &[u8],
) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    if work.exists() {
        std::fs::remove_dir_all(work).unwrap();
    }
    copy_dir(package, work);
    let manifest = work.join("una.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replace(EDITION_0_2, edition)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["--json", command, "."])
        .current_dir(work)
        .output()
        .unwrap();
    std::fs::remove_dir_all(work).unwrap();
    // A run that fails the same way in both editions, before it checks anything, must not count
    // as the same result: each run exits 0 or 1 with a diagnostics document.
    assert!(
        matches!(output.status.code(), Some(0 | 1)) && output.stdout.starts_with(document),
        "{} as {edition}: status {:?}, stdout {:?}, stderr {:?}",
        package.display(),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), output.stdout, output.stderr)
}

#[test]
fn every_edition_0_2_package_checks_the_same_in_edition_0_3() {
    let root = conformance();
    let mut all = Vec::new();
    packages(&root, &mut all);
    // An edition line that names `0.2` in another spelling would drop its package from the test
    // without a word.
    for package in &all {
        let text = std::fs::read_to_string(package.join("una.toml")).unwrap();
        for line in text.lines() {
            let line = line.trim();
            assert!(
                !(line.starts_with("edition") && line.contains("0.2")) || line == EDITION_0_2,
                "{}: the edition line `{line}` is not `{EDITION_0_2}`",
                package.display()
            );
        }
    }
    let cases: Vec<(String, PathBuf)> = all
        .into_iter()
        .filter(|package| {
            std::fs::read_to_string(package.join("una.toml"))
                .unwrap()
                .lines()
                .any(|line| line.trim() == EDITION_0_2)
        })
        .map(|package| {
            let name = package
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (name, package)
        })
        .collect();
    let (_, control) = cases
        .iter()
        .find(|(name, _)| name == CONTROL)
        .expect("the control is an edition 0.2 package");
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions/control");
    assert_ne!(
        check_as(control, &work, EDITION_0_2),
        check_as(control, &work, "edition = \"0.1\""),
        "{CONTROL} checks the same as edition 0.1"
    );
    for expected in EXPECTED_DIFFERENCES {
        for name in expected.packages() {
            assert!(
                cases.iter().any(|(case, _)| case == name),
                "{name} is not an edition 0.2 package"
            );
        }
    }

    // Linear in the packages: each one is checked twice, on its own copy.
    let workers = 4;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions");
    let results: Vec<(String, bool)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let cases = &cases;
                let work = scratch.join(format!("w{worker}"));
                scope.spawn(move || {
                    cases
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(name, package)| {
                            let before = check_as(package, &work, EDITION_0_2);
                            let after = check_as(package, &work, EDITION_0_3);
                            (name.clone(), before == after)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    let mut wrong = Vec::new();
    for (name, same) in &results {
        let expected = EXPECTED_DIFFERENCES
            .iter()
            .any(|difference| difference.case() == name);
        if *same == expected {
            wrong.push(if expected {
                format!("{name}: listed as differing, but checks the same in edition 0.3")
            } else {
                format!("{name}: checks differently in edition 0.3")
            });
        }
    }
    wrong.sort();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The `file:line:col` of a location of the JSON documents.
fn location(at: &Value) -> String {
    let file = at["file"].as_str().unwrap();
    let start = &at["start"];
    format!("{file}:{}:{}", start["line"], start["col"])
}

/// Every file of a package by its path relative to the package, a module without its leading
/// comment lines, which state the test.
fn package_files(package: &Path) -> std::collections::BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
                continue;
            }
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).unwrap();
            let text = if name.ends_with(".una") {
                text.lines()
                    .skip_while(|line| line.starts_with("--"))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                text
            };
            out.insert(name, text);
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(package, package, &mut out);
    out
}

/// The code and `file:line:col` of each diagnostic of a diagnostics document.
fn sites(document: &Value) -> Vec<(String, String)> {
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_owned(),
                location(&d["primary"]),
            )
        })
        .collect()
}

/// Each expected difference is exactly the pinned one, so any other drift of its package still
/// fails.
#[test]
fn every_expected_difference_is_exact() {
    let root = conformance();
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions/differences");
    for expected in EXPECTED_DIFFERENCES {
        match expected {
            Difference::Syntax {
                case,
                edition_0_2,
                edition_0_3,
                twin,
                erased,
            } => syntax(
                &root,
                &scratch,
                case,
                *edition_0_2,
                edition_0_3,
                twin,
                *erased,
            ),
            Difference::Stricter { case, edition_0_3 } => {
                stricter(&root, &scratch, case, edition_0_3);
            }
            Difference::Looser { case, edition_0_2 } => {
                looser(&root, &scratch, case, edition_0_2);
            }
        }
    }
}

/// A [`Difference::Syntax`]: the pinned syntax error in edition `0.2`, a clean check with
/// exactly the pinned obligations in edition `0.3`, and an erasure twin that edition `0.2`
/// accepts and edition `0.3` checks the same.
fn syntax(
    root: &Path,
    scratch: &Path,
    name: &str,
    edition_0_2: (&str, &str),
    edition_0_3: &[&str],
    twin_name: &str,
    erased: (&str, &str),
) {
    let case = root.join(name);
    let twin = root.join(twin_name);

    // Edition 0.2: exactly one diagnostic, the pinned syntax error.
    let (status, stdout, _) = check_as(&case, scratch, EDITION_0_2);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    let (code, at) = edition_0_2;
    assert_eq!(status, Some(1), "{name} as edition 0.2");
    assert_eq!(
        sites(&document),
        vec![(code.to_owned(), at.to_owned())],
        "{name} as edition 0.2"
    );

    // Edition 0.3: a clean check, and exactly the pinned obligations of the construct.
    let (status, stdout, _) = check_as(&case, scratch, EDITION_0_3);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(status, Some(0), "{name} as edition 0.3");
    assert_eq!(sites(&document), Vec::new(), "{name} as edition 0.3");
    let (_, stdout, _) = run_as(&case, scratch, EDITION_0_3, "obligations", OBLIGATIONS);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    let found: Vec<String> = document["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] == "effect_condition")
        .map(|o| {
            format!(
                "{} at {}",
                o["status"].as_str().unwrap(),
                location(&o["source"])
            )
        })
        .collect();
    assert_eq!(found, edition_0_3, "{name} as edition 0.3");

    // The twin is the case with the construct erased: the same files, each the same, except
    // the one that held the construct exactly once, without it.
    let (file, text) = erased;
    let mut expected = package_files(&case);
    let held = expected
        .get_mut(file)
        .unwrap_or_else(|| panic!("{name}: no file `{file}`"));
    assert_eq!(
        held.matches(text).count(),
        1,
        "{name}: `{text}` is not written exactly once in `{file}`"
    );
    *held = held.replacen(text, "", 1);
    assert_eq!(
        expected,
        package_files(&twin),
        "{twin_name} is not {name} with `{text}` erased from `{file}`"
    );

    // Edition 0.2 accepts the twin, and edition 0.3 checks it the same, with no obligation of
    // the construct.
    let before = check_as(&twin, scratch, EDITION_0_2);
    let document: Value = serde_json::from_slice(&before.1).unwrap();
    assert_eq!(before.0, Some(0), "{twin_name} as edition 0.2");
    assert_eq!(sites(&document), Vec::new(), "{twin_name} as edition 0.2");
    assert_eq!(
        check_as(&twin, scratch, EDITION_0_3),
        before,
        "{twin_name} as edition 0.3"
    );
    let (_, stdout, _) = run_as(&twin, scratch, EDITION_0_3, "obligations", OBLIGATIONS);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    assert!(
        !document["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "effect_condition"),
        "{twin_name} as edition 0.3"
    );
}

/// A [`Difference::Stricter`]: the whole diagnostics document of edition `0.3` is that of
/// edition `0.2` with exactly the listed diagnostics added, nothing removed and the others in
/// their order, and edition `0.3` exits 1.
fn stricter(root: &Path, scratch: &Path, name: &str, added: &[(&str, &str)]) {
    let case = root.join(name);
    let (_, before, _) = check_as(&case, scratch, EDITION_0_2);
    let (status, after, _) = check_as(&case, scratch, EDITION_0_3);
    let before: Value = serde_json::from_slice(&before).unwrap();
    let after: Value = serde_json::from_slice(&after).unwrap();
    assert!(!added.is_empty(), "{name}: a stricter entry adds nothing");
    assert_eq!(status, Some(1), "{name} as edition 0.3");
    differs_by(name, "edition 0.3", "edition 0.2", after, &before, added);
}

/// A [`Difference::Looser`]: edition `0.2` reports exactly the pinned diagnostics more than
/// edition `0.3` does, and nothing else differs.
fn looser(root: &Path, scratch: &Path, name: &str, removed: &[(&str, &str)]) {
    let case = root.join(name);
    let (status, before, _) = check_as(&case, scratch, EDITION_0_2);
    let (_, after, _) = check_as(&case, scratch, EDITION_0_3);
    let before: Value = serde_json::from_slice(&before).unwrap();
    let after: Value = serde_json::from_slice(&after).unwrap();
    assert!(
        !removed.is_empty(),
        "{name}: a looser entry removes nothing"
    );
    assert_eq!(status, Some(1), "{name} as edition 0.2");
    differs_by(name, "edition 0.2", "edition 0.3", before, &after, removed);
}

/// That `more`, the document of `more_edition`, is `less` with exactly the `listed`
/// diagnostics added.
fn differs_by(
    name: &str,
    more_edition: &str,
    less_edition: &str,
    mut more: Value,
    less: &Value,
    listed: &[(&str, &str)],
) {
    // Each listed diagnostic is taken out once, so a duplicate or a missing one fails, and
    // `fixable` loses those that `una fix` applies (`spec/13 §5`).
    let mut fixable = 0;
    let diagnostics = more["diagnostics"].as_array_mut().unwrap();
    for (code, at) in listed {
        let found = diagnostics
            .iter()
            .position(|d| d["code"] == *code && location(&d["primary"]) == *at);
        let Some(index) = found else {
            panic!("{name} as {more_edition}: no {code} at {at}");
        };
        let removed = diagnostics.remove(index);
        let first = &removed["repairs"][0];
        if !first["edit"].is_null()
            && first["escape_cost"] == 0
            && first.get("widens").is_none_or(Value::is_null)
        {
            fixable += 1;
        }
    }
    more["fixable"] = Value::from(more["fixable"].as_u64().unwrap() - fixable);
    assert_eq!(
        &more, less,
        "{name}: {more_edition} differs from {less_edition} by more than the listed diagnostics"
    );
}
