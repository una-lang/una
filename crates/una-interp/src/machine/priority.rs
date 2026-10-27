//! The `PriorityQueue` of `std.collections` (`spec/28 §8`, ADR 0119 rules 13 and 19): a stable
//! 4-ary heap of `(priority, sequence, item)` entries. An entry comes first by its priority, in
//! the queue's direction, and among equal priorities by its sequence number, the count of the
//! queue's pushes before it, so the order of the pops is fixed by the pushes alone, whatever
//! heap holds them.
//!
//! A queue is a value. A push or a removal updates the receiver in place when the machine holds
//! the only reference to it, a binding given up at its last use, and copies it first otherwise
//! (`spec/16 §4` rule 7). `push` and `pop` make O(log n) comparisons, amortized for the buffer's
//! growth and shrinking, the reads are O(1), and `items` sorts a copy of the entries, O(n log n)
//! comparisons. The buffer shrinks by half when fewer than a quarter of its slots are used, so a
//! queue holds memory for its current length, not for the largest it ever had (ADR 0119 rule 1).

use std::cmp::Ordering;
use std::rc::Rc;

use una_core::{Ctor, Path, PriorityOp};

use super::{Flow, fault, none, some};
use crate::value::{Heap, Key, Ranked, Value};

/// The capacity below which a buffer is never shrunk.
const SMALLEST: usize = 8;

/// The children of each node of the heap.
const ARITY: usize = 4;

/// An empty queue of the direction `highest`: `highest_first()` when `true`, `lowest_first()`
/// otherwise. O(1): it allocates no buffer.
pub(super) fn empty<'p>(highest: bool) -> Value<'p> {
    Value::Priority(Rc::new(Heap {
        highest,
        next: 0,
        entries: Vec::new(),
    }))
}

/// A priority queue's operation over its arguments, the queue first.
pub(super) fn priority_apply<'p>(op: PriorityOp, args: Vec<Value<'p>>) -> Flow<'p> {
    let bad = || {
        fault(format!(
            "priority queue `{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let mut args = args.into_iter();
    let Some(Value::Priority(mut heap)) = args.next() else {
        return Err(bad());
    };
    Ok(match op {
        PriorityOp::Push => {
            let item = args.next().ok_or_else(bad)?;
            let priority = args.next().ok_or_else(bad)?;
            let key = Key::of(&priority).ok_or_else(bad)?;
            push(Rc::make_mut(&mut heap), key, priority, item);
            Value::Priority(heap)
        }
        // An empty queue is given back unchanged, with no copy.
        PriorityOp::Pop => {
            if !heap.entries.is_empty() {
                pop(Rc::make_mut(&mut heap));
            }
            Value::Priority(heap)
        }
        PriorityOp::Peek => heap
            .entries
            .first()
            .map_or_else(none, |entry| some(entry.item.clone())),
        PriorityOp::PeekPriority => heap
            .entries
            .first()
            .map_or_else(none, |entry| some(entry.priority.clone())),
        PriorityOp::PeekRanked => heap.entries.first().map_or_else(none, |entry| {
            some(Value::data(
                ranked_ctor(),
                vec![entry.item.clone(), entry.priority.clone()],
            ))
        }),
        PriorityOp::Items => Value::List(Rc::new(
            listed(&heap)
                .into_iter()
                .map(|entry| entry.item.clone())
                .collect(),
        )),
    })
}

/// The record constructor of the kind `Ranked[T, P]` of `std.collections`, whose fields are
/// `item` and `priority`, in that order (ADR 0119 rule 13).
fn ranked_ctor() -> Ctor {
    Ctor {
        kind: Path {
            module: una_core::COLLECTIONS_MODULE.to_owned(),
            name: "Ranked".to_owned(),
        },
        name: "Ranked".to_owned(),
        index: 0,
    }
}

/// The entries of `heap` in its listing order, the order of the pops: a stable sort of
/// references to them by the heap's order, O(n log n) comparisons. Rendering lists a queue
/// this way too.
pub(crate) fn listed<'h, 'p>(heap: &'h Heap<'p>) -> Vec<&'h Ranked<'p>> {
    let mut entries: Vec<&Ranked<'p>> = heap.entries.iter().collect();
    entries.sort_by(|a, b| order(heap.highest, a, b));
    entries
}

/// The order of two entries of a queue of the direction `highest`: by priority, the greatest
/// first when `highest` and the least first otherwise, then by sequence number, ascending.
fn order(highest: bool, a: &Ranked<'_>, b: &Ranked<'_>) -> Ordering {
    let by_priority = if highest {
        b.key.cmp(&a.key)
    } else {
        a.key.cmp(&b.key)
    };
    by_priority.then(a.sequence.cmp(&b.sequence))
}

/// Whether the entry at `i` comes before the one at `j`; `false` when either is missing.
fn before(heap: &Heap<'_>, i: usize, j: usize) -> bool {
    match (heap.entries.get(i), heap.entries.get(j)) {
        (Some(a), Some(b)) => order(heap.highest, a, b) == Ordering::Less,
        _ => false,
    }
}

/// Adds `item` at `priority`, whose order key is `key`, and moves it up to its place: O(log n)
/// comparisons, and O(1) amortized for the buffer's growth.
fn push<'p>(heap: &mut Heap<'p>, key: Key, priority: Value<'p>, item: Value<'p>) {
    if heap.next == u64::MAX {
        renumber(heap);
    }
    let sequence = heap.next;
    heap.next += 1;
    heap.entries.push(Ranked {
        key,
        sequence,
        priority,
        item,
    });
    let mut at = heap.entries.len() - 1;
    while at > 0 {
        let parent = (at - 1) / ARITY;
        if !before(heap, at, parent) {
            break;
        }
        heap.entries.swap(at, parent);
        at = parent;
    }
}

/// Removes the first entry of a nonempty heap: the last entry takes its place and moves down
/// to its own, O(log n) comparisons (4 per level of log₄ n levels), and O(1) amortized for the
/// buffer's shrinking.
fn pop(heap: &mut Heap<'_>) {
    if heap.entries.is_empty() {
        return;
    }
    heap.entries.swap_remove(0);
    let len = heap.entries.len();
    let mut at = 0;
    loop {
        let first_child = at * ARITY + 1;
        if first_child >= len {
            break;
        }
        let mut best = first_child;
        for child in first_child + 1..(first_child + ARITY).min(len) {
            if before(heap, child, best) {
                best = child;
            }
        }
        if !before(heap, best, at) {
            break;
        }
        heap.entries.swap(at, best);
        at = best;
    }
    shrink(&mut heap.entries);
}

/// Renumbers the entries 0, 1, 2, ... in the order of their sequence numbers, when the counter
/// of pushes reaches its last value (ADR 0119 rule 19). The relative order of every two entries
/// is kept, so the heap stays a heap. O(n log n), which no run reaches in practice: it takes
/// 2^64 - 1 pushes to one queue.
fn renumber(heap: &mut Heap<'_>) {
    let mut by_sequence: Vec<(u64, usize)> = heap
        .entries
        .iter()
        .enumerate()
        .map(|(position, entry)| (entry.sequence, position))
        .collect();
    by_sequence.sort_unstable();
    for (rank, (_, position)) in by_sequence.into_iter().enumerate() {
        if let Some(entry) = heap.entries.get_mut(position) {
            entry.sequence = rank as u64;
        }
    }
    heap.next = heap.entries.len() as u64;
}

/// Halves the capacity of `entries` when fewer than a quarter of its slots are used, down to
/// [`SMALLEST`]: O(n) for the n entries moved, after at least n removals since the buffer last
/// changed size, so O(1) amortized per removal.
fn shrink(entries: &mut Vec<Ranked<'_>>) {
    let capacity = entries.capacity();
    if capacity > SMALLEST && entries.len() < capacity / 4 {
        entries.shrink_to((capacity / 2).max(SMALLEST));
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
            Err(_) => panic!("a priority queue operation stopped"),
        }
    }

    fn int(value: &Value<'_>) -> i64 {
        match value {
            Value::Int(n) => i64::try_from(n.clone()).unwrap(),
            other => panic!("not an integer: {other:?}"),
        }
    }

    /// The queue with `(item, priority)` pushed in order.
    fn queue(highest: bool, pushes: &[(i64, i64)]) -> Value<'static> {
        let mut q = empty(highest);
        for (item, priority) in pushes {
            q = ok(priority_apply(
                PriorityOp::Push,
                vec![q, Value::int(*item), Value::int(*priority)],
            ));
        }
        q
    }

    /// The items of `q` in the order of its pops, by popping it empty.
    fn drained(mut q: Value<'_>) -> Vec<i64> {
        let mut out = Vec::new();
        loop {
            let first = ok(priority_apply(PriorityOp::Peek, vec![q.clone()]));
            let Value::Data(data) = &first else {
                panic!("not an option");
            };
            let Some(item) = data.fields.first() else {
                return out;
            };
            out.push(int(item));
            q = ok(priority_apply(PriorityOp::Pop, vec![q]));
        }
    }

    fn items(q: &Value<'_>) -> Vec<i64> {
        let Value::List(items) = ok(priority_apply(PriorityOp::Items, vec![q.clone()])) else {
            panic!("not a list");
        };
        items.iter().map(int).collect()
    }

    #[test]
    fn equal_priorities_pop_in_push_order_in_both_directions() {
        let pushes = [(1, 3), (2, 1), (3, 3), (4, 2), (5, 1), (6, 3)];
        let lowest = queue(false, &pushes);
        assert_eq!(items(&lowest), [2, 5, 4, 1, 3, 6]);
        assert_eq!(drained(lowest), [2, 5, 4, 1, 3, 6]);
        let highest = queue(true, &pushes);
        assert_eq!(items(&highest), [1, 3, 6, 4, 2, 5]);
        assert_eq!(drained(highest), [1, 3, 6, 4, 2, 5]);
    }

    #[test]
    fn an_empty_queue_pops_and_peeks_nothing() {
        let q = empty(false);
        assert!(drained(q.clone()).is_empty());
        let popped = ok(priority_apply(PriorityOp::Pop, vec![q]));
        assert!(items(&popped).is_empty());
    }

    #[test]
    fn a_shared_queue_is_copied_before_an_update() {
        let original = queue(false, &[(1, 2), (2, 1)]);
        let pushed = ok(priority_apply(
            PriorityOp::Push,
            vec![original.clone(), Value::int(3), Value::int(0)],
        ));
        assert_eq!(items(&original), [2, 1]);
        assert_eq!(items(&pushed), [3, 2, 1]);
    }

    /// Many pushes with few distinct priorities, then pops interleaved with pushes: the pops
    /// agree with a stable sort of what was pushed, at every step.
    #[test]
    fn the_heap_agrees_with_a_stable_sort() {
        let mut q = empty(false);
        let mut model: Vec<(i64, i64)> = Vec::new();
        for i in 0..2_000_i64 {
            let priority = (i * 7_919) % 13;
            q = ok(priority_apply(
                PriorityOp::Push,
                vec![q, Value::int(i), Value::int(priority)],
            ));
            model.push((priority, i));
            if i % 3 == 0 {
                model.sort_by_key(|(priority, _)| *priority);
                let (_, expected) = model.remove(0);
                let first = ok(priority_apply(PriorityOp::Peek, vec![q.clone()]));
                let Value::Data(data) = &first else {
                    panic!("not an option");
                };
                assert_eq!(data.fields.first().map(int), Some(expected));
                q = ok(priority_apply(PriorityOp::Pop, vec![q]));
            }
        }
        model.sort_by_key(|(priority, _)| *priority);
        let expected: Vec<i64> = model.into_iter().map(|(_, item)| item).collect();
        assert_eq!(items(&q), expected);
        assert_eq!(drained(q), expected);
    }

    #[test]
    fn renumbering_keeps_the_order_of_ties() {
        let Value::Priority(mut heap) = queue(false, &[(1, 5), (2, 5), (3, 4)]) else {
            panic!("not a queue");
        };
        Rc::make_mut(&mut heap).next = u64::MAX;
        let q = ok(priority_apply(
            PriorityOp::Push,
            vec![Value::Priority(heap), Value::int(4), Value::int(5)],
        ));
        assert_eq!(drained(q), [3, 1, 2, 4]);
    }

    /// n pushes then n pops at growing sizes: the capacity left is bounded, so memory follows
    /// the length (ADR 0119 rule 1).
    #[test]
    fn a_drained_queue_gives_its_memory_back() {
        for n in [1_000_usize, 10_000] {
            let mut q = empty(true);
            for i in 0..n {
                q = ok(priority_apply(
                    PriorityOp::Push,
                    vec![q, Value::int(i as i64), Value::int((i % 17) as i64)],
                ));
            }
            for _ in 0..n {
                q = ok(priority_apply(PriorityOp::Pop, vec![q]));
                let Value::Priority(heap) = &q else {
                    panic!("not a queue");
                };
                assert!(
                    heap.entries.capacity() <= 4 * heap.entries.len() + 2 * SMALLEST,
                    "n = {n}: capacity {} for length {}",
                    heap.entries.capacity(),
                    heap.entries.len()
                );
            }
        }
    }
}
