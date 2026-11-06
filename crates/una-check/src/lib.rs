//! Resolution, types, effects, capabilities, labels, inference and elaboration to Core.
//!
//! M3 implements names (`names`), M4 types (`types`); later milestones add the other checks.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod names;
pub mod types;

pub use names::{
    Lock, LockClause, ModuleInput, PackageInput, Poison, RunInput, check_names,
    check_names_indexed, check_names_poisoned,
};
/// The names of `std.host` that only an import brings into scope, edition 0.4 (ADR 0123 rule
/// 0, `spec/31 §1`): the one list of them, which the standard library listing reads.
pub fn server_names() -> &'static [&'static str] {
    &names::prelude::SERVER_NAMES
}

pub use types::{
    DriveUnits, HandlerHeader, Header, HeaderUse, Index, PendingObligation, Rejected,
    StandardSource, TimingSite, TypeCheck, builtin_methods, check_types, check_types_with_rejected,
    standard_source,
};
