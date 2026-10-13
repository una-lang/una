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
    /// The label of the name's declared type, as `secret`, when it is not `public`: the value
    /// is the solver's, hypothetical data of a labeled type (ADR 0078). An added key, omitted
    /// when `public`, so the schema stays `una.diagnostic/1` (ADR 0076).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The escape hatch a repair takes (`spec/13 §3`, ADR 0095 rule 2). A repair that takes none
/// keeps the guarantee the diagnostic enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Escape {
    /// A runtime test replaces a static proof (`spec/06 §6`).
    Checked,
    /// `[escape_budget] checked_per_module` is raised.
    EscapeBudget,
    /// A module is declared or listed as trusted, or a value is declassified through a policy
    /// (`spec/07 §5, §6`).
    Trusted,
}

impl Escape {
    /// The kind in the `escape` key of a repair, for example `checked`.
    pub fn kind(self) -> &'static str {
        match self {
            Escape::Checked => "checked",
            Escape::EscapeBudget => "escape_budget",
            Escape::Trusted => "trusted",
        }
    }

    /// One sentence that says what the guarantee loses.
    pub fn cost(self) -> &'static str {
        match self {
            Escape::Checked => {
                "a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report"
            }
            Escape::EscapeBudget => {
                "the module may use more runtime tests in place of proofs; each one is listed in the build report and needs review"
            }
            Escape::Trusted => {
                "the guarantee then rests on trusted code, whose labels are not checked: a policy releases the data, or a module joins the trusted base; it is listed in the build report and needs review"
            }
        }
    }
}

/// A repair candidate (`spec/13 §3`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repair {
    /// The rank among candidates; 1 is the best. Output renumbers the ranks in output order
    /// (ADR 0095 rule 3).
    pub rank: u32,
    /// The escape hatch the repair takes, if any. The `escape_cost` of the JSON shape is
    /// derived from it, so the two cannot disagree (ADR 0095 rule 5).
    pub escape: Option<Escape>,
    /// What the repair does.
    pub description: String,
    /// The mechanical edit, when the repair is mechanical.
    pub edit: Option<Edit>,
    /// Whether the edit widens authority or visibility: it adds operations or a `uses` clause,
    /// or makes a declaration `pub`. `una fix` never applies such an edit (ADR 0084). Not part
    /// of the JSON shape.
    pub widens_authority: bool,
}

impl Repair {
    /// The number of escape hatches the repair introduces (`spec/13 §3`).
    pub fn escape_cost(&self) -> u32 {
        u32::from(self.escape.is_some())
    }
}

/// A repair in output order, with the rank output gives it (ADR 0095 rule 3).
#[derive(Clone, Copy, Debug)]
pub struct Ranked<'a> {
    /// The rank in output order: 1 to n, without gaps.
    pub rank: u32,
    /// The repair.
    pub repair: &'a Repair,
}

impl Serialize for Ranked<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RepairWire {
            rank: self.rank,
            escape_cost: self.repair.escape_cost(),
            escape: self.repair.escape.map(|escape| EscapeWire {
                kind: escape.kind(),
                cost: escape.cost(),
            }),
            description: &self.repair.description,
            edit: self.repair.edit.as_ref(),
        }
        .serialize(serializer)
    }
}

/// The JSON shape of a repair, with fields in the order of `spec/13 §1`.
#[derive(Serialize)]
struct RepairWire<'a> {
    rank: u32,
    escape_cost: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    escape: Option<EscapeWire>,
    description: &'a str,
    edit: Option<&'a Edit>,
}

/// The `escape` key of a repair (ADR 0095 rule 5).
#[derive(Serialize)]
struct EscapeWire {
    kind: &'static str,
    cost: &'static str,
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
    /// Repair candidates, in any order; output sorts them by escape cost, then rank, and
    /// renumbers the ranks in that order.
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

    /// The repairs in output order, by escape cost, then rank (`spec/13 §1`), each ranked by its
    /// position: every escape comes after every repair that keeps the guarantee (ADR 0095
    /// rule 3). The sort is stable, so equal keys keep their reporting order. O(r log r) in
    /// the r repairs.
    pub fn repairs_in_order(&self) -> Vec<Ranked<'_>> {
        let mut repairs: Vec<&Repair> = self.repairs.iter().collect();
        repairs.sort_by_key(|repair| (repair.escape_cost(), repair.rank));
        repairs
            .into_iter()
            .zip(1u32..)
            .map(|(repair, rank)| Ranked { rank, repair })
            .collect()
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
    repairs: Vec<Ranked<'a>>,
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
