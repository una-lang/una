//! Lexical and syntactic errors, located by span until they become diagnostics.

use una_diag::{Code, Diagnostic, Edit, Repair};

use crate::source::{SourceDb, Span};

/// A lexical or syntactic problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// The registered code.
    pub code: Code,
    /// The smallest construct that explains the problem.
    pub span: Span,
    /// One sentence, lowercase start, no trailing period.
    pub message: String,
    /// Repair candidates, best first.
    pub repairs: Vec<SyntaxRepair>,
}

/// A repair candidate for a syntax error. Syntax repairs use no escape hatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxRepair {
    /// What the repair does.
    pub description: String,
    /// The mechanical edit: the range to replace and its replacement.
    pub edit: Option<(Span, String)>,
}

impl SyntaxError {
    /// An error without repairs.
    pub fn new(code: Code, span: Span, message: impl Into<String>) -> SyntaxError {
        SyntaxError {
            code,
            span,
            message: message.into(),
            repairs: Vec::new(),
        }
    }

    /// Adds a repair that replaces `span` with `replacement`.
    pub fn with_edit(
        mut self,
        description: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
    ) -> SyntaxError {
        self.repairs.push(SyntaxRepair {
            description: description.into(),
            edit: Some((span, replacement.into())),
        });
        self
    }

    /// The diagnostic of this error, with positions resolved in `sources`.
    pub fn to_diagnostic(&self, sources: &SourceDb) -> Diagnostic {
        let mut diagnostic =
            Diagnostic::error(self.code, &self.message, sources.location(self.span));
        diagnostic.repairs = self
            .repairs
            .iter()
            .enumerate()
            .map(|(index, repair)| Repair {
                rank: u32::try_from(index + 1).unwrap_or(u32::MAX),
                escape_cost: 0,
                description: repair.description.clone(),
                edit: repair.edit.as_ref().map(|(span, replacement)| Edit {
                    location: sources.location(*span),
                    replacement: replacement.clone(),
                }),
            })
            .collect();
        diagnostic
    }
}
