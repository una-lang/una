//! The run-time environment `E` of the interpreter (`spec/11 §2`): the bindings in scope, in the
//! order they were made, with an index from each name to the positions that bind it, so a read
//! takes the latest binding of its name in expected O(1), not a scan of every binding (#329).
//!
//! Shadowing pushes a position onto the name's stack and leaving a scope pops it, so the index
//! always names the binding a scan from the end would find. The index is never iterated, so its
//! order is never observable.

use std::collections::HashMap;

use crate::value::Value;

/// The bindings in scope, in order, and the positions of each name.
#[derive(Clone, Debug, Default)]
pub(crate) struct Env<'p> {
    bindings: Vec<(&'p str, Value<'p>)>,
    positions: HashMap<&'p str, Vec<usize>>,
}

impl<'p> Env<'p> {
    /// An empty environment.
    pub(crate) fn new() -> Env<'p> {
        Env::default()
    }

    /// How many bindings are in scope: a mark that [`Env::truncate`] returns to.
    pub(crate) fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Binds `name` to `value`, shadowing an earlier binding of `name`.
    pub(crate) fn push(&mut self, (name, value): (&'p str, Value<'p>)) {
        self.positions
            .entry(name)
            .or_default()
            .push(self.bindings.len());
        self.bindings.push((name, value));
    }

    /// Leaves the bindings made after the mark `len`: each removal is O(1), so a scope's exit
    /// costs what its bindings cost.
    pub(crate) fn truncate(&mut self, len: usize) {
        while self.bindings.len() > len {
            let Some((name, _)) = self.bindings.pop() else {
                break;
            };
            if let Some(stack) = self.positions.get_mut(name) {
                stack.pop();
                if stack.is_empty() {
                    self.positions.remove(name);
                }
            }
        }
    }

    /// The latest binding of `name`.
    pub(crate) fn lookup(&self, name: &str) -> Option<&Value<'p>> {
        let at = *self.positions.get(name)?.last()?;
        self.bindings.get(at).map(|(_, value)| value)
    }

    /// The latest binding of `name`, to change in place.
    pub(crate) fn lookup_mut(&mut self, name: &str) -> Option<&mut Value<'p>> {
        let at = *self.positions.get(name)?.last()?;
        self.bindings.get_mut(at).map(|(_, value)| value)
    }

    /// The bindings in the order they were made.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &(&'p str, Value<'p>)> {
        self.bindings.iter()
    }

    /// The binding at position `at`.
    pub(crate) fn get(&self, at: usize) -> Option<&(&'p str, Value<'p>)> {
        self.bindings.get(at)
    }

    /// The value of the binding at position `at`, to change in place.
    pub(crate) fn get_mut(&mut self, at: usize) -> Option<&mut Value<'p>> {
        self.bindings.get_mut(at).map(|(_, value)| value)
    }
}

impl<'p> Extend<(&'p str, Value<'p>)> for Env<'p> {
    fn extend<I: IntoIterator<Item = (&'p str, Value<'p>)>>(&mut self, bindings: I) {
        for binding in bindings {
            self.push(binding);
        }
    }
}

impl<'p> FromIterator<(&'p str, Value<'p>)> for Env<'p> {
    fn from_iter<I: IntoIterator<Item = (&'p str, Value<'p>)>>(bindings: I) -> Env<'p> {
        let mut env = Env::new();
        env.extend(bindings);
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_finds_the_latest_binding_and_scope_exit_restores_the_earlier() {
        let mut env = Env::new();
        env.push(("x", Value::Unit));
        env.push(("y", Value::Bool(true)));
        let mark = env.len();
        env.push(("x", Value::Bool(false)));
        assert!(matches!(env.lookup("x"), Some(Value::Bool(false))));
        env.truncate(mark);
        assert!(matches!(env.lookup("x"), Some(Value::Unit)));
        assert!(matches!(env.lookup("y"), Some(Value::Bool(true))));
        env.truncate(0);
        assert!(env.lookup("x").is_none());
        assert_eq!(env.len(), 0);
    }

    #[test]
    fn a_change_in_place_reaches_the_latest_binding_only() {
        let mut env: Env<'_> = [("x", Value::Unit), ("x", Value::Unit)]
            .into_iter()
            .collect();
        if let Some(value) = env.lookup_mut("x") {
            *value = Value::Bool(true);
        }
        assert!(matches!(env.get(0), Some((_, Value::Unit))));
        assert!(matches!(env.get(1), Some((_, Value::Bool(true)))));
    }
}
