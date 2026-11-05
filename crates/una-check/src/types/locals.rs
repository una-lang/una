//! The local names in scope while a body is checked, innermost last, with an index from each
//! name to the positions that bind it, so a name resolves in expected O(1) whatever the number
//! of locals (#326). The index is only looked up, never iterated, so its order is never seen.

use std::collections::{BTreeMap, HashMap};

use una_syntax::Span;

use super::check::Local;

/// The locals in scope, in binding order, and where each name is bound.
#[derive(Clone, Debug, Default)]
pub(crate) struct Locals<'a> {
    items: Vec<Local<'a>>,
    /// For each name, the positions in `items` that bind it, innermost last.
    slots: HashMap<&'a str, Vec<usize>>,
    /// The positions of the locals passed by `take`, and where: the `taken` of each local,
    /// kept sparse, so the states saved around branches cost the taken locals only.
    taken: BTreeMap<usize, Span>,
}

/// The taken state of the locals at a point (`spec/17 §3` rule 3): how many locals there were,
/// and which of them were taken, and where.
#[derive(Clone, Debug, Default)]
pub(crate) struct TakenState {
    len: usize,
    taken: BTreeMap<usize, Span>,
}

impl TakenState {
    /// The taken locals among the first `len`, with where.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (usize, Span)> + '_ {
        self.taken.iter().map(|(&index, &span)| (index, span))
    }

    /// Where the local at `index` was taken, if it was.
    pub(crate) fn get(&self, index: usize) -> Option<Span> {
        self.taken.get(&index).copied()
    }
}

impl<'a> Locals<'a> {
    /// Binds `local` innermost.
    pub(crate) fn push(&mut self, local: Local<'a>) {
        let index = self.items.len();
        self.slots.entry(local.name).or_default().push(index);
        if let Some(span) = local.taken {
            self.taken.insert(index, span);
        }
        self.items.push(local);
    }

    /// Binds each of `locals` innermost, in order.
    pub(crate) fn extend(&mut self, locals: impl IntoIterator<Item = Local<'a>>) {
        for local in locals {
            self.push(local);
        }
    }

    /// Drops the locals from position `len` on, as a scope ends.
    pub(crate) fn truncate(&mut self, len: usize) {
        while self.items.len() > len {
            self.pop();
        }
    }

    /// Removes the locals from position `start` on, and returns them in order.
    pub(crate) fn drain_from(&mut self, start: usize) -> Vec<Local<'a>> {
        let mut drained = Vec::new();
        while self.items.len() > start {
            if let Some(local) = self.pop() {
                drained.push(local);
            }
        }
        drained.reverse();
        drained
    }

    fn pop(&mut self) -> Option<Local<'a>> {
        let local = self.items.pop()?;
        self.taken.remove(&self.items.len());
        if let Some(positions) = self.slots.get_mut(local.name) {
            positions.pop();
            if positions.is_empty() {
                self.slots.remove(local.name);
            }
        }
        Some(local)
    }

    /// The position of the innermost local named `name`.
    pub(crate) fn position(&self, name: &str) -> Option<usize> {
        self.slots
            .get(name)
            .and_then(|positions| positions.last().copied())
    }

    /// The innermost local named `name`.
    pub(crate) fn find(&self, name: &str) -> Option<&Local<'a>> {
        self.position(name).and_then(|index| self.items.get(index))
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&Local<'a>> {
        self.items.get(index)
    }

    /// Sets where the local at `index` was taken, or that it is not (`spec/17 §3` rule 3).
    pub(crate) fn set_taken(&mut self, index: usize, taken: Option<Span>) {
        let Some(local) = self.items.get_mut(index) else {
            return;
        };
        local.taken = taken;
        match taken {
            Some(span) => self.taken.insert(index, span),
            None => self.taken.remove(&index),
        };
    }

    /// The taken state now: O(taken locals).
    pub(crate) fn taken_state(&self) -> TakenState {
        TakenState {
            len: self.items.len(),
            taken: self.taken.clone(),
        }
    }

    /// Restores the taken state of the locals that `state` covers.
    pub(crate) fn restore_taken(&mut self, state: &TakenState) {
        let covered: Vec<usize> = self
            .taken
            .keys()
            .copied()
            .filter(|&index| index < state.len)
            .chain(state.taken.keys().copied())
            .collect();
        for index in covered {
            self.set_taken(index, state.get(index));
        }
    }

    /// Adds to `merged` every local of its own that the current state takes.
    pub(crate) fn merge_taken(&self, merged: &mut TakenState) {
        for (&index, &span) in &self.taken {
            if index < merged.len {
                merged.taken.entry(index).or_insert(span);
            }
        }
    }

    /// The locals taken now, with where.
    pub(crate) fn taken_now(&self) -> impl Iterator<Item = (usize, Span)> + '_ {
        self.taken.iter().map(|(&index, &span)| (index, span))
    }

    pub(crate) fn last_mut(&mut self) -> Option<&mut Local<'a>> {
        self.items.last_mut()
    }
}

impl<'a> IntoIterator for Locals<'a> {
    type Item = Local<'a>;
    type IntoIter = std::vec::IntoIter<Local<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'l, 'a> IntoIterator for &'l Locals<'a> {
    type Item = &'l Local<'a>;
    type IntoIter = std::slice::Iter<'l, Local<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ty::Ty;

    fn local(name: &str) -> Local<'_> {
        Local {
            name,
            ty: Ty::Int,
            editable: false,
            taken: None,
            lambda_depth: 0,
            refined: false,
            loop_of: None,
        }
    }

    #[test]
    fn the_innermost_binding_of_a_name_wins_and_scopes_restore_the_outer_one() {
        let mut locals = Locals::default();
        locals.push(local("x"));
        locals.push(local("y"));
        let mark = locals.len();
        locals.push(local("x"));
        assert_eq!(locals.position("x"), Some(2));
        locals.truncate(mark);
        assert_eq!(locals.position("x"), Some(0));
        let drained = locals.drain_from(1);
        assert_eq!(drained.len(), 1);
        assert_eq!(locals.position("y"), None);
        locals.extend(drained);
        assert_eq!(locals.position("y"), Some(1));
    }
}
