//! JSON rendering of diagnostics: the Agent ABI of `spec/13 §1` and ADR 0012.

use serde::{Serialize, Serializer};

use crate::diagnostic::{Diagnostic, in_output_order};

/// The schema of a list of diagnostics (`spec/13 §1`, ADR 0012).
pub const DIAGNOSTICS_SCHEMA: &str = "una.diagnostics/1";

/// A list of diagnostics as printed with `--json`, in output order (`spec/13 §1`).
#[derive(Clone, Debug)]
pub struct DiagnosticList<'a> {
    diagnostics: Vec<&'a Diagnostic>,
}

impl<'a> DiagnosticList<'a> {
    /// Wraps diagnostics and puts them in output order: file, start position, code.
    pub fn new(diagnostics: &'a [Diagnostic]) -> Self {
        DiagnosticList {
            diagnostics: in_output_order(diagnostics),
        }
    }
}

impl Serialize for DiagnosticList<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Wire {
            schema: DIAGNOSTICS_SCHEMA,
            diagnostics: &self.diagnostics,
            fixable: self
                .diagnostics
                .iter()
                .filter(|diagnostic| fixable_one(diagnostic))
                .count(),
        }
        .serialize(serializer)
    }
}

/// The JSON shape of a list of diagnostics (ADR 0012), with the count of ADR 0096 rule 6.
#[derive(Serialize)]
struct Wire<'a> {
    schema: &'static str,
    diagnostics: &'a [&'a Diagnostic],
    fixable: usize,
}

/// Whether `una fix` applies the first repair of `diagnostic` (ADR 0084): its edit is not
/// `null`, its escape cost is 0, it widens nothing, and it edits a module. A U7001 counts too,
/// since `una fix`'s format step applies it (ADR 0096 rule 6).
fn fixable_one(diagnostic: &Diagnostic) -> bool {
    diagnostic
        .repairs
        .iter()
        .min_by_key(|repair| repair.order_key())
        .is_some_and(|repair| {
            repair.escape.is_none()
                && !repair.widens()
                && repair
                    .edit
                    .as_ref()
                    .is_some_and(|edit| edit.location.file.ends_with(".una"))
        })
}

/// How many of `diagnostics` have a first repair that `una fix` applies (ADR 0096 rule 6).
pub fn fixable(diagnostics: &[Diagnostic]) -> usize {
    diagnostics.iter().filter(|d| fixable_one(d)).count()
}

/// Renders diagnostics as the compact JSON document of `spec/13 §1`, without the final newline.
pub fn render_json(diagnostics: &[Diagnostic]) -> Result<String, serde_json::Error> {
    serde_json::to_string(&DiagnosticList::new(diagnostics))
}
