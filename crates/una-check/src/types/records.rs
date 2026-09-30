//! What pass 2 of the checker resolves, recorded for elaboration to Core IR (`spec/09`,
//! DECISION D-098).

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
    /// refinements (DECISION D-111).
    pub(crate) annotations: BTreeMap<Span, Declared>,
    /// The refinements of each lambda parameter, by the span of its name: its annotation's,
    /// or the expected parameter type's (ADR 0037 rule 1).
    pub(crate) lambda_params: BTreeMap<Span, Vec<Refinement>>,
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
    }

    /// Adds records whose types are already resolved.
    pub(crate) fn merge(&mut self, other: Records) {
        self.types.extend(other.types);
        self.res.extend(other.res);
        self.args.extend(other.args);
        self.annotations.extend(other.annotations);
        self.lambda_params.extend(other.lambda_params);
    }
}
