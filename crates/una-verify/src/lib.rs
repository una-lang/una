//! Verification (`spec/12`): obligation generation over Core IR, the SMT-LIB encoding, the
//! solver driver, the proof cache, counterexamples and the diagnostics U5001 and U5002.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cache;
mod generate;
mod report;
mod run;
mod smt;
mod solver;

pub use generate::{Obligation, obligations, trivial_ensures};
pub use report::diagnostics;
pub use run::{Lock, Outcome, Problem, Settings, queries, verify};
