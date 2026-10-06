//! The type descriptors of the native backend (`spec/20 §7`, ADR 0066 G4): the hidden word a
//! function takes for each constrained type parameter, which says how to compare and convert
//! values of the type that instantiates it. The code generator writes them and the runtime
//! reads them, both through this module.

use crate::IntType;

/// How values of the type that instantiates a constrained type parameter compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Descriptor {
    /// `Int`: integers, in order.
    Int,
    /// A fixed-width type, with its width and signedness: integers in its range, in order.
    Fixed(IntType),
    /// `Text`, in the order of `spec/03 §1`.
    Text,
    /// A sum whose variants have no payload, by the position of the variant.
    Sum,
    /// Any other type that satisfies `Eq`: structural equality, and no order.
    Structural,
}

/// The code of the first fixed-width type: the next ones follow in `IntType::ALL` order.
const FIXED: i64 = 8;

impl Descriptor {
    /// The word that carries the descriptor: an immediate, `(code << 1) | 1`, which reference
    /// counting never follows, also when a closure holds it.
    pub fn word(self) -> i64 {
        let code = match self {
            Descriptor::Int => 0,
            Descriptor::Text => 1,
            Descriptor::Sum => 2,
            Descriptor::Structural => 3,
            Descriptor::Fixed(int) => {
                FIXED
                    + IntType::ALL
                        .iter()
                        .position(|other| *other == int)
                        .and_then(|i| i64::try_from(i).ok())
                        .unwrap_or(0)
            }
        };
        (code << 1) | 1
    }

    /// The descriptor that `word` carries, or `None` when it carries none.
    pub fn from_word(word: i64) -> Option<Descriptor> {
        if word & 1 != 1 {
            return None;
        }
        match word >> 1 {
            0 => Some(Descriptor::Int),
            1 => Some(Descriptor::Text),
            2 => Some(Descriptor::Sum),
            3 => Some(Descriptor::Structural),
            code => usize::try_from(code - FIXED)
                .ok()
                .and_then(|i| IntType::ALL.get(i).copied())
                .map(Descriptor::Fixed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_descriptor_has_its_own_immediate_word() {
        let mut all = vec![
            Descriptor::Int,
            Descriptor::Text,
            Descriptor::Sum,
            Descriptor::Structural,
        ];
        all.extend(IntType::ALL.iter().copied().map(Descriptor::Fixed));
        let mut words = std::collections::BTreeSet::new();
        for descriptor in all {
            let word = descriptor.word();
            assert_eq!(word & 1, 1, "{descriptor:?}");
            assert!(words.insert(word), "{descriptor:?}");
            assert_eq!(Descriptor::from_word(word), Some(descriptor));
        }
        assert_eq!(Descriptor::from_word(2), None);
        assert_eq!(Descriptor::from_word((99 << 1) | 1), None);
    }
}
