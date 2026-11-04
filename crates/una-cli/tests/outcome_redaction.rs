//! S-45 (#1552, `spec/11 §5`): a run's trace renders each operation's outcome at the label of
//! the operation site's decision, joined with the labels of its failure type and of its result.
//! An outcome above `public` is `{"kind":"redacted","level":...}`: whether the operation
//! failed, its variant and its value are hidden, so the outcomes of `Files` operations no longer
//! spell out an `internal` path that the same trace redacts. The leak tests compare whole
//! `una --json run` documents across secrets; the others check that outcomes whose decision,
//! failure type and result are `public` stay in clear, and that an example's trace keeps its
//! outcomes in clear. They check at tier 1 first, so they need the solver of
//! `verification.lock`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs the `una` binary in `dir`.
fn una_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        // The proof cache's key lives under this home, never the user's (#1164).
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// A fresh package of edition `0.1` named `name` whose module `leak` is `program`, granted
/// `console` and `files` read and write under an empty `data/`.
fn package(name: &str, program: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("outcome_redaction")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("data/data")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n\n[run]\nentry = \"leak\"\ngrants = [\"console\", \"files:data:read,write\"]\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/leak.una"), program).unwrap();
    root
}

/// The `una --json run` document of `source` with `SECRET` replaced by `secret`, in a fresh
/// package with an empty `data/`.
fn run(name: &str, source: &str, secret: i64) -> Value {
    let program = source.replace("SECRET", &secret.to_string());
    let dir = package(&format!("{name}{secret}"), &program);
    let output = una_in(&dir, &["--json", "run", "."]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{name} {secret}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    document
}

/// The one `una --json run` document of `source` for every secret of `secrets`: the whole
/// document, trace and output, is the same for each.
fn one_document(name: &str, source: &str, secrets: &[i64]) -> Value {
    let mut first: Option<Value> = None;
    for &secret in secrets {
        let document = run(name, source, secret);
        match &first {
            None => first = Some(document),
            Some(seen) => assert_eq!(
                &document, seen,
                "{name} {secret}: the run's document depends on the secret"
            ),
        }
    }
    first.unwrap()
}

/// The outcome of each event of `document`'s trace, compactly: `op:kind`, `op:fail:<variant>`
/// or `op:redacted:<level>`.
fn outcomes(document: &Value) -> Vec<String> {
    document["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|event| {
            let outcome = &event["outcome"];
            let kind = outcome["kind"].as_str().unwrap_or("?");
            let shown = match (outcome["variant"].as_str(), outcome["level"].as_str()) {
                (Some(variant), _) => format!("fail:{variant}"),
                (None, Some(level)) => format!("{kind}:{level}"),
                (None, None) => kind.to_owned(),
            };
            format!("{}:{shown}", event["op"].as_str().unwrap_or("?"))
        })
        .collect()
}

/// A `Files.write` of an `internal` path at a `public` program counter, then public reads of
/// both paths it may have written (`reviews/1552-repro-rel/pub01.una`). On v0.1.13 the reads'
/// outcomes, `ok` and `fail NotFound`, gave the path the write's event shows as
/// `<redacted:internal>`.
const PUBLIC_PC_WRITE: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  let p: internal Text = if s > 0 then "data/f1" else "data/f0"
  let w = attempt @files.write(p, content: "x")
  let r0 = attempt @files.read("data/f0")
  let r1 = attempt @files.read("data/f1")
  @console.print("done")
"#;

/// Three bits of the secret written as the names of files, read back at public paths
/// (`reviews/1552-repro-rel/bits01.una`): on v0.1.13 the reads gave the secret exactly.
const BITS: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  let p0: internal Text = "data/b0_{s mod 2}"
  let p1: internal Text = "data/b1_{(s / 2) mod 2}"
  let p2: internal Text = "data/b2_{(s / 4) mod 2}"
  let w0 = attempt @files.write(p0, content: "x")
  let w1 = attempt @files.write(p1, content: "x")
  let w2 = attempt @files.write(p2, content: "x")
  let r0 = attempt @files.read("data/b0_1")
  let r1 = attempt @files.read("data/b1_1")
  let r2 = attempt @files.read("data/b2_1")
  @console.print("done")
"#;

/// The write's own outcome decided by its `internal` path: a directory that does not exist
/// (`reviews/1552-repro-rel/r3write01.una`). On v0.1.13 it was `ok` or `fail NotFound`.
const OWN_OUTCOME: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[write]
=
  let s: internal Int = SECRET
  let p: internal Text = if s > 0 then "data/missing/f" else "data/f"
  let w = attempt @files.write(p, content: "x")
  @console.print("done")
"#;

#[test]
fn a_write_of_an_internal_path_shows_in_no_later_read_s45() {
    let document = one_document("pubpc", PUBLIC_PC_WRITE, &[0, 1, 9]);
    assert_eq!(
        outcomes(&document),
        [
            "write:redacted:internal",
            "read:redacted:internal",
            "read:redacted:internal",
            "print:ok",
        ]
    );
    assert_eq!(
        document["trace"][0]["args"],
        json!({"path": "<redacted:internal>", "content": "\"x\""})
    );
    // The redacted kind carries the level only: no variant, no value.
    assert_eq!(
        document["trace"][1]["outcome"],
        json!({"kind": "redacted", "level": "internal"})
    );
}

#[test]
fn bits_of_a_secret_written_as_paths_read_back_the_same_s45() {
    let document = one_document("bits", BITS, &[0, 5, 6, 7]);
    assert!(
        outcomes(&document)
            .iter()
            .filter(|o| !o.starts_with("print:"))
            .all(|o| o.ends_with(":redacted:internal")),
        "{:?}",
        outcomes(&document)
    );
}

#[test]
fn the_outcome_of_a_write_to_an_internal_path_is_redacted_s45() {
    let document = one_document("own", OWN_OUTCOME, &[0, 1, 9]);
    assert_eq!(outcomes(&document), ["write:redacted:internal", "print:ok"]);
}

/// The program sees the same outcome only under the label: printing on it is U4003
/// (`reviews/1552-repro-rel/control01.una`), so the trace was the only channel.
#[test]
fn the_program_cannot_print_the_outcome_s45() {
    let program = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = 1
  let p: internal Text = if s > 0 then "data/f1" else "data/f0"
  let w = attempt @files.write(p, content: "x")
  let r0 = attempt @files.read("data/f0")
  when r0
    is Ok -> @console.print("zero")
    is Err -> @console.print("positive")
"#;
    let dir = package("control", program);
    let output = una_in(&dir, &["--json", "check", "."]);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("U4003"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// A package whose writes all have `public` paths at a `public` program counter has a
/// `public` file-state label: a write's outcome stays in clear, `ok` or its failure's variant.
/// A read's result is `internal Text`, so its outcome is redacted whatever the file state.
#[test]
fn public_decisions_keep_their_outcomes_in_clear_s45() {
    let program = r#"module leak

pub kind BoxError = | Full

pub capability Box
  op put(n: Int) -> Int
    fails BoxError

pub handler small() for Box
  op put(n) = if n > 2 then fail Full else n + 1

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let w0 = attempt @files.write("data/f", content: "x")
  let w1 = attempt @files.write("data/missing/f", content: "x")
  let r = attempt @files.read("data/f")
  let b0 = attempt @box.put(1) with @box = small()
  let b1 = attempt @box.put(5) with @box = small()
  @console.print("done")
"#;
    let document = run("public", program, 0);
    assert_eq!(
        outcomes(&document),
        [
            "write:ok",
            "write:fail:NotFound",
            "read:redacted:internal",
            "put:ok",
            "put:fail:Full",
            "print:ok",
        ]
    );
    assert_eq!(
        document["trace"][3]["outcome"],
        json!({"kind": "ok", "value": "2"})
    );
    assert_eq!(
        document["trace"][4]["outcome"],
        json!({"kind": "fail", "variant": "Full", "value": "Full"})
    );
}

/// A handler operation whose result is `internal` is redacted whole in a run, and the same
/// operation in an example's trace keeps its outcome in clear, its data from its source.
#[test]
fn an_examples_trace_keeps_its_outcomes_in_clear_s45() {
    let program = r#"module leak

pub kind VaultError = | Locked

pub capability Vault
  op open(n: Int) -> internal Int
    fails VaultError

pub handler vault() for Vault
  op open(n) = if n > 2 then fail Locked else n + 1

pub peek(n: Int) -> Unit
  uses @vault: Vault[open]
=
  let r = attempt @vault.open(n)
  unit

pub main() -> Unit
  uses @console: Console[print]
=
  let u = peek(1) with @vault = vault()
  let v = peek(5) with @vault = vault()
  @console.print("done")

example "the vault opens"
  given @vault = vault()
  do peek(1)
  expect performed @vault.open

example "the vault is locked"
  given @vault = vault()
  do peek(5)
  expect performed @vault.open
"#;
    let document = run("example", program, 0);
    assert_eq!(
        outcomes(&document),
        [
            "open:redacted:internal",
            "open:redacted:internal",
            "print:ok"
        ]
    );
    let dir = package("example-test", program);
    let output = una_in(&dir, &["--json", "test", "."]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tested: Value = serde_json::from_slice(&output.stdout).unwrap();
    let traces: Vec<Value> = tested["examples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|example| example["trace"][0]["outcome"].clone())
        .collect();
    assert_eq!(
        traces,
        [
            json!({"kind": "ok", "value": "<redacted:internal>"}),
            json!({"kind": "fail", "variant": "Locked", "value": "Locked"}),
        ]
    );
}

/// Core IR carries the outcome label, and `--dump-core` prints it after an operation whose
/// outcome a run's trace redacts.
#[test]
fn dump_core_prints_the_outcome_label_s45() {
    let dir = package("dump", &PUBLIC_PC_WRITE.replace("SECRET", "1"));
    let output = una_in(&dir, &["check", "--dump-core", "leak", "."]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let core = String::from_utf8_lossy(&output.stdout);
    assert!(
        core.contains(r#"files.write(p, "x") outcome internal"#),
        "{core}"
    );
    assert!(core.contains("console.print(\"done\") in"), "{core}");
    assert!(!core.contains("console.print(\"done\") outcome"), "{core}");
}
