//! The `BitSet` of `std.collections` (`spec/28 §9`, ADR 0119 rules 14 and 19): the integers
//! from 0 to `size - 1`, one bit each, in words of 64 bits, with the size and the count of
//! members kept beside them, and the conversions of a list (`spec/28 §10`, rule 16).
//!
//! A bit set is a value. An operation that gives a new bit set updates the receiver in place
//! when the machine holds the only reference to it, a binding given up at its last use, and
//! copies its words first otherwise (`spec/16 §4` rule 7). `has`, `add`, `remove`, `with`,
//! `without`, `size` and `len` are O(1) on a bit set held once; the set operations, `first`,
//! `last` and a copy are O(size / 64), and `items` O(size / 64 + len): the words are scanned
//! with the population count and the trailing and leading zero counts.
//!
//! The range precondition of `add` and `with`, and the size precondition of the set
//! operations, are proven by tier 1; the check stays, and if a wrong proof lets one fail, the
//! run aborts (U8001) and never writes outside the bits.

// A set's `Key` holds a `Text`, whose scalar index is filled lazily in a `OnceCell`; `Key`'s
// order and equality read only the text's bytes, never that index, so mutating it never moves a
// key in the tree. Clippy cannot see that, hence the allowance, as in `sets.rs`.
#![allow(clippy::mutable_key_type)]

use std::collections::VecDeque;
use std::rc::Rc;

use una_core::BitSetOp;
use una_core::bigint::{BigInt, Sign};
use una_syntax::Span;

use super::{Abort, Flow, Machine, Stop, fault, none, precondition, some};
use crate::value::{Bits, Items, Key, Value};

/// The fault of `add`, `with` or a set operation outside its precondition, which a proof
/// excludes (ADR 0119 rule 14).
const OUTSIDE: &str = "a bit set operation outside its range";

/// The bits of one word.
const WORD: usize = 64;

/// `n` as an `Int` value.
fn int<'p>(n: usize) -> Value<'p> {
    Value::Int(BigInt::from(n))
}

/// The position of `value` in a bit set of `size` bits: `None` for an integer outside the
/// range, and `Err` for a value that is not an integer, which checking excludes.
fn position(value: Option<&Value<'_>>, size: usize) -> Result<Option<usize>, ()> {
    match value {
        Some(Value::Int(n)) => Ok(usize::try_from(n).ok().filter(|i| *i < size)),
        _ => Err(()),
    }
}

impl Bits {
    /// Whether `i`, below the size, is a member. O(1).
    fn has(&self, i: usize) -> bool {
        self.words
            .get(i / WORD)
            .is_some_and(|word| word & (1u64 << (i % WORD)) != 0)
    }

    /// Makes `i`, below the size, a member, and counts it when it was not one. O(1).
    fn insert(&mut self, i: usize) {
        if let Some(word) = self.words.get_mut(i / WORD) {
            let bit = 1u64 << (i % WORD);
            if *word & bit == 0 {
                *word |= bit;
                self.count += 1;
            }
        }
    }

    /// Makes `i`, below the size, no member, and uncounts it when it was one. O(1).
    fn delete(&mut self, i: usize) {
        if let Some(word) = self.words.get_mut(i / WORD) {
            let bit = 1u64 << (i % WORD);
            if *word & bit != 0 {
                *word &= !bit;
                self.count -= 1;
            }
        }
    }

    /// Recounts the members after a word-wise operation. O(size / 64).
    fn recount(&mut self) {
        self.count = self
            .words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum();
    }

    /// The least member. O(size / 64).
    fn first(&self) -> Option<usize> {
        self.words
            .iter()
            .enumerate()
            .find(|(_, word)| **word != 0)
            .map(|(index, word)| index * WORD + word.trailing_zeros() as usize)
    }

    /// The greatest member. O(size / 64).
    fn last(&self) -> Option<usize> {
        self.words
            .iter()
            .enumerate()
            .rev()
            .find(|(_, word)| **word != 0)
            .map(|(index, word)| index * WORD + (WORD - 1 - word.leading_zeros() as usize))
    }

    /// The members in ascending order. O(size / 64 + len).
    pub(super) fn members(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.count);
        for (index, word) in self.words.iter().enumerate() {
            let mut rest = *word;
            while rest != 0 {
                out.push(index * WORD + rest.trailing_zeros() as usize);
                rest &= rest - 1;
            }
        }
        out
    }
}

/// The word-wise set operations, over two bit sets of one size.
#[derive(Clone, Copy)]
enum Words {
    Union,
    Intersection,
    Difference,
}

/// `op` over the words of `bits` and `other`, in place when `bits` is held only here, on a copy
/// otherwise. O(size / 64).
fn combine(mut bits: Rc<Bits>, other: &Bits, op: Words) -> Rc<Bits> {
    let owned = Rc::make_mut(&mut bits);
    for (word, theirs) in owned.words.iter_mut().zip(&other.words) {
        *word = match op {
            Words::Union => *word | theirs,
            Words::Intersection => *word & theirs,
            Words::Difference => *word & !theirs,
        };
    }
    owned.recount();
    bits
}

/// A bit set's built-in method or its property `size` over its arguments, receiver first.
/// `span` is the call's, where a failed precondition aborts.
pub(super) fn bit_set_apply<'p>(
    op: BitSetOp,
    args: Vec<Value<'p>>,
    span: Option<Span>,
) -> Flow<'p> {
    let bad = || {
        fault(format!(
            "bit set `{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let outside = || {
        Stop::Abort(Abort::Fault {
            message: OUTSIDE.to_owned(),
            span,
        })
    };
    let mut args = args.into_iter();
    let Some(Value::BitSet(mut bits)) = args.next() else {
        return Err(bad());
    };
    let argument = args.next();
    let size = bits.size;
    // The other bit set of a set operation, which MUST have the receiver's size.
    let other = || match &argument {
        Some(Value::BitSet(other)) if other.size == size => Ok(Rc::clone(other)),
        Some(Value::BitSet(_)) => Err(outside()),
        _ => Err(bad()),
    };
    Ok(match op {
        // O(1).
        BitSetOp::Size => int(bits.size),
        // O(1); `false` outside the range.
        BitSetOp::Has => {
            let at = position(argument.as_ref(), bits.size).map_err(|()| bad())?;
            Value::Bool(at.is_some_and(|i| bits.has(i)))
        }
        // O(1) in place; a bit set that already holds the item is given back unchanged, with no
        // copy. Outside the range it aborts, and writes nothing.
        BitSetOp::With => {
            let at = position(argument.as_ref(), bits.size).map_err(|()| bad())?;
            let i = at.ok_or_else(outside)?;
            if !bits.has(i) {
                Rc::make_mut(&mut bits).insert(i);
            }
            Value::BitSet(bits)
        }
        // O(1) in place; an item outside the range, or not a member, changes nothing.
        BitSetOp::Without => {
            let at = position(argument.as_ref(), bits.size).map_err(|()| bad())?;
            if let Some(i) = at
                && bits.has(i)
            {
                Rc::make_mut(&mut bits).delete(i);
            }
            Value::BitSet(bits)
        }
        BitSetOp::Union | BitSetOp::Intersection => {
            let other = other()?;
            if Rc::ptr_eq(&bits, &other) {
                Value::BitSet(bits)
            } else {
                let words = match op {
                    BitSetOp::Union => Words::Union,
                    _ => Words::Intersection,
                };
                Value::BitSet(combine(bits, &other, words))
            }
        }
        BitSetOp::Difference => {
            let other = other()?;
            Value::BitSet(combine(bits, &other, Words::Difference))
        }
        // O(1) when the receiver has more members, else O(size / 64).
        BitSetOp::IsSubset => {
            let other = other()?;
            Value::Bool(
                bits.count <= other.count
                    && bits
                        .words
                        .iter()
                        .zip(&other.words)
                        .all(|(word, theirs)| word & !theirs == 0),
            )
        }
        BitSetOp::First => bits.first().map_or_else(none, |i| some(int(i))),
        BitSetOp::Last => bits.last().map_or_else(none, |i| some(int(i))),
        BitSetOp::Items => Value::List(Rc::new(bits.members().into_iter().map(int).collect())),
    })
}

impl<'p> Machine<'p, '_> {
    /// `empty_bit_set(size: n)`: `n` bits, none set, after the memory budget admits their words,
    /// before any is allocated: a size beyond the budget, or beyond what this machine can
    /// index, aborts with U6003 (ADR 0119 rule 14). O(n / 64).
    pub(super) fn empty_bit_set(&self, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let over = || {
            Stop::Abort(Abort::Memory {
                span,
                limit: self.max_memory,
            })
        };
        let size = match args.first() {
            Some(Value::Int(n)) if n.sign() == Sign::Minus => {
                return Err(precondition(
                    "`empty_bit_set` with a negative size, which its precondition excludes",
                    span,
                ));
            }
            Some(Value::Int(n)) => usize::try_from(n)
                .ok()
                .filter(|n| isize::try_from(*n).is_ok())
                .ok_or_else(over)?,
            _ => return Err(fault("`empty_bit_set` without an `Int` size")),
        };
        let n = size.div_ceil(WORD);
        self.reserve(n, std::mem::size_of::<u64>(), span)?;
        let mut words = Vec::new();
        words.try_reserve_exact(n).map_err(|_| over())?;
        words.resize(n, 0);
        Ok(Value::BitSet(Rc::new(Bits {
            words,
            size,
            count: 0,
        })))
    }

    /// `xs.to_stack()`, `xs.to_queue()` and `xs.to_set()` (`spec/28 §10`, ADR 0119 rule 16): a
    /// stack with the last element on top and a queue with the first in front, in O(n), and a
    /// set of the elements, each once, in O(n log n) comparisons, O(n) when `xs` is sorted.
    pub(super) fn conversion(name: &str, args: &[Value<'p>]) -> Flow<'p> {
        let Some(Value::List(xs)) = args.first() else {
            return Err(fault(format!("`{name}` without a list")));
        };
        Ok(match name {
            // A stack's top is the front of its sequence.
            "to_stack" => Value::Seq(Rc::new(xs.iter().rev().cloned().collect::<VecDeque<_>>())),
            "to_queue" => Value::Seq(Rc::new(xs.iter().cloned().collect::<VecDeque<_>>())),
            // Rust's B-tree sorts its input with a stable sort, linear on sorted input, and
            // builds the tree in bulk; equal items are equal values, so either one is kept.
            _ => {
                let mut items = Vec::with_capacity(xs.len());
                for x in xs.iter() {
                    let key = Key::of(x).ok_or_else(|| {
                        fault("`to_set` of an item that is not of an ordered type")
                    })?;
                    items.push((key, x.clone()));
                }
                Value::Set(Rc::new(items.into_iter().collect::<Items<'p>>()))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;

    /// The value of a run that does not stop.
    fn ok(flow: Flow<'_>) -> Value<'_> {
        match flow {
            Ok(value) => value,
            Err(_) => panic!("the operation stopped"),
        }
    }

    fn bits(size: usize, members: &[usize]) -> Value<'static> {
        let mut out = Bits {
            words: vec![0; size.div_ceil(WORD)],
            size,
            count: 0,
        };
        for i in members {
            out.insert(*i);
        }
        Value::BitSet(Rc::new(out))
    }

    fn listed(value: &Value<'_>) -> Vec<usize> {
        match value {
            Value::BitSet(bits) => bits.members(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn the_word_operations_agree_with_a_set_of_integers() {
        let size = 200;
        let a: Vec<usize> = (0..size).step_by(3).collect();
        let b: Vec<usize> = (0..size)
            .step_by(5)
            .chain([63, 64, 127, 128, 199])
            .collect();
        let sa: std::collections::BTreeSet<usize> = a.iter().copied().collect();
        let sb: std::collections::BTreeSet<usize> = b.iter().copied().collect();
        let apply = |op| {
            ok(bit_set_apply(
                op,
                vec![bits(size, &a), bits(size, &b)],
                None,
            ))
        };
        let union = apply(BitSetOp::Union);
        assert_eq!(listed(&union), (&sa | &sb).into_iter().collect::<Vec<_>>());
        let both = apply(BitSetOp::Intersection);
        assert_eq!(listed(&both), (&sa & &sb).into_iter().collect::<Vec<_>>());
        let only = apply(BitSetOp::Difference);
        assert_eq!(listed(&only), (&sa - &sb).into_iter().collect::<Vec<_>>());
        let Value::BitSet(union) = union else {
            panic!("not a bit set");
        };
        assert_eq!(union.count, (&sa | &sb).len());
        assert_eq!(union.first(), Some(0));
        assert_eq!(union.last(), Some(199));
    }

    #[test]
    fn a_failed_precondition_aborts_and_writes_nothing() {
        let held = bits(64, &[1]);
        for item in [-1, 64, 1000] {
            let result = bit_set_apply(BitSetOp::With, vec![held.clone(), Value::int(item)], None);
            assert!(matches!(
                result,
                Err(Stop::Abort(Abort::Fault { ref message, .. })) if message == OUTSIDE
            ));
        }
        let result = bit_set_apply(BitSetOp::Union, vec![held.clone(), bits(65, &[])], None);
        assert!(matches!(result, Err(Stop::Abort(Abort::Fault { .. }))));
        assert_eq!(listed(&held), [1]);
        // `remove`, `without` and `has` outside the range find nothing.
        let kept = bit_set_apply(BitSetOp::Without, vec![held.clone(), Value::int(-5)], None);
        assert_eq!(listed(&ok(kept)), [1]);
        let found = ok(bit_set_apply(
            BitSetOp::Has,
            vec![held, Value::int(64)],
            None,
        ));
        assert!(matches!(found, Value::Bool(false)));
    }

    #[test]
    fn a_shared_receiver_is_never_changed() {
        let held = bits(130, &[0, 129]);
        let Value::BitSet(shared) = &held else {
            panic!("not a bit set");
        };
        let kept = Rc::clone(shared);
        let grown = bit_set_apply(BitSetOp::With, vec![held.clone(), Value::int(64)], None);
        assert_eq!(listed(&ok(grown)), [0, 64, 129]);
        assert_eq!(kept.members(), [0, 129]);
        assert_eq!(kept.count, 2);
    }
}
