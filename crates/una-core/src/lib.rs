//! Core IR (`spec/09`): the declarations, types and expressions over which the checker's
//! rules, the verifier and the interpreter are defined, the labels of `spec/07 §2`, and the
//! printer of `una check --dump-core`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod ir;
mod label;
mod print;

pub use ir::{
    Arm, Atom, Base, Bound, CapOp, Ctor, DataVariant, Decl, Example, Expectation, Expr, Fun,
    HandlerOp, Module, Name, Path, Prim, Program, Rhs, Source, Type, Use,
};
pub use label::{Accepts, Label, Level};
pub use print::{print_atom, print_module, print_type, quote};
