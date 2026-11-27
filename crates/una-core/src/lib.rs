//! Core IR (`spec/09`): the declarations, types and expressions over which the checker's
//! rules, the verifier and the interpreter are defined, the labels of `spec/07 §2`, and the
//! printer of `una check --dump-core`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod bigint;
pub mod complex;
pub mod decimal;
pub mod dense;
pub mod depth;
mod descriptor;
pub mod elementary;
pub mod float;
pub mod float32;
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
/// (`spec/14 §1`): a longer text is refused in O(b) before any conversion, as in CPython
/// (#914). It bounds what a run reads, not what it writes: an `Int` of any size is written as
/// text, in O(M(d) log d) for its d digits by the divide and conquer of [`bigint`], the cost
/// class of one multiplication of its size, and its text is counted by the memory budget as any
/// value's (`spec/11 §7`, #1223). Reading divides and conquers too, but stays superlinear in its
/// input, so the bound keeps the work a text from outside can ask for linear in its bytes.
pub const MAX_INT_DIGITS: usize = 4300;

/// The bytes from which a write of an `Int` as text first reserves them against the run's
/// memory budget (`spec/11 §7`, #1223): a smaller write is within the slack of every check.
pub const INT_TEXT_RESERVED_FROM: usize = 1 << 16;

/// The bytes per limb that writing an `Int` in decimal may hold at once: the digits of the
/// conversion, 20 bytes per limb, their copy into the text written, 20 more, and the
/// temporaries of the divide and conquer of [`bigint`], within 7 words per limb (the powers
/// `10^(19·2^i)` up to half the value, the quotient and remainder of each split and the work
/// space of a division).
const INT_TEXT_BYTES_PER_LIMB: usize = 96;

/// An upper bound on the bytes that writing `n` in decimal holds at once, its text and the
/// conversion's temporaries, from its bit length in O(1) (`spec/11 §7`, #1223); `None` below
/// [`INT_TEXT_RESERVED_FROM`], where the engines write without reserving. Each engine checks
/// it against the run's memory budget before it converts an `Int`, or a `Decimal`'s
/// coefficient, and aborts the run with U6003 when the write does not fit, so the conversion
/// never passes the hard ceiling of twice the budget; the text it is appended to reserves its
/// own growth.
pub fn int_text_room(n: &bigint::BigInt) -> Option<usize> {
    let limbs = usize::try_from(n.bits().div_ceil(64)).unwrap_or(usize::MAX);
    let bytes = limbs.saturating_mul(INT_TEXT_BYTES_PER_LIMB);
    (bytes >= INT_TEXT_RESERVED_FROM).then_some(bytes)
}

pub use descriptor::Descriptor;
pub use ir::{
    Arm, Atom, Base, BinOp, BitSetOp, Bound, COLLECTIONS_MODULE, CapOp, Clause, Coll,
    ConditionEdit, ConstantCheck, Constraint, Ctor, DataVariant, DecimalOp, Decl, Example, ExitTag,
    Expectation, Expr, FixedOp, FloatOp, FloatResult, Fun, HOST_MODULE, HandlerOp, HashMapOp,
    HashSetOp, IntType, LIMIT_MODULE, LIMITER, Lock, LoopSource, MapOp, Module, Name, OnStop,
    PRELUDE_ENGINE_KINDS, PRELUDE_MODULE, Path, PreludeField, PreludeVariant, Prim, PriorityOp,
    Profile, Program, Refinement, Rhs, SeqOp, SetOp, ShapeFun, Signature, Source, Term, Type, Use,
    prelude_variants,
};
pub use label::{Accepts, Label, Level, Names, counters as label_counters};
pub use names::{binders, has_tasks, rhs_vars, vars};
pub use print::{atomic, print_atom, print_module, print_term, print_term_with, print_type, quote};
pub use text::Text;
