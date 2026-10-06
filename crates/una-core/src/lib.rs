//! Core IR (`spec/09`): the declarations, types and expressions over which the checker's
//! rules, the verifier and the interpreter are defined, the labels of `spec/07 §2`, and the
//! printer of `una check --dump-core`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod ir;
mod label;
mod names;
mod print;

pub use ir::{
    Arm, Atom, Base, BinOp, Bound, CapOp, Clause, Ctor, DataVariant, Decl, Example, Expectation,
    Expr, FixedOp, Fun, HOST_MODULE, HandlerOp, IntType, LoopSource, Module, Name, PRELUDE_MODULE,
    Path, Prim, Program, Refinement, Rhs, Signature, Source, Term, Type, Use,
};
pub use label::{Accepts, Label, Level};
pub use names::{rhs_vars, vars};
pub use print::{print_atom, print_module, print_term, print_term_with, print_type, quote};
