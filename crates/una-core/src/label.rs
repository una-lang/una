//! Labels (`spec/07 §2`): a level and a set of categories, ordered componentwise.

use std::collections::BTreeSet;
use std::fmt;

pub use una_syntax::ast::Level;

/// An information-flow label. `public` without categories or symbols is `⊥`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label {
    /// The level.
    pub level: Level,
    /// The categories, in alphabetical order.
    pub categories: BTreeSet<String>,
    /// The symbolic labels it contains, in order (`spec/07 §7` rule 8, ADR 0038 D3): the name of
    /// a type parameter `T` stands for `ℓ_T`, the label of the values of `T` in a generic
    /// body. The type checker also uses names that start with `?` for the labels of its
    /// unification variables, which it replaces before labels are compared or shown.
    pub symbols: BTreeSet<String>,
}

impl Label {
    /// `⊥`: `public`, without categories.
    pub fn public() -> Label {
        Label {
            level: Level::Public,
            categories: BTreeSet::new(),
            symbols: BTreeSet::new(),
        }
    }

    /// `ℓ_name`, the symbolic label `name` alone.
    pub fn symbol(name: impl Into<String>) -> Label {
        let mut label = Label::public();
        label.symbols.insert(name.into());
        label
    }

    /// The label without its symbolic labels.
    pub fn concrete(&self) -> Label {
        Label {
            level: self.level,
            categories: self.categories.clone(),
            symbols: BTreeSet::new(),
        }
    }

    /// `⊤`: `restricted` with every declared category.
    pub fn top(categories: &[String]) -> Label {
        Label {
            level: Level::Restricted,
            categories: categories.iter().cloned().collect(),
            symbols: BTreeSet::new(),
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
            categories: self.categories.union(&other.categories).cloned().collect(),
            symbols: self.symbols.union(&other.symbols).cloned().collect(),
        }
    }

    /// The greatest lower bound: the lower level and the common categories and symbols.
    pub fn meet(&self, other: &Label) -> Label {
        Label {
            level: self.level.min(other.level),
            categories: self
                .categories
                .intersection(&other.categories)
                .cloned()
                .collect(),
            symbols: self.symbols.intersection(&other.symbols).cloned().collect(),
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
        let categories: Vec<&str> = label.categories.iter().map(String::as_str).collect();
        write!(f, "[{}]", categories.join(", "))?;
    }
    Ok(())
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
            symbols: BTreeSet::new(),
        }
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
