//! The nodes of the checker's types (ADR 0077 stage 2). This module is the only one that
//! holds an `Rc` of a type node, and it gives no mutable access to one: `tests/shared_types.rs`
//! checks that it has no `&mut` and calls no `Rc` function but the ones that make, clone or
//! compare a handle.

use std::fmt;
use std::rc::Rc;

use super::ty::Ty;

/// A node of a type, shared by every type that holds it (ADR 0077 stage 2): cloning a handle
/// costs O(1) and allocates nothing. A node is immutable once made: this type has no mutable
/// access to what it holds (no `DerefMut`, no `get_mut`, no access to the `Rc`), so no code can
/// change a node that another type shares. To change a type, build a new node; its unchanged
/// children stay shared. Equality is by content, never by address.
#[derive(PartialEq, Eq)]
pub(crate) struct Shared<T: ?Sized>(Rc<T>);

impl<T> Shared<T> {
    /// A new node holding `value`.
    pub(crate) fn new(value: T) -> Shared<T> {
        una_core::label_counters::node();
        Shared(Rc::new(value))
    }
}

impl<T: ?Sized> Shared<T> {
    /// Whether `a` and `b` are the same node. Only ever a fast path to "the same value": two
    /// different nodes may hold equal values, so `false` says nothing about equality.
    pub(crate) fn ptr_eq(a: &Shared<T>, b: &Shared<T>) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }
}

impl<T: ?Sized> Clone for Shared<T> {
    fn clone(&self) -> Shared<T> {
        Shared(Rc::clone(&self.0))
    }
}

impl<T: ?Sized> std::ops::Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Shared<T> {
    /// As the value it holds.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: ?Sized> AsRef<T> for Shared<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<'a> IntoIterator for &'a Shared<[Ty]> {
    type Item = &'a Ty;
    type IntoIter = std::slice::Iter<'a, Ty>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Vec<Ty>> for Shared<[Ty]> {
    /// The type arguments of a kind, as one node.
    fn from(args: Vec<Ty>) -> Shared<[Ty]> {
        una_core::label_counters::node();
        Shared(Rc::from(args))
    }
}

impl FromIterator<Ty> for Shared<[Ty]> {
    /// One allocation when the iterator knows its length, as a map over a slice does.
    fn from_iter<I: IntoIterator<Item = Ty>>(args: I) -> Shared<[Ty]> {
        una_core::label_counters::node();
        Shared(args.into_iter().collect())
    }
}
