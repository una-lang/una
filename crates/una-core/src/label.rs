//! Labels (`spec/07 §2`): a level and a set of categories, ordered componentwise.

use std::collections::BTreeSet;
use std::fmt;

pub use una_syntax::ast::Level;

/// An information-flow label. `public` without categories is `⊥`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label {
    /// The level.
    pub level: Level,
    /// The categories, in alphabetical order.
    pub categories: BTreeSet<String>,
}

impl Label {
    /// `⊥`: `public`, without categories.
    pub fn public() -> Label {
        Label {
            level: Level::Public,
            categories: BTreeSet::new(),
        }
    }

    /// `⊤`: `restricted` with every declared category.
    pub fn top(categories: &[String]) -> Label {
        Label {
            level: Level::Restricted,
            categories: categories.iter().cloned().collect(),
        }
    }

    /// Whether this is `⊥`.
    pub fn is_public(&self) -> bool {
        self.level == Level::Public && self.categories.is_empty()
    }

    /// The least upper bound: the higher level and the union of the categories.
    pub fn join(&self, other: &Label) -> Label {
        Label {
            level: self.level.max(other.level),
            categories: self.categories.union(&other.categories).cloned().collect(),
        }
    }

    /// The greatest lower bound: the lower level and the common categories.
    pub fn meet(&self, other: &Label) -> Label {
        Label {
            level: self.level.min(other.level),
            categories: self
                .categories
                .intersection(&other.categories)
                .cloned()
                .collect(),
        }
    }

    /// `self ⊑ other`: a level no higher and categories among the other's.
    pub fn flows_to(&self, other: &Label) -> bool {
        self.level <= other.level && self.categories.is_subset(&other.categories)
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.level {
            Level::Public => "public",
            Level::Internal => "internal",
            Level::Confidential => "confidential",
            Level::Secret => "secret",
            Level::Restricted => "restricted",
        };
        write!(f, "{level}")?;
        if !self.categories.is_empty() {
            let categories: Vec<&str> = self.categories.iter().map(String::as_str).collect();
            write!(f, "[{}]", categories.join(", "))?;
        }
        Ok(())
    }
}

/// The sink label of an operation: `None` when it accepts every label (`Keys.hmac`).
pub type Accepts = Option<Label>;

#[cfg(test)]
mod tests {
    use super::*;

    fn label(level: Level, categories: &[&str]) -> Label {
        Label {
            level,
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
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
}
