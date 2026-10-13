//! The JSON shape of diagnostics is exactly that of `spec/13 §1` and ADR 0012.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use una_diag::{
    Binding, Code, Counterexample, Diagnostic, DiagnosticList, Edit, Escape, Location,
    ObligationKind, ObligationRef, ObligationStatus, Position, Related, Repair, Widening,
    render_json,
};

/// The JSON code blocks of `spec/13 §1`, in document order.
fn spec_section_1_json_blocks() -> Vec<String> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/13-diagnostics-and-abi.md");
    let spec = std::fs::read_to_string(path).unwrap();
    let section = spec
        .split("\n## 1. Diagnostic model\n")
        .nth(1)
        .expect("spec/13 has a section 1, Diagnostic model")
        .split("\n## ")
        .next()
        .unwrap();
    section
        .split("```json\n")
        .skip(1)
        .map(|block| block.split("\n```").next().unwrap().to_owned())
        .collect()
}

fn at(line: u32, col: u32) -> Position {
    Position { line, col }
}

fn location(file: &str, start: Position, end: Position) -> Location {
    Location {
        file: file.to_owned(),
        start,
        end,
    }
}

/// The example diagnostic of `spec/13 §1`, built through the public API.
fn spec_example() -> Diagnostic {
    let file = "src/events/seating.una";
    let mut diagnostic = Diagnostic::error(
        Code::U5001,
        "postcondition of availability is refuted",
        location(file, at(27, 18), at(27, 21)),
    );
    diagnostic.related.push(Related {
        location: location(file, at(23, 11), at(23, 52)),
        message: "ensures clause".to_owned(),
    });
    diagnostic.obligation = Some(ObligationRef {
        id: "ob-3f09a1c2e7b4".to_owned(),
        kind: ObligationKind::Postcondition,
        goal: "e.remaining < 1 implies result is SoldOut".to_owned(),
        status: ObligationStatus::Refuted,
    });
    diagnostic.counterexample = Some(Counterexample {
        bindings: vec![
            Binding {
                name: "e.remaining".to_owned(),
                value: json!(0),
                label: None,
            },
            Binding {
                name: "result".to_owned(),
                value: json!("Few"),
                label: None,
            },
        ],
    });
    diagnostic.repairs.push(Repair {
        widens: None,
        rank: 1,
        escape: None,
        description:
            "add the arm 'in 0..0 -> SoldOut' before 'in 0..<20' or adjust the ensures clause"
                .to_owned(),
        edit: None,
    });
    diagnostic
}

fn minimal(code: Code, file: &str, line: u32) -> Diagnostic {
    Diagnostic::error(code, "message", location(file, at(line, 1), at(line, 2)))
}

#[test]
fn spec_example_serializes_exactly_as_in_spec_13_section_1() {
    let expected: Value = serde_json::from_str(&spec_section_1_json_blocks()[0]).unwrap();
    assert_eq!(serde_json::to_value(spec_example()).unwrap(), expected);
}

#[test]
fn empty_list_matches_spec_13_section_1() {
    let expected: Value = serde_json::from_str(&spec_section_1_json_blocks()[1]).unwrap();
    let rendered: Value = serde_json::from_str(&render_json(&[]).unwrap()).unwrap();
    assert_eq!(rendered, expected);
}

#[test]
fn empty_list_is_compact() {
    assert_eq!(
        render_json(&[]).unwrap(),
        r#"{"schema":"una.diagnostics/1","diagnostics":[],"fixable":0}"#
    );
}

#[test]
fn listed_diagnostic_shape_is_pinned() {
    let list = [spec_example()];
    let pretty = serde_json::to_string_pretty(&DiagnosticList::new(&list)).unwrap();
    insta::assert_snapshot!("list_with_spec_example", pretty);
}

#[test]
fn minimal_diagnostic_is_exact_and_omits_absent_fields() {
    let diagnostic = Diagnostic::error(
        Code::U3005,
        "capability @log is used as a value",
        location("cap_as_value.una", at(12, 11), at(12, 15)),
    );
    assert_eq!(
        serde_json::to_string(&diagnostic).unwrap(),
        concat!(
            r#"{"schema":"una.diagnostic/1","code":"U3005","severity":"error","domain":"authority","#,
            r#""message":"capability @log is used as a value","#,
            r#""primary":{"file":"cap_as_value.una","start":{"line":12,"col":11},"end":{"line":12,"col":15}},"#,
            r#""related":[],"repairs":[],"explain":"una explain U3005"}"#,
        )
    );
}

#[test]
fn edit_is_null_or_a_located_replacement() {
    let mut diagnostic = minimal(Code::U7001, "src/a.una", 3);
    diagnostic.repairs.push(Repair {
        widens: None,
        rank: 2,
        escape: None,
        description: "reindent the line".to_owned(),
        edit: Some(Edit {
            location: location("src/a.una", at(3, 1), at(3, 4)),
            replacement: "  ".to_owned(),
        }),
    });
    diagnostic.repairs.push(Repair {
        widens: None,
        rank: 1,
        escape: None,
        description: "run una fmt".to_owned(),
        edit: None,
    });
    let value = serde_json::to_value(&diagnostic).unwrap();
    assert_eq!(value["repairs"][0]["edit"], Value::Null);
    assert_eq!(
        value["repairs"][1]["edit"],
        json!({
            "file": "src/a.una",
            "start": { "line": 3, "col": 1 },
            "end": { "line": 3, "col": 4 },
            "replacement": "  "
        })
    );
}

#[test]
fn repairs_are_ordered_by_escape_cost_then_rank_and_renumbered() {
    // ADR 0095 rule 3: every escape after every repair that keeps the guarantee, ranks 1 to n
    // in output order.
    let mut diagnostic = minimal(Code::U5002, "src/a.una", 1);
    for (rank, escape) in [
        (2, Some(Escape::Checked)),
        (1, Some(Escape::Checked)),
        (2, None),
        (1, None),
    ] {
        diagnostic.repairs.push(Repair {
            widens: None,
            rank,
            escape,
            description: format!("rank {rank}, escape {escape:?}"),
            edit: None,
        });
    }
    let value = serde_json::to_value(&diagnostic).unwrap();
    let order: Vec<(u64, u64, &str)> = value["repairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repair| {
            (
                repair["escape_cost"].as_u64().unwrap(),
                repair["rank"].as_u64().unwrap(),
                repair["description"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        order,
        [
            (0, 1, "rank 1, escape None"),
            (0, 2, "rank 2, escape None"),
            (1, 3, "rank 1, escape Some(Checked)"),
            (1, 4, "rank 2, escape Some(Checked)"),
        ]
    );
}

#[test]
fn a_widening_ranks_after_the_repairs_that_keep_authority_and_before_an_escape() {
    // ADR 0097 rules 3 and 4: keep < widen < escape, renumbered, and `widens` carries its kind
    // and cost while `escape_cost` stays 0.
    let mut diagnostic = minimal(Code::U3002, "src/a.una", 1);
    for (escape, widens, description) in [
        (Some(Escape::Checked), None, "escape"),
        (None, Some(Widening::Authority), "widen"),
        (None, None, "keep"),
    ] {
        diagnostic.repairs.push(Repair {
            widens,
            rank: 1,
            escape,
            description: description.to_owned(),
            edit: None,
        });
    }
    let value = serde_json::to_value(&diagnostic).unwrap();
    let repairs = value["repairs"].as_array().unwrap();
    let order: Vec<(u64, &str)> = repairs
        .iter()
        .map(|r| {
            (
                r["rank"].as_u64().unwrap(),
                r["description"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(order, [(1, "keep"), (2, "widen"), (3, "escape")]);
    assert!(repairs[0].get("widens").is_none());
    assert_eq!(repairs[1]["escape_cost"], 0);
    assert_eq!(repairs[1]["widens"]["kind"], "authority");
    assert_eq!(repairs[1]["widens"]["cost"], Widening::Authority.cost());
    // Printed order: `widens` after `escape_cost`, before `description`.
    let text = serde_json::to_string(&diagnostic).unwrap();
    let widen_at = text.find("\"widens\":").unwrap();
    assert!(text[..widen_at].contains("\"escape_cost\":"));
    assert!(text[widen_at..].contains("\"description\":\"widen\""));
}

#[test]
fn an_escape_carries_its_kind_and_cost_and_others_carry_no_escape_key() {
    // ADR 0095 rule 5: `escape` is present exactly when `escape_cost` is above 0.
    let mut diagnostic = minimal(Code::U7003, "src/a.una", 1);
    for escape in [None, Some(Escape::EscapeBudget)] {
        diagnostic.repairs.push(Repair {
            widens: None,
            rank: 1,
            escape,
            description: "a route".to_owned(),
            edit: None,
        });
    }
    let value = serde_json::to_value(&diagnostic).unwrap();
    let repairs = value["repairs"].as_array().unwrap();
    assert!(repairs[0].get("escape").is_none());
    assert_eq!(repairs[0]["escape_cost"], 0);
    assert_eq!(repairs[1]["escape_cost"], 1);
    assert_eq!(repairs[1]["escape"]["kind"], "escape_budget");
    assert_eq!(
        repairs[1]["escape"]["cost"],
        Escape::EscapeBudget.cost(),
        "the cost is one sentence of the diagnostic model"
    );
    // Printed order (ADR 0076 rule 1): `escape` right after `escape_cost`.
    let text = serde_json::to_string(&diagnostic).unwrap();
    let at = |key: &str| text.rfind(&format!("\"{key}\":")).unwrap();
    assert!(at("rank") < at("escape_cost"));
    assert!(at("escape_cost") < at("escape"));
    assert!(at("escape") < at("description"));
    assert!(at("description") < at("edit"));
}

#[test]
fn list_is_ordered_by_file_then_start_position_then_code() {
    let mut late_column = minimal(Code::U0101, "src/a.una", 2);
    late_column.primary.start.col = 9;
    let diagnostics = [
        minimal(Code::U2001, "src/b.una", 1),
        late_column,
        minimal(Code::U0102, "src/a.una", 2),
        minimal(Code::U0101, "src/a.una", 2),
        minimal(Code::U1001, "src/a.una", 1),
    ];
    let value: Value = serde_json::from_str(&render_json(&diagnostics).unwrap()).unwrap();
    let order: Vec<(String, u64, u64, String)> = value["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["primary"]["file"].as_str().unwrap().to_owned(),
                diagnostic["primary"]["start"]["line"].as_u64().unwrap(),
                diagnostic["primary"]["start"]["col"].as_u64().unwrap(),
                diagnostic["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected = [
        ("src/a.una", 1, 1, "U1001"),
        ("src/a.una", 2, 1, "U0101"),
        ("src/a.una", 2, 1, "U0102"),
        ("src/a.una", 2, 9, "U0101"),
        ("src/b.una", 1, 1, "U2001"),
    ]
    .map(|(file, line, col, code)| (file.to_owned(), line, col, code.to_owned()));
    assert_eq!(order, expected);
}

#[test]
fn every_element_keeps_its_own_schema() {
    let diagnostics = [minimal(Code::U0001, "src/a.una", 1)];
    let value: Value = serde_json::from_str(&render_json(&diagnostics).unwrap()).unwrap();
    assert_eq!(value["schema"], "una.diagnostics/1");
    assert_eq!(value["diagnostics"][0]["schema"], "una.diagnostic/1");
}

#[test]
fn a_binding_label_is_written_only_when_not_public() {
    let bindings = [
        Binding {
            name: "pin".to_owned(),
            value: json!(-1),
            label: Some("secret".to_owned()),
        },
        Binding {
            name: "result".to_owned(),
            value: json!(0),
            label: None,
        },
    ];
    let value = serde_json::to_value(bindings).unwrap();
    assert_eq!(
        value,
        json!([
            {"name": "pin", "value": -1, "label": "secret"},
            {"name": "result", "value": 0},
        ])
    );
}
