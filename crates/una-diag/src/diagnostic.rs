//! The diagnostic model of `spec/13 §1`.

use serde::{Serialize, Serializer};

use crate::codes::{Code, Domain};

/// The schema of one diagnostic (`spec/13 §1`).
pub const DIAGNOSTIC_SCHEMA: &str = "una.diagnostic/1";

/// The severity of a diagnostic. v0.1 defines errors only (`spec/13 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The program is rejected.
    Error,
    /// A likely problem that does not reject the program. Not used in v0.1.
    Warning,
    /// Additional information. Not used in v0.1.
    Note,
}

impl Severity {
    /// The name of the severity in diagnostics, for example `error`.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

impl Serialize for Severity {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A position in a source file: a 1-based line and a 1-based column counted in Unicode scalar
/// values (`spec/13 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Position {
    /// The 1-based line.
    pub line: u32,
    /// The 1-based column, counted in Unicode scalar values.
    pub col: u32,
}

/// A range of a source file. `end` is exclusive.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Location {
    /// The file, relative to the package root, with `/` as separator.
    pub file: String,
    /// The first position of the range.
    pub start: Position,
    /// The position just past the range.
    pub end: Position,
}

/// A location that explains the other side of a diagnostic, such as a declaration, a contract
/// or a capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Related {
    /// Where the related construct is.
    #[serde(flatten)]
    pub location: Location,
    /// What the related construct is, for example `ensures clause`.
    pub message: String,
}

/// The kind of a proof obligation (`spec/12 §3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObligationKind {
    /// A target refinement at a subsumption point.
    Refinement,
    /// The `requires` of a callee or capability operation.
    Precondition,
    /// The `ensures` of a function or handler operation, on a normal return path.
    Postcondition,
    /// A nonzero divisor.
    Division,
    /// The arms of a `when` cover every value.
    Exhaustive,
    /// The exact result of an operation on a fixed-width integer type, or of a narrowing
    /// conversion, lies in the type's range (`spec/18 §3`).
    Overflow,
}

impl ObligationKind {
    /// The name of the kind in diagnostics, for example `postcondition`.
    pub fn as_str(self) -> &'static str {
        match self {
            ObligationKind::Refinement => "refinement",
            ObligationKind::Precondition => "precondition",
            ObligationKind::Postcondition => "postcondition",
            ObligationKind::Division => "division",
            ObligationKind::Exhaustive => "exhaustive",
            ObligationKind::Overflow => "overflow",
        }
    }
}

impl Serialize for ObligationKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// The verification status of a proof obligation (`spec/12 §6`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObligationStatus {
    /// The solver proved the obligation.
    Proven,
    /// The solver found a counterexample.
    Refuted,
    /// The solver could not decide the obligation.
    Unknown,
}

impl ObligationStatus {
    /// The name of the status in diagnostics, for example `refuted`.
    pub fn as_str(self) -> &'static str {
        match self {
            ObligationStatus::Proven => "proven",
            ObligationStatus::Refuted => "refuted",
            ObligationStatus::Unknown => "unknown",
        }
    }
}

impl Serialize for ObligationStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// The obligation that a verification diagnostic (U5001, U5002) reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ObligationRef {
    /// The stable obligation id, for example `ob-3f09a1c2e7b4` (`spec/12 §8`).
    pub id: String,
    /// The kind of the obligation.
    pub kind: ObligationKind,
    /// The goal, as source text.
    pub goal: String,
    /// The verification status.
    pub status: ObligationStatus,
}

/// A model of a refuted obligation, mapped back to source names (`spec/12 §7`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Counterexample {
    /// The values of source names in the model.
    pub bindings: Vec<Binding>,
}

/// The value of one source name in a counterexample.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Binding {
    /// The source name, for example `e.remaining`.
    pub name: String,
    /// The value. How `Int` values beyond 64 bits are written is decided with verification (M8).
    pub value: serde_json::Value,
}

/// A repair candidate (`spec/13 §3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Repair {
    /// The rank among candidates; 1 is the best.
    pub rank: u32,
    /// The number of escape hatches the repair introduces (`spec/13 §3`).
    pub escape_cost: u32,
    /// What the repair does.
    pub description: String,
    /// The mechanical edit, when the repair is mechanical.
    pub edit: Option<Edit>,
}

/// A mechanical edit: replace a range of a file with new text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Edit {
    /// The range to replace.
    #[serde(flatten)]
    pub location: Location,
    /// The replacement text.
    pub replacement: String,
}

/// A problem reported by the toolchain (`spec/13 §1`).
///
/// The `schema`, `domain` and `explain` fields of the JSON shape are derived from `code`, so they
/// cannot disagree with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The registered code (`spec/13 §4`).
    pub code: Code,
    /// The severity. v0.1 defines errors only.
    pub severity: Severity,
    /// One sentence, lowercase start, no trailing period, naming the construct.
    pub message: String,
    /// The smallest construct that explains the problem.
    pub primary: Location,
    /// Locations that explain the other side of the problem.
    pub related: Vec<Related>,
    /// The obligation, present only for U5001 and U5002.
    pub obligation: Option<ObligationRef>,
    /// The counterexample, present only for U5001 and U5002.
    pub counterexample: Option<Counterexample>,
    /// Repair candidates, in any order; output sorts them by escape cost, then rank.
    pub repairs: Vec<Repair>,
}

impl Diagnostic {
    /// An error with a code, a message and a primary location, and no other part.
    pub fn error(code: Code, message: impl Into<String>, primary: Location) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            message: message.into(),
            primary,
            related: Vec::new(),
            obligation: None,
            counterexample: None,
            repairs: Vec::new(),
        }
    }

    /// The domain of the diagnostic, from its code.
    pub fn domain(&self) -> Domain {
        self.code.domain()
    }

    /// The command that explains the diagnostic, for example `una explain U5001`.
    pub fn explain_command(&self) -> String {
        format!("una explain {}", self.code)
    }

    /// The repairs in output order: by escape cost, then rank (`spec/13 §1`).
    pub(crate) fn repairs_in_order(&self) -> Vec<&Repair> {
        let mut repairs: Vec<&Repair> = self.repairs.iter().collect();
        repairs.sort_by_key(|repair| (repair.escape_cost, repair.rank));
        repairs
    }
}

impl Serialize for Diagnostic {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Wire {
            schema: DIAGNOSTIC_SCHEMA,
            code: self.code,
            severity: self.severity,
            domain: self.domain(),
            message: &self.message,
            primary: &self.primary,
            related: &self.related,
            obligation: self.obligation.as_ref(),
            counterexample: self.counterexample.as_ref(),
            repairs: self.repairs_in_order(),
            explain: self.explain_command(),
        }
        .serialize(serializer)
    }
}

/// The JSON shape of a diagnostic, with fields in the order of `spec/13 §1`.
#[derive(Serialize)]
struct Wire<'a> {
    schema: &'static str,
    code: Code,
    severity: Severity,
    domain: Domain,
    message: &'a str,
    primary: &'a Location,
    related: &'a [Related],
    #[serde(skip_serializing_if = "Option::is_none")]
    obligation: Option<&'a ObligationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    counterexample: Option<&'a Counterexample>,
    repairs: Vec<&'a Repair>,
    explain: String,
}

/// Diagnostics in output order: by file, start position, then code (`spec/13 §1`). The sort is
/// stable, so equal keys keep their reporting order.
pub fn in_output_order(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    let mut sorted: Vec<&Diagnostic> = diagnostics.iter().collect();
    sorted.sort_by(|a, b| {
        (a.primary.file.as_str(), a.primary.start, a.code).cmp(&(
            b.primary.file.as_str(),
            b.primary.start,
            b.code,
        ))
    });
    sorted
}
