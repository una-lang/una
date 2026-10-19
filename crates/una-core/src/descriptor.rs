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
    /// `F64`: equality by its bits, and IEEE totalOrder (ADR 0114 rule 9.4).
    F64,
    /// A sum whose variants have no payload, by the position of the variant, with the index of
    /// its type description, which interpolation writes it by (ADR 0068).
    Sum(usize),
    /// Any other type that satisfies `Eq`: structural equality, and no order, with the index of
    /// its type description.
    Structural(usize),
    /// `Decimal[s]`, by its coefficient, in order, with the index of its type description,
    /// which writes it with its scale (`spec/29 §16`).
    Decimal(usize),
    /// A type that holds type parameters of the caller, as `Box[T]` or `List[T]`: an entry of
    /// the runtime's table of composite descriptors, which holds the type's description and
    /// the descriptor of each of its parameters (ADR 0068 D6).
    Composite(usize),
}

/// The code of the first fixed-width type: the next ones follow in `IntType::ALL` order.
const FIXED: i64 = 8;

/// The bits of the code, below the index of a type description.
const CODE_BITS: u32 = 4;

impl Descriptor {
    /// The word that carries the descriptor: an immediate, `(code << 1) | 1`, which reference
    /// counting never follows, also when a closure holds it.
    pub fn word(self) -> i64 {
        let index = |ty: usize| i64::try_from(ty).unwrap_or(0) << CODE_BITS;
        let payload = match self {
            Descriptor::Int => 0,
            Descriptor::Text => 1,
            Descriptor::F64 => 6,
            Descriptor::Sum(ty) => 2 | index(ty),
            Descriptor::Structural(ty) => 3 | index(ty),
            Descriptor::Composite(entry) => 4 | index(entry),
            Descriptor::Decimal(ty) => 5 | index(ty),
            Descriptor::Fixed(int) => {
                FIXED
                    + IntType::ALL
                        .iter()
                        .position(|other| *other == int)
                        .and_then(|i| i64::try_from(i).ok())
                        .unwrap_or(0)
            }
        };
        (payload << 1) | 1
    }

    /// The descriptor that `word` carries, or `None` when it carries none.
    pub fn from_word(word: i64) -> Option<Descriptor> {
        if word & 1 != 1 {
            return None;
        }
        let payload = word >> 1;
        let ty = usize::try_from(payload >> CODE_BITS).ok()?;
        match payload & ((1 << CODE_BITS) - 1) {
            0 if ty == 0 => Some(Descriptor::Int),
            1 if ty == 0 => Some(Descriptor::Text),
            2 => Some(Descriptor::Sum(ty)),
            3 => Some(Descriptor::Structural(ty)),
            4 => Some(Descriptor::Composite(ty)),
            5 => Some(Descriptor::Decimal(ty)),
            6 if ty == 0 => Some(Descriptor::F64),
            code if ty == 0 => usize::try_from(code - FIXED)
                .ok()
                .and_then(|i| IntType::ALL.get(i).copied())
                .map(Descriptor::Fixed),
            _ => None,
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
            Descriptor::F64,
            Descriptor::Sum(0),
            Descriptor::Sum(41),
            Descriptor::Structural(0),
            Descriptor::Structural(7),
            Descriptor::Composite(0),
            Descriptor::Composite(3),
            Descriptor::Decimal(0),
            Descriptor::Decimal(9),
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
        assert_eq!(Descriptor::from_word((7 << 1) | 1), None);
    }
}
