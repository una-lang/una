//! The `Set` of `std.collections` (`spec/28 §6`, ADR 0119 rules 7 and 19): a B-tree of items in
//! ascending order, each kept as its value for listing and rendering, as a map keeps its keys.
//!
//! A set is a value. An operation that gives a new set updates the receiver in place when the
//! machine holds the only reference to it, a binding given up at its last use, and copies it
//! first otherwise (`spec/16 §4` rule 7). The set operations take the cheaper of a merge of both
//! sets in order, O(n + m), and lookups of the smaller side's items, as `spec/28 §6` states.

// A set's `Key` holds a `Text`, whose scalar index is filled lazily in a `OnceCell`; `Key`'s
// order and equality read only the text's bytes, never that index, so mutating it never moves a
// key in the tree. Clippy cannot see that, hence the allowance.
#![allow(clippy::mutable_key_type)]

use std::cmp::Ordering;
use std::ops::Bound;
use std::rc::Rc;

use una_core::SetOp;

use super::{Flow, fault, none, some};
use crate::value::{Items, Key, Value};

/// A set's built-in method over its arguments, receiver first.
pub(super) fn set_apply<'p>(op: SetOp, args: Vec<Value<'p>>) -> Flow<'p> {
    let bad = || {
        fault(format!(
            "set `{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let mut args = args.into_iter();
    let Some(Value::Set(mut items)) = args.next() else {
        return Err(bad());
    };
    let mut item = || -> Result<(Key, Value<'p>), _> {
        let value = args.next().ok_or_else(bad)?;
        Ok((Key::of(&value).ok_or_else(bad)?, value))
    };
    Ok(match op {
        // O(log n).
        SetOp::Has => Value::Bool(items.contains_key(&item()?.0)),
        // O(log n) in place; a set that already holds the item is given back unchanged, with no
        // copy.
        SetOp::With => {
            let (key, value) = item()?;
            if !items.contains_key(&key) {
                Rc::make_mut(&mut items).insert(key, value);
            }
            Value::Set(items)
        }
        SetOp::Without => {
            let (key, _) = item()?;
            if items.contains_key(&key) {
                Rc::make_mut(&mut items).remove(&key);
            }
            Value::Set(items)
        }
        // O(log n).
        SetOp::First => items
            .first_key_value()
            .map_or_else(none, |(_, v)| some(v.clone())),
        SetOp::Last => items
            .last_key_value()
            .map_or_else(none, |(_, v)| some(v.clone())),
        // O(log n + k) for the k items listed; empty when `low` is above `high`.
        SetOp::Between => {
            let (low, _) = item()?;
            let (high, _) = item()?;
            let listed = if low > high {
                Vec::new()
            } else {
                items
                    .range((Bound::Included(low), Bound::Included(high)))
                    .map(|(_, v)| v.clone())
                    .collect()
            };
            Value::List(Rc::new(listed))
        }
        // O(n), ascending.
        SetOp::Items => Value::List(Rc::new(items.values().cloned().collect())),
        SetOp::Union | SetOp::Intersection | SetOp::Difference | SetOp::IsSubset => {
            let Some(Value::Set(other)) = args.next() else {
                return Err(bad());
            };
            match op {
                SetOp::Union => Value::Set(union(items, &other)),
                SetOp::Intersection => Value::Set(intersection(&items, &other)),
                SetOp::Difference => Value::Set(difference(items, &other)),
                _ => Value::Bool(is_subset(&items, &other)),
            }
        }
    })
}

/// The cost, in comparisons, of `lookups` lookups in a B-tree of `size` items: `lookups` times
/// the bits of `size + 1`, an upper bound of its depth in comparisons.
fn lookup_cost(lookups: usize, size: usize) -> usize {
    let depth = usize::BITS - (size.saturating_add(1)).leading_zeros();
    lookups.saturating_mul(depth as usize)
}

/// A set built from items in ascending order, in O(n): Rust's B-tree sorts its input with a
/// stable sort, linear on sorted input, and builds the tree in bulk.
fn sorted<'p>(items: impl Iterator<Item = (Key, Value<'p>)>) -> Rc<Items<'p>> {
    Rc::new(items.collect())
}

/// `s.union(other: t)`: the receiver's items and the other's, the receiver's value of each item
/// kept. O(m log(n + m)) by inserting t's items when the receiver is held only here and that is
/// cheaper, else a merge in O(n + m).
fn union<'p>(mut s: Rc<Items<'p>>, t: &Rc<Items<'p>>) -> Rc<Items<'p>> {
    if t.is_empty() || Rc::ptr_eq(&s, t) {
        return s;
    }
    if s.is_empty() {
        return Rc::clone(t);
    }
    let (n, m) = (s.len(), t.len());
    if let Some(owned) = Rc::get_mut(&mut s)
        && lookup_cost(m, n + m) < n + m
    {
        for (key, value) in t.iter() {
            owned.entry(key.clone()).or_insert_with(|| value.clone());
        }
        return s;
    }
    let mut out = Vec::with_capacity(n + m);
    let (mut a, mut b) = (s.iter().peekable(), t.iter().peekable());
    loop {
        let next = match (a.peek(), b.peek()) {
            (Some((x, _)), Some((y, _))) => x.cmp(y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => break,
        };
        let taken = match next {
            Ordering::Less => a.next(),
            Ordering::Greater => b.next(),
            Ordering::Equal => {
                b.next();
                a.next()
            }
        };
        if let Some((key, value)) = taken {
            out.push((key.clone(), value.clone()));
        }
    }
    sorted(out.into_iter())
}

/// `s.intersection(other: t)`: the receiver's items that the other holds. O(min(n, m) ·
/// log(n + m)) by looking up the smaller set's items in the larger, or a merge in O(n + m) when
/// that is cheaper.
fn intersection<'p>(s: &Rc<Items<'p>>, t: &Rc<Items<'p>>) -> Rc<Items<'p>> {
    if Rc::ptr_eq(s, t) {
        return Rc::clone(s);
    }
    let (n, m) = (s.len(), t.len());
    if lookup_cost(n.min(m), n.max(m)) < n + m {
        let out: Vec<(Key, Value<'p>)> = if n <= m {
            s.iter()
                .filter(|(key, _)| t.contains_key(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        } else {
            t.keys()
                .filter_map(|key| s.get_key_value(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        };
        return sorted(out.into_iter());
    }
    let mut out = Vec::new();
    let (mut a, mut b) = (s.iter().peekable(), t.iter().peekable());
    while let (Some((x, value)), Some((y, _))) = (a.peek(), b.peek()) {
        match x.cmp(y) {
            Ordering::Less => {
                a.next();
            }
            Ordering::Greater => {
                b.next();
            }
            Ordering::Equal => {
                out.push(((*x).clone(), (*value).clone()));
                a.next();
                b.next();
            }
        }
    }
    sorted(out.into_iter())
}

/// `s.difference(other: t)`: the receiver's items that the other lacks. O(m log n) by removing
/// t's items when the receiver is held only here and that is cheaper, O(n log m) by looking up
/// s's items in t when that is, else a merge in O(n + m).
fn difference<'p>(mut s: Rc<Items<'p>>, t: &Rc<Items<'p>>) -> Rc<Items<'p>> {
    if Rc::ptr_eq(&s, t) {
        return Rc::new(Items::new());
    }
    if s.is_empty() || t.is_empty() {
        return s;
    }
    let (n, m) = (s.len(), t.len());
    if let Some(owned) = Rc::get_mut(&mut s)
        && lookup_cost(m, n) < n + m
    {
        for key in t.keys() {
            owned.remove(key);
        }
        return s;
    }
    if lookup_cost(n, m) < n + m {
        return sorted(
            s.iter()
                .filter(|(key, _)| !t.contains_key(key))
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    let mut out = Vec::new();
    let mut b = t.keys().peekable();
    for (key, value) in s.iter() {
        while b.next_if(|y| *y < key).is_some() {}
        if b.peek() != Some(&key) {
            out.push((key.clone(), value.clone()));
        }
    }
    sorted(out.into_iter())
}

/// `s.is_subset(of: t)`: whether t holds every item of s. O(1) when s is larger, O(n log m) by
/// lookups when that is cheaper, else a merge in O(n + m).
fn is_subset(s: &Items<'_>, t: &Items<'_>) -> bool {
    let (n, m) = (s.len(), t.len());
    if n > m {
        return false;
    }
    if lookup_cost(n, m) < n + m {
        return s.keys().all(|key| t.contains_key(key));
    }
    let mut b = t.keys().peekable();
    s.keys().all(|key| {
        while b.next_if(|y| *y < key).is_some() {}
        b.next_if(|y| *y == key).is_some()
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn set(items: impl IntoIterator<Item = i64>) -> Rc<Items<'static>> {
        Rc::new(
            items
                .into_iter()
                .map(|i| {
                    let v = Value::int(i);
                    (Key::of(&v).unwrap(), v)
                })
                .collect(),
        )
    }

    fn listed(items: &Items<'_>) -> Vec<String> {
        items
            .values()
            .map(|v| match v {
                Value::Int(n) => n.to_string(),
                _ => "?".to_owned(),
            })
            .collect()
    }

    #[test]
    fn every_method_of_the_set_operations_agrees() {
        // Sizes that take each path: lookups, merges, and the in-place update of a held set.
        let shapes: [(Vec<i64>, Vec<i64>); 5] = [
            ((0..1000).collect(), vec![3, 500, 2000]),
            (vec![3, 500, 2000], (0..1000).collect()),
            (
                (0..1000).step_by(2).collect(),
                (0..1000).step_by(3).collect(),
            ),
            (Vec::new(), vec![1, 2]),
            (vec![1, 2], Vec::new()),
        ];
        for (a, b) in shapes {
            let (sa, sb) = (
                a.iter().copied().collect::<std::collections::BTreeSet<_>>(),
                b.iter().copied().collect::<std::collections::BTreeSet<_>>(),
            );
            let text = |s: std::collections::BTreeSet<i64>| -> Vec<String> {
                s.iter().map(ToString::to_string).collect()
            };
            // A shared receiver, then one held only here.
            for held in [false, true] {
                let s = set(a.clone());
                let keep = (!held).then(|| Rc::clone(&s));
                let u = union(Rc::clone(&s), &set(b.clone()));
                drop(keep);
                assert_eq!(listed(&u), text(&sa | &sb));
                let d = difference(set(a.clone()), &set(b.clone()));
                assert_eq!(listed(&d), text(&sa - &sb));
                let i = intersection(&set(a.clone()), &set(b.clone()));
                assert_eq!(listed(&i), text(&sa & &sb));
                assert_eq!(
                    is_subset(&set(a.clone()), &set(b.clone())),
                    sa.is_subset(&sb)
                );
                assert!(is_subset(&i, &set(a.clone())));
            }
        }
    }

    #[test]
    fn a_shared_receiver_is_never_changed() {
        let s = set([1, 2, 3]);
        let kept = Rc::clone(&s);
        let u = union(s, &set([4]));
        assert_eq!(listed(&kept), ["1", "2", "3"]);
        assert_eq!(listed(&u), ["1", "2", "3", "4"]);
        let d = difference(Rc::clone(&kept), &set([2]));
        assert_eq!(listed(&kept), ["1", "2", "3"]);
        assert_eq!(listed(&d), ["1", "3"]);
    }
}
