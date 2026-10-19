//! Runtime values (`spec/11 §2`).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use num_bigint::BigInt;
use una_core::{Ctor, Expr, Name};

use crate::machine::{Caps, Env, Instance};
pub(crate) use una_core::Text;

/// A value of the Una Abstract Machine; `'p` is the program's lifetime.
#[derive(Clone, Debug)]
pub(crate) enum Value<'p> {
    /// An arbitrary-precision integer (ADR 0004).
    Int(BigInt),
    /// A boolean.
    Bool(bool),
    /// A text.
    Text(Rc<Text>),
    /// `unit`.
    Unit,
    /// A constructor applied to its fields.
    Data(Rc<Data<'p>>),
    /// A list.
    List(Rc<Vec<Value<'p>>>),
    /// A closure `⟨λ, ρ⟩`.
    Closure(Rc<Closure<'p>>),
    /// A keyed hash.
    Hashed(Rc<[u8]>),
    /// A tuple, internal to the desugaring of the memory model (`spec/17 §4`).
    Tuple(Rc<Vec<Value<'p>>>),
    /// A map, in the order of its keys (`spec/21 §5`): each entry keeps its key as a value,
    /// for listing and rendering.
    Map(Rc<Entries<'p>>),
}

/// The entries of a map, by the order of their keys.
pub(crate) type Entries<'p> = BTreeMap<Key, (Value<'p>, Value<'p>)>;

/// A map's key, in the order of `spec/03 §1` and `spec/20 §5`: an integer, a text by its
/// scalar values, or a variant without fields by its declaration. A map holds keys of one
/// type, so the order across these never applies.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Int(BigInt),
    Text(Rc<Text>),
    Variant(usize),
}

impl Key {
    /// The key of a value of an `Ord` type, `None` for any other value.
    pub(crate) fn of(value: &Value<'_>) -> Option<Key> {
        match value {
            Value::Int(n) => Some(Key::Int(n.clone())),
            Value::Text(t) => Some(Key::Text(t.clone())),
            Value::Data(data) if data.fields.is_empty() => Some(Key::Variant(data.ctor.index)),
            _ => None,
        }
    }
}

/// A constructor and its fields.
#[derive(Clone, Debug)]
pub(crate) struct Data<'p> {
    /// The constructor.
    pub(crate) ctor: Ctor,
    /// The fields, in order.
    pub(crate) fields: Vec<Value<'p>>,
}

// A value can be deeper than the call depth bounds, since a loop builds it without calls
// (`spec/17 §4`), so dropping and comparing values never recurse on the native stack, which
// would overflow (ADR 0032).
impl Drop for Data<'_> {
    fn drop(&mut self) {
        unwind(std::mem::take(&mut self.fields), Vec::new());
    }
}

// A function value holds the values it closes over and the handlers of its capabilities, whose
// arguments are values too, so a loop nests values through function values as deep as through
// records; they drop by the same worklist (S-30).
impl Drop for Closure<'_> {
    fn drop(&mut self) {
        unwind(self.env.take_values(), vec![std::mem::take(&mut self.caps)]);
    }
}

/// Drops `pending` and the capabilities in `caps`, with everything only they hold, by a worklist
/// instead of recursion. A value or a capability held elsewhere too is only released.
fn unwind<'p>(mut pending: Vec<Value<'p>>, mut caps: Vec<Caps<'p>>) {
    loop {
        if let Some(value) = pending.pop() {
            match value {
                Value::Data(data) => {
                    if let Ok(mut data) = Rc::try_unwrap(data) {
                        pending.append(&mut data.fields);
                    }
                }
                Value::List(items) | Value::Tuple(items) => {
                    if let Ok(items) = Rc::try_unwrap(items) {
                        pending.extend(items);
                    }
                }
                Value::Map(entries) => {
                    if let Ok(entries) = Rc::try_unwrap(entries) {
                        for (_, (key, value)) in entries {
                            pending.push(key);
                            pending.push(value);
                        }
                    }
                }
                Value::Closure(closure) => {
                    if let Ok(mut closure) = Rc::try_unwrap(closure) {
                        pending.extend(closure.env.take_values());
                        caps.push(std::mem::take(&mut closure.caps));
                    }
                }
                _ => {}
            }
        } else if let Some(held) = caps.pop() {
            if let Ok(held) = Rc::try_unwrap(held) {
                for (_, instance) in held {
                    if let Ok(Instance::Handled {
                        values,
                        caps: inner,
                        ..
                    }) = Rc::try_unwrap(instance)
                    {
                        pending.extend(values);
                        caps.push(inner);
                    }
                }
            }
        } else {
            return;
        }
    }
}

/// A lambda with the environment it closes over.
#[derive(Clone, Debug)]
pub(crate) struct Closure<'p> {
    /// The parameters.
    pub(crate) params: Vec<&'p Name>,
    /// The body.
    pub(crate) body: &'p Expr,
    /// The environment at the lambda.
    pub(crate) env: Env<'p>,
    /// The capabilities in scope at the lambda.
    pub(crate) caps: Caps<'p>,
}

impl<'p> Value<'p> {
    /// A constructor value.
    pub(crate) fn data(ctor: Ctor, fields: Vec<Value<'p>>) -> Value<'p> {
        Value::Data(Rc::new(Data { ctor, fields }))
    }

    /// A text value.
    pub(crate) fn text(text: &str) -> Value<'p> {
        Value::Text(Rc::new(Text::new(text.to_owned())))
    }

    /// An integer value.
    pub(crate) fn int(value: i64) -> Value<'p> {
        Value::Int(BigInt::from(value))
    }
}

/// The bytes a structural equality accounts for each pair of composite values it holds, in
/// both engines (ADR 0061, amendment of 2026-10-17, #815): the budget is checked from this fixed
/// size, never from what the allocator holds. It covers the interpreter's peak per pair: a
/// visited entry while its set grows, old and new tables alive, about 86 bytes, and a pending
/// entry, up to 32.
pub(crate) const EQUALITY_PAIR_BYTES: usize = 128;

/// The steps a structural equality takes before it records the pairs it visits, the same in both
/// engines: a small comparison allocates no set, and a larger one expands the pairs of these
/// first steps at most once more, so its time stays O(distinct pairs) (#815).
pub(crate) const EQUALITY_STEPS_UNRECORDED: usize = 1024;

#[cfg(test)]
thread_local! {
    /// The pairs of composite values the equalities of this thread compared, in tests (#815).
    pub(crate) static EQUALITY_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The address of a composite value's allocation and its kind, for a value with parts: what
/// identifies a pair of values a structural equality has visited. A value without parts, a
/// variant without fields or an empty list or map, is a leaf, compared at once, as the native
/// backend, which holds such a variant as an immediate word, compares it (#815).
fn composite(value: &Value<'_>) -> Option<(u8, usize)> {
    match value {
        Value::Data(x) if !x.fields.is_empty() => Some((0, Rc::as_ptr(x) as usize)),
        Value::List(x) if !x.is_empty() => Some((1, Rc::as_ptr(x) as usize)),
        Value::Tuple(x) if !x.is_empty() => Some((2, Rc::as_ptr(x) as usize)),
        Value::Map(x) if !x.is_empty() => Some((3, Rc::as_ptr(x) as usize)),
        _ => None,
    }
}

/// The equality of a pair with a leaf on either side: a leaf equals only a leaf of the same
/// value, and a composite value never equals a leaf.
fn leaf_equal(a: &Value<'_>, b: &Value<'_>) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => Rc::ptr_eq(x, y) || x == y,
        (Value::Unit, Value::Unit) => true,
        (Value::Hashed(x), Value::Hashed(y)) => Rc::ptr_eq(x, y) || x == y,
        (Value::Data(x), Value::Data(y)) => {
            x.ctor.kind == y.ctor.kind
                && x.ctor.index == y.ctor.index
                && x.fields.is_empty()
                && y.fields.is_empty()
        }
        (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
            x.is_empty() && y.is_empty()
        }
        (Value::Map(x), Value::Map(y)) => x.is_empty() && y.is_empty(),
        _ => false,
    }
}

/// A pair met by a structural equality: `true` when it is equal at once (one allocation, or two
/// equal leaves) or when it is a pair of composite values, now pending; `false` when it differs.
fn admit<'v, 'p>(
    a: &'v Value<'p>,
    b: &'v Value<'p>,
    pending: &mut Vec<(&'v Value<'p>, &'v Value<'p>)>,
) -> bool {
    match (composite(a), composite(b)) {
        (Some((kind, x)), Some((other, y))) if kind == other => {
            if x != y {
                pending.push((a, b));
            }
            true
        }
        (Some(_), _) | (_, Some(_)) => false,
        (None, None) => leaf_equal(a, b),
    }
}

/// Structural equality within a budget of `pairs` pairs held at once, visited and pending pairs
/// of composite values together: `None` when it would hold more (U6003 at the comparison, ADR
/// 0061).
///
/// It compares with a worklist, so a deep value does not recurse on the native stack. Leaves are
/// compared as they are met, and two values that are one allocation are equal at once, so
/// `x == x` takes no step. A pair of composite values compared once, past the first
/// [`EQUALITY_STEPS_UNRECORDED`] steps, is not compared again, so a value that shares its
/// parts, such as `x = (y, y)` built over and over, takes a step per distinct pair, not per
/// path: time and space are O(distinct pairs of composite values) (#815).
///
/// Identity implies equality, in the shortcut and in the skip of a visited pair, because no
/// value has a `==` that is not reflexive. A type with one, such as a float under IEEE `==`,
/// where NaN differs from itself, must revisit both (#819).
pub(crate) fn equal_within(a: &Value<'_>, b: &Value<'_>, pairs: usize) -> Option<bool> {
    let mut pending = Vec::new();
    if !admit(a, b, &mut pending) {
        return Some(false);
    }
    let mut visited: HashSet<(u8, usize, usize)> = HashSet::new();
    let mut steps = 0_usize;
    while let Some((a, b)) = pending.pop() {
        #[cfg(test)]
        EQUALITY_STEPS.with(|steps| steps.set(steps.get() + 1));
        // What the comparison holds, its visited pairs and its pending ones, each accounted at
        // `EQUALITY_PAIR_BYTES`: the same count in both engines, which push the same pairs in the
        // same order.
        if visited.len().saturating_add(pending.len()) > pairs {
            return None;
        }
        steps += 1;
        if steps > EQUALITY_STEPS_UNRECORDED
            && let (Some((kind, x)), Some((_, y))) = (composite(a), composite(b))
            // A pair already visited is equal or still pending, its parts pushed: any mismatch
            // anywhere ends the whole comparison with `false`, so skipping it is sound.
            && !visited.insert((kind, x, y))
        {
            continue;
        }
        let same = match (a, b) {
            (Value::Data(x), Value::Data(y)) => {
                x.ctor.kind == y.ctor.kind
                    && x.ctor.index == y.ctor.index
                    && x.fields.len() == y.fields.len()
                    && x.fields
                        .iter()
                        .zip(&y.fields)
                        .all(|(a, b)| admit(a, b, &mut pending))
            }
            (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
                x.len() == y.len()
                    && x.iter()
                        .zip(y.iter())
                        .all(|(a, b)| admit(a, b, &mut pending))
            }
            // The same keys, each with an equal value (`spec/21 §1` rule 5).
            (Value::Map(x), Value::Map(y)) => {
                x.len() == y.len()
                    && x.keys().eq(y.keys())
                    && x.values()
                        .zip(y.values())
                        .all(|((_, a), (_, b))| admit(a, b, &mut pending))
            }
            _ => false,
        };
        if !same {
            return Some(false);
        }
    }
    Some(true)
}

/// The order of `<`: integers, texts by scalar values, and variants without fields by
/// declaration (`spec/03 §1`).
pub(crate) fn compare(a: &Value<'_>, b: &Value<'_>) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Data(x), Value::Data(y)) => x.ctor.index.cmp(&y.ctor.index),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// `x0 = [leaf]`, `x(k+1) = [xk, xk]`: a value of `depth` levels whose every level shares
    /// its one child, 2^depth paths through `depth + 1` distinct lists.
    fn doubled(depth: usize, leaf: i64) -> Value<'static> {
        let mut value = Value::List(Rc::new(vec![Value::int(leaf)]));
        for _ in 0..depth {
            value = Value::List(Rc::new(vec![value.clone(), value]));
        }
        value
    }

    /// The pairs `equal_within` visits to compare `a` and `b`, and its verdict.
    fn steps(a: &Value<'_>, b: &Value<'_>) -> (usize, Option<bool>) {
        let before = EQUALITY_STEPS.with(std::cell::Cell::get);
        let verdict = equal_within(a, b, usize::MAX);
        (EQUALITY_STEPS.with(std::cell::Cell::get) - before, verdict)
    }

    #[test]
    fn a_value_equals_itself_at_once() {
        for depth in [8, 16, 32, 64] {
            let x = doubled(depth, 1);
            assert_eq!(steps(&x, &x), (0, Some(true)), "depth {depth}");
        }
    }

    #[test]
    fn shared_parts_are_compared_once_however_deep() {
        // #815: two values built apart, each sharing its parts, compare in a number of steps
        // linear in their depth past the unrecorded ones, not in their 2^depth paths.
        for depth in [8, 16, 32, 64] {
            let (a, b) = (doubled(depth, 1), doubled(depth, 1));
            let (taken, verdict) = steps(&a, &b);
            assert_eq!(verdict, Some(true), "depth {depth}");
            let bound = EQUALITY_STEPS_UNRECORDED + 3 * (depth + 2);
            assert!(taken <= bound, "depth {depth}: {taken} steps");
        }
    }

    #[test]
    fn a_difference_under_a_shared_part_is_found() {
        // The skip of a visited pair must not hide a difference: one side shares a part, the
        // other holds a near copy whose deepest leaf differs.
        for depth in [1, 8, 16, 32, 64] {
            let a = doubled(depth, 1);
            let b = doubled(depth, 2);
            assert_eq!(steps(&a, &b).1, Some(false), "depth {depth}");
            // One branch the same as `a`'s, the other differing deep down.
            let mixed = Value::List(Rc::new(vec![doubled(depth, 1), doubled(depth, 2)]));
            let shared = Value::List(Rc::new(vec![a.clone(), a.clone()]));
            assert_eq!(steps(&shared, &mixed).1, Some(false), "depth {depth}");
            assert_eq!(steps(&shared, &shared), (0, Some(true)));
        }
    }

    #[test]
    fn leaves_are_compared_at_once_and_never_held() {
        // Two flat lists hold one pair, their own, however long: a budget of one pair compares
        // lists of any length, and a difference in a leaf is found.
        let flat = |n: i64, last: i64| {
            Value::List(Rc::new(
                (0..n)
                    .map(|i| Value::int(if i == n - 1 { last } else { i }))
                    .collect(),
            ))
        };
        let (a, b) = (flat(100_000, 7), flat(100_000, 7));
        assert_eq!(equal_within(&a, &b, 1), Some(true));
        assert_eq!(steps(&a, &b), (1, Some(true)));
        assert_eq!(equal_within(&a, &flat(100_000, 8), 1), Some(false));
        // A leaf never equals a composite value.
        let empty = Value::List(Rc::new(Vec::new()));
        assert_eq!(equal_within(&empty, &flat(1, 0), 1), Some(false));
        assert_eq!(equal_within(&empty, &empty.clone(), 0), Some(true));
    }

    #[test]
    fn a_comparison_past_its_budget_of_pairs_stops() {
        let (a, b) = (doubled(40, 1), doubled(40, 1));
        assert_eq!(equal_within(&a, &b, 10), None);
        assert_eq!(equal_within(&a, &b, 1000), Some(true));
        assert_eq!(equal_within(&a, &a, 0), Some(true));
    }

    #[test]
    fn a_deep_chain_of_function_values_drops_without_native_recursion() {
        // S-30: a function value that closes over the previous one, in a record or not, as a
        // loop builds it, and one whose capability's handler holds it, drop by the worklist on
        // a small stack.
        let dropped = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let mut sources = una_syntax::SourceDb::new();
                let file = sources.add("chain.una", b"");
                let body: &'static Expr = Box::leak(Box::new(Expr::Atom {
                    atom: una_core::Atom::Unit,
                    span: una_syntax::Span::new(file, 0, 0),
                }));
                let closing = |value: Value<'static>, caps: Caps<'static>| {
                    let mut env = Env::new();
                    env.push(("prev", value));
                    Value::Closure(Rc::new(Closure {
                        params: Vec::new(),
                        body,
                        env,
                        caps,
                    }))
                };
                let node = Ctor {
                    kind: una_core::Path {
                        module: "chain".to_owned(),
                        name: "F".to_owned(),
                    },
                    name: "L".to_owned(),
                    index: 1,
                };
                let mut through_records = Value::Unit;
                for _ in 0..100_000 {
                    let closure = closing(through_records, Rc::default());
                    through_records = Value::data(node.clone(), vec![closure]);
                }
                drop(through_records);
                let mut through_closures = Value::Unit;
                for _ in 0..100_000 {
                    through_closures = closing(through_closures, Rc::default());
                }
                drop(through_closures);
                true
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(dropped);
    }
}
