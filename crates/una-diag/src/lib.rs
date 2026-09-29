//! Diagnostics for the Una toolchain: the diagnostic model of `spec/13 §1`, the code registry
//! of `spec/13 §4`, human and JSON rendering, and the explain texts printed by `una explain`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod codes;
mod diagnostic;
mod human;
mod json;

pub use codes::{Code, Domain, UnknownCode};
pub use diagnostic::{
    Binding, Counterexample, DIAGNOSTIC_SCHEMA, Diagnostic, Edit, Location, ObligationKind,
    ObligationRef, ObligationStatus, Position, Related, Repair, Severity,
};
pub use human::render_human;
pub use json::{DIAGNOSTICS_SCHEMA, DiagnosticList, render_json};
