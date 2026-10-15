//! What pass 2 of the checker resolves, recorded for elaboration to Core IR (`spec/09`).

use std::collections::BTreeMap;

use una_syntax::Span;

use super::ty::{CapId, Declared, KindId, Refinement, Ty};

/// What an expression or pattern resolves to.
#[derive(Clone, Debug)]
pub(crate) enum Res {
    /// A call of, or reference to, a declared function, predicate or policy, with its type
    /// arguments.
    Function { id: usize, type_args: Vec<Ty> },
    /// A call of a function value.
    Value,
    /// A call of a record field of function type.
    FieldFn { kind: KindId, index: usize },
    /// A constructor: a variant, or the record constructor when `variant` is `None`.
    Ctor {
        kind: KindId,
        variant: Option<usize>,
    },
    /// `.len`.
    Len,
    /// A built-in method of an integer type, as the primitive it elaborates to (`spec/18 §4`,
    /// `§5`).
    IntMethod(una_core::Prim),
    /// `measured()`, `magnitude()` or `convert()` (ADR 0104 rule 4): erased to the receiver,
    /// multiplied by `ratio` when it is not 1 (rule 5).
    UnitMethod { ratio: u128 },
    /// A built-in method of a map that reads it (`spec/21 §2`).
    MapMethod(una_core::MapOp),
    /// `edit m.set(...)` or `edit m.remove(...)`: the map rebound to `With` or `Without`.
    MapUpdate(una_core::MapOp),
    /// A record field.
    Field { kind: KindId, index: usize },
    /// An operation of a capability.
    Op { capability: CapId },
    /// A handler installed with `with` or `given`.
    Handler { id: usize },
}

/// The records of pass 2.
#[derive(Default)]
pub(crate) struct Records {
    /// The type of each expression and binder, labels included, by its span.
    pub(crate) types: BTreeMap<Span, Ty>,
    /// What each call, name, field, constructor, operation, handler and pattern resolves to.
    pub(crate) res: BTreeMap<Span, Res>,
    /// For each call, constructor and pattern, the parameter or field of each argument or
    /// binding, in written order; a method call's receiver comes first.
    pub(crate) args: BTreeMap<Span, Vec<Option<usize>>>,
    /// The written type of each annotated `let`, by the span of its name, with its
    /// refinements.
    pub(crate) annotations: BTreeMap<Span, Declared>,
    /// The refinements of each lambda parameter, by the span of its name: its annotation's,
    /// or the expected parameter type's (ADR 0037 rule 1).
    pub(crate) lambda_params: BTreeMap<Span, Vec<Refinement>>,
    /// The types recorded before a call settled its type arguments, by span, with the type
    /// they were taken from, unresolved: a lambda's parameters and the lambda itself. Their
    /// refinements are refitted to that type's final value (ADR 0103 rule 5).
    pub(crate) refits: Vec<(Span, Ty)>,
}

impl Records {
    /// Adds `other`, with its types resolved by `resolve`.
    pub(crate) fn absorb(&mut self, other: Records, resolve: impl Fn(&Ty) -> Ty) {
        for (span, ty) in other.types {
            self.types.insert(span, resolve(&ty));
        }
        for (span, res) in other.res {
            let res = match res {
                Res::Function { id, type_args } => Res::Function {
                    id,
                    type_args: type_args.iter().map(&resolve).collect(),
                },
                other => other,
            };
            self.res.insert(span, res);
        }
        self.args.extend(other.args);
        self.annotations.extend(other.annotations);
        self.lambda_params.extend(other.lambda_params);
        for (span, from) in other.refits {
            if let Some(recorded) = self.types.get(&span) {
                let refitted = recorded.refit(&resolve(&from));
                self.types.insert(span, refitted);
            }
        }
    }
}
