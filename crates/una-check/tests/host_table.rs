//! The host table against the formal model (#46, M3): the response of every host operation is
//! labeled at least what the operation accepts. A response may depend on everything the request
//! sent, so a label not above `accepts`, by level and by categories, would let data the sink accepted come back at a lower label.
//! T6 in `formal/UnaFormal/F1/T6.lean` assumes this property of the host (`hget`); this test keeps
//! the prelude's declarations and that assumption from drifting apart.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

const PRELUDE: &str = include_str!("../src/std/prelude.una");
const PRELUDE_0_2: &str = include_str!("../src/std/prelude_0_2.una");

/// The rank of a level in the order of `spec/07 §2`.
fn rank(level: &str) -> Option<usize> {
    ["public", "internal", "confidential", "secret", "restricted"]
        .iter()
        .position(|l| *l == level)
}

/// A label as the prelude writes it, `level` or `level[Cat, ...]`: its level's rank and its
/// categories; `None` when the text starts with no level.
fn label(text: &str) -> Option<(usize, Vec<String>)> {
    let word = text.split_whitespace().next().unwrap_or("");
    let (level, cats) = match word.split_once('[') {
        Some((level, rest)) => (level, rest.trim_end_matches(']')),
        None => (word, ""),
    };
    let cats = cats
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();
    rank(level).map(|r| (r, cats))
}

/// The top label of a declared result type: its leading label, `public` when it has none.
fn top_label(result: &str) -> (usize, Vec<String>) {
    label(result).unwrap_or((0, Vec::new()))
}

/// `a ⊑ b`: the level at most, and the categories a subset (`spec/07 §2`).
fn below(a: &(usize, Vec<String>), b: &(usize, Vec<String>)) -> bool {
    a.0 <= b.0 && a.1.iter().all(|c| b.1.contains(c))
}

/// One host operation: its capability, name, declared result and `accepts`, if declared.
struct Op {
    capability: String,
    name: String,
    result: String,
    accepts: Option<String>,
}

/// The operations of every capability in a prelude source, in order.
fn operations(source: &str) -> Vec<Op> {
    let mut ops: Vec<Op> = Vec::new();
    let mut capability = String::new();
    for line in source.lines() {
        if let Some(rest) = line.strip_prefix("pub capability ") {
            capability = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("  op ") {
            let name = rest.split('(').next().unwrap().trim().to_string();
            let result = rest.rsplit("->").next().unwrap().trim().to_string();
            ops.push(Op {
                capability: capability.clone(),
                name,
                result,
                accepts: None,
            });
        } else if let Some(rest) = line.trim_start().strip_prefix("accepts ") {
            if line.starts_with("    ") {
                ops.last_mut().unwrap().accepts = Some(rest.trim().to_string());
            }
        } else if !line.starts_with(' ') && !line.is_empty() {
            capability.clear();
        }
    }
    ops
}

#[test]
fn every_host_response_is_labeled_at_least_what_its_operation_accepts() {
    let mut checked = 0;
    for source in [PRELUDE, PRELUDE_0_2] {
        for op in operations(source) {
            let qualified = format!("{}.{}", op.capability, op.name);
            let Some(accepts) = op.accepts else {
                // `Keys.hmac` declares its labels in the checker (ADR 0039): `key_name` accepts
                // `public`, and its result carries the label of `data`, whatever it is.
                assert_eq!(qualified, "Keys.hmac", "{qualified} declares no accepts");
                assert!(below(&(0, Vec::new()), &top_label(&op.result)));
                continue;
            };
            let accepts =
                label(&accepts).unwrap_or_else(|| panic!("{qualified}: accepts {accepts}"));
            if op.result == "Unit" {
                continue;
            }
            assert!(
                below(&accepts, &top_label(&op.result)),
                "{qualified} returns {} below what it accepts",
                op.result
            );
            checked += 1;
        }
    }
    // Clock.now, Env.get, Files.read and the four operations of Http.
    assert_eq!(checked, 7);
}
