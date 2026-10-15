//! S-27: in edition 0.2 and later, interpolating a value writes its Una form without redaction
//! (`spec/10 §12` rule 14), so the text holds the value's labeled fields in clear. The Core that
//! `una core --json` exports binds that text to a `to_text` temporary, and its annotation must
//! carry the label the checker gives the interpolation: the join of every label of the value's
//! type, deep, kinds' fields included. It used to leave the fields' labels out, which exported a
//! labeled field in clear under `public` (#799).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

const LOCK: &str = "encoding_version = \"una-smt-1\"\nsolver           = \"z3\"\nsolver_version   = \"5.1.0\"\nlogic            = \"QF_UFLIA\"\nstep_budget      = 5000000\nrandom_seed      = 0\n";

/// A package of one module, `app.main`, in `edition`, written fresh under the test's directory.
fn package(name: &str, edition: &str, source: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("core_text_labels")
        .join(format!("{name}-{}", edition.replace('.', "_")));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/app")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n\n[labels]\ncategories = [\"PII\"]\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("verification.lock"), LOCK).unwrap();
    std::fs::write(dir.join("src/app/main.una"), source).unwrap();
    dir
}

/// The labels of every `to_text` temporary of the export, as `level[categories]`, in order.
fn to_text_labels(dir: &PathBuf) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["--json", "core", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["complete"], Value::Bool(true), "{document}");
    let mut out = Vec::new();
    walk(&document, &mut out);
    out
}

fn walk(node: &Value, out: &mut Vec<String>) {
    match node {
        Value::Object(fields) => {
            let is_to_text = fields.get("kind").and_then(Value::as_str) == Some("let")
                && fields["value"]["kind"] == "rhs"
                && fields["value"]["value"]["kind"] == "prim"
                && fields["value"]["value"]["prim"]["kind"] == "to_text";
            if is_to_text {
                let label = &fields["ty"]["label"];
                let categories: Vec<&str> = label["categories"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                out.push(format!(
                    "{}[{}]",
                    label["level"].as_str().unwrap(),
                    categories.join(",")
                ));
            }
            for value in fields.values() {
                walk(value, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
        _ => {}
    }
}

const KINDS: &str = "kind Card\n  holder: Text\n  number: secret[PII] Text\n\nkind Wallet\n  keeper: Text\n  card: Card\n\nkind Plain\n  name: Text\n  size: Int\n\nkind Code = Hidden(value: confidential Int) | Open\n";

fn main_with(lines: &str) -> String {
    format!(
        "module app.main\n\n{KINDS}\npub main() -> Unit\n  uses @console: Console[print]\n=\n{lines}\n"
    )
}

#[test]
fn an_interpolated_value_text_carries_its_fields_labels_in_the_core_export() {
    // Each case interpolates one shape into a text that reaches no sink, so the program checks,
    // and the export holds one `to_text` temporary per interpolated value.
    let cases = [
        (
            "record",
            "  let s: secret[PII] Text = \"0612\"\n  let c = Card(holder: \"Ana\", number: s)\n  let t = \"{c}\"\n  @console.print(\"done\")",
            vec!["secret[PII]"],
        ),
        (
            "nested",
            "  let s: secret[PII] Text = \"0612\"\n  let w = Wallet(keeper: \"Ana\", card: Card(holder: \"Ana\", number: s))\n  let t = \"{w}\"\n  @console.print(\"done\")",
            vec!["secret[PII]"],
        ),
        (
            "list",
            "  let s: secret[PII] Text = \"0612\"\n  let cs = [Card(holder: \"Ana\", number: s)]\n  let t = \"{cs}\"\n  @console.print(\"done\")",
            vec!["secret[PII]"],
        ),
        (
            "option",
            "  let s: secret[PII] Text = \"0612\"\n  let o = Some(Card(holder: \"Ana\", number: s))\n  let t = \"{o}\"\n  @console.print(\"done\")",
            vec!["secret[PII]"],
        ),
        (
            "variant",
            "  let k = Hidden(value: 7)\n  let t = \"{k}\"\n  @console.print(\"done\")",
            vec!["confidential[]"],
        ),
        (
            "plain",
            "  let p = Plain(name: \"Ana\", size: 3)\n  @console.print(\"{p}\")",
            vec!["public[]"],
        ),
        (
            "two",
            "  let s: secret[PII] Text = \"0612\"\n  let c = Card(holder: \"Ana\", number: s)\n  let p = Plain(name: \"Ana\", size: 3)\n  let t = \"{p} {c}\"\n  @console.print(\"done\")",
            vec!["public[]", "secret[PII]"],
        ),
    ];
    for edition in ["0.2", "0.3"] {
        for (name, lines, expected) in &cases {
            let dir = package(name, edition, &main_with(lines));
            let mut got = to_text_labels(&dir);
            got.sort();
            let mut want: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
            want.sort();
            assert_eq!(got, want, "{name}, edition {edition}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
