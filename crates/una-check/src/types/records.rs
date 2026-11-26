//! What pass 2 of the checker resolves, recorded for elaboration to Core IR (`spec/09`).

use std::collections::{BTreeMap, BTreeSet};

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
    /// `convert()` on a magnitude that is not an integer, by a ratio that is not 1 (ADR 0114
    /// rule 12.5, ADR 0120 rule 63): erased to the receiver multiplied by `ratio`, or divided
    /// by it when `divide`, as an operation of `number` with a literal `ratio`.
    Convert {
        ratio: u128,
        divide: bool,
        number: ConvertNumber,
    },
    /// A built-in method of a map that reads it (`spec/21 §2`).
    MapMethod(una_core::MapOp),
    /// A method of `Complex[F64]` or `Complex[F32]`, which elaborates to the operations of the
    /// components (`spec/29 §19`, ADR 0120 rule 23).
    Complex(super::complex::ComplexMethod),
    /// `edit m.set(...)` or `edit m.remove(...)`: the map rebound to `With` or `Without`.
    MapUpdate(una_core::MapOp),
    /// A built-in method of a `Stack` or a `Queue` that reads it (`spec/28 §5`).
    SeqMethod(una_core::SeqOp),
    /// `edit q.push(...)`, `edit q.pop()` and the other updates of a `Stack` or a `Queue`: the
    /// sequence rebound to the operation's result, and for a removal, the end it removes read
    /// first as the call's value.
    SeqUpdate(una_core::SeqOp),
    /// A built-in method of a `Set` that reads it or gives a new set (`spec/28 §6`).
    SetMethod(una_core::SetOp),
    /// `edit s.add(...)` or `edit s.remove(...)`: the set rebound to `With` or `Without`.
    SetUpdate(una_core::SetOp),
    /// A built-in method of a `HashMap` that reads it or gives a new map (`spec/28 §7`).
    HashMapMethod(una_core::HashMapOp),
    /// `edit m.set(...)` or `edit m.remove(...)` on a `HashMap`: the map rebound to `With` or
    /// `Without`.
    HashMapUpdate(una_core::HashMapOp),
    /// A built-in method of a `HashSet` that reads it or gives a new set (`spec/28 §7`).
    HashSetMethod(una_core::HashSetOp),
    /// `edit s.add(...)` or `edit s.remove(...)` on a `HashSet`: the set rebound to `With` or
    /// `Without`.
    HashSetUpdate(una_core::HashSetOp),
    /// A built-in method of a `PriorityQueue` that reads it (`spec/28 §8`).
    PriorityMethod(una_core::PriorityOp),
    /// `edit q.push(...)`, `edit q.pop()` or `edit q.pop_ranked()`: the queue rebound to `op`'s
    /// result, and for a removal, the first item read first by `read` as the call's value.
    PriorityUpdate {
        op: una_core::PriorityOp,
        read: Option<una_core::PriorityOp>,
    },
    /// A built-in method of a `BitSet` that reads it or gives a new bit set, or its property
    /// `size` (`spec/28 §9`).
    BitSetMethod(una_core::BitSetOp),
    /// `edit b.add(...)` or `edit b.remove(...)`: the bit set rebound to `With` or `Without`.
    BitSetUpdate(una_core::BitSetOp),
    /// A record field.
    Field { kind: KindId, index: usize },
    /// An operation of a capability.
    Op { capability: CapId },
    /// A handler installed with `with` or `given`.
    Handler { id: usize },
    /// A built-in constant of `std.time`, elaborated to its value (ADR 0116 rule 0, amended
    /// 2026-10-19).
    Constant(i64),
    /// A module constant, edition 0.5 (ADR 0126 rule 8): an index of the environment's
    /// `constants`, elaborated to its value.
    Const(usize),
    /// `m.rows`, `m.columns`, `t.rank` or `f.size` of a dense value (`spec/29 §17`, ADR 0120
    /// rules 33 and 38).
    Shape(una_core::ShapeFun),
    /// `a.get(index: i)` or `m.get(row: i, column: j)` in a clause (ADR 0120 rule 33): the
    /// element term, with the position of each axis's argument in the written arguments.
    DenseElement(Vec<usize>),
    /// `t.get(at: [i, ...])` in a clause (ADR 0120 rule 33): the element term of a tensor,
    /// with the position of the argument whose list literal holds the indices.
    TensorElement(usize),
    /// `t.dim(axis: k)` in a clause (ADR 0120 rule 33), with the position of the axis
    /// argument.
    Dim(usize),
    /// An operator of dense values, elaborated as `map` or `zip_map` (the prelude function
    /// `id`) with a lambda that applies the element type's operator (ADR 0120 rule 34).
    DenseOp { id: usize, op: DenseOp },
}

/// The magnitude of a [`Res::Convert`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConvertNumber {
    /// `F64`: `*` or `/` by the `F64` literal of the ratio.
    F64,
    /// `F32`: `*` or `/` by the `F32` literal of the ratio.
    F32,
    /// `Decimal[s]`: `*` by the `Decimal[0]` literal of the ratio, which keeps the scale.
    Decimal(u8),
}

/// The operator of a [`Res::DenseOp`], as its lambda applies it to each element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DenseOp {
    /// `x + y` of two dense values.
    Add,
    /// `x - y` of two dense values.
    Sub,
    /// `-x`.
    Neg,
    /// `k * x` or `x * k`, with the scalar first or last as written.
    Scale { scalar_first: bool },
    /// `x / k`.
    Divide,
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
    /// For each call that passes the elements of a list literal as its arguments, the position
    /// of that argument among the written ones: a tensor's shape, index or permutation, whose
    /// length selects the function of its rank (`spec/29 §17`).
    pub(crate) spread: BTreeMap<Span, usize>,
    /// The refinements of each lambda parameter, by the span of its name: its annotation's,
    /// or the expected parameter type's (ADR 0037 rule 1).
    pub(crate) lambda_params: BTreeMap<Span, Vec<Refinement>>,
    /// The types recorded before a call settled its type arguments, by span, with the type
    /// they were taken from, unresolved: a lambda's parameters and the lambda itself. Their
    /// refinements are refitted to that type's final value (ADR 0103 rule 5).
    pub(crate) refits: Vec<(Span, Ty)>,
    /// Each flow of a value into a type, in edition `0.3`, for the pending `refinement`
    /// obligations that ADR 0103 adds: where tier 1 generates one, as
    /// `una_core::flow::Kinds::pending` decides over Core IR (#615).
    pub(crate) refined_flows: Vec<RefinedFlow>,
    /// The span of each call of a function that computes new elements (ADR 0120,
    /// `super::dense::SYNTHESIZING`): the value it gives keeps no refinement of the element
    /// type, even of a type argument solved to a refined type after the call.
    pub(crate) synthesized: BTreeSet<Span>,
    /// The failure of each `e within d`, `Timed[E]` or `Deadline` of `std.time`, by the span of
    /// `d` (ADR 0117 rule 5).
    pub(crate) withins: BTreeMap<Span, Ty>,
    /// The function's own error kind of each `e within d` whose `if error pass` passes the
    /// time-out as its `TimedOut`, by the span of `d` (ADR 0117 rule 5).
    pub(crate) within_passes: BTreeMap<Span, Ty>,
    /// For each operation call, by its span, the types whose labels, joined, are the label its
    /// outcome is rendered at in a run's trace (S-45, #1552, `spec/11 §5`): the decision's
    /// label on `Unit`, the result and the failure type.
    pub(crate) outcomes: BTreeMap<Span, Vec<Ty>>,
}

/// A flow of a value at `span` into `target`, in the module `module` and the function
/// `function`, whose type parameters name the types' parameters.
#[derive(Clone, Debug)]
pub(crate) struct RefinedFlow {
    pub(crate) span: Span,
    pub(crate) module: usize,
    pub(crate) function: Option<usize>,
    pub(crate) target: Ty,
    pub(crate) source: Ty,
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
        self.spread.extend(other.spread);
        for (span, ty) in other.withins {
            self.withins.insert(span, resolve(&ty));
        }
        for (span, ty) in other.within_passes {
            self.within_passes.insert(span, resolve(&ty));
        }
        for (span, parts) in other.outcomes {
            self.outcomes
                .entry(span)
                .or_default()
                .extend(parts.iter().map(&resolve));
        }
        self.annotations.extend(other.annotations);
        self.lambda_params.extend(other.lambda_params);
        // A lambda's own type takes the refinements of the type arguments its call settles
        // on, in its flow as in its record (ADR 0103 rule 5).
        let refitted: BTreeMap<Span, Ty> = other
            .refits
            .iter()
            .map(|(span, from)| (*span, resolve(from)))
            .collect();
        for flow in other.refined_flows {
            let source = resolve(&flow.source);
            let source = match refitted.get(&flow.span) {
                Some(settled) => source.refit(settled),
                None => source,
            };
            // ADR 0120: the result of a call that computes new elements is the base of its
            // type arguments as they finally resolve, as elaboration gives it, so a refinement
            // solved after the call is never carried into the flow.
            let source = if other.synthesized.contains(&flow.span) {
                source.without_refinements()
            } else {
                source
            };
            self.refined_flows.push(RefinedFlow {
                target: resolve(&flow.target),
                source,
                ..flow
            });
        }
        for (span, from) in other.refits {
            if let Some(recorded) = self.types.get(&span) {
                let refitted = recorded.refit(&resolve(&from));
                self.types.insert(span, refitted);
            }
        }
    }
}
