//! The run-time environment `E` of the interpreter (`spec/11 §2`): the bindings in scope, in the
//! order they were made, with an index from each name to the positions that bind it, so a read
//! takes the latest binding of its name in expected O(1), not a scan of every binding (#329).
//!
//! Shadowing pushes a position onto the name's stack and leaving a scope pops it, so the index
//! always names the binding a scan from the end would find. The index is never iterated, so its
//! order is never observable. A small environment, as a closure's, has no index: a scan of up
//! to [`INDEXED`] bindings costs less than hashing, and copying it costs only its bindings.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::value::Value;

/// How many bindings an environment holds before it indexes them by name.
const INDEXED: usize = 64;

/// FNV-1a over a name's bytes: names are short, and the index is never iterated, so a fast
/// hash with no protection against chosen collisions is enough (a program's own names).
#[derive(Default)]
struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        self.0 = hash;
    }
}

/// The index of an environment: each name's positions.
type Positions<'p> = HashMap<&'p str, Vec<usize>, BuildHasherDefault<Fnv>>;

/// The bindings in scope, in order, and, past [`INDEXED`] bindings, the positions of each name.
#[derive(Clone, Debug, Default)]
pub(crate) struct Env<'p> {
    bindings: Vec<(&'p str, Value<'p>)>,
    positions: Option<Positions<'p>>,
}

impl<'p> Env<'p> {
    /// An empty environment.
    pub(crate) fn new() -> Env<'p> {
        Env::default()
    }

    /// Takes the values of every binding out of the environment, which is left empty: what
    /// dropping it releases, for a worklist (`crate::value`, S-30).
    pub(crate) fn take_values(&mut self) -> Vec<Value<'p>> {
        self.positions = None;
        std::mem::take(&mut self.bindings)
            .into_iter()
            .map(|(_, value)| value)
            .collect()
    }

    /// How many bindings are in scope: a mark that [`Env::truncate`] returns to.
    pub(crate) fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Binds `name` to `value`, shadowing an earlier binding of `name`.
    pub(crate) fn push(&mut self, (name, value): (&'p str, Value<'p>)) {
        if let Some(positions) = &mut self.positions {
            positions.entry(name).or_default().push(self.bindings.len());
        } else if self.bindings.len() >= INDEXED {
            let mut positions = Positions::default();
            for (at, (bound, _)) in self.bindings.iter().enumerate() {
                positions.entry(*bound).or_default().push(at);
            }
            positions.entry(name).or_default().push(self.bindings.len());
            self.positions = Some(positions);
        }
        self.bindings.push((name, value));
    }

    /// The position of the latest binding of `name`.
    fn position(&self, name: &str) -> Option<usize> {
        match &self.positions {
            Some(positions) => positions.get(name)?.last().copied(),
            None => self.bindings.iter().rposition(|(bound, _)| *bound == name),
        }
    }

    /// Leaves the bindings made after the mark `len`: each removal is O(1), so a scope's exit
    /// costs what its bindings cost.
    pub(crate) fn truncate(&mut self, len: usize) {
        while self.bindings.len() > len {
            let Some((name, _)) = self.bindings.pop() else {
                break;
            };
            if let Some(positions) = &mut self.positions
                && let Some(stack) = positions.get_mut(name)
            {
                stack.pop();
                if stack.is_empty() {
                    positions.remove(name);
                }
            }
        }
    }

    /// The latest binding of `name`.
    pub(crate) fn lookup(&self, name: &str) -> Option<&Value<'p>> {
        let at = self.position(name)?;
        self.bindings.get(at).map(|(_, value)| value)
    }

    /// The latest binding of `name`, with its name as the environment holds it.
    pub(crate) fn binding(&self, name: &str) -> Option<(&'p str, Value<'p>)> {
        let at = self.position(name)?;
        self.bindings
            .get(at)
            .map(|(name, value)| (*name, value.clone()))
    }

    /// The latest binding of `name`, to change in place.
    pub(crate) fn lookup_mut(&mut self, name: &str) -> Option<&mut Value<'p>> {
        let at = self.position(name)?;
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
    fn an_environment_past_the_threshold_finds_the_same_bindings() {
        // The index is built at INDEXED bindings: reads, shadowing and scope exit agree with
        // the scan on either side of it.
        let names: Vec<String> = (0..3 * INDEXED).map(|i| format!("v{}", i % 7)).collect();
        let mut env = Env::new();
        for (i, name) in names.iter().enumerate() {
            env.push((name.as_str(), Value::Int(i.into())));
            for probe in 0..7 {
                let probe = format!("v{probe}");
                let scanned = names[..=i].iter().rposition(|n| *n == probe);
                let found = env.lookup(&probe).map(|v| match v {
                    Value::Int(n) => usize::try_from(n.clone()).unwrap(),
                    _ => usize::MAX,
                });
                assert_eq!(found, scanned, "{i} {probe}");
            }
        }
        env.truncate(INDEXED / 2);
        let latest = (0..INDEXED / 2).rev().find(|i| i % 7 == 0).unwrap_or(0);
        assert!(matches!(env.lookup("v0"), Some(Value::Int(n)) if *n == latest.into()));
        assert!(env.lookup("v6").is_some());
        env.truncate(0);
        assert!(env.lookup("v0").is_none());
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
