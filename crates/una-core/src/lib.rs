//! Core IR (`spec/09`): the declarations, types and expressions over which the checker's
//! rules, the verifier and the interpreter are defined, the labels of `spec/07 §2`, and the
//! printer of `una check --dump-core`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod descriptor;
pub mod http;
mod ir;
pub mod json;
mod label;
mod names;
mod print;
mod text;

pub use descriptor::Descriptor;
pub use ir::{
    Arm, Atom, Base, BinOp, Bound, CapOp, Clause, Constraint, Ctor, DataVariant, Decl, Example,
    Expectation, Expr, FixedOp, Fun, HOST_MODULE, HandlerOp, IntType, LoopSource, MapOp, Module,
    Name, PRELUDE_ENGINE_KINDS, PRELUDE_MODULE, Path, PreludeField, PreludeVariant, Prim, Program,
    Refinement, Rhs, Signature, Source, Term, Type, Use, prelude_variants,
};
pub use label::{Accepts, Label, Level, Names, counters as label_counters};
pub use names::{binders, rhs_vars, vars};
pub use print::{print_atom, print_module, print_term, print_term_with, print_type, quote};
pub use text::Text;
