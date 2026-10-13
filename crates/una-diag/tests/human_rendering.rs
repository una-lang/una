//! Human rendering of diagnostics (`spec/13 §1`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;

use pretty_assertions::assert_eq;
use serde_json::json;
use una_diag::{
    Binding, Code, Counterexample, Diagnostic, Location, ObligationKind, ObligationRef,
    ObligationStatus, Position, Related, Repair, render_human,
};

const FILE: &str = "src/postcondition_refuted.una";

const SOURCE: &str = "module postcondition_refuted

pub sold_out(remaining: Int) -> Bool
  requires remaining >= 0
  ensures remaining < 1 implies result == true
=
  remaining < 0
";

fn location(start: (u32, u32), end: (u32, u32)) -> Location {
    Location {
        file: FILE.to_owned(),
        start: Position {
            line: start.0,
            col: start.1,
        },
        end: Position {
            line: end.0,
            col: end.1,
        },
    }
}

fn sources() -> BTreeMap<String, String> {
    BTreeMap::from([(FILE.to_owned(), SOURCE.to_owned())])
}

/// A refuted postcondition over `SOURCE`, with every optional part present.
fn refuted() -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        Code::U5001,
        "postcondition of sold_out is refuted",
        location((7, 3), (7, 16)),
    );
    diagnostic.related.push(Related {
        location: location((5, 11), (5, 47)),
        message: "ensures clause".to_owned(),
    });
    diagnostic.obligation = Some(ObligationRef {
        id: "ob-0123456789ab".to_owned(),
        kind: ObligationKind::Postcondition,
        goal: "remaining < 1 implies result == true".to_owned(),
        status: ObligationStatus::Refuted,
    });
    diagnostic.counterexample = Some(Counterexample {
        bindings: vec![
            Binding {
                name: "remaining".to_owned(),
                value: json!(0),
                label: None,
            },
            Binding {
                name: "result".to_owned(),
                value: json!(false),
                label: None,
            },
        ],
    });
    diagnostic.repairs.push(Repair {
        widens: None,
        rank: 1,
        escape: None,
        description: "compare with 1 or adjust the ensures clause".to_owned(),
        edit: None,
    });
    diagnostic
}

#[test]
fn renders_source_excerpts_without_color() {
    insta::assert_snapshot!(
        "refuted_with_source",
        render_human(&[refuted()], &sources(), false)
    );
}

#[test]
fn falls_back_to_plain_text_without_the_source() {
    insta::assert_snapshot!(
        "refuted_without_source",
        render_human(&[refuted()], &BTreeMap::new(), false)
    );
}

#[test]
fn falls_back_to_plain_text_when_a_position_is_outside_the_source() {
    let mut diagnostic = refuted();
    diagnostic.related[0].location.start.line = 99;
    let rendered = render_human(&[diagnostic], &sources(), false);
    assert!(rendered.starts_with("[U5001] Error: postcondition of sold_out is refuted\n"));
    assert!(rendered.contains("src/postcondition_refuted.una:99:11"));
}

#[test]
fn renders_nothing_for_no_diagnostics() {
    assert_eq!(render_human(&[], &sources(), false), "");
}

#[test]
fn accepts_a_position_just_past_the_end_of_the_file() {
    let diagnostic = Diagnostic::error(
        Code::U0101,
        "unexpected end of file",
        location((8, 1), (8, 1)),
    );
    let rendered = render_human(&[diagnostic], &sources(), false);
    assert!(rendered.starts_with("[U0101] Error: unexpected end of file\n"));
}

#[test]
fn color_is_opt_in() {
    let plain = render_human(&[refuted()], &sources(), false);
    let colored = render_human(&[refuted()], &sources(), true);
    assert!(!plain.contains('\u{1b}'));
    assert!(colored.contains('\u{1b}'));
}
