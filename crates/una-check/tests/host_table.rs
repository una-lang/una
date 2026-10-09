//! The host table against the formal model (#46, M3): the response of every host operation is
//! labeled at least what the operation accepts. A response may depend on everything the request
//! sent, so a label below `accepts` would let data the sink accepted come back at a lower label.
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

/// The top label of a declared result type: its leading level, `public` when it has none.
fn top_rank(result: &str) -> usize {
    let first = result.split_whitespace().next().unwrap_or("");
    rank(first).unwrap_or(0)
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
                assert!(top_rank(&op.result) >= rank("public").unwrap());
                continue;
            };
            let accepts =
                rank(&accepts).unwrap_or_else(|| panic!("{qualified}: accepts {accepts}"));
            if op.result == "Unit" {
                continue;
            }
            assert!(
                top_rank(&op.result) >= accepts,
                "{qualified} returns {} below what it accepts",
                op.result
            );
            checked += 1;
        }
    }
    // Clock.now, Env.get, Files.read and the four operations of Http.
    assert_eq!(checked, 7);
}
