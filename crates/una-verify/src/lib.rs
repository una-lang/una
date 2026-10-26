//! Verification (`spec/12`): obligation generation over Core IR, the SMT-LIB encoding, the
//! solver driver, the proof cache, counterexamples and the diagnostics U5001 and U5002.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cache;
mod frame;
mod generate;
mod report;
mod run;
mod smt;
mod solver;

pub use cache::{KEY_DOMAIN, ProofCache};
pub use generate::{
    Effect, EffectAt, Obligation, Site, SourceLayout, Within, obligations, trivial_ensures,
    unwrap_layout,
};
pub use report::diagnostics;
pub use run::{Lock, Outcome, Problem, Settings, queries, verify};
