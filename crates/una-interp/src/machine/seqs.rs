//! `Stack` and `Queue` of `std.collections` (`spec/28 §5`, ADR 0119 rules 5, 6 and 19): one
//! ring buffer, front first, whose front is a stack's top.
//!
//! A sequence is a value. A push or a removal updates the receiver in place when the machine
//! holds the only reference to it, a binding given up at its last use, and copies it first
//! otherwise (`spec/16 §4` rule 7). Every operation is O(1), amortized for the updates, except
//! `items`, O(n). The buffer shrinks by half when fewer than a quarter of its slots are used, so
//! it holds memory for its current length, not for the largest it ever had (ADR 0119 rule 1).

use std::collections::VecDeque;
use std::rc::Rc;

use una_core::SeqOp;

use super::{Flow, fault, none, some};
use crate::value::Value;

/// The capacity below which a buffer is never shrunk.
const SMALLEST: usize = 8;

/// A sequence's operation over its arguments, the sequence first.
pub(super) fn seq_apply<'p>(op: SeqOp, args: Vec<Value<'p>>) -> Flow<'p> {
    let bad = || {
        fault(format!(
            "sequence `{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let mut args = args.into_iter();
    let Some(Value::Seq(mut items)) = args.next() else {
        return Err(bad());
    };
    Ok(match op {
        SeqOp::PushFront => {
            let item = args.next().ok_or_else(bad)?;
            Rc::make_mut(&mut items).push_front(item);
            Value::Seq(items)
        }
        SeqOp::PushBack => {
            let item = args.next().ok_or_else(bad)?;
            Rc::make_mut(&mut items).push_back(item);
            Value::Seq(items)
        }
        // An empty sequence is given back unchanged, with no copy.
        SeqOp::PopFront => {
            if !items.is_empty() {
                let held = Rc::make_mut(&mut items);
                held.pop_front();
                shrink(held);
            }
            Value::Seq(items)
        }
        SeqOp::PopBack => {
            if !items.is_empty() {
                let held = Rc::make_mut(&mut items);
                held.pop_back();
                shrink(held);
            }
            Value::Seq(items)
        }
        SeqOp::PeekFront => items.front().map_or_else(none, |item| some(item.clone())),
        SeqOp::PeekBack => items.back().map_or_else(none, |item| some(item.clone())),
        SeqOp::Items => Value::List(Rc::new(items.iter().cloned().collect())),
    })
}

/// Halves the capacity of `items` when fewer than a quarter of its slots are used, down to
/// [`SMALLEST`]: O(n) for the n items moved, after at least n removals since the buffer last
/// changed size, so O(1) amortized per removal.
fn shrink(items: &mut VecDeque<Value<'_>>) {
    let capacity = items.capacity();
    if capacity > SMALLEST && items.len() < capacity / 4 {
        items.shrink_to((capacity / 2).max(SMALLEST));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    /// The value of an operation that must succeed: `Stop` has no `Debug`, so no `unwrap`.
    fn ok<'p>(result: Flow<'p>) -> Value<'p> {
        match result {
            Ok(value) => value,
            Err(_) => panic!("a sequence operation stopped"),
        }
    }

    fn seq(items: &[i64]) -> Value<'static> {
        Value::Seq(Rc::new(items.iter().map(|n| Value::int(*n)).collect()))
    }

    fn ints(value: &Value<'_>) -> Vec<i64> {
        let Value::Seq(items) = value else {
            panic!("not a sequence: {value:?}");
        };
        items
            .iter()
            .map(|item| match item {
                Value::Int(n) => i64::try_from(n.clone()).unwrap(),
                other => panic!("not an integer: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn pushes_and_removals_work_at_both_ends() {
        let pushed = ok(seq_apply(
            SeqOp::PushBack,
            vec![seq(&[1, 2]), Value::int(3)],
        ));
        assert_eq!(ints(&pushed), [1, 2, 3]);
        let pushed = ok(seq_apply(SeqOp::PushFront, vec![pushed, Value::int(0)]));
        assert_eq!(ints(&pushed), [0, 1, 2, 3]);
        let popped = ok(seq_apply(SeqOp::PopFront, vec![pushed]));
        let popped = ok(seq_apply(SeqOp::PopBack, vec![popped]));
        assert_eq!(ints(&popped), [1, 2]);
        let empty = ok(seq_apply(SeqOp::PopFront, vec![seq(&[])]));
        assert!(ints(&empty).is_empty());
    }

    #[test]
    fn a_shared_sequence_is_copied_before_an_update() {
        let original = seq(&[1, 2]);
        let pushed = ok(seq_apply(
            SeqOp::PushBack,
            vec![original.clone(), Value::int(3)],
        ));
        assert_eq!(ints(&original), [1, 2]);
        assert_eq!(ints(&pushed), [1, 2, 3]);
    }

    /// n pushes then n removals at growing sizes: the capacity left is bounded, so memory
    /// follows the length (ADR 0119 rule 1), and each operation stays O(1) amortized: the
    /// buffer is never larger than four times its length plus the smallest size.
    #[test]
    fn a_drained_queue_gives_its_memory_back_at_every_size() {
        for n in [1_000_usize, 10_000, 100_000] {
            let mut queue = seq(&[]);
            for i in 0..n {
                queue = ok(seq_apply(
                    SeqOp::PushBack,
                    vec![queue, Value::int(i as i64)],
                ));
            }
            for _ in 0..n {
                queue = ok(seq_apply(SeqOp::PopFront, vec![queue]));
                let Value::Seq(items) = &queue else {
                    panic!("not a sequence");
                };
                assert!(
                    items.capacity() <= 4 * items.len() + 2 * SMALLEST,
                    "n = {n}: capacity {} for length {}",
                    items.capacity(),
                    items.len()
                );
            }
            let Value::Seq(items) = &queue else {
                panic!("not a sequence");
            };
            assert!(items.is_empty());
            assert!(
                items.capacity() <= 2 * SMALLEST,
                "n = {n}: {}",
                items.capacity()
            );
        }
    }
}
