//! Core IR (`spec/09`): the declarations, types and expressions over which the checker's
//! rules, the verifier and the interpreter are defined, the labels of `spec/07 §2`, and the
//! printer of `una check --dump-core`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod bigint;
pub mod complex;
pub mod decimal;
pub mod dense;
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
/// (`spec/14 §1`): a longer text is refused before any conversion, as in CPython (#914). The
/// same bound holds the other way: an integer of more digits is never written as text
/// ([`int_text`], `spec/11 §7`, #923). The bound is the specification's; the conversions of
/// [`bigint`] are subquadratic past it, O(M(d) log d), and lifting it is #1223.
pub const MAX_INT_DIGITS: usize = 4300;

/// The message of U6003 when a run writes an integer beyond [`MAX_INT_DIGITS`] as text
/// (`spec/11 §7`, #923), the same in both engines.
pub const INT_TEXT_LIMIT_MESSAGE: &str =
    "an integer of more than 4300 digits is written as text here, past the limit of a run";

/// The decimal text of `n` when it has at most [`MAX_INT_DIGITS`] digits, its sign excluded,
/// else `None` (`spec/11 §7`, #923): a longer integer is refused before any conversion, as a
/// longer text is before parsing. Decided in O(1) from the bit length: only an integer with as
/// many bits as 10^4300 is compared with it, computed once.
pub fn int_text(n: &bigint::BigInt) -> Option<String> {
    int_fits(n).then(|| n.to_string())
}

/// Whether `n` has at most [`MAX_INT_DIGITS`] decimal digits, its sign excluded, in O(1) apart
/// from one comparison at the bound.
pub fn int_fits(n: &bigint::BigInt) -> bool {
    static BOUND: std::sync::OnceLock<bigint::BigUint> = std::sync::OnceLock::new();
    let bound = BOUND.get_or_init(|| {
        bigint::BigUint::from(10u32).pow(u32::try_from(MAX_INT_DIGITS).unwrap_or(u32::MAX))
    });
    let bits = n.bits();
    if bits < bound.bits() {
        true
    } else if bits > bound.bits() {
        false
    } else {
        n.magnitude() < bound
    }
}

pub use descriptor::Descriptor;
pub use ir::{
    Arm, Atom, Base, BinOp, BitSetOp, Bound, COLLECTIONS_MODULE, CapOp, Clause, Coll,
    ConditionEdit, ConstantCheck, Constraint, Ctor, DataVariant, DecimalOp, Decl, Example, ExitTag, Expectation,
    Expr, FixedOp, FloatOp, FloatResult, Fun, HOST_MODULE, HandlerOp, HashMapOp, HashSetOp,
    IntType, LIMIT_MODULE, LIMITER, Lock, LoopSource, MapOp, Module, Name, OnStop,
    PRELUDE_ENGINE_KINDS, PRELUDE_MODULE, Path, PreludeField, PreludeVariant, Prim, PriorityOp,
    Profile, Program, Refinement, Rhs, SeqOp, SetOp, ShapeFun, Signature, Source, Term, Type, Use,
    prelude_variants,
};
pub use label::{Accepts, Label, Level, Names, counters as label_counters};
pub use names::{binders, has_tasks, rhs_vars, vars};
pub use print::{atomic, print_atom, print_module, print_term, print_term_with, print_type, quote};
pub use text::Text;

#[cfg(test)]
mod int_text_tests {
    use super::*;
    use crate::bigint::BigInt;

    #[test]
    fn the_bound_of_writing_is_the_bound_of_parsing() {
        let ten = BigInt::from(10);
        let at = ten.pow(u32::try_from(MAX_INT_DIGITS).unwrap()) - 1;
        assert_eq!(int_text(&at).map(|t| t.len()), Some(MAX_INT_DIGITS));
        assert_eq!(int_text(&-&at).map(|t| t.len()), Some(MAX_INT_DIGITS + 1));
        let over = &at + 1;
        assert_eq!(int_text(&over), None);
        assert_eq!(int_text(&-&over), None);
        assert!(int_fits(&BigInt::from(0)));
        assert!(!int_fits(&(&over * &ten)));
    }
}
