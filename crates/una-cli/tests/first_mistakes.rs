//! Repairs for common first mistakes (#743), found by writing the programs of Una by Example
//! (#733, #576): a reserved keyword where a module path segment or an operation is named, a
//! requirement identifier off the convention, and a method in a contract. Each diagnostic says
//! what the rule is and what to write instead; an edit is offered only where it stays inside
//! the declaration (ADR 0096 rule 2(a)), and the edited program checks. A declaration keyword
//! bound as a local name is renamed through its scope (ADR 0096, amendment of 2026-10-15).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/binding_words.rs"]
mod binding_words;

use pretty_assertions::assert_eq;
use serde_json::Value;

/// A package of edition 0.2 named `name`, whose only module is `src/<file>.una`.
fn package(name: &str, file: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("first-mistakes")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n",
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src").join(format!("{file}.una")), source).unwrap();
    root
}

fn diagnostics(root: &Path) -> Vec<Value> {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "check", "--tier", "0", "."])
        .current_dir(root)
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The only diagnostic of `code` of the package.
fn only(root: &Path, code: &str) -> Value {
    let found = diagnostics(root);
    let of: Vec<&Value> = found.iter().filter(|d| d["code"] == code).collect();
    assert_eq!(of.len(), 1, "{found:#?}");
    of[0].clone()
}

#[test]
fn a_reserved_keyword_as_a_module_path_segment_names_the_rule_and_a_name() {
    let root = package("module-when", "when", "module when\n\nf() -> Int = 1\n");
    let found = only(&root, "U0101");
    assert_eq!(
        found["message"],
        "`when` is a reserved keyword, so it cannot be a module path segment"
    );
    let repair = &found["repairs"][0];
    assert_eq!(
        repair["description"],
        "choose another name, such as `when_module`"
    );
    // Renaming a module moves its file and changes its imports: advice, not an edit.
    assert!(repair["edit"].is_null(), "{repair:#?}");
}

#[test]
fn an_entry_module_whose_path_is_refused_is_reported_once() {
    let root = package(
        "entry-when",
        "when",
        "module when\n\npub main() -> Unit = unit\n",
    );
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"when\"\n",
    )
    .unwrap();
    let codes: Vec<String> = diagnostics(&root)
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, vec!["U0101"]);
}

#[test]
fn a_reserved_keyword_as_an_operation_name_names_the_rule_and_a_name() {
    let root = package(
        "op-take",
        "m",
        "module m\n\ncapability Stock\n  op take(item: Text) -> Bool\n",
    );
    let found = only(&root, "U0101");
    assert_eq!(
        found["message"],
        "`take` is a reserved keyword, so it cannot be an operation name"
    );
    let repair = &found["repairs"][0];
    assert_eq!(
        repair["description"],
        "choose another name, such as `take_op`"
    );
    assert!(repair["edit"].is_null(), "{repair:#?}");
}

const REQUIREMENT: &str = "module m\n\nrequirement SOLD_OUT\n  text \"x\"\n  source human\n  owner team\n  status active\n";

#[test]
fn an_unreferenced_requirement_identifier_is_renamed_from_its_own_name() {
    let root = package("requirement-free", "m", REQUIREMENT);
    let found = only(&root, "U1005");
    assert_eq!(
        found["message"],
        "requirement identifier `SOLD_OUT` must be upper snake case ending with `_` and a number, as in `SOLD_OUT_1`"
    );
    let repair = &found["repairs"][0];
    assert_eq!(repair["edit"]["replacement"], "SOLD_OUT_1", "{repair:#?}");
    // The edited program has no U1005.
    let fixed = REQUIREMENT.replace("requirement SOLD_OUT\n", "requirement SOLD_OUT_1\n");
    let after = package("requirement-free-fixed", "m", &fixed);
    assert_eq!(diagnostics(&after), Vec::<Value>::new());
}

#[test]
fn a_cited_requirement_identifier_gets_advice_that_names_where_it_is_cited() {
    let source = format!(
        "{REQUIREMENT}\nkind Availability = SoldOut | Open\n\navailability(remaining: Int) -> Availability\n  derived from SOLD_OUT\n=\n  if remaining < 1 then SoldOut else Open\n"
    );
    let root = package("requirement-cited", "m", &source);
    let found = only(&root, "U1005");
    let repair = &found["repairs"][0];
    assert!(repair["edit"].is_null(), "{repair:#?}");
    assert_eq!(
        repair["description"],
        "rename `SOLD_OUT` to `SOLD_OUT_1` here and in the `derived from` clauses that name it, at src/m.una:12"
    );
}

#[test]
fn the_suggested_requirement_identifier_is_one_the_package_does_not_hold() {
    let source = format!(
        "{REQUIREMENT}\nrequirement SOLD_OUT_1\n  text \"y\"\n  source human\n  owner team\n  status active\n"
    );
    let root = package("requirement-taken", "m", &source);
    let found = only(&root, "U1005");
    assert_eq!(found["repairs"][0]["edit"]["replacement"], "SOLD_OUT_2");
}

#[test]
fn to_int_in_a_contract_says_what_the_refinement_language_offers() {
    let root = package(
        "to-int-requires",
        "m",
        "module m\n\nadd(a: U8, b: U8) -> U8\n  requires a.to_int() + b.to_int() <= 255\n=\n  a + b\n",
    );
    let found = diagnostics(&root);
    let first = found.iter().find(|d| d["code"] == "U2014").unwrap();
    let description = first["repairs"][0]["description"].as_str().unwrap();
    assert!(
        description.starts_with("state the bound on the value in its own type, such as `a <= 100`"),
        "{first:#?}"
    );
    assert!(first["repairs"][0]["edit"].is_null());
}

/// Applies the edit of every diagnostic's first repair to `src/<file>.una` and returns the
/// diagnostics of the check that follows.
fn apply_edits(root: &Path, file: &str) -> Vec<Value> {
    let path = root.join("src").join(format!("{file}.una"));
    let text = std::fs::read_to_string(&path).unwrap();
    let offset = |position: &Value| {
        let line = usize::try_from(position["line"].as_u64().unwrap()).unwrap();
        let col = usize::try_from(position["col"].as_u64().unwrap()).unwrap();
        text.split('\n')
            .take(line - 1)
            .map(|l| l.chars().count() + 1)
            .sum::<usize>()
            + col
            - 1
    };
    let mut edits: Vec<(usize, usize, String)> = diagnostics(root)
        .iter()
        .filter_map(|d| d["repairs"].as_array()?.first()?.get("edit").cloned())
        .filter(|edit| !edit.is_null())
        .map(|edit| {
            (
                offset(&edit["start"]),
                offset(&edit["end"]),
                edit["replacement"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(!edits.is_empty(), "no edit to apply");
    edits.sort();
    let mut chars: Vec<char> = text.chars().collect();
    for (start, end, replacement) in edits.iter().rev() {
        chars.splice(start..end, replacement.chars());
    }
    std::fs::write(&path, chars.into_iter().collect::<String>()).unwrap();
    diagnostics(root)
}

#[test]
fn a_declaration_keyword_as_a_local_name_is_renamed_and_the_program_checks() {
    // ADR 0096, amendment of 2026-10-15: a `let`, a lambda's parameter and a handler
    // operation's parameter named `op` are renamed through their scopes, including an
    // interpolation hole, and the edited program checks.
    let root = package(
        "keyword-locals",
        "m",
        "module m\n\ncapability Stock\n  op hold(item: Text) -> Bool\n\npub handler fake() for Stock\n  op hold(op) = op == \"x\"\n\npub next(xs: List[Int]) -> List[Int] = xs.map(each: fn(op) -> op + 1)\n\npub show(n: Int) -> Text\n=\n  let op = n + 1\n  \"{op}!\"\n",
    );
    let found = diagnostics(&root);
    assert_eq!(found.len(), 3, "{found:#?}");
    for diagnostic in &found {
        assert_eq!(diagnostic["code"], "U0101");
        assert_eq!(
            diagnostic["repairs"][0]["description"],
            "rename the binding and its uses to `op_value`"
        );
    }
    assert_eq!(apply_edits(&root, "m"), Vec::<Value>::new());
    let fixed = std::fs::read_to_string(root.join("src/m.una")).unwrap();
    assert!(
        fixed.contains("op hold(op_value) = op_value == \"x\""),
        "{fixed}"
    );
    assert!(fixed.contains("fn(op_value) -> op_value + 1"), "{fixed}");
    assert!(
        fixed.contains("let op_value = n + 1\n  \"{op_value}!\""),
        "{fixed}"
    );
}

#[test]
fn the_rename_leaves_a_field_name_and_an_argument_label_alone() {
    // A name after `.` and an argument label are not uses of the binding. `op` is a keyword,
    // so neither is legal: the edited program still reports each, with advice and no edit.
    for (name, use_site) in [("keyword-dot", "n + op.op"), ("keyword-label", "g(op: op)")] {
        let root = package(
            name,
            "m",
            &format!(
                "module m\n\ng(size: Int) -> Int = size\n\npub f(n: Int) -> Int\n=\n  let op = n\n  {use_site}\n"
            ),
        );
        let after = apply_edits(&root, "m");
        assert_eq!(after.len(), 1, "{after:#?}");
        assert_eq!(after[0]["code"], "U0101");
        assert!(after[0]["repairs"][0]["edit"].is_null(), "{after:#?}");
        let fixed = std::fs::read_to_string(root.join("src/m.una")).unwrap();
        let expected = use_site
            .replace("op.op", "op_value.op")
            .replace(": op)", ": op_value)");
        assert!(fixed.contains(&expected), "{fixed}");
    }
}

#[test]
fn a_function_parameter_named_with_a_declaration_keyword_gets_advice() {
    // A call names the parameter (`f(op: 1)`), so renaming it is outside the declaration.
    let root = package(
        "keyword-param",
        "m",
        "module m\n\npub f(op: Int) -> Int = op\n",
    );
    let found = only(&root, "U0101");
    assert_eq!(
        found["repairs"][0]["description"],
        "choose another name, such as `op_value`"
    );
    assert!(found["repairs"][0]["edit"].is_null(), "{found:#?}");
}

/// A loop variable has no `edit` form, so U2019 on it does not suggest declaring it `let edit`
/// but a separate `let edit` binding in the body, and that binding checks (#931).
#[test]
fn assigning_a_loop_variable_suggests_a_separate_let_edit_binding() {
    let assigned = "module m\n\npub total(xs: List[Int]) -> Int\n=\n  let edit sum = 0\n  for x in xs\n    x = x + 1\n    sum = sum + x\n  sum\n";
    let root = package("loop-variable", "m", assigned);
    let found = only(&root, "U2019");
    assert_eq!(
        found["message"],
        "`x` is the variable of a `for` loop, which is read-only; bind a copy in the body with `let edit`, as `let edit next_x = x`"
    );
    assert_eq!(found["repairs"], Value::Array(Vec::new()), "{found:#?}");
    let fixed = assigned.replace(
        "    x = x + 1\n    sum = sum + x\n",
        "    let edit next_x = x\n    next_x = next_x + 1\n    sum = sum + next_x\n",
    );
    let after = package("loop-variable-fixed", "m", &fixed);
    assert_eq!(diagnostics(&after), Vec::<Value>::new());
    // A plain `let` keeps the advice to declare it `let edit`.
    let plain = package(
        "plain-let",
        "m",
        "module m\n\npub f(n: Int) -> Int\n=\n  let y = n\n  y = 2\n  y\n",
    );
    assert_eq!(
        only(&plain, "U2019")["message"],
        "`y` is not editable; declare it `let edit y`, or the parameter `edit y`"
    );
}

/// The advice of each U3013 of the package at `root`, after "statement or `let`; ", with its
/// line and column.
fn u3013_advice(root: &Path) -> Vec<(u64, u64, String)> {
    diagnostics(root)
        .iter()
        .filter(|d| d["code"] == "U3013")
        .map(|d| {
            let message = d["message"].as_str().unwrap();
            // Edition 0.5 names its binding words (ADR 0137 rule 10).
            let (_, advice) = message
                .split_once("statement or `let`; ")
                .or_else(|| message.split_once("statement, or of a `const` or `var`; "))
                .unwrap();
            (
                d["primary"]["start"]["line"].as_u64().unwrap(),
                d["primary"]["start"]["col"].as_u64().unwrap(),
                advice.to_owned(),
            )
        })
        .collect()
}

#[test]
fn u3013_advises_the_form_that_resolves_it_where_the_call_stands() {
    // #837: "bind its result with `let` first" resolves U3013 only where a `let` before the
    // statement runs the call as the statement would. Under a nested `attempt` the whole
    // `attempt` is what is bound, in every place; a lambda body holds no `let`; code that runs
    // only when something else decides would run the call every time; a `together` branch is
    // one `let`; a call with `edit` cannot be wrapped in `attempt`; a `do` line binds nothing.
    // `authority/u3013_advice_followed` follows each advice and checks.
    const BIND: &str = "bind its result with `let` first";
    const ATTEMPT: &str = "bind the whole `attempt` with `let` first, as `let outcome = attempt ...`, and use `outcome` in its place; binding the call alone would leave its failure unhandled";
    const MARKED: &str = "`attempt` cannot wrap it, so make the call itself the whole expression of a statement or `let`, and handle a failure with `if error`";
    const DROP: &str = "`attempt` cannot wrap a call with `edit` or `take`, so drop it and handle a failure with `if error`; ";
    const DO: &str = "a `do` line binds nothing, so call a function that takes a plain argument and edits a local copy of it, as `let edit copy = n` in its body";
    // What the advice holds in the place it names: the call, or the `attempt` bound.
    let held = |attempt: bool| {
        if attempt {
            "`let outcome = attempt ...` as a `let`"
        } else {
            "the call as a statement or `let`"
        }
    };
    let lambda = |a: bool| {
        format!(
            "the body of a lambda is one line and cannot hold a `let`, so move the work into a named function, with {} of its body, and pass that function",
            held(a)
        )
    };
    let error_clause = |a: bool| {
        format!(
            "the value of `if error use` runs only when the call before it fails, so a `let` before the statement would run this call every time: give the clause a value without effects that cannot fail, or bind `attempt` of the first call with `let` and branch on it with `when`, with {} of an arm `is Err(e)` written as a block",
            held(a)
        )
    };
    let none_clause = |a: bool| {
        format!(
            "the value of `if none use` runs only when the value before it is `None`, so a `let` before the statement would run this call every time: give the clause a value without effects that cannot fail, or bind the value without this clause and branch on it with `when`, with {} of an arm `is None` written as a block",
            held(a)
        )
    };
    let branch = |a: bool| {
        format!(
            "a branch of `if ... then ... else` runs only when the condition selects it, so a `let` before the `if` would run the call every time: write the `if` on several lines, with {} of its branch",
            held(a)
        )
    };
    let short = |a: bool| {
        format!(
            "the right operand of `and`, `or` and `implies` runs only when the left one does not decide, so a `let` before it would run the call every time: test the left operand with an `if` on several lines, with {} of the branch that needs it",
            held(a)
        )
    };
    let arm = |a: bool| {
        format!(
            "an arm of `when` runs only when its pattern matches, so a `let` before the `when` would run the call every time: write the arm as a block, with {} of the arm",
            held(a)
        )
    };
    let together = format!(
        "a branch of `together` is one `let`, and a `let` before `together` would take the call out of it: call a named function from the branch's `let`, with {} of its body",
        held(false)
    );
    let expected: Vec<(u64, u64, String)> = vec![
        (81, 16, ATTEMPT.to_owned()),
        (86, 7, BIND.to_owned()),
        (91, 24, lambda(false)),
        // A one-line `if` in a lambda: still the named function only.
        (96, 34, lambda(false)),
        (101, 39, lambda(true)),
        (108, 18, error_clause(false)),
        (115, 30, error_clause(true)),
        (124, 17, none_clause(false)),
        (132, 30, none_clause(true)),
        (138, 17, branch(false)),
        (143, 30, branch(true)),
        (148, 6, BIND.to_owned()),
        (153, 13, short(false)),
        (158, 25, short(true)),
        (163, 3, BIND.to_owned()),
        (169, 17, arm(false)),
        (176, 26, arm(true)),
        (183, 17, together.clone()),
        (191, 16, ATTEMPT.to_owned()),
        (191, 28, BIND.to_owned()),
        (196, 15, BIND.to_owned()),
        (202, 25, MARKED.to_owned()),
        (208, 39, format!("{DROP}{}", branch(false))),
        (211, 50, ATTEMPT.to_owned()),
        (211, 72, ATTEMPT.to_owned()),
        // #2213: inside a `together` branch, which is one `let`, a one-line `if` or a handling
        // clause keeps the branch's advice, since the inner one cannot be written there.
        (217, 27, together.clone()),
        (228, 19, together),
    ];
    let conformance =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/authority");
    assert_eq!(u3013_advice(&conformance.join("u3013_advice")), expected);
    assert_eq!(
        diagnostics(&conformance.join("u3013_advice_followed")),
        Vec::<Value>::new()
    );
    // #1628, ADR 0137 rules 10 and 11: in edition 0.5 the case reads `const` and `var`, as
    // `una fix` rewrites it, and every text above names the edition's binding words, at the
    // positions of the rewritten case.
    assert_eq!(
        una_diag::binding_words::words_of_0_5(BIND),
        "bind its result with `const` first"
    );
    let written = std::fs::read_to_string(conformance.join("u3013_advice/src/advice.una")).unwrap();
    let expected: Vec<(u64, u64, String)> = expected
        .into_iter()
        .map(|(line, col, advice)| {
            (
                line,
                binding_words::column_in_0_5(&written, line, col),
                una_diag::binding_words::words_of_0_5(&advice),
            )
        })
        .collect();
    for case in ["u3013_advice", "u3013_advice_followed"] {
        let copy = package(&format!("u3013-0.5-{case}"), "advice", "");
        std::fs::remove_file(copy.join("una.toml")).unwrap();
        std::fs::remove_file(copy.join("verification.lock")).unwrap();
        std::fs::write(
            copy.join("ita.una"),
            "ita case\n  version 0.1.0\n  edition 0.5\n",
        )
        .unwrap();
        std::fs::copy(
            conformance.join("../types/priority_queue_methods/ita.lock"),
            copy.join("ita.lock"),
        )
        .unwrap();
        let source =
            std::fs::read_to_string(conformance.join(case).join("src/advice.una")).unwrap();
        std::fs::write(copy.join("src/advice.una"), binding_words::of_0_5(&source)).unwrap();
        let found = u3013_advice(&copy);
        if case == "u3013_advice" {
            assert_eq!(found, expected, "edition 0.5");
        } else {
            assert_eq!(diagnostics(&copy), Vec::<Value>::new(), "edition 0.5");
        }
    }
    // A `do` line binds nothing (`spec/17 §3` rule 5), in the case that covers it.
    let memdo = diagnostics(&conformance.join("../types/memory_example_do"));
    let on_do: Vec<&str> = memdo
        .iter()
        .filter(|d| d["code"] == "U3013")
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert_eq!(
        on_do,
        [format!(
            "`bump` is called with `edit` or `take`, so the call must be the whole expression of a statement or `let`; {DO}"
        )]
    );
}
