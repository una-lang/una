//! Resolution, types, effects, capabilities, labels, inference and elaboration to Core.
//!
//! M3 implements names (`names`), M4 types (`types`); later milestones add the other checks.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod names;
pub mod types;

pub use names::{ModuleInput, PackageInput, RunInput, check_names};
pub use types::{Header, HeaderUse, PendingObligation, TypeCheck, check_types};
