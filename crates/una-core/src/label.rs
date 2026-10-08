//! Labels (`spec/07 §2`): a level and a set of categories, ordered componentwise.

use std::fmt;
use std::sync::Arc;

pub use una_syntax::ast::Level;

/// An information-flow label. `public` without categories or symbols is `⊥`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label {
    /// The level.
    pub level: Level,
    /// The categories, in alphabetical order.
    pub categories: Names,
    /// The symbolic labels it contains, in order (`spec/07 §7` rule 8, ADR 0038 D3): the name of
    /// a type parameter `T` stands for `ℓ_T`, the label of the values of `T` in a generic
    /// body. The type checker also uses names that start with `?` for the labels of its
    /// unification variables, which it replaces before labels are compared or shown.
    pub symbols: Names,
}

impl Label {
    /// `⊥`: `public`, without categories.
    pub fn public() -> Label {
        Label {
            level: Level::Public,
            categories: Names::new(),
            symbols: Names::new(),
        }
    }

    /// `ℓ_name`, the symbolic label `name` alone.
    pub fn symbol(name: impl Into<String>) -> Label {
        let mut label = Label::public();
        label.symbols.insert(name.into().as_str());
        label
    }

    /// The label without its symbolic labels.
    pub fn concrete(&self) -> Label {
        Label {
            level: self.level,
            categories: self.categories.clone(),
            symbols: Names::new(),
        }
    }

    /// `⊤`: `restricted` with every declared category.
    pub fn top(categories: &[String]) -> Label {
        Label {
            level: Level::Restricted,
            categories: categories.iter().map(String::as_str).collect(),
            symbols: Names::new(),
        }
    }

    /// Whether this is `⊥`.
    pub fn is_public(&self) -> bool {
        self.level == Level::Public && self.categories.is_empty() && self.symbols.is_empty()
    }

    /// The least upper bound: the higher level and the union of the categories and symbols.
    pub fn join(&self, other: &Label) -> Label {
        Label {
            level: self.level.max(other.level),
            categories: self.categories.union(&other.categories),
            symbols: self.symbols.union(&other.symbols),
        }
    }

    /// The greatest lower bound: the lower level and the common categories and symbols.
    pub fn meet(&self, other: &Label) -> Label {
        Label {
            level: self.level.min(other.level),
            categories: self.categories.intersection(&other.categories),
            symbols: self.symbols.intersection(&other.symbols),
        }
    }

    /// `self ⊑ other`: a level no higher, categories among the other's, and symbolic labels
    /// among the other's, since `ℓ_T` may stand for any label (ADR 0038 D3).
    pub fn flows_to(&self, other: &Label) -> bool {
        self.level <= other.level
            && self.categories.is_subset(&other.categories)
            && self.symbols.is_subset(&other.symbols)
    }
}

impl fmt::Display for Label {
    /// `public`, `secret[PII]`, and a symbolic label as `label(T)`, joined with `+`, as in
    /// `internal + label(T)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let symbols: Vec<String> = self
            .symbols
            .iter()
            .filter(|symbol| !symbol.starts_with('?'))
            .map(|symbol| format!("label({symbol})"))
            .collect();
        let concrete = self.concrete();
        if !symbols.is_empty() && concrete.is_public() {
            return write!(f, "{}", symbols.join(" + "));
        }
        write_concrete(f, &concrete)?;
        for symbol in symbols {
            write!(f, " + {symbol}")?;
        }
        Ok(())
    }
}

/// A label without symbols, as `secret[PII]`.
fn write_concrete(f: &mut fmt::Formatter<'_>, label: &Label) -> fmt::Result {
    let level = match label.level {
        Level::Public => "public",
        Level::Internal => "internal",
        Level::Confidential => "confidential",
        Level::Secret => "secret",
        Level::Restricted => "restricted",
    };
    write!(f, "{level}")?;
    if !label.categories.is_empty() {
        let categories: Vec<&str> = label.categories.iter().map(|c| &**c).collect();
        write!(f, "[{}]", categories.join(", "))?;
    }
    Ok(())
}

/// A set of names, the categories or the symbolic labels of a [`Label`], in the order of their
/// text (ADR 0077 stage 1). It is shared: cloning it, or a label, allocates nothing, and the
/// operations that would return a set equal to one of their operands return that operand.
/// Names are compared, ordered and hashed by their text, never by address, so it orders,
/// compares and prints as a `BTreeSet<String>` of the same names did.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Names(Option<Arc<[Arc<str>]>>);

impl Names {
    /// The empty set; it allocates nothing.
    pub fn new() -> Names {
        Names(None)
    }

    /// The names, sorted by their text and without repeats.
    fn from_sorted(names: Vec<Arc<str>>) -> Names {
        if names.is_empty() {
            return Names(None);
        }
        counters::set();
        Names(Some(names.into()))
    }

    fn slice(&self) -> &[Arc<str>] {
        self.0.as_deref().unwrap_or(&[])
    }

    /// The names in the order of their text.
    pub fn iter(&self) -> std::slice::Iter<'_, Arc<str>> {
        self.slice().iter()
    }

    /// How many names it holds.
    pub fn len(&self) -> usize {
        self.slice().len()
    }

    /// Whether it holds no name.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Whether it holds `name`, in O(log n).
    pub fn contains(&self, name: &str) -> bool {
        self.slice()
            .binary_search_by(|held| (**held).cmp(name))
            .is_ok()
    }

    /// A name, made once: a set that takes it shares it.
    pub fn name(text: &str) -> Arc<str> {
        counters::string();
        Arc::from(text)
    }

    /// Adds `name`; returns whether it was not there yet.
    pub fn insert(&mut self, name: &str) -> bool {
        match self.slice().binary_search_by(|held| (**held).cmp(name)) {
            Ok(_) => false,
            Err(at) => {
                let mut names = self.slice().to_vec();
                names.insert(at, Names::name(name));
                *self = Names::from_sorted(names);
                true
            }
        }
    }

    /// Keeps the names for which `keep` holds; the set is shared when it keeps them all.
    /// `keep` may be called twice for a name, so it must not count or change anything: a `Fn`.
    pub fn retain(&mut self, keep: impl Fn(&Arc<str>) -> bool) {
        if self.iter().all(&keep) {
            return;
        }
        let names: Vec<Arc<str>> = self.iter().filter(|name| keep(name)).cloned().collect();
        *self = Names::from_sorted(names);
    }

    /// Removes every name.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// The names of both; one of them when it holds the other's.
    pub fn union(&self, other: &Names) -> Names {
        if other.is_subset(self) {
            return self.clone();
        }
        if self.is_subset(other) {
            return other.clone();
        }
        let (mut a, mut b) = (self.iter().peekable(), other.iter().peekable());
        let mut names = Vec::with_capacity(self.len() + other.len());
        loop {
            let next = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => match (***x).cmp(&***y) {
                    std::cmp::Ordering::Less => a.next(),
                    std::cmp::Ordering::Greater => b.next(),
                    std::cmp::Ordering::Equal => {
                        b.next();
                        a.next()
                    }
                },
                (Some(_), None) => a.next(),
                (None, Some(_)) => b.next(),
                (None, None) => break,
            };
            if let Some(name) = next {
                names.push(Arc::clone(name));
            }
        }
        Names::from_sorted(names)
    }

    /// The names common to both; one of them when it is a subset of the other.
    pub fn intersection(&self, other: &Names) -> Names {
        if self.is_subset(other) {
            return self.clone();
        }
        if other.is_subset(self) {
            return other.clone();
        }
        let names: Vec<Arc<str>> = self
            .iter()
            .filter(|name| other.contains(name))
            .cloned()
            .collect();
        Names::from_sorted(names)
    }

    /// Whether every name of `self` is in `other`, in O(n + m).
    pub fn is_subset(&self, other: &Names) -> bool {
        if self.len() > other.len() {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.0, &other.0) {
            // Only a fast path to `true`: the same allocation holds the same names.
            if Arc::ptr_eq(a, b) {
                return true;
            }
        }
        let mut theirs = other.iter();
        self.iter().all(|name| theirs.any(|held| **held == **name))
    }
}

impl fmt::Debug for Names {
    /// As a `BTreeSet<String>` of the same names prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(|name| &**name))
            .finish()
    }
}

impl<'a> IntoIterator for &'a Names {
    type Item = &'a Arc<str>;
    type IntoIter = std::slice::Iter<'a, Arc<str>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl FromIterator<Arc<str>> for Names {
    /// The names, shared with the ones given.
    fn from_iter<I: IntoIterator<Item = Arc<str>>>(names: I) -> Names {
        let mut names: Vec<Arc<str>> = names.into_iter().collect();
        // `Arc<str>` orders and compares by its text.
        names.sort_unstable();
        names.dedup();
        Names::from_sorted(names)
    }
}

impl<'a> FromIterator<&'a str> for Names {
    /// The names, each made once.
    fn from_iter<I: IntoIterator<Item = &'a str>>(names: I) -> Names {
        names.into_iter().map(Names::name).collect()
    }
}

impl FromIterator<String> for Names {
    /// The names, each made once.
    fn from_iter<I: IntoIterator<Item = String>>(names: I) -> Names {
        names.into_iter().map(|name| Names::name(&name)).collect()
    }
}

/// Counts allocations for the scaling tests of ADR 0077: the sets and names of [`Names`], and
/// the type nodes of the checker. Only with the feature `counters`, which tests enable; a release build counts
/// nothing. The counts never reach any output.
pub mod counters {
    #[cfg(feature = "counters")]
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "counters")]
    static SETS: AtomicUsize = AtomicUsize::new(0);
    #[cfg(feature = "counters")]
    static STRINGS: AtomicUsize = AtomicUsize::new(0);
    #[cfg(feature = "counters")]
    static NODES: AtomicUsize = AtomicUsize::new(0);

    /// A set was made.
    #[inline]
    pub(crate) fn set() {
        #[cfg(feature = "counters")]
        SETS.fetch_add(1, Ordering::Relaxed);
    }

    /// A name was made.
    #[inline]
    pub(crate) fn string() {
        #[cfg(feature = "counters")]
        STRINGS.fetch_add(1, Ordering::Relaxed);
    }

    /// A type node of the checker was made (ADR 0077 stage 2).
    #[inline]
    pub fn node() {
        #[cfg(feature = "counters")]
        NODES.fetch_add(1, Ordering::Relaxed);
    }

    /// The type nodes the checker made so far in this process; always 0 without the feature
    /// `counters`.
    pub fn nodes_made() -> usize {
        #[cfg(feature = "counters")]
        {
            NODES.load(Ordering::Relaxed)
        }
        #[cfg(not(feature = "counters"))]
        {
            0
        }
    }

    /// The sets and names made so far by this process: `(sets, names)`. Always `(0, 0)`
    /// without the feature `counters`.
    pub fn made() -> (usize, usize) {
        #[cfg(feature = "counters")]
        {
            (
                SETS.load(Ordering::Relaxed),
                STRINGS.load(Ordering::Relaxed),
            )
        }
        #[cfg(not(feature = "counters"))]
        {
            (0, 0)
        }
    }
}

/// The sink label of an operation: `None` when it accepts every label.
pub type Accepts = Option<Label>;

#[cfg(test)]
mod tests {
    use super::*;

    fn label(level: Level, categories: &[&str]) -> Label {
        Label {
            level,
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
            symbols: Names::new(),
        }
    }

    /// The names of `bits`, a subset of a fixed alphabet that includes `T`, `T*` and `?n`.
    fn sample(bits: u32) -> Vec<&'static str> {
        const ALPHABET: [&str; 8] = ["?1", "?12", "PII", "T", "T*", "U", "U*", "V"];
        ALPHABET
            .iter()
            .enumerate()
            .filter(|(i, _)| bits & (1 << i) != 0)
            .map(|(_, name)| *name)
            .collect()
    }

    #[test]
    fn names_behave_as_a_set_of_strings_in_text_order() {
        // ADR 0077 stage 1: every operation, the order, equality and the printed form agree
        // with `BTreeSet<String>` over every pair of subsets of an alphabet with `T` and `T*`.
        use std::collections::BTreeSet;
        let all = |bits| -> (Names, BTreeSet<String>) {
            let names = sample(bits);
            (
                names.iter().copied().collect(),
                names.iter().map(|n| (*n).to_owned()).collect(),
            )
        };
        let back =
            |names: &Names| -> BTreeSet<String> { names.iter().map(|n| n.to_string()).collect() };
        for a in 0..256u32 {
            let (na, sa) = all(a);
            assert_eq!(back(&na), sa);
            assert_eq!(na.len(), sa.len());
            assert_eq!(na.is_empty(), sa.is_empty());
            assert_eq!(format!("{na:?}"), format!("{sa:?}"));
            for name in sample(255) {
                assert_eq!(na.contains(name), sa.contains(name));
                let (mut ni, mut si) = (na.clone(), sa.clone());
                assert_eq!(ni.insert(name), si.insert(name.to_owned()));
                assert_eq!(back(&ni), si);
                ni.retain(|n| **n != *name);
                si.retain(|n| n != name);
                assert_eq!(back(&ni), si);
            }
            for b in 0..256u32 {
                let (nb, sb) = all(b);
                assert_eq!(back(&na.union(&nb)), sa.union(&sb).cloned().collect());
                assert_eq!(
                    back(&na.intersection(&nb)),
                    sa.intersection(&sb).cloned().collect()
                );
                assert_eq!(na.is_subset(&nb), sa.is_subset(&sb));
                assert_eq!(na == nb, sa == sb);
                assert_eq!(na.cmp(&nb), sa.cmp(&sb));
            }
        }
    }

    #[test]
    fn a_label_prints_and_orders_as_before() {
        let mut deep = Label::symbol("T");
        deep.symbols.insert("T*");
        deep.symbols.insert("?3");
        assert_eq!(deep.to_string(), "label(T) + label(T*)");
        let secret = label(Level::Secret, &["PII"]).join(&deep);
        assert_eq!(secret.to_string(), "secret[PII] + label(T) + label(T*)");
        assert!(Label::symbol("T").flows_to(&deep));
        assert!(!deep.flows_to(&Label::symbol("T")));
        let mut kept = deep.clone();
        kept.symbols.retain(|s| !s.ends_with('*'));
        assert_eq!(kept.symbols, ["?3", "T"].into_iter().collect::<Names>());
        assert!(Label::symbol("T") < Label::symbol("T*"));
    }

    #[test]
    fn operations_that_change_nothing_share_their_operand() {
        let a: Names = ["T", "T*"].into_iter().collect();
        let b: Names = ["T"].into_iter().collect();
        let union = a.union(&b);
        let meet = a.intersection(&b);
        let (Some(x), Some(y), Some(z), Some(w)) = (&a.0, &union.0, &b.0, &meet.0) else {
            panic!("non-empty sets hold an allocation");
        };
        assert!(Arc::ptr_eq(x, y));
        assert!(Arc::ptr_eq(z, w));
        let mut same = a.clone();
        same.retain(|_| true);
        assert!(matches!((&a.0, &same.0), (Some(x), Some(y)) if Arc::ptr_eq(x, y)));
        assert!(Names::new().is_empty() && Names::new().0.is_none());
    }

    #[test]
    fn the_order_is_componentwise() {
        let pii = label(Level::Secret, &["PII"]);
        let location = label(Level::Confidential, &["Location"]);
        assert!(Label::public().flows_to(&pii));
        assert!(!pii.flows_to(&location));
        assert!(!location.flows_to(&pii));
        let joined = pii.join(&location);
        assert_eq!(joined, label(Level::Secret, &["Location", "PII"]));
        assert_eq!(joined.to_string(), "secret[Location, PII]");
        assert_eq!(pii.meet(&location), label(Level::Confidential, &[]));
        assert!(pii.flows_to(&Label::top(&["Location".to_owned(), "PII".to_owned()])));
        assert!(Label::public().is_public());
        assert_eq!(Label::public().to_string(), "public");
    }

    #[test]
    fn a_symbolic_label_is_below_only_labels_that_contain_it() {
        let t = Label::symbol("T");
        let secret = label(Level::Secret, &[]);
        assert!(!t.is_public());
        assert!(!t.flows_to(&secret));
        assert!(t.flows_to(&t.join(&secret)));
        assert!(!secret.join(&t).flows_to(&t));
        assert_eq!(t.join(&Label::public()), t);
        assert_eq!(t.meet(&secret), Label::public());
        assert_eq!(t.to_string(), "label(T)");
        assert_eq!(secret.join(&t).to_string(), "secret + label(T)");
        assert_eq!(secret.join(&t).concrete(), secret);
    }
}
