//! The nodes of the checker's types (ADR 0077 stage 2). This module is the only one that
//! holds an `Rc` of a type node, and it gives no mutable access to one: `tests/shared_types.rs`
//! checks that it has no `&mut` and calls no `Rc` function but the ones that make, clone or
//! compare a handle.

use std::fmt;
use std::rc::Rc;

use super::ty::{Refinement, Ty};

/// What a type node holds, anywhere below it, computed once when the node is made from the
/// flags of its children (ADR 0077 stage 3a). A node never changes, so neither do its flags:
/// the operations that can only change what a flag says is absent return the node as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Flags {
    /// A unification variable, `Ty::Var`.
    pub(crate) var: bool,
    /// A labeled position, `Ty::Labeled`.
    pub(crate) label: bool,
    /// A label that names a variable's label, `?n`.
    pub(crate) var_label: bool,
    /// Every labeled position in the form `Ty::with_label` builds: its label is not `public`,
    /// and what it labels is not labeled, `Error` or `Never`.
    pub(crate) normal: bool,
    /// A unit of measure, `Ty::Measured` (ADR 0104), so that a type without one is never
    /// rebuilt to remove it.
    pub(crate) unit: bool,
    /// A refinement at a nested position, `Ty::Refined` (ADR 0103). A variable's solution
    /// may hold one that this flag does not see.
    pub(crate) refined: bool,
}

impl Flags {
    /// A leaf's flags: nothing below it.
    pub(crate) const LEAF: Flags = Flags {
        var: false,
        label: false,
        var_label: false,
        normal: true,
        unit: false,
        refined: false,
    };

    /// What `self` or `other` holds.
    pub(crate) fn and(self, other: Flags) -> Flags {
        Flags {
            var: self.var || other.var,
            label: self.label || other.label,
            var_label: self.var_label || other.var_label,
            normal: self.normal && other.normal,
            unit: self.unit || other.unit,
            refined: self.refined || other.refined,
        }
    }
}

/// A value whose flags follow from those of its immediate children, in O(its arity).
pub(crate) trait Flagged {
    /// The flags of the value.
    fn flags(&self) -> Flags;
}

/// A node of a type, shared by every type that holds it (ADR 0077 stage 2): cloning a handle
/// costs O(1) and allocates nothing. A node is immutable once made: this type has no mutable
/// access to what it holds (no `DerefMut`, no `get_mut`, no access to the `Rc`), so no code can
/// change a node that another type shares. To change a type, build a new node; its unchanged
/// children stay shared. Equality is by content, never by address. The handle carries the
/// node's flags, computed when it is made.
pub(crate) struct Shared<T: ?Sized> {
    flags: Flags,
    node: Rc<T>,
}

impl<T: Flagged> Shared<T> {
    /// A new node holding `value`.
    pub(crate) fn new(value: T) -> Shared<T> {
        una_core::label_counters::node();
        Shared {
            flags: value.flags(),
            node: Rc::new(value),
        }
    }
}

impl<T: ?Sized> Shared<T> {
    /// Whether `a` and `b` are the same node. Only ever a fast path to "the same value": two
    /// different nodes may hold equal values, so `false` says nothing about equality.
    pub(crate) fn ptr_eq(a: &Shared<T>, b: &Shared<T>) -> bool {
        Rc::ptr_eq(&a.node, &b.node)
    }

    /// The flags of the node.
    pub(crate) fn flags(&self) -> Flags {
        self.flags
    }

    /// The node's address, for a memo that holds the handle as long as it holds the address
    /// (ADR 0077 memo rule 1): never compared to decide anything about the value.
    pub(crate) fn address(&self) -> *const () {
        Rc::as_ptr(&self.node).cast::<()>()
    }
}

impl<T: ?Sized + PartialEq> PartialEq for Shared<T> {
    /// By content: the flags follow from it.
    fn eq(&self, other: &Shared<T>) -> bool {
        *self.node == *other.node
    }
}

impl<T: ?Sized + Eq> Eq for Shared<T> {}

impl<T: ?Sized> Clone for Shared<T> {
    fn clone(&self) -> Shared<T> {
        Shared {
            flags: self.flags,
            node: Rc::clone(&self.node),
        }
    }
}

impl<T: ?Sized> std::ops::Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.node
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Shared<T> {
    /// As the value it holds.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

impl<T: ?Sized> AsRef<T> for Shared<T> {
    fn as_ref(&self) -> &T {
        &self.node
    }
}

impl<'a> IntoIterator for &'a Shared<[Ty]> {
    type Item = &'a Ty;
    type IntoIter = std::slice::Iter<'a, Ty>;

    fn into_iter(self) -> Self::IntoIter {
        self.node.iter()
    }
}

impl From<Vec<Ty>> for Shared<[Ty]> {
    /// The type arguments of a kind, as one node.
    fn from(args: Vec<Ty>) -> Shared<[Ty]> {
        una_core::label_counters::node();
        Shared {
            flags: args.as_slice().flags(),
            node: Rc::from(args),
        }
    }
}

impl FromIterator<Ty> for Shared<[Ty]> {
    /// One allocation when the iterator knows its length, as a map over a slice does.
    fn from_iter<I: IntoIterator<Item = Ty>>(args: I) -> Shared<[Ty]> {
        una_core::label_counters::node();
        let node: Rc<[Ty]> = args.into_iter().collect();
        Shared {
            flags: node.flags(),
            node,
        }
    }
}

impl FromIterator<Refinement> for Shared<[Refinement]> {
    /// The refinements at a nested position (ADR 0103), as one node. As a function type's
    /// refinements, they hold no type that an operation changes, so the node's flags are a
    /// leaf's.
    fn from_iter<I: IntoIterator<Item = Refinement>>(refinements: I) -> Shared<[Refinement]> {
        una_core::label_counters::node();
        Shared {
            flags: Flags::LEAF,
            node: refinements.into_iter().collect(),
        }
    }
}
