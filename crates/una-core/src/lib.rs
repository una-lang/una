//! Core IR (`spec/09`): the declarations, types and expressions over which the checker's
//! rules, the verifier and the interpreter are defined, the labels of `spec/07 §2`, and the
//! printer of `una check --dump-core`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod decimal;
pub mod dense;
mod descriptor;
pub mod float;
pub mod flow;
pub mod http;
mod ir;
pub mod json;
mod label;
mod names;
mod print;
mod text;

/// The most decimal digits of an integer that a text may write, its sign excluded and leading
/// zeros included, for JSON (`spec/23 §3`), `parse_int` (`spec/22 §4`) and `to_int`
/// (`spec/14 §1`): converting d digits to an exact integer takes O(d²), so a longer text is
/// refused before any conversion, as in CPython (#914).
pub const MAX_INT_DIGITS: usize = 4300;

pub use descriptor::Descriptor;
pub use ir::{
    Arm, Atom, Base, BinOp, BitSetOp, Bound, COLLECTIONS_MODULE, CapOp, Clause, Coll,
    ConditionEdit, Constraint, Ctor, DataVariant, DecimalOp, Decl, Example, ExitTag, Expectation,
    Expr, FixedOp, FloatOp, FloatResult, Fun, HOST_MODULE, HandlerOp, HashMapOp, HashSetOp,
    IntType, Lock, LoopSource, MapOp, Module, Name, OnStop, PRELUDE_ENGINE_KINDS, PRELUDE_MODULE,
    Path, PreludeField, PreludeVariant, Prim, PriorityOp, Program, Refinement, Rhs, SeqOp, SetOp,
    ShapeFun, Signature, Source, Term, Type, Use, prelude_variants,
};
pub use label::{Accepts, Label, Level, Names, counters as label_counters};
pub use names::{binders, rhs_vars, vars};
pub use print::{print_atom, print_module, print_term, print_term_with, print_type, quote};
pub use text::Text;
