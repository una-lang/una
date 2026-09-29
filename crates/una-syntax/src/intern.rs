//! Interned names (`compiler/architecture.md §4`).

use indexmap::IndexSet;

/// An interned name. Symbols are numbered in the order names are first interned, which is
/// deterministic for a given input; output never iterates the interner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(usize);

/// A deterministic interner for names.
#[derive(Clone, Debug, Default)]
pub struct Interner {
    names: IndexSet<Box<str>>,
}

impl Interner {
    /// An empty interner.
    pub fn new() -> Interner {
        Interner::default()
    }

    /// The symbol of `name`, interning it on first use.
    pub fn intern(&mut self, name: &str) -> Symbol {
        if let Some(index) = self.names.get_index_of(name) {
            return Symbol(index);
        }
        let (index, _) = self.names.insert_full(name.into());
        Symbol(index)
    }

    /// The name of `symbol`, which this interner created.
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.names.get_index(symbol.0).map_or("", |name| name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_idempotent_and_ordered_by_first_use() {
        let mut interner = Interner::new();
        let b = interner.intern("b");
        let a = interner.intern("a");
        assert_eq!(interner.intern("b"), b);
        assert!(b < a);
        assert_eq!(interner.resolve(a), "a");
        assert_eq!(interner.resolve(b), "b");
    }
}
