//! Diagnostics for the Una toolchain: the diagnostic model of `spec/13 §1`, the code registry
//! of `spec/13 §4`, human and JSON rendering, and the explain texts printed by `una explain`;
//! and [`files`], the writes under a package that a planted link cannot redirect (#1160), which
//! every crate that writes under a package shares.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod codes;
mod diagnostic;
pub mod files;
mod human;
mod json;

pub use codes::{Code, Domain, UnknownCode};
pub use diagnostic::{
    Binding, Counterexample, DIAGNOSTIC_SCHEMA, Diagnostic, Edit, Escape, Location, ObligationKind,
    ObligationRef, ObligationStatus, Position, Ranked, Related, Repair, Severity, Widening,
    in_output_order,
};
pub use human::render_human;
pub use json::{DIAGNOSTICS_SCHEMA, DiagnosticList, fixable, render_json};
