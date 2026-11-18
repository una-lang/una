//! The standard functions with a function parameter keep no copy of what that parameter is
//! given (#1023, ADR 0116 IF5 item 4, `spec/26 §2` rule 5).
//!
//! The timing label counts a lambda written at the call of such a function, as the argument of
//! a parameter whose declared type is a function type, by its captures and its body, not as `⊤`,
//! because the function applies it only while the call runs. This test keeps that premise as far
//! as signatures show it: a standard function with a function parameter that is not in the
//! reviewed list below, or whose declared result names a function type, fails it, and the change
//! then needs a review of the rule. A lambda given to a parameter of another type, such as
//! `fold`'s `initial: U`, may be returned, and the checker keeps `⊤` for it
//! (`Checker::known_function_arguments`); the list's review covers that each function returns no
//! value of a function-typed parameter.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::Path;

/// The standard functions with a function parameter, each reviewed for #1023: each applies the
/// argument of its function-typed parameters during the call and returns no copy of it.
const REVIEWED: &[&str] = &[
    "dense_array_fold",
    "dense_array_map",
    "dense_array_update",
    "dense_array_zip_map",
    "dense_matrix_fold",
    "dense_matrix_map",
    "dense_matrix_update",
    "dense_matrix_zip_map",
    "dense_tensor_fold",
    "dense_tensor_map",
    "dense_tensor_update",
    "dense_tensor_zip_map",
    "filter",
    "first_where",
    "fold",
    "map",
    "update",
];

/// The parameter list of a one-line signature after `pub `, and what follows it.
fn split_signature(rest: &str) -> Option<(&str, &str, &str)> {
    let open = rest.find('(')?;
    let name = rest[..open].split('[').next().unwrap_or("").trim();
    let mut depth = 0usize;
    for (at, c) in rest[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let close = open + at;
                    return Some((name, &rest[open + 1..close], &rest[close + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn standard_functions_with_a_function_parameter_keep_no_copy_of_it() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/std");
    let mut scanned = 0;
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut problems = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("una") {
            continue;
        }
        scanned += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let code_of = |line: &str| line.split("--").next().unwrap_or("").to_owned();
        for (number, line) in lines.iter().enumerate() {
            let code = code_of(line);
            let Some(first_line) = code.strip_prefix("pub ") else {
                continue;
            };
            // A signature may span lines until its parameter list closes.
            let mut rest = first_line.to_owned();
            let mut next = number + 1;
            while rest.matches('(').count() > rest.matches(')').count() && next < lines.len() {
                rest.push(' ');
                rest.push_str(&code_of(lines[next]));
                next += 1;
            }
            let rest = rest.as_str();
            let first = rest.split_whitespace().next().unwrap_or("");
            if matches!(
                first,
                "kind" | "capability" | "handler" | "predicate" | "policy"
            ) {
                continue;
            }
            if !rest.contains('(') {
                continue;
            }
            let where_ = format!("{}:{}", path.display(), number + 1);
            let Some((name, params, after)) = split_signature(rest) else {
                problems.push(format!("{where_}: a signature that does not close: {line}"));
                continue;
            };
            if !params.contains("fn(") {
                continue;
            }
            found.insert(name.to_owned());
            if !REVIEWED.contains(&name) {
                problems.push(format!(
                    "{where_}: `{name}` takes a function, not reviewed for #1023"
                ));
            }
            let result = after.split('=').next().unwrap_or("");
            if result.contains("fn(") {
                problems.push(format!("{where_}: `{name}` returns a function type"));
            }
        }
    }
    assert!(
        scanned >= 5,
        "only {scanned} standard modules found in {}",
        dir.display()
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let reviewed: BTreeSet<String> = REVIEWED.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(
        found, reviewed,
        "the reviewed list names a function the modules do not hold"
    );
}
