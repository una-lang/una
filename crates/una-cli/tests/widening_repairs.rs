//! Widening repairs rank after the ones that keep authority and say what they widen (ADR 0097,
//! #579): the repairs of `una --json check` come in the order of the decision's table, and each
//! widening carries its kind and cost. The invariant over every case is in the conformance
//! runner.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::Value;

/// The conformance case at `case`, relative to `tests/conformance`.
fn case(case: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance")
        .join(case)
}

/// A package of one module `m` with `source`, written for this test under `name`.
fn package(name: &str, source: &str) -> PathBuf {
    package_with(name, "", source)
}

/// [`package`] with `extra` appended to its `una.toml`.
fn package_with(name: &str, extra: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("widening")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!("[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n{extra}"),
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/m.una"), source).unwrap();
    root
}

/// The diagnostics of `una --json check` on the package directory or single file `path`.
fn check_json(path: &Path) -> Vec<Value> {
    let (dir, target) = if path.is_dir() {
        (path.to_path_buf(), ".".to_owned())
    } else {
        (
            path.parent().unwrap().to_path_buf(),
            path.file_name().unwrap().to_string_lossy().into_owned(),
        )
    };
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["--json", "check", "--tier", "0", &target])
        .current_dir(dir)
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The only diagnostic with `code`, or the one at `line`:`col` when given.
fn with_code(diagnostics: &[Value], code: &str, at: Option<(u64, u64)>) -> Value {
    let found: Vec<&Value> = diagnostics
        .iter()
        .filter(|d| {
            d["code"] == code
                && at.is_none_or(|(line, col)| {
                    d["primary"]["start"]["line"] == line && d["primary"]["start"]["col"] == col
                })
        })
        .collect();
    match found.as_slice() {
        [only] => (*only).clone(),
        _ => panic!("not exactly one {code} at {at:?} in {diagnostics:#?}"),
    }
}

/// Each repair as (rank, widening kind or "", description), in output order.
fn repairs(diagnostic: &Value) -> Vec<(u64, String, String)> {
    diagnostic["repairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repair| {
            assert!(
                repair.get("escape").is_none(),
                "a widening is not an escape"
            );
            assert_eq!(repair["escape_cost"], 0);
            (
                repair["rank"].as_u64().unwrap(),
                repair["widens"]["kind"].as_str().unwrap_or("").to_owned(),
                repair["description"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn row(rank: u64, widens: &str, description: &str) -> (u64, String, String) {
    (rank, widens.to_owned(), description.to_owned())
}

#[test]
fn u3002_keeps_authority_first_at_a_call_and_at_an_operation() {
    let diagnostics = check_json(&case("authority/attenuated_binding_repair_call.una"));
    // At a call: rebinding the callee's reference, then widening the attenuated binding.
    assert_eq!(
        repairs(&with_code(&diagnostics, "U3002", Some((21, 3)))),
        [
            row(
                1,
                "",
                "add `with @front = @back` to this statement, since `@back` permits `unstock`"
            ),
            row(
                2,
                "authority",
                "add `unstock` to the `uses` clause of `@front`"
            ),
        ]
    );
    // At an operation: another binding that permits it, then the widening; the message states
    // only the problem.
    let operation = with_code(&diagnostics, "U3002", Some((28, 10)));
    assert_eq!(
        operation["message"],
        "`@front` permits `stock`, not `unstock`"
    );
    let rows = repairs(&operation);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].1, "");
    assert!(rows[0].2.starts_with("use `@"), "{rows:?}");
    assert_eq!(
        rows[1],
        row(
            2,
            "authority",
            "add `unstock` to the `uses` clause of `@front`"
        )
    );
    let cost = operation["repairs"][1]["widens"]["cost"].as_str().unwrap();
    assert!(cost.contains("every caller must hold it too"), "{cost}");
}

#[test]
fn u3001_and_u3003_declare_uses_as_a_marked_widening() {
    const LOG: &str = "module m\n\ncapability Log\n  op write(line: Text) -> Unit\n\nwrite_it() -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"x\")\n\n";
    let reference = check_json(&package(
        "u3001-reference",
        &format!("{LOG}pub a() -> Unit\n  uses @log: Log[write]\n=\n  @other.write(\"x\")\n"),
    ));
    let diagnostic = with_code(&reference, "U3001", None);
    assert_eq!(diagnostic["message"], "`@other` is not in scope");
    assert_eq!(
        repairs(&diagnostic),
        [row(
            1,
            "authority",
            "declare `uses @other: ...` in the header, with the capability and the operations used"
        )]
    );
    let call = check_json(&package(
        "u3001-call",
        &format!("{LOG}pub b() -> Unit\n  uses @other: Log[write]\n=\n  write_it()\n"),
    ));
    let diagnostic = with_code(&call, "U3001", None);
    assert_eq!(
        diagnostic["message"],
        "`write_it` needs `@log: Log[write]`, which is not in scope here"
    );
    assert_eq!(
        repairs(&diagnostic)
            .into_iter()
            .map(|(rank, widens, _)| (rank, widens))
            .collect::<Vec<_>>(),
        [(1, "authority".to_owned())]
    );
    let pure = check_json(&package(
        "u3003",
        &format!("{LOG}pub c() -> Unit = write_it()\n"),
    ));
    let rows = repairs(&with_code(&pure, "U3003", None));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].1, "authority");
}

#[test]
fn u3004_and_u1004_mark_their_only_route() {
    let pure = check_json(&case("authority/cap_in_pure_function.una"));
    let diagnostic = with_code(&pure, "U3004", Some((9, 24)));
    assert_eq!(
        diagnostic["message"],
        "`@log` is used here, but this code declares no `uses`, so it is pure"
    );
    assert_eq!(
        repairs(&diagnostic),
        [row(
            1,
            "authority",
            "declare `uses @log: ...` in the header, with the capability and the operations used"
        )]
    );
    let private = check_json(&case("names/private_import"));
    let diagnostic = with_code(&private, "U1004", Some((6, 11)));
    assert_eq!(
        repairs(&diagnostic),
        [row(1, "visibility", "declare `helper` with `pub`")]
    );
    assert!(
        diagnostic["repairs"][0]["widens"]["cost"]
            .as_str()
            .unwrap()
            .contains("every module may then call it")
    );
}

#[test]
fn u3012_grants_as_a_marked_widening_and_installs_a_handler_otherwise() {
    let host = check_json(&case("authority/ungranted_host_capability"));
    let rows = repairs(&with_code(&host, "U3012", Some((8, 3))));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].1, "grant");
    assert!(
        rows[0]
            .2
            .contains("in `[run] grants` of `una.toml`, or with `--grant`"),
        "{rows:?}"
    );
    // A capability that is not a host capability: no grant provides it, and the handler is
    // the route, which keeps authority.
    let module = check_json(&package_with(
        "u3012-module",
        "\n[run]\nentry = \"m\"\n",
        "module m\n\ncapability Store\n  op get() -> Int\n\npub main() -> Unit\n  uses @store: Store\n=\n  let n = @store.get()\n  unit\n",
    ));
    let diagnostic = with_code(&module, "U3012", Some((7, 3)));
    assert_eq!(
        diagnostic["message"],
        "`main` uses `@store: Store`, which is not a host capability, so no grant can provide it"
    );
    assert_eq!(
        repairs(&diagnostic),
        [row(
            1,
            "",
            "install a handler of `Store` for `@store` with `with` inside `main`"
        )]
    );
}

#[test]
fn an_inferred_uses_is_a_route_that_does_not_widen() {
    // The ruling on #579: declaring the `uses` that inference cannot settle writes down
    // authority the callers are already charged for, so its repair carries no `widens`, and
    // the message states only the problem.
    let diagnostics = check_json(&package(
        "u3001-inferred",
        "module m\n\ncapability Log\n  op write(line: Text) -> Unit\n\nlonely() -> Unit = @log.write(\"x\")\n",
    ));
    let diagnostic = with_code(&diagnostics, "U3001", None);
    assert_eq!(
        diagnostic["message"],
        "no caller of `lonely` binds `@log`, so its capability cannot be inferred"
    );
    assert_eq!(
        repairs(&diagnostic),
        [row(
            1,
            "",
            "declare `uses @log: <capability>[write]` in the header; this states authority its callers are already charged for"
        )]
    );
}
