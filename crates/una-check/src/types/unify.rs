//! Unification variables, for generic instantiations, empty lists and lambda parameters
//! (`spec/03 §7`).

use una_syntax::Span;

use super::ty::{FnTy, Ty};

/// Why a unification variable exists, for the diagnostic when inference leaves it unknown
/// (DECISION D-054).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The element type of an empty list.
    Element,
    /// A type parameter of a generic function or kind, instantiated at a use.
    Instance { param: String, of: String },
}

#[derive(Clone, Debug)]
struct Slot {
    value: Option<Ty>,
    origin: Origin,
    span: Span,
}

/// The unification variables of one declaration.
#[derive(Clone, Debug, Default)]
pub(crate) struct Vars {
    slots: Vec<Slot>,
}

impl Vars {
    /// A new variable.
    pub(crate) fn fresh(&mut self, origin: Origin, span: Span) -> Ty {
        self.slots.push(Slot {
            value: None,
            origin,
            span,
        });
        Ty::Var(self.slots.len() - 1)
    }

    /// `ty` with its outermost variables replaced by their values.
    fn shallow(&self, ty: &Ty) -> Ty {
        let mut current = ty.clone();
        while let Ty::Var(var) = current {
            match self.slots.get(var).and_then(|slot| slot.value.clone()) {
                Some(value) => current = value,
                None => break,
            }
        }
        current
    }

    /// `ty` with every solved variable replaced by its value.
    pub(crate) fn resolve(&self, ty: &Ty) -> Ty {
        match self.shallow(ty) {
            Ty::List(element) => Ty::List(Box::new(self.resolve(&element))),
            Ty::Kind(id, args) => Ty::Kind(id, args.iter().map(|arg| self.resolve(arg)).collect()),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function.params.iter().map(|p| self.resolve(p)).collect(),
                result: self.resolve(&function.result),
                uses: function.uses.clone(),
                fails: function.fails.as_ref().map(|f| self.resolve(f)),
            })),
            other => other,
        }
    }

    fn occurs(&self, var: usize, ty: &Ty) -> bool {
        match self.shallow(ty) {
            Ty::Var(other) => other == var,
            Ty::List(element) => self.occurs(var, &element),
            Ty::Kind(_, args) => args.iter().any(|arg| self.occurs(var, arg)),
            Ty::Fn(function) => {
                function.params.iter().any(|p| self.occurs(var, p))
                    || self.occurs(var, &function.result)
            }
            _ => false,
        }
    }

    fn bind(&mut self, var: usize, ty: Ty) {
        if let Some(slot) = self.slots.get_mut(var) {
            slot.value = Some(ty);
        }
    }

    /// Makes `a` and `b` equal, solving variables; false when they cannot be. The error type
    /// and `Never` agree with every type (ADR 0024 rule 2); a variable meeting `Never` stays
    /// unsolved.
    pub(crate) fn unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let a = self.shallow(a);
        let b = self.shallow(b);
        match (&a, &b) {
            (Ty::Var(x), Ty::Var(y)) if x == y => true,
            (Ty::Never, _) | (_, Ty::Never) => true,
            (Ty::Var(x), Ty::Var(y)) => {
                // The later variable points to the earlier one, whose origin is reported.
                let (early, late) = if x < y { (*x, *y) } else { (*y, *x) };
                self.bind(late, Ty::Var(early));
                true
            }
            (Ty::Var(var), other) | (other, Ty::Var(var)) => {
                if self.occurs(*var, other) {
                    return false;
                }
                self.bind(*var, other.clone());
                true
            }
            (Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Int, Ty::Int)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Text, Ty::Text)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Hashed, Ty::Hashed) => true,
            (Ty::List(x), Ty::List(y)) => self.unify(x, y),
            (Ty::Kind(i, xs), Ty::Kind(j, ys)) => {
                i == j
                    && xs.len() == ys.len()
                    && xs
                        .iter()
                        .zip(ys)
                        .fold(true, |ok, (x, y)| self.unify(x, y) && ok)
            }
            (Ty::Param(i), Ty::Param(j)) => i == j,
            (Ty::Fn(f), Ty::Fn(g)) => {
                f.params.len() == g.params.len()
                    && f.params
                        .iter()
                        .zip(&g.params)
                        .fold(true, |ok, (x, y)| self.unify(x, y) && ok)
                    && self.unify(&f.result, &g.result)
            }
            _ => false,
        }
    }

    /// The variables still unsolved, each with its origin and span, in creation order.
    pub(crate) fn unsolved(&self) -> Vec<(Origin, Span)> {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_none())
            .map(|slot| (slot.origin.clone(), slot.span))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use una_syntax::{FileId, SourceDb};

    use super::*;

    fn span() -> Span {
        let mut db = SourceDb::new();
        let file: FileId = db.add("a.una", b"x");
        Span::at(file, 0)
    }

    #[test]
    fn variables_are_solved_and_resolved() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        assert!(vars.unify(&Ty::List(Box::new(a.clone())), &Ty::List(Box::new(Ty::Int))));
        assert_eq!(vars.resolve(&a), Ty::Int);
        assert!(!vars.unify(&a, &Ty::Text));
        assert!(vars.unsolved().is_empty());
    }

    #[test]
    fn never_and_error_agree_with_everything() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        assert!(vars.unify(&a, &Ty::Never));
        assert_eq!(vars.unsolved().len(), 1);
        assert!(vars.unify(&Ty::Error, &Ty::Text));
        assert!(vars.unify(&a, &Ty::Error));
        assert!(vars.unsolved().is_empty());
    }

    #[test]
    fn the_occurs_check_rejects_infinite_types() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        assert!(!vars.unify(&a, &Ty::List(Box::new(a.clone()))));
    }

    #[test]
    fn two_variables_keep_the_earlier_origin() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        let b = vars.fresh(
            Origin::Instance {
                param: "T".to_owned(),
                of: "first".to_owned(),
            },
            span(),
        );
        assert!(vars.unify(&b, &a));
        assert_eq!(vars.unsolved(), vec![(Origin::Element, span())]);
    }
}
