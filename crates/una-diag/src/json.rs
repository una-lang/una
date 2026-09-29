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
        }
        .serialize(serializer)
    }
}

/// The JSON shape of a list of diagnostics (ADR 0012).
#[derive(Serialize)]
struct Wire<'a> {
    schema: &'static str,
    diagnostics: &'a [&'a Diagnostic],
}

/// Renders diagnostics as the compact JSON document of `spec/13 §1`, without the final newline.
pub fn render_json(diagnostics: &[Diagnostic]) -> Result<String, serde_json::Error> {
    serde_json::to_string(&DiagnosticList::new(diagnostics))
}
