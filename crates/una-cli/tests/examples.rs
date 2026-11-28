//! The example packages (`plan/milestones-v0.1.md` M10): every package of
//! `examples/` is canonical, checks at tier 1 with every obligation proven, passes its examples,
//! reports no trivial `ensures`, and runs when it has an entry; together they contain every
//! construct of the language list of charter §5. Tier 1 needs Z3 at the version of
//! `verification.lock` on the path. A package of edition `0.4` is written in `ita.una` (ADR
//! 0122); one with a `serve` section (`spec/31 §7`) is checked and tested, its tests calling
//! `handle` directly, but not run: `una run` of a serving program waits for `spec/31 §1` rule 10.
//! One with a `drive` section (`spec/26 §2`) runs until a host stop, so it runs with a stop
//! requested after five steps.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::Value;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn una(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(repository())
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The packages of `examples/`, by directory name: a manifest in `una.toml`, or in `ita.una`
/// from edition `0.4` on (ADR 0122).
fn packages() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repository().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            entry.path().join("una.toml").is_file() || entry.path().join("ita.una").is_file()
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The sources of a package, every `.una` file under `src/`.
fn sources(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(sources(&path));
        } else if path.extension().is_some_and(|e| e == "una") {
            out.push(std::fs::read_to_string(&path).unwrap());
        }
    }
    out
}

/// What `una run` prints for each package with an entry or a `drive` section, with its own
/// grants; a driven one with `--stop-after-iterations 5`.
const RUNS: &[(&str, &str)] = &[
    ("events", "availability of Launch: open\n"),
    (
        "greenhouse",
        "vent at 66%\nthe warmer side asks for 66% open\n",
    ),
    ("hello", "Hello, Una\n"),
    (
        "journal",
        "today's entry runs to about 2 hundred characters\n",
    ),
    (
        "metronome",
        "bar 1: tick\nbar 1: tock\nbar 1: tock\nbar 1: tock\nbar 2: tick\nstopped at bar 2, beat 1\n",
    ),
    ("pantry", "to buy: rice, oil\ngrams in stock: 700\n"),
    (
        "shortlinks",
        "docs leads to https://example.org/docs\nblog leads to /\n",
    ),
    (
        "toolshelf",
        "Ana took the ladder for 14 days\nreturned 20 days later, it is 6 days late\n",
    ),
    (
        "wordfreq",
        "{\"and\":2,\"bat\":1,\"cat\":1,\"hat\":1,\"the\":3}\n",
    ),
];

/// A `kind` declaration line: its name and what follows it.
fn kind_line(line: &str) -> Option<&str> {
    line.strip_prefix("pub kind ")
        .or_else(|| line.strip_prefix("kind "))
}

/// Whether `line` calls a function in method form, `value.name(...)`, on something other than
/// a capability reference.
fn method_call(line: &str) -> bool {
    let bytes = line.as_bytes();
    for (dot, _) in line.match_indices('.') {
        let name: String = line[dot + 1..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        let before = dot.checked_sub(1).and_then(|i| bytes.get(i)).copied();
        let after = bytes.get(dot + 1 + name.len()).copied();
        let receiver = line[..dot]
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '@'))
            .next()
            .unwrap_or("");
        if !name.is_empty()
            && after == Some(b'(')
            && before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b')' || b == b']')
            && !receiver.starts_with('@')
        {
            return true;
        }
    }
    false
}

/// Whether the sources of every example contain a construct.
type Found = fn(&str) -> bool;

/// The constructs of charter §5's language list, each with a test over the sources of every
/// example, each file introduced by a line break (ADR 0035 rule 1).
fn constructs() -> Vec<(&'static str, Found)> {
    vec![
        ("modules", |all| all.contains("\nmodule ")),
        ("imports", |all| all.contains("\nimport ")),
        ("`pub` visibility", |all| all.contains("\npub ")),
        ("an alias", |all| {
            all.lines().any(|line| {
                kind_line(line)
                    .and_then(|rest| rest.split_once(" = "))
                    .is_some_and(|(_, body)| {
                        !body.contains('|') && !body.contains(" where ") && !body.contains(" in ")
                    })
            })
        }),
        ("a record", |all| {
            let lines: Vec<&str> = all.lines().collect();
            lines.windows(2).any(|pair| {
                kind_line(pair[0]).is_some_and(|rest| !rest.contains('='))
                    && pair[1].starts_with("  ")
                    && pair[1]
                        .trim_start()
                        .starts_with(|c: char| c.is_ascii_lowercase())
                    && pair[1].contains(": ")
            })
        }),
        ("a sum with payloads", |all| {
            all.lines().any(|line| {
                kind_line(line).is_some_and(|rest| rest.contains('|') && rest.contains('('))
            })
        }),
        ("a generic kind", |all| {
            all.lines().any(|line| {
                kind_line(line).is_some_and(|rest| {
                    rest.split([' ', '='])
                        .next()
                        .is_some_and(|name| name.contains('['))
                })
            })
        }),
        ("a generic function", |all| {
            all.lines().any(|line| {
                let line = line.strip_prefix("pub ").unwrap_or(line);
                line.starts_with(|c: char| c.is_ascii_lowercase())
                    && line
                        .split('(')
                        .next()
                        .is_some_and(|name| name.ends_with(']') && name.contains('['))
            })
        }),
        ("a `where` refinement", |all| {
            all.lines()
                .any(|line| kind_line(line).is_some_and(|rest| rest.contains(" where ")))
        }),
        ("range sugar", |all| {
            all.lines().any(|line| {
                kind_line(line).is_some_and(|rest| rest.contains(" in ") && rest.contains(".."))
            })
        }),
        ("a reflect predicate", |all| {
            all.contains("predicate reflect ")
        }),
        ("an opaque predicate", |all| {
            all.contains("predicate opaque ")
        }),
        ("`requires`", |all| all.contains("\n  requires ")),
        ("`ensures`", |all| all.contains("\n  ensures ")),
        ("`uses`", |all| all.contains("\n  uses @")),
        ("`fails`", |all| all.contains("\n  fails ")),
        ("`derived from`", |all| all.contains("\n  derived from ")),
        ("labeled arguments", |all| {
            all.lines().any(|line| {
                line.match_indices(", ").any(|(at, _)| {
                    let rest = &line[at + 2..];
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                        .collect();
                    !name.is_empty() && rest[name.len()..].starts_with(": ")
                })
            })
        }),
        ("method calls", |all| all.lines().any(method_call)),
        ("lambdas", |all| {
            all.match_indices("fn(")
                .any(|(at, _)| all[at + 3..].starts_with(|c: char| c.is_ascii_lowercase()))
        }),
        ("function types", |all| {
            all.match_indices("fn(")
                .any(|(at, _)| all[at + 3..].starts_with(|c: char| c.is_ascii_uppercase()))
        }),
        ("`Int`", |all| all.contains(" Int")),
        ("`Nat`", |all| all.contains(" Nat")),
        ("`Bool`", |all| all.contains(" Bool")),
        ("`Text`", |all| all.contains(" Text")),
        ("`Unit`", |all| all.contains(" Unit")),
        ("`List`", |all| all.contains("List[")),
        ("`Option`", |all| all.contains("Option[")),
        ("`Result`", |all| all.contains("Result[")),
        ("`Hashed`", |all| all.contains(" Hashed")),
        ("a capability declaration", |all| {
            all.contains("\npub capability ") || all.contains("\ncapability ")
        }),
        ("`with` rebinding", |all| {
            all.lines().any(|line| {
                line.split_once(" with @")
                    .is_some_and(|(_, binding)| binding.contains(" = @"))
            })
        }),
        ("`with` a handler instance", |all| {
            all.lines().any(|line| {
                line.split_once(" with @").is_some_and(|(_, binding)| {
                    binding.split_once(" = ").is_some_and(|(_, source)| {
                        source.starts_with(|c: char| c.is_ascii_lowercase())
                    })
                })
            })
        }),
        ("a handler", |all| {
            all.contains("\npub handler ") || all.contains("\nhandler ")
        }),
        ("a handler with `uses`", |all| {
            let lines: Vec<&str> = all.lines().collect();
            lines.windows(3).any(|w| {
                (w[0].starts_with("handler ") || w[0].starts_with("pub handler "))
                    && (w[1].starts_with("  uses @") || w[2].starts_with("  uses @"))
            })
        }),
        ("`Console`", |all| all.contains("Console[")),
        ("`Clock`", |all| all.contains("Clock[")),
        ("`Env`", |all| all.contains("Env[")),
        ("`Files`", |all| all.contains("Files[")),
        ("`Keys`", |all| all.contains("Keys[")),
        ("`fail`", |all| {
            all.contains("= fail ")
                || all
                    .lines()
                    .any(|line| line.trim_start().starts_with("fail "))
        }),
        ("`if error fail`", |all| all.contains("if error fail ")),
        ("`if error pass`", |all| all.contains("if error pass")),
        ("`if error use`", |all| all.contains("if error use ")),
        ("`if none fail`", |all| all.contains("if none fail ")),
        ("`if none use`", |all| all.contains("if none use ")),
        ("`attempt`", |all| all.contains("attempt ")),
        ("`checked`", |all| all.contains("checked(")),
        ("label levels", |all| {
            ["internal ", "confidential", "secret"]
                .iter()
                .all(|level| all.contains(level))
        }),
        ("label categories", |all| {
            ["internal[", "confidential[", "secret[", "restricted["]
                .iter()
                .any(|level| all.contains(level))
        }),
        ("`accepts`", |all| all.contains("\n    accepts ")),
        ("a trusted module", |all| all.contains("\ntrusted module ")),
        ("a policy", |all| {
            all.contains("\npub policy ") || all.contains("\npolicy ")
        }),
        ("a requirement from a person", |all| {
            all.contains("\n  source human\n")
        }),
        ("a requirement from a policy", |all| {
            all.contains("\n  source policy \"")
        }),
        ("a requirement from a regulation", |all| {
            all.contains("\n  source regulation \"")
        }),
        ("a generated requirement, proposed", |all| {
            all.contains("\n  source generated \"") && all.contains("\n  status proposed\n")
        }),
        ("an active requirement", |all| {
            all.contains("\n  status active\n")
        }),
        ("examples", |all| all.contains("\nexample \"")),
        ("`given`", |all| all.contains("\n  given @")),
        ("`do`", |all| all.contains("\n  do ")),
        ("`expect`", |all| all.contains("\n  expect result")),
        ("`expect fails`", |all| all.contains("\n  expect fails ")),
        ("`expect performed`", |all| {
            all.contains("\n  expect performed @")
        }),
        ("`expect not performed`", |all| {
            all.contains("\n  expect not performed @")
        }),
        ("`when` over variants", |all| all.contains("\n    is ")),
        ("`when` over ranges", |all| all.contains("\n    in ")),
        ("`if` expressions", |all| {
            all.contains(" then ") && all.contains(" else ")
        }),
        ("text interpolation", |all| {
            all.lines().any(|line| {
                line.split('"')
                    .skip(1)
                    .step_by(2)
                    .any(|inside| inside.contains('{'))
            })
        }),
        ("doc comments", |all| all.contains("\n--- ")),
        ("an entry point", |all| {
            all.contains("\npub main() -> Unit\n")
        }),
    ]
}

#[test]
fn the_examples_contain_every_construct_of_the_charter_scope() {
    // Charter §5, at least five packages beside `events` and `hello`.
    let packages = packages();
    assert!(packages.len() >= 7, "{packages:?}");
    let mut all = String::new();
    for name in &packages {
        for source in sources(&repository().join("examples").join(name).join("src")) {
            all.push('\n');
            all.push_str(&source);
        }
    }
    let missing: Vec<&str> = constructs()
        .into_iter()
        .filter(|(_, found)| !found(&all))
        .map(|(construct, _)| construct)
        .collect();
    assert_eq!(missing, Vec::<&str>::new(), "constructs without an example");
    // Each pattern tells programs apart: `hello` has only what these six name.
    let mut hello = String::from("\n");
    hello.push_str(&sources(&repository().join("examples/hello/src")).concat());
    let in_hello: Vec<&str> = constructs()
        .into_iter()
        .filter(|(_, found)| found(&hello))
        .map(|(construct, _)| construct)
        .collect();
    assert_eq!(
        in_hello,
        [
            "modules",
            "`pub` visibility",
            "`uses`",
            "`Unit`",
            "`Console`",
            "an entry point"
        ]
    );
}

#[test]
fn every_example_package_is_canonical_verified_and_tested() {
    let packages = packages();
    assert!(packages.len() >= 3, "{packages:?}");
    let mut serving = Vec::new();
    let mut driven = Vec::new();
    for name in &packages {
        let path = format!("examples/{name}");
        let fmt = una(&["fmt", "--check", &path]);
        assert_eq!(fmt.status.code(), Some(0), "{name}: {}", text(&fmt.stderr));
        let check = una(&["check", &path]);
        assert_eq!(
            check.status.code(),
            Some(0),
            "{name}: {}",
            text(&check.stderr)
        );
        assert_eq!(text(&check.stderr), "", "{name}");
        let obligations = una(&["--json", "obligations", &path]);
        assert_eq!(obligations.status.code(), Some(0), "{name}");
        for obligation in document(&obligations)["obligations"].as_array().unwrap() {
            assert_eq!(obligation["status"], "proven", "{name}: {obligation}");
        }
        let test = una(&["--json", "test", &path]);
        assert_eq!(
            test.status.code(),
            Some(0),
            "{name}: {}",
            text(&test.stdout)
        );
        let declared: usize = sources(&repository().join(&path).join("src"))
            .iter()
            .map(|source| {
                // `example` blocks, and their edition `0.4` name `test`.
                source.matches("\nexample \"").count() + source.matches("\ntest \"").count()
            })
            .sum();
        let tested = document(&test);
        assert_eq!(tested["failed"], 0, "{name}");
        assert_eq!(tested["passed"], declared, "{name}: every example runs");
        let report = una(&["--json", "report", &path]);
        assert_eq!(report.status.code(), Some(0), "{name}");
        assert_eq!(
            document(&report)["coverage"]["trivial_ensures"],
            0,
            "{name}"
        );
        let (runs, serves, drives) =
            match std::fs::read_to_string(repository().join(&path).join("una.toml")) {
                Ok(manifest) => (manifest.contains("\n[run]\n"), false, false),
                Err(_) => {
                    let manifest =
                        std::fs::read_to_string(repository().join(&path).join("ita.una")).unwrap();
                    // A program of the language form that serves (an entry and a `serve:`
                    // grant, `spec/31 §1` rule 6) serves until it is stopped, as a `serve`
                    // section does: it is tested in `serve_live.rs`, not run here (#1793).
                    let grants_serve = manifest.contains("\n    serve \"");
                    (
                        manifest.contains("\nrun\n  entry ") && !grants_serve,
                        manifest.contains("\nserve\n") || grants_serve,
                        manifest.contains("\ndrive\n"),
                    )
                }
            };
        let expected = RUNS.iter().find(|(package, _)| package == name);
        assert_eq!(
            runs || drives,
            expected.is_some(),
            "{name}: a package with an entry or a `drive` section has its output in RUNS"
        );
        // A serving package is tested through `handle`; it runs in `serve_live.rs`.
        assert!(
            u8::from(runs) + u8::from(serves) + u8::from(drives) <= 1,
            "{name}: a package runs one program"
        );
        if serves {
            serving.push(name.as_str());
        }
        if drives {
            driven.push(name.as_str());
        }
        if let Some((_, output)) = expected {
            // `examples/journal` copies its entry only when `JOURNAL_COPY` is set.
            let run = Command::new(env!("CARGO_BIN_EXE_una"))
                .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                .env_remove("UNA_PROOF_KEY")
                .args(["run", &path])
                .args(if drives {
                    &["--stop-after-iterations", "5"][..]
                } else {
                    &[]
                })
                .env_remove("JOURNAL_COPY")
                .current_dir(repository())
                .output()
                .unwrap();
            assert_eq!(run.status.code(), Some(0), "{name}: {}", text(&run.stderr));
            assert_eq!(text(&run.stdout), *output, "{name}");
        }
    }
    // `examples/health`, `orders` and `proxy` serve (ADR 0123, "Examples"): the first two by
    // a `serve` section, the proxy in the language form (#1793).
    assert_eq!(serving, ["health", "orders", "proxy"]);
    // `examples/metronome` is driven by the host (`spec/26 §2`).
    assert_eq!(driven, ["metronome"]);
}
