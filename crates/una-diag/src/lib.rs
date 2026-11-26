//! Diagnostics for the Una toolchain: the diagnostic model of `spec/13 §1`, the code registry
//! of `spec/13 §4`, human and JSON rendering, and the explain texts printed by `una explain`.
//! It also holds [`safe_write`], the writes into a package directory shared by the command line
//! and the verifier (#1160).
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod binding_words;
mod codes;
mod diagnostic;
mod human;
mod json;
pub mod safe_write;
pub mod text;

pub use codes::{Code, Domain, UnknownCode};
pub use diagnostic::{
    Binding, Counterexample, DIAGNOSTIC_SCHEMA, Diagnostic, Edit, Escape, Limit, Location,
    ObligationKind, ObligationRef, ObligationStatus, Position, Ranked, Related, Repair, Severity,
    Widening, in_output_order,
};
pub use human::render_human;
pub use json::{DIAGNOSTICS_SCHEMA, DiagnosticList, fixable, render_json};
