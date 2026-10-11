//! Resolution, types, effects, capabilities, labels, inference and elaboration to Core.
//!
//! M3 implements names (`names`), M4 types (`types`); later milestones add the other checks.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod names;
pub mod types;

pub use names::{ModuleInput, PackageInput, RunInput, check_names, check_names_indexed};
pub use types::{
    Header, HeaderUse, Index, PendingObligation, Rejected, TypeCheck, check_types,
    check_types_with_rejected,
};
