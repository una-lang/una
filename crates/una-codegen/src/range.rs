//! The ranges that integer types prove (ADR 0055 NB-3, ADR 0057 FW-1): a fixed-width type's
//! own, or what an `Int`'s refinements bound. A value whose range lies within a tagged word
//! (`[-2^62, 2^62)`) never needs arbitrary precision, so arithmetic among such values, when its
//! result is proven in range too, is a plain machine instruction with no check.
//!
//! A refinement counts only where verification proves it of every value: a function's
//! parameters, which each call proves, a `let` whose type the source writes, which is an
//! obligation (`spec/12 §3`), and fixed-width types, whose arithmetic carries an `overflow`
//! obligation. The code reads every other binding's type through [`unrefined`].

use una_core::{Base, BinOp, Term, Type};

/// The integers a tagged word holds.
const TAGGED: (i128, i128) = (-(1 << 62), (1 << 62) - 1);

/// The range that values of type `ty` are proven to lie in, when it is bounded on both sides.
pub fn proven_range(ty: &Type) -> Option<(i128, i128)> {
    let Type::Base {
        base, refinements, ..
    } = ty
    else {
        return None;
    };
    let mut range = match base {
        Base::Fixed(int) => (int.least(), int.greatest()),
        Base::Int => (i128::MIN, i128::MAX),
        _ => return None,
    };
    for refinement in refinements {
        range = intersect(range, bounds(&refinement.clause.term));
    }
    (range.0 > i128::MIN && range.1 < i128::MAX && range.0 <= range.1).then_some(range)
}

/// Whether values of type `ty` are proven to fit a tagged word: machine width.
pub fn machine_width(ty: &Type) -> bool {
    proven_range(ty).is_some_and(|(low, high)| TAGGED.0 <= low && high <= TAGGED.1)
}

/// `ty` without the refinements of an `Int`, for a binding whose refinements verification does
/// not prove of every value. A fixed-width type keeps its width, which is always proven.
pub(crate) fn unrefined(ty: &Type) -> Type {
    match ty {
        Type::Base {
            base: Base::Int,
            label,
            ..
        } => Type::Base {
            base: Base::Int,
            refinements: Vec::new(),
            label: label.clone(),
        },
        other => other.clone(),
    }
}

fn intersect(a: (i128, i128), b: (i128, i128)) -> (i128, i128) {
    (a.0.max(b.0), a.1.min(b.1))
}

/// The bounds on `it` that `term` implies, when it is a conjunction of ranges and comparisons
/// of `it` with literals; what it does not bound stays unbounded.
fn bounds(term: &Term) -> (i128, i128) {
    let any = (i128::MIN, i128::MAX);
    let literal = |term: &Term| match term {
        Term::Int(digits) => digits.parse::<i128>().ok(),
        _ => None,
    };
    match term {
        Term::In {
            value,
            low,
            high,
            exclusive,
        } if matches!(value.as_ref(), Term::It) => {
            let (Ok(low), Ok(high)) = (low.parse::<i128>(), high.parse::<i128>()) else {
                return any;
            };
            (low, if *exclusive { high - 1 } else { high })
        }
        Term::Binary {
            op: BinOp::And,
            left,
            right,
        } => intersect(bounds(left), bounds(right)),
        Term::Binary { op, left, right } => {
            // `it op c`, or `c op it` read as `it op' c`.
            let (op, c) = match (left.as_ref(), right.as_ref()) {
                (Term::It, other) => (*op, literal(other)),
                (other, Term::It) => (mirror(*op), literal(other)),
                _ => return any,
            };
            let Some(c) = c else {
                return any;
            };
            match op {
                BinOp::Ge => (c, i128::MAX),
                BinOp::Gt => (c.saturating_add(1), i128::MAX),
                BinOp::Le => (i128::MIN, c),
                BinOp::Lt => (i128::MIN, c.saturating_sub(1)),
                BinOp::Eq => (c, c),
                _ => any,
            }
        }
        _ => any,
    }
}

/// The comparison `op` with its operands swapped.
fn mirror(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Le => BinOp::Ge,
        BinOp::Gt => BinOp::Lt,
        BinOp::Ge => BinOp::Le,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use una_core::{Clause, IntType, Label, Refinement};

    fn int(refinements: Vec<Term>) -> Type {
        Type::Base {
            base: Base::Int,
            refinements: refinements
                .into_iter()
                .map(|term| Refinement {
                    name: String::new(),
                    clause: Clause {
                        term,
                        text: String::new(),
                        span: None,
                    },
                })
                .collect(),
            label: Label::public(),
        }
    }

    fn cmp(op: BinOp, left: Term, right: Term) -> Term {
        Term::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    #[test]
    fn ranges_come_from_refinements_and_fixed_widths() {
        let range = Term::In {
            value: Box::new(Term::It),
            low: "0".to_owned(),
            high: "5000".to_owned(),
            exclusive: true,
        };
        assert_eq!(proven_range(&int(vec![range])), Some((0, 4999)));
        let both = cmp(
            BinOp::And,
            cmp(BinOp::Ge, Term::It, Term::Int("-3".to_owned())),
            cmp(BinOp::Gt, Term::Int("10".to_owned()), Term::It),
        );
        assert_eq!(proven_range(&int(vec![both])), Some((-3, 9)));
        // One side only, or no refinement: unbounded.
        let positive = cmp(BinOp::Gt, Term::It, Term::Int("0".to_owned()));
        assert_eq!(proven_range(&int(vec![positive])), None);
        assert_eq!(proven_range(&int(Vec::new())), None);
        let u32 = Type::Base {
            base: Base::Fixed(IntType::U32),
            refinements: Vec::new(),
            label: Label::public(),
        };
        assert!(machine_width(&u32));
        let u64 = Type::Base {
            base: Base::Fixed(IntType::U64),
            refinements: Vec::new(),
            label: Label::public(),
        };
        assert!(!machine_width(&u64));
        assert!(!machine_width(&unrefined(&int(vec![cmp(
            BinOp::Eq,
            Term::It,
            Term::Int("1".to_owned())
        )]))));
    }
}
