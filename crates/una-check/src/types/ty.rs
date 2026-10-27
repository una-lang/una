//! Types of the type checker (`spec/03`, `spec/10 §1`).

use std::fmt;

use una_syntax::Span;

use una_core::{Coll, IntType};

use super::label::Label;
pub(crate) use super::shared::{Flagged, Flags, Shared};

/// An index into the kinds (records and sums) of the environment.
pub(crate) type KindId = usize;
/// An index into the aliases (aliases and refinements) of the environment.
pub(crate) type AliasId = usize;
/// An index into the capabilities of the environment.
pub(crate) type CapId = usize;

/// What the label rules need to know of each record and sum kind, from the types of its fields
/// and variant fields (ADR 0046).
/// D-168: the labels a kind's fields declare count at positions that hold data, as
/// in [`Ty::deep_label`]; the labels inside a field of function type do not, since every use of
/// the field's function reads them from its declared type. The type arguments of a kind count
/// whatever positions their parameters occupy, as before.
#[derive(Clone, Debug, Default)]
pub(crate) struct KindLabels {
    /// For each kind, the join of the labels its fields declare at positions that hold data,
    /// through the kinds they contain: part of the deep label of every value of the kind,
    /// whatever its type arguments.
    own: Vec<Label>,
    /// For each kind, which of its type parameters are invariant for labels: those that occur
    /// inside a function type, or under an invariant parameter of another kind (ADR 0046
    /// rule 1). The others are covariant.
    invariant: Vec<Vec<bool>>,
    /// Whether a deep label marks the symbolic labels it joins as deep, `ℓ*_T` (written `T*`),
    /// apart from the label of a value of `T` itself, `ℓ_T` (ADR 0067 amendment): in edition
    /// 0.2, where result summaries tell them apart.
    pub(crate) deep_symbols: bool,
}

/// `label` with each symbolic label of a type parameter `T` written as `T*`: the join of every
/// label of `T`'s type argument, as a deep label takes it (ADR 0067 amendment).
pub(crate) fn starred(label: &Label) -> Label {
    if !label
        .symbols
        .iter()
        .any(|s| !s.starts_with('?') && !s.ends_with('*'))
    {
        return label.clone();
    }
    let mut out = label.clone();
    out.symbols = label
        .symbols
        .iter()
        .map(|s| {
            if s.starts_with('?') || s.ends_with('*') {
                std::sync::Arc::clone(s)
            } else {
                una_core::Names::name(&format!("{s}*"))
            }
        })
        .collect();
    out
}

impl KindLabels {
    /// The labels of the kinds whose field types are `fields`, by kind: the least solution
    /// over recursive and mutually recursive kinds (ADR 0046 rule 1).
    pub(crate) fn new(fields: &[Vec<&Ty>], params: &[usize]) -> KindLabels {
        let mut labels = KindLabels {
            own: vec![Label::public(); fields.len()],
            invariant: params.iter().map(|count| vec![false; *count]).collect(),
            deep_symbols: false,
        };
        loop {
            let mut changed = false;
            for (id, types) in fields.iter().enumerate() {
                let mut own = Label::public();
                let mut invariant = Vec::new();
                for ty in types {
                    own = own.join(&labels.written(ty));
                    labels.occurrences(ty, false, &mut invariant);
                }
                if let Some(slot) = labels.own.get_mut(id)
                    && *slot != slot.join(&own)
                {
                    *slot = slot.join(&own);
                    changed = true;
                }
                if let Some(slots) = labels.invariant.get_mut(id) {
                    for index in invariant {
                        if let Some(slot) = slots.get_mut(index)
                            && !*slot
                        {
                            *slot = true;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                return labels;
            }
        }
    }

    /// The join of the labels written in `ty` at positions that hold data, and of the kinds
    /// it contains there.
    fn written(&self, ty: &Ty) -> Label {
        match ty {
            Ty::Labeled(label, inner) => label.join(&self.written(inner)),
            Ty::Refined(_, inner) => self.written(inner),
            Ty::List(element) | Ty::Dense(_, element) => self.written(element),
            Ty::Map(key, value) => self.written(key).join(&self.written(value)),
            Ty::Coll(_, args) => args
                .iter()
                .fold(Label::public(), |label, arg| label.join(&self.written(arg))),
            Ty::Kind(id, args) => args
                .iter()
                .fold(self.own(*id), |label, arg| label.join(&self.written(arg))),
            _ => Label::public(),
        }
    }

    /// Adds to `out` the type parameters that occur in `ty` in an invariant position:
    /// inside a function type (`in_fn`), or under an invariant parameter of a kind.
    fn occurrences(&self, ty: &Ty, in_fn: bool, out: &mut Vec<usize>) {
        match ty {
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => self.occurrences(inner, in_fn, out),
            Ty::Param(index) => {
                if in_fn {
                    out.push(*index);
                }
            }
            Ty::List(element) | Ty::Dense(_, element) => self.occurrences(element, in_fn, out),
            Ty::Map(key, value) => {
                self.occurrences(key, in_fn, out);
                self.occurrences(value, in_fn, out);
            }
            Ty::Coll(_, args) => {
                for arg in args.iter() {
                    self.occurrences(arg, in_fn, out);
                }
            }
            Ty::Kind(id, args) => {
                for (index, arg) in args.iter().enumerate() {
                    self.occurrences(arg, in_fn || self.is_invariant(*id, index), out);
                }
            }
            Ty::Fn(function) => {
                for param in &function.params {
                    self.occurrences(param, true, out);
                }
                self.occurrences(&function.result, true, out);
                if let Some(fails) = &function.fails {
                    self.occurrences(fails, true, out);
                }
            }
            _ => {}
        }
    }

    /// The labels that the fields of kind `id` declare, as in [`KindLabels::new`].
    pub(crate) fn own(&self, id: KindId) -> Label {
        self.own.get(id).cloned().unwrap_or_else(Label::public)
    }

    /// Whether type parameter `index` of kind `id` is invariant for labels (ADR 0046).
    pub(crate) fn is_invariant(&self, id: KindId, index: usize) -> bool {
        self.invariant
            .get(id)
            .and_then(|params| params.get(index))
            .copied()
            .unwrap_or(false)
    }
}

/// Where an error type comes from (#581).
///
/// `Unknown` is the type of a reference to a name that does not resolve: U1001 is reported at
/// it by name resolution, or the name is one a broken declaration would have declared (ADR
/// 0096 rule 8). A package that holds one is rejected already, so the diagnostics that would
/// only follow from it are not reported: an inference variable it reaches, a lambda it gives
/// no parameter type. Every other error type is `Other`, and absorbs nothing more than it did:
/// a call of a `Never` value, for one, is not an unknown name, and its lambda argument still
/// reports U2010.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ErrorOrigin {
    /// A reference to a name that does not resolve.
    Unknown,
    /// Any other failure to type.
    Other,
}

/// Two error types are the same type, whatever their origins: the origin decides only which
/// diagnostics follow, never what types compare.
impl PartialEq for ErrorOrigin {
    fn eq(&self, _: &ErrorOrigin) -> bool {
        true
    }
}

impl Eq for ErrorOrigin {}

/// A base type: what type checking compares (`spec/03 §2`). Refinements and labels are kept
/// beside it, in [`Declared`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
    Int,
    /// A fixed-width integer type of edition 0.2 (`spec/18`).
    Fixed(IntType),
    /// `F64`, IEEE binary64, edition 0.3 (`spec/29`, ADR 0114).
    F64,
    /// `Decimal[s]`, an exact decimal of scale s from 0 to 38, edition 0.4 (`spec/29 §16`,
    /// ADR 0120 rule 12).
    Decimal(u8),
    /// `F32`, IEEE binary32, edition 0.4 (`spec/29`, ADR 0120 rule 5).
    F32,
    Bool,
    Text,
    Unit,
    Hashed,
    List(Shared<Ty>),
    /// A dense value of edition 0.4, `Array[T]` or `Matrix[T]` (`spec/29 §17`, ADR 0120
    /// rules 26 to 30): its shape is the top position, its elements the element position, so
    /// its labels are a list's.
    Dense(Dense, Shared<Ty>),
    /// `Map[K, V]`, keyed by an `Ord` type, from edition 0.2 (`spec/21 §1`). Its keys are part
    /// of its shape, so a read takes the labels of the key position with the top label (§3).
    Map(Shared<Ty>, Shared<Ty>),
    /// A collection type of `std.collections` and its type arguments, edition 0.4 (ADR 0119).
    /// The arguments that are part of its shape ([`Coll::is_shape`]) are read with its top
    /// label, as a map's keys are.
    Coll(Coll, Shared<[Ty]>),
    /// A record or sum kind and its type arguments; nominal (`spec/03 §2.1`).
    Kind(KindId, Shared<[Ty]>),
    /// The type parameter with this index in the declaration being checked.
    Param(usize),
    Fn(Shared<FnTy>),
    /// A unification variable.
    Var(usize),
    /// The type of `fail`, below every type (ADR 0024 rule 2).
    Never,
    /// The type of something that failed to type; compatible with everything, so one mistake
    /// yields one diagnostic. Its origin says whether it comes from a name that does not
    /// resolve (#581); two error types are equal whatever their origins.
    Error(ErrorOrigin),
    /// A type whose top position carries a label above `public` (`spec/03 §5`, `spec/09 §2`).
    /// Type equality ignores labels.
    Labeled(Label, Shared<Ty>),
    /// A number type with a unit of measure (ADR 0104 rule 2): `Int` or a fixed-width integer
    /// type from edition 0.3, `F64` from edition 0.3 (ADR 0114 rule 12), and `F32`,
    /// `Decimal[s]`, `Complex[F64]` and `Complex[F32]` in edition 0.4 (ADR 0120 rule 61). The
    /// magnitude is never labeled itself (a label wraps the measured type). A type of its own:
    /// it equals another measured type with the same unit and magnitude type, and nothing else.
    Measured(Shared<super::units::Unit>, Shared<Ty>),
    /// A nested position whose values satisfy `refinements` (ADR 0103): an argument of a type
    /// written or instantiated with a refined kind or a range, in edition 0.3. It sits inside
    /// its position's label, `Labeled(ℓ, Refined(r, base))`, and the top of a declared type
    /// keeps its refinements in [`Declared`]. Type equality and unification ignore it, as they
    /// ignore labels; the verifier proves every flow into it and assumes it where values are
    /// read from it.
    Refined(Shared<[Refinement]>, Shared<Ty>),
}

/// The dense types of edition 0.4 (`spec/29 §17`, ADR 0120 rules 26 and 30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Dense {
    /// `Array[T]`, the one fixed-length array, of any element type.
    Array,
    /// `Matrix[T]`, of a numeric element type.
    Matrix,
}

impl Dense {
    /// The type's name, as source writes it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Dense::Array => "Array",
            Dense::Matrix => "Matrix",
        }
    }
}

/// A function type `fn(params) -> result uses ... fails ...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FnTy {
    pub(crate) params: Vec<Ty>,
    pub(crate) result: Ty,
    /// The capabilities of its `uses` clauses: calling a value of this type needs them.
    pub(crate) uses: Vec<FnUse>,
    /// The error kind of `fails`.
    pub(crate) fails: Option<Ty>,
    /// The refinements of the parameters and the result (ADR 0037 rule 1).
    pub(crate) refinements: FnRefinements,
}

/// The refinements of a function type's parameters and result, from its written form, the
/// header of a named function used as a value, or a lambda's annotations (ADR 0037 rule 1).
/// They sit beside the base types, as in [`Declared`]: type equality compares bases, so two
/// function types are equal whatever their refinements, and the verifier proves the flows
/// between them (`spec/12 §3`).
#[derive(Clone, Debug, Default, Eq)]
pub(crate) struct FnRefinements {
    /// The refinements of each parameter, by position; a missing entry has none.
    pub(crate) params: Vec<Vec<Refinement>>,
    /// The refinements of the result.
    pub(crate) result: Vec<Refinement>,
}

impl PartialEq for FnRefinements {
    /// Always equal: refinements are not part of the base that type checking compares.
    fn eq(&self, _: &FnRefinements) -> bool {
        true
    }
}

impl FnRefinements {
    /// The refinements of parameter `index`.
    pub(crate) fn param(&self, index: usize) -> &[Refinement] {
        self.params.get(index).map_or(&[], Vec::as_slice)
    }

    /// The refinements of a join (`join`) or a meet (not `join`) of two function types:
    /// a join accepts what both accept, so its parameters carry the refinements of either and
    /// its result those both share; a meet the other way round. Every value of either type is
    /// then of the join, and the flows the verifier checks are trivially implied.
    /// D-150: ADR 0037 does not say how branches join; this join never assumes a
    /// refinement that one branch lacks.
    pub(crate) fn combine(&self, other: &FnRefinements, join: bool) -> FnRefinements {
        let count = self.params.len().max(other.params.len());
        let params = (0..count)
            .map(|index| mix(self.param(index), other.param(index), !join))
            .collect();
        FnRefinements {
            params,
            result: mix(&self.result, &other.result, join),
        }
    }
}

/// The refinements that `a` and `b` share (`common`), or those of either, in order.
fn mix(a: &[Refinement], b: &[Refinement], common: bool) -> Vec<Refinement> {
    if common {
        a.iter().filter(|r| b.contains(r)).cloned().collect()
    } else {
        let mut out = a.to_vec();
        out.extend(b.iter().filter(|r| !a.contains(r)).cloned());
        out
    }
}

/// The refinements of a nested position where two types meet (ADR 0103 rule 5): those both
/// carry (`common`), as a join of covariant positions keeps, or those of either, as a meet
/// keeps. Two ranges are the same when they read the same, wherever they are written.
pub(crate) fn mix_nested(
    a: Option<&Shared<[Refinement]>>,
    b: Option<&Shared<[Refinement]>>,
    common: bool,
) -> Option<Shared<[Refinement]>> {
    let same = |x: &Refinement, y: &Refinement| match (x, y) {
        (Refinement::Range(x, _), Refinement::Range(y, _)) => x == y,
        _ => x == y,
    };
    let (a, b): (&[Refinement], &[Refinement]) =
        (a.map_or(&[][..], |a| &a[..]), b.map_or(&[][..], |b| &b[..]));
    let out: Vec<Refinement> = if common {
        a.iter()
            .filter(|r| b.iter().any(|q| same(r, q)))
            .cloned()
            .collect()
    } else {
        let mut out = a.to_vec();
        out.extend(b.iter().filter(|r| !a.iter().any(|q| same(r, q))).cloned());
        out
    };
    (!out.is_empty()).then(|| out.into_iter().collect())
}

/// A capability that a function type or a function requires, `uses @name: C[ops]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FnUse {
    pub(crate) name: String,
    /// The capability, when its name resolves to one.
    pub(crate) capability: Option<CapId>,
    /// The operations it needs, in the capability's declaration order.
    pub(crate) ops: Vec<String>,
}

/// A refinement contributed by an alias kind, with the alias's type arguments, or by range
/// sugar written on a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refinement {
    Alias(AliasId, Vec<Ty>),
    /// Range sugar, as written: `in 0..5000` or `in 1..<20`, and where it is written.
    Range(String, Span),
}

impl Refinement {
    /// The refinement with each type parameter of its alias's arguments replaced by the
    /// matching argument: a refinement over a type parameter, as `NonEmpty[T]`, instantiated.
    pub(crate) fn substitute(&self, args: &[Ty]) -> Refinement {
        match self {
            Refinement::Alias(id, alias_args) => Refinement::Alias(
                *id,
                alias_args.iter().map(|arg| arg.substitute(args)).collect(),
            ),
            Refinement::Range(..) => self.clone(),
        }
    }
}

/// A written type: its base, the refinements it carries and whether it has a label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Declared {
    pub(crate) base: Ty,
    pub(crate) refinements: Vec<Refinement>,
}

impl Declared {
    pub(crate) fn plain(base: Ty) -> Declared {
        Declared {
            base,
            refinements: Vec::new(),
        }
    }

    pub(crate) fn error() -> Declared {
        Declared::plain(Ty::error())
    }

    /// The declared type of a type name that does not resolve (#581).
    pub(crate) fn unknown() -> Declared {
        Declared::plain(Ty::unknown())
    }

    /// Whether a value flowing into this type is a refinement obligation (`spec/12 §3`).
    pub(crate) fn is_refined(&self) -> bool {
        !self.refinements.is_empty()
    }
}

/// `new` as a child node: `old` itself when `new` is the same value, so an operation that
/// changes nothing below a node allocates nothing for it (ADR 0077 stage 2b).
pub(crate) fn reuse(old: &Shared<Ty>, new: Ty) -> Shared<Ty> {
    if new.same(old) {
        old.clone()
    } else {
        Shared::new(new)
    }
}

/// Each of `items` mapped by `f`, or `None` when every result is the same value as its item:
/// nothing is allocated until a first item changes.
pub(crate) fn map_same(items: &[Ty], mut f: impl FnMut(&Ty) -> Ty) -> Option<Vec<Ty>> {
    let mut out: Option<Vec<Ty>> = None;
    for (index, item) in items.iter().enumerate() {
        let new = f(item);
        match &mut out {
            Some(out) => out.push(new),
            None if new.same(item) => {}
            None => {
                let mut changed = items.get(..index).unwrap_or(&[]).to_vec();
                changed.push(new);
                out = Some(changed);
            }
        }
    }
    out
}

/// The arguments of a kind mapped by `f`: `args` itself when nothing changes.
pub(crate) fn map_args(args: &Shared<[Ty]>, f: impl FnMut(&Ty) -> Ty) -> Shared<[Ty]> {
    match map_same(args, f) {
        None => args.clone(),
        Some(changed) => changed.into_iter().collect(),
    }
}

/// A function type with its parameters, result and failure kind mapped by `f`, and its `uses`
/// and refinements kept: `function` itself when none of them changes. Refinements are copied
/// as they are, so keeping the node keeps them.
pub(crate) fn map_function(
    function: &Shared<FnTy>,
    mut param: impl FnMut(&Ty) -> Ty,
    mut result: impl FnMut(&Ty) -> Ty,
    fails: Option<&dyn Fn(&Ty) -> Ty>,
) -> Shared<FnTy> {
    let params = map_same(&function.params, &mut param);
    let new_result = result(&function.result);
    let new_fails = match (fails, &function.fails) {
        (Some(map), Some(kind)) => {
            let new = map(kind);
            (!new.same(kind)).then_some(new)
        }
        _ => None,
    };
    if params.is_none() && new_result.same(&function.result) && new_fails.is_none() {
        return function.clone();
    }
    Shared::new(FnTy {
        params: params.unwrap_or_else(|| function.params.clone()),
        result: new_result,
        uses: function.uses.clone(),
        fails: new_fails.or_else(|| function.fails.clone()),
        refinements: function.refinements.clone(),
    })
}

impl Flagged for Ty {
    /// From the flags of the children's nodes, in O(1) but for a kind's arguments, whose node
    /// carries their flags too.
    fn flags(&self) -> Flags {
        match self {
            Ty::Var(_) => Flags {
                var: true,
                ..Flags::LEAF
            },
            Ty::List(element) | Ty::Dense(_, element) => element.flags(),
            Ty::Map(key, value) => key.flags().and(value.flags()),
            Ty::Kind(_, args) | Ty::Coll(_, args) => args.flags(),
            Ty::Fn(function) => function.flags(),
            Ty::Refined(_, inner) => Flags {
                refined: true,
                ..inner.flags()
            },
            Ty::Labeled(label, inner) => {
                let own = Flags {
                    var: false,
                    refined: false,
                    label: true,
                    var_label: label.symbols.iter().any(|s| s.starts_with('?')),
                    normal: !label.is_public()
                        && !matches!(**inner, Ty::Labeled(..) | Ty::Error(_) | Ty::Never),
                    unit: false,
                };
                own.and(inner.flags())
            }
            // A measured type holds a magnitude type, with no variable and no label (ADR 0104).
            Ty::Measured(..) => Flags {
                unit: true,
                ..Flags::LEAF
            },
            Ty::Int
            | Ty::Fixed(_)
            | Ty::Bool
            | Ty::Text
            | Ty::Unit
            | Ty::Hashed
            | Ty::F64
            | Ty::Decimal(_)
            | Ty::F32
            | Ty::Param(_)
            | Ty::Never
            | Ty::Error(_) => Flags::LEAF,
        }
    }
}

impl Flagged for [Ty] {
    fn flags(&self) -> Flags {
        self.iter()
            .fold(Flags::LEAF, |flags, ty| flags.and(ty.flags()))
    }
}

impl Flagged for FnTy {
    /// The parameters, the result and the failure kind: what the operations walk. `uses` and
    /// refinements hold no type that an operation changes.
    fn flags(&self) -> Flags {
        let flags = self.params.flags().and(self.result.flags());
        match &self.fails {
            Some(fails) => flags.and(fails.flags()),
            None => flags,
        }
    }
}

impl Ty {
    /// The error type of a failure to type that is not an unknown name.
    pub(crate) fn error() -> Ty {
        Ty::Error(ErrorOrigin::Other)
    }

    /// The error type of a reference to a name that does not resolve (#581).
    pub(crate) fn unknown() -> Ty {
        Ty::Error(ErrorOrigin::Unknown)
    }

    /// Whether this is the error type of a name that does not resolve, under its labels.
    pub(crate) fn is_unknown(&self) -> bool {
        matches!(self.unlabeled(), Ty::Error(ErrorOrigin::Unknown))
    }

    /// Whether `self` and `other` are the same value by identity: the same variant, with the
    /// same nodes as children and equal labels. Only ever a fast path to "unchanged" (ADR 0077
    /// stage 2b): `false` says nothing about equality.
    pub(crate) fn same(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::List(a), Ty::List(b)) => Shared::ptr_eq(a, b),
            (Ty::Dense(x, a), Ty::Dense(y, b)) => x == y && Shared::ptr_eq(a, b),
            (Ty::Map(a, b), Ty::Map(c, d)) => Shared::ptr_eq(a, c) && Shared::ptr_eq(b, d),
            (Ty::Kind(i, a), Ty::Kind(j, b)) => i == j && Shared::ptr_eq(a, b),
            (Ty::Coll(i, a), Ty::Coll(j, b)) => i == j && Shared::ptr_eq(a, b),
            (Ty::Fn(a), Ty::Fn(b)) => Shared::ptr_eq(a, b),
            (Ty::Labeled(l, a), Ty::Labeled(m, b)) => Shared::ptr_eq(a, b) && l == m,
            (Ty::Refined(r, a), Ty::Refined(q, b)) => Shared::ptr_eq(a, b) && r == q,
            (Ty::Int, Ty::Int)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Text, Ty::Text)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Hashed, Ty::Hashed)
            | (Ty::F64, Ty::F64)
            | (Ty::F32, Ty::F32)
            | (Ty::Never, Ty::Never)
            | (Ty::Error(_), Ty::Error(_)) => true,
            (Ty::Fixed(a), Ty::Fixed(b)) => a == b,
            (Ty::Decimal(a), Ty::Decimal(b)) => a == b,
            (Ty::Measured(u, a), Ty::Measured(v, b)) => u == v && a == b,
            (Ty::Param(a), Ty::Param(b)) | (Ty::Var(a), Ty::Var(b)) => a == b,
            _ => false,
        }
    }

    /// `inner.with_label(label)`, where `self` was the type before an operation and `inner` is
    /// the operation's result under `self`'s top label: `self` itself when that is the same
    /// value, that is when `self` is a labeled node in the form `with_label` builds whose
    /// inner node and label the operation left as they were.
    pub(crate) fn relabeled(&self, inner: Ty, label: &Label) -> Ty {
        // `inner` was computed from `self.unlabeled()`, below `self`'s refinements (ADR 0103):
        // they wrap it again, so no operation on labels drops a refinement.
        let inner = match (self.refinements(), &inner) {
            (Some(_), Ty::Refined(..) | Ty::Error(_) | Ty::Never) | (None, _) => inner,
            (Some(refinements), _) => Ty::Refined(refinements.clone(), Shared::new(inner)),
        };
        if self.keeps(&inner, label) {
            self.clone()
        } else {
            inner.with_label(label)
        }
    }

    /// Whether `inner.with_label(label)` is `self` itself: `self` is a labeled node in the form
    /// `with_label` builds, with `label` and with `inner` as the node it labels.
    pub(crate) fn keeps(&self, inner: &Ty, label: &Label) -> bool {
        matches!(self, Ty::Labeled(own, old)
            if own == label
                && inner.same(old)
                && !own.is_public()
                && !matches!(**old, Ty::Labeled(..) | Ty::Error(_) | Ty::Never))
    }

    /// `self` with its top label raised by `label`; `Error` and `Never` carry no label.
    pub(crate) fn with_label(self, label: &Label) -> Ty {
        if label.is_public() {
            return self;
        }
        match self {
            Ty::Labeled(own, inner) => Ty::Labeled(own.join(label), inner),
            Ty::Error(_) | Ty::Never => self,
            other => Ty::Labeled(label.clone(), Shared::new(other)),
        }
    }

    /// The label of the top position, `public` when it has none.
    pub(crate) fn top_label(&self) -> Label {
        match self {
            Ty::Labeled(label, _) => label.clone(),
            _ => Label::public(),
        }
    }

    /// The type without its top label, and without the refinements of its position (ADR
    /// 0103): the shape at its top. [`Ty::relabeled`] puts the refinements back.
    pub(crate) fn unlabeled(&self) -> &Ty {
        match self {
            Ty::Labeled(_, inner) => match &**inner {
                Ty::Refined(_, base) => base,
                other => other,
            },
            Ty::Refined(_, base) => base,
            other => other,
        }
    }

    /// The numeric type of a value, its label and unit set aside: `Int` for `Int of Meter`
    /// (ADR 0104 rule 2), where the unit does not matter, as for a range's bounds or the
    /// solver's sort. Any other type is returned unlabeled.
    pub(crate) fn magnitude(&self) -> &Ty {
        match self.unlabeled() {
            Ty::Measured(_, inner) => inner,
            other => other,
        }
    }

    /// The measured type of `unit` over `magnitude`, a number type without a label: `Int`, a
    /// fixed-width type, `F64`, `F32`, `Decimal[s]` or a complex kind. A label wraps the
    /// measured type, never its magnitude (ADR 0104, ADR 0077, ADR 0120 rule 61).
    pub(crate) fn measured(unit: super::units::Unit, magnitude: Ty) -> Ty {
        debug_assert!(
            matches!(
                magnitude,
                Ty::Int | Ty::Fixed(_) | Ty::F64 | Ty::F32 | Ty::Decimal(_) | Ty::Kind(..)
            ),
            "a magnitude is an unlabeled number type: {magnitude:?}"
        );
        Ty::Measured(Shared::new(unit), Shared::new(magnitude))
    }

    /// `self`, a number type, measured in `unit` in place of any unit it has, its top label
    /// kept outside the unit (ADR 0104, ADR 0120 rule 61). No unit, or the unit one, leaves
    /// the magnitude unitless; a type that is not a number type is returned as it is, so an
    /// error type stays one.
    pub(crate) fn in_unit(self, unit: Option<&super::units::Unit>) -> Ty {
        let magnitude = self.magnitude().clone();
        let number = match &magnitude {
            Ty::Int | Ty::Fixed(_) | Ty::F64 | Ty::F32 | Ty::Decimal(_) => true,
            // A complex kind has no type argument (ADR 0120 rule 22).
            Ty::Kind(_, args) => args.is_empty(),
            _ => false,
        };
        if !number {
            return self;
        }
        let label = self.top_label();
        match unit {
            Some(unit) if !unit.is_one() => {
                Ty::measured(unit.clone(), magnitude).with_label(&label)
            }
            _ => magnitude.with_label(&label),
        }
    }

    /// The type with every unit removed, at every position (ADR 0104): what a unit-only
    /// mismatch leaves equal.
    pub(crate) fn without_units(&self) -> Ty {
        if !self.has_units() {
            return self.clone();
        }
        match self {
            Ty::Measured(_, inner) => inner.without_units(),
            Ty::Refined(refinements, inner) => {
                Ty::Refined(refinements.clone(), Shared::new(inner.without_units()))
            }
            Ty::Labeled(label, inner) => {
                Ty::Labeled(label.clone(), Shared::new(inner.without_units()))
            }
            Ty::List(element) => Ty::List(Shared::new(element.without_units())),
            Ty::Dense(dense, element) => Ty::Dense(*dense, Shared::new(element.without_units())),
            Ty::Map(key, value) => Ty::Map(
                Shared::new(key.without_units()),
                Shared::new(value.without_units()),
            ),
            Ty::Kind(id, args) => Ty::Kind(
                *id,
                args.iter()
                    .map(Ty::without_units)
                    .collect::<Vec<_>>()
                    .into(),
            ),
            Ty::Coll(coll, args) => Ty::Coll(
                *coll,
                args.iter()
                    .map(Ty::without_units)
                    .collect::<Vec<_>>()
                    .into(),
            ),
            Ty::Fn(function) => Ty::Fn(Shared::new(FnTy {
                params: function.params.iter().map(Ty::without_units).collect(),
                result: function.result.without_units(),
                ..(**function).clone()
            })),
            other => other.clone(),
        }
    }

    /// Whether a unit appears anywhere in the type: its flags, O(1) over shared nodes.
    pub(crate) fn has_units(&self) -> bool {
        self.flags().unit
    }

    /// The unit of a measured type (ADR 0104), its label set aside.
    pub(crate) fn unit(&self) -> Option<&super::units::Unit> {
        match self.unlabeled() {
            Ty::Measured(unit, _) => Some(unit),
            _ => None,
        }
    }

    /// The refinements of the type's top position, when it is a refined nested position (ADR
    /// 0103), under its label.
    pub(crate) fn refinements(&self) -> Option<&Shared<[Refinement]>> {
        match self {
            Ty::Labeled(_, inner) => match &**inner {
                Ty::Refined(refinements, _) => Some(refinements),
                _ => None,
            },
            Ty::Refined(refinements, _) => Some(refinements),
            _ => None,
        }
    }

    /// `self` with the refinements of `final_ty` at each position the two share, its own
    /// labels and shape kept: a lambda parameter's type as its body assumed it, refitted to
    /// the type argument that the call settles on (ADR 0103 rule 5). `fold`'s accumulator
    /// assumed `Pos` from `initial` until `step`'s `Int` result made it `Int`. Linear in the
    /// size of the types.
    pub(crate) fn refit(&self, final_ty: &Ty) -> Ty {
        if !self.flags().refined && !final_ty.flags().refined {
            return self.clone();
        }
        let label = self.top_label();
        let shape = match (self.unlabeled(), final_ty.unlabeled()) {
            (Ty::List(a), Ty::List(b)) => Ty::List(Shared::new(a.refit(b))),
            (Ty::Dense(dense, a), Ty::Dense(_, b)) => Ty::Dense(*dense, Shared::new(a.refit(b))),
            (Ty::Map(ak, av), Ty::Map(bk, bv)) => {
                Ty::Map(Shared::new(ak.refit(bk)), Shared::new(av.refit(bv)))
            }
            (Ty::Kind(i, a), Ty::Kind(j, b)) if i == j && a.len() == b.len() => Ty::Kind(
                *i,
                a.iter().zip(b.iter()).map(|(x, y)| x.refit(y)).collect(),
            ),
            (Ty::Coll(i, a), Ty::Coll(j, b)) if i == j && a.len() == b.len() => Ty::Coll(
                *i,
                a.iter().zip(b.iter()).map(|(x, y)| x.refit(y)).collect(),
            ),
            (Ty::Fn(a), Ty::Fn(b)) if a.params.len() == b.params.len() => {
                Ty::Fn(Shared::new(FnTy {
                    params: a
                        .params
                        .iter()
                        .zip(&b.params)
                        .map(|(x, y)| x.refit(y))
                        .collect(),
                    result: a.result.refit(&b.result),
                    ..(**a).clone()
                }))
            }
            (own, _) => own.clone(),
        };
        let shape = match final_ty.refinements() {
            Some(refinements) if !matches!(shape, Ty::Error(_) | Ty::Never) => {
                Ty::Refined(refinements.clone(), Shared::new(shape))
            }
            _ => shape,
        };
        shape.with_label(&label)
    }

    /// `self` with every refinement removed, at every position (ADR 0103 rule 5): the base of
    /// a type, as an inferred type argument that its values do not agree on, or an editable
    /// binding without an annotation.
    pub(crate) fn without_refinements(&self) -> Ty {
        if !self.flags().refined {
            return self.clone();
        }
        match self {
            Ty::Refined(_, inner) => inner.without_refinements(),
            Ty::Labeled(label, inner) => {
                let new = inner.without_refinements();
                if new.same(inner) {
                    self.clone()
                } else {
                    new.with_label(label)
                }
            }
            Ty::List(element) => Ty::List(reuse(element, element.without_refinements())),
            Ty::Dense(dense, element) => {
                Ty::Dense(*dense, reuse(element, element.without_refinements()))
            }
            Ty::Map(key, value) => Ty::Map(
                reuse(key, key.without_refinements()),
                reuse(value, value.without_refinements()),
            ),
            Ty::Kind(id, args) => Ty::Kind(*id, map_args(args, Ty::without_refinements)),
            Ty::Coll(coll, args) => Ty::Coll(*coll, map_args(args, Ty::without_refinements)),
            Ty::Fn(function) => Ty::Fn(map_function(
                function,
                Ty::without_refinements,
                Ty::without_refinements,
                None,
            )),
            other => other.clone(),
        }
    }

    /// The join of the labels of every position that holds data: the top, list elements, kind
    /// arguments and the labels that the fields and variant fields of a kind declare,
    /// instantiated and through the kinds they contain (`spec/07 §7` rule 3).
    /// A function type holds its top label only.
    pub(crate) fn deep_label(&self, kinds: &KindLabels) -> Label {
        match self {
            Ty::Labeled(label, inner) if kinds.deep_symbols => {
                starred(label).join(&inner.deep_label(kinds))
            }
            Ty::Labeled(label, inner) => label.join(&inner.deep_label(kinds)),
            Ty::Refined(_, inner) => inner.deep_label(kinds),
            Ty::List(element) | Ty::Dense(_, element) => element.deep_label(kinds),
            Ty::Map(key, value) => key.deep_label(kinds).join(&value.deep_label(kinds)),
            Ty::Kind(id, args) => args.iter().fold(kinds.own(*id), |label, arg| {
                label.join(&arg.deep_label(kinds))
            }),
            Ty::Coll(_, args) => args.iter().fold(Label::public(), |label, arg| {
                label.join(&arg.deep_label(kinds))
            }),
            _ => Label::public(),
        }
    }

    /// [`Ty::deep_label`] without the labels that kinds' fields declare: what Core IR records
    /// for the arguments of an operation, since the runtime redacts the fields of a kind value
    /// by their own declared labels (`spec/11 §5`).
    /// D-165: traces keep redacting field by field, as before kinds' fields were labeled (ADR 0046).
    /// A value computed from a whole value of the type, such as its interpolated text, has no
    /// fields left to redact, so its label is [`Ty::computed_label`] instead (S-27).
    ///
    /// The label sits on a position that is not a type parameter, an argument or a `Text`
    /// temporary, so a symbolic label it joins is starred: a plain `ℓ_T` labels only a position
    /// of type `T` (`spec/07 §7` rule 8, the formal model's `LTy.InPlace`, #440). Redaction asks
    /// only whether the label is `public`, which starring does not change.
    pub(crate) fn argument_label(&self) -> Label {
        starred(&self.deep_label(&KindLabels::default()))
    }

    /// The label of a value computed from a whole value of this type: the join of every label of
    /// the type, deep, the labels that kinds' fields declare included, as the checker labels it.
    /// The text that interpolating the value writes renders those fields without redaction in
    /// edition 0.2 and later (`spec/10 §12` rule 14), and a comparison with the value depends on
    /// all of it, so leaving the fields' labels out would export the result under a lower label
    /// than what it holds (S-27). Starred as [`Ty::argument_label`] is.
    pub(crate) fn computed_label(&self, kinds: &KindLabels) -> Label {
        starred(&self.deep_label(kinds))
    }

    /// The join of every label of the type, at every position: [`Ty::deep_label`] and the
    /// labels of the parameters, results and failures of its function types. An argument of an
    /// invariant kind parameter counts every label in an operation's sink, since data may flow
    /// both ways there ([`Ty::sent_label`], ADR 0052, which supersedes D-164).
    pub(crate) fn every_label(&self, kinds: &KindLabels) -> Label {
        match self {
            Ty::Labeled(label, inner) => label.join(&inner.every_label(kinds)),
            Ty::Refined(_, inner) => inner.every_label(kinds),
            Ty::List(element) | Ty::Dense(_, element) => element.every_label(kinds),
            Ty::Map(key, value) => key.every_label(kinds).join(&value.every_label(kinds)),
            Ty::Kind(id, args) => args.iter().fold(kinds.own(*id), |label, arg| {
                label.join(&arg.every_label(kinds))
            }),
            Ty::Coll(_, args) => args.iter().fold(Label::public(), |label, arg| {
                label.join(&arg.every_label(kinds))
            }),
            Ty::Fn(function) => function
                .params
                .iter()
                .chain([&function.result])
                .chain(function.fails.as_ref())
                .fold(Label::public(), |label, ty| {
                    label.join(&ty.every_label(kinds))
                }),
            _ => Label::public(),
        }
    }

    /// The declared type of an operation parameter as an operation argument flows into it
    /// (ADR 0050): every covariant position (`positive`) is `top`, since `accepts` bounds the
    /// data that the caller sends there; contravariant positions, the parameters of its
    /// function types, and invariant kind arguments keep their declared labels.
    pub(crate) fn op_parameter(&self, positive: bool, top: &Label, kinds: &KindLabels) -> Ty {
        let inner = match self.unlabeled() {
            Ty::Dense(dense, element) => Ty::Dense(
                *dense,
                reuse(element, element.op_parameter(positive, top, kinds)),
            ),
            Ty::List(element) => {
                Ty::List(reuse(element, element.op_parameter(positive, top, kinds)))
            }
            Ty::Map(key, value) => Ty::Map(
                reuse(key, key.op_parameter(positive, top, kinds)),
                reuse(value, value.op_parameter(positive, top, kinds)),
            ),
            Ty::Kind(id, args) => {
                let mut index = 0;
                Ty::Kind(
                    *id,
                    map_args(args, |arg| {
                        let mapped = if kinds.is_invariant(*id, index) {
                            arg.clone()
                        } else {
                            arg.op_parameter(positive, top, kinds)
                        };
                        index += 1;
                        mapped
                    }),
                )
            }
            Ty::Coll(coll, args) => Ty::Coll(
                *coll,
                map_args(args, |arg| arg.op_parameter(positive, top, kinds)),
            ),
            Ty::Fn(function) => Ty::Fn(map_function(
                function,
                |p| p.op_parameter(!positive, top, kinds),
                |r| r.op_parameter(positive, top, kinds),
                None,
            )),
            other => other.clone(),
        };
        if positive {
            self.relabeled(inner, top)
        } else {
            self.relabeled(inner, &self.top_label())
        }
    }

    /// `ℓ⁺`: the join of the labels at the covariant positions of the type (`positive`), which
    /// an operation argument sends to the handler and its sink bounds (ADR 0052,
    /// `spec/07 §7` rule 4): the top, list elements, covariant kind
    /// arguments and the labels their kinds' fields declare, function results, and the
    /// parameters of function parameters. An argument of an invariant kind parameter counts
    /// every label, since data flows both ways there. The contravariant positions, which carry
    /// the handler's data to the caller's function, are compared by the flow of ADR 0050.
    pub(crate) fn sent_label(&self, positive: bool, kinds: &KindLabels) -> Label {
        let own = if positive {
            self.top_label()
        } else {
            Label::public()
        };
        let inner = match self.unlabeled() {
            Ty::List(element) | Ty::Dense(_, element) => element.sent_label(positive, kinds),
            Ty::Map(key, value) => key
                .sent_label(positive, kinds)
                .join(&value.sent_label(positive, kinds)),
            Ty::Kind(id, args) => args.iter().enumerate().fold(
                if positive {
                    kinds.own(*id)
                } else {
                    Label::public()
                },
                |label, (index, arg)| {
                    label.join(&if kinds.is_invariant(*id, index) {
                        arg.every_label(kinds)
                    } else {
                        arg.sent_label(positive, kinds)
                    })
                },
            ),
            Ty::Coll(_, args) => args.iter().fold(Label::public(), |label, arg| {
                label.join(&arg.sent_label(positive, kinds))
            }),
            Ty::Fn(function) => function.params.iter().fold(
                function.result.sent_label(positive, kinds),
                |label, param| label.join(&param.sent_label(!positive, kinds)),
            ),
            _ => Label::public(),
        };
        own.join(&inner)
    }

    /// `self` with every position that holds data raised by `label`.
    pub(crate) fn raise_deep(&self, label: &Label) -> Ty {
        let inner = match self.unlabeled() {
            Ty::List(element) => Ty::List(reuse(element, element.raise_deep(label))),
            Ty::Dense(dense, element) => {
                Ty::Dense(*dense, reuse(element, element.raise_deep(label)))
            }
            Ty::Map(key, value) => Ty::Map(
                reuse(key, key.raise_deep(label)),
                reuse(value, value.raise_deep(label)),
            ),
            Ty::Kind(id, args) => Ty::Kind(*id, map_args(args, |a| a.raise_deep(label))),
            Ty::Coll(coll, args) => Ty::Coll(*coll, map_args(args, |a| a.raise_deep(label))),
            other => other.clone(),
        };
        self.relabeled(inner, &self.top_label().join(label))
    }

    /// `self` without any label.
    /// The checker erases through [`super::unify::Vars::erase`], which memoizes this per
    /// declaration; the tests compare it with this.
    #[cfg(test)]
    pub(crate) fn erase(&self) -> Ty {
        // Erasing changes only labeled positions (ADR 0077 stage 3a).
        if !self.flags().label {
            return self.clone();
        }
        match self {
            Ty::Labeled(_, inner) => inner.erase(),
            Ty::Refined(refinements, inner) => {
                Ty::Refined(refinements.clone(), reuse(inner, inner.erase()))
            }
            Ty::List(element) => Ty::List(reuse(element, element.erase())),
            Ty::Dense(dense, element) => Ty::Dense(*dense, reuse(element, element.erase())),
            Ty::Map(key, value) => Ty::Map(reuse(key, key.erase()), reuse(value, value.erase())),
            Ty::Kind(id, args) => Ty::Kind(*id, map_args(args, Ty::erase)),
            Ty::Coll(coll, args) => Ty::Coll(*coll, map_args(args, Ty::erase)),
            Ty::Fn(function) => Ty::Fn(map_function(
                function,
                Ty::erase,
                Ty::erase,
                Some(&Ty::erase),
            )),
            other => other.clone(),
        }
    }

    /// Replaces each type parameter by the matching argument.
    pub(crate) fn substitute(&self, args: &[Ty]) -> Ty {
        match self {
            Ty::Labeled(label, inner) => self.relabeled(inner.substitute(args), label),
            // A refinement over a type parameter, as `NonEmpty[T]`, is instantiated too.
            Ty::Refined(refinements, inner) => Ty::Refined(
                refinements
                    .iter()
                    .map(|refinement| refinement.substitute(args))
                    .collect(),
                reuse(inner, inner.substitute(args)),
            ),
            Ty::Param(index) => args.get(*index).cloned().unwrap_or(Ty::error()),
            Ty::List(element) => Ty::List(reuse(element, element.substitute(args))),
            Ty::Dense(dense, element) => {
                Ty::Dense(*dense, reuse(element, element.substitute(args)))
            }
            Ty::Map(key, value) => Ty::Map(
                reuse(key, key.substitute(args)),
                reuse(value, value.substitute(args)),
            ),
            Ty::Kind(id, kind_args) => {
                Ty::Kind(*id, map_args(kind_args, |arg| arg.substitute(args)))
            }
            Ty::Coll(coll, coll_args) => {
                Ty::Coll(*coll, map_args(coll_args, |arg| arg.substitute(args)))
            }
            Ty::Fn(function) => Ty::Fn(map_function(
                function,
                |p| p.substitute(args),
                |r| r.substitute(args),
                Some(&|f: &Ty| f.substitute(args)),
            )),
            other => other.clone(),
        }
    }

    /// `self` with every label, at every position, replaced by `f` of it.
    pub(crate) fn map_labels(&self, f: &dyn Fn(&Label) -> Label) -> Ty {
        match self {
            Ty::Labeled(label, inner) => {
                // The inner node first, then the label, as before: `f` may gather what it sees.
                let mapped = inner.map_labels(f);
                self.relabeled(mapped, &f(label))
            }
            Ty::Refined(refinements, inner) => {
                Ty::Refined(refinements.clone(), reuse(inner, inner.map_labels(f)))
            }
            Ty::List(element) => Ty::List(reuse(element, element.map_labels(f))),
            Ty::Dense(dense, element) => Ty::Dense(*dense, reuse(element, element.map_labels(f))),
            Ty::Map(key, value) => Ty::Map(
                reuse(key, key.map_labels(f)),
                reuse(value, value.map_labels(f)),
            ),
            Ty::Kind(id, args) => Ty::Kind(*id, map_args(args, |a| a.map_labels(f))),
            Ty::Coll(coll, args) => Ty::Coll(*coll, map_args(args, |a| a.map_labels(f))),
            Ty::Fn(function) => Ty::Fn(map_function(
                function,
                |p| p.map_labels(f),
                |r| r.map_labels(f),
                None,
            )),
            other => other.clone(),
        }
    }

    /// `self` with the value of each type parameter labeled with its symbolic label `ℓ_T`
    /// (`spec/07 §7` rule 8, ADR 0038 D3.2): the type of a value in the body of a generic
    /// function whose type parameters are `names`.
    pub(crate) fn symbolic(&self, names: &[String]) -> Ty {
        match self {
            Ty::Labeled(label, inner) => self.relabeled(inner.symbolic(names), label),
            Ty::Refined(refinements, inner) => {
                Ty::Refined(refinements.clone(), reuse(inner, inner.symbolic(names)))
            }
            Ty::Param(index) => match names.get(*index) {
                Some(name) => Ty::Param(*index).with_label(&Label::symbol(name.clone())),
                None => Ty::Param(*index),
            },
            Ty::List(element) => Ty::List(reuse(element, element.symbolic(names))),
            Ty::Dense(dense, element) => Ty::Dense(*dense, reuse(element, element.symbolic(names))),
            // A key of type `T` carries `ℓ*_T` (`spec/21 §3` rule 5): it is a scalar of `T`'s
            // argument, whose every label is its top.
            Ty::Map(key, value) => Ty::Map(
                reuse(key, key.symbolic(names).map_labels(&starred)),
                reuse(value, value.symbolic(names)),
            ),
            Ty::Kind(id, args) => Ty::Kind(*id, map_args(args, |a| a.symbolic(names))),
            // The positions of a collection's shape carry `ℓ*_T`, as a map's keys do (ADR 0119
            // IF2 to IF4).
            Ty::Coll(coll, args) => {
                let mut index = 0;
                Ty::Coll(
                    *coll,
                    map_args(args, |a| {
                        let mapped = if coll.is_shape(index) {
                            a.symbolic(names).map_labels(&starred)
                        } else {
                            a.symbolic(names)
                        };
                        index += 1;
                        mapped
                    }),
                )
            }
            Ty::Fn(function) => Ty::Fn(map_function(
                function,
                |p| p.symbolic(names),
                |r| r.symbolic(names),
                None,
            )),
            other => other.clone(),
        }
    }

    /// Whether the type is second-class or contains a second-class type: a function type with
    /// `uses` or `fails` at any position (`spec/05 §6`, ADR 0038 D2).
    /// D-139: a function type whose parameter or result holds a second-class type
    /// holds one too, as `fn(fn() -> Unit uses @log: Log[write]) -> Unit` does.
    pub(crate) fn second_class(&self) -> bool {
        match self {
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => inner.second_class(),
            Ty::List(element) | Ty::Dense(_, element) => element.second_class(),
            Ty::Map(key, value) => key.second_class() || value.second_class(),
            Ty::Kind(_, args) | Ty::Coll(_, args) => args.iter().any(Ty::second_class),
            Ty::Fn(function) => {
                !function.uses.is_empty()
                    || function.fails.is_some()
                    || function.params.iter().any(Ty::second_class)
                    || function.result.second_class()
            }
            _ => false,
        }
    }

    /// Adds the indices of the type parameters that occur in the type to `params`.
    pub(crate) fn params(&self, params: &mut Vec<usize>) {
        match self {
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => inner.params(params),
            Ty::Param(index) => params.push(*index),
            Ty::List(element) | Ty::Dense(_, element) => element.params(params),
            Ty::Map(key, value) => {
                key.params(params);
                value.params(params);
            }
            Ty::Kind(_, args) | Ty::Coll(_, args) => args.iter().for_each(|arg| arg.params(params)),
            Ty::Fn(function) => {
                function
                    .params
                    .iter()
                    .for_each(|param| param.params(params));
                function.result.params(params);
            }
            _ => {}
        }
    }
}

/// Names for printing types in messages.
pub(crate) trait Names {
    fn kind_name(&self, id: KindId) -> String;
    fn param_name(&self, index: usize) -> String;
    /// How a refined nested position is written (ADR 0103): the name of the refined kind its
    /// last refinement comes from, or its base followed by its range. `None` prints the base.
    fn refined_name(&self, refinements: &[Refinement], base: &str) -> Option<String> {
        match refinements.last()? {
            Refinement::Range(range, _) => Some(format!("{base} {range}")),
            Refinement::Alias(..) => None,
        }
    }
}

/// The shape of a type node, as the printers read it (ADR 0080): the checker's [`Ty`] and the
/// editor index's [`super::hover::HoverType`] both give it, so [`Show`] and [`ShowLabeled`]
/// print both alike, and a hover's text is the text a message gives the same type.
pub(crate) enum View<'t, T> {
    Labeled(&'t Label, &'t T),
    /// A refined nested position (ADR 0103).
    Refined(&'t [Refinement], &'t T),
    Int,
    Fixed(IntType),
    /// A measured type: its unit and its magnitude type (ADR 0104).
    Measured(&'t super::units::Unit, &'t T),
    Bool,
    Text,
    Unit,
    Hashed,
    F64,
    Decimal(u8),
    F32,
    List(&'t T),
    Dense(Dense, &'t T),
    Map(&'t T, &'t T),
    /// A collection type of `std.collections` (ADR 0119).
    Coll(Coll, &'t [T]),
    Kind(KindId, &'t [T]),
    Param(usize),
    /// A function type's parameters and result: its `uses`, `fails` and refinements are not
    /// printed.
    Fn(&'t [T], &'t T),
    Var,
    Never,
    Error,
}

/// A tree of types that the printers can read (ADR 0080).
pub(crate) trait TypeView: Sized {
    /// The shape of this node, borrowing its children.
    fn view(&self) -> View<'_, Self>;
}

impl TypeView for Ty {
    fn view(&self) -> View<'_, Ty> {
        match self {
            Ty::Labeled(label, inner) => View::Labeled(label, inner),
            Ty::Refined(refinements, inner) => View::Refined(refinements, inner),
            Ty::Int => View::Int,
            Ty::Fixed(int) => View::Fixed(*int),
            Ty::Measured(unit, inner) => View::Measured(unit, inner),
            Ty::Bool => View::Bool,
            Ty::Text => View::Text,
            Ty::Unit => View::Unit,
            Ty::Hashed => View::Hashed,
            Ty::F64 => View::F64,
            Ty::Decimal(scale) => View::Decimal(*scale),
            Ty::F32 => View::F32,
            Ty::List(element) => View::List(element),
            Ty::Dense(dense, element) => View::Dense(*dense, element),
            Ty::Map(key, value) => View::Map(key, value),
            Ty::Kind(id, args) => View::Kind(*id, args),
            Ty::Coll(coll, args) => View::Coll(*coll, args),
            Ty::Param(index) => View::Param(*index),
            Ty::Fn(function) => View::Fn(&function.params, &function.result),
            Ty::Var(_) => View::Var,
            Ty::Never => View::Never,
            Ty::Error(_) => View::Error,
        }
    }
}

/// A type printed as written in Una, for messages.
pub(crate) struct Show<'t, 'n, T: TypeView, N: Names>(pub(crate) &'t T, pub(crate) &'n N);

impl<T: TypeView, N: Names> fmt::Display for Show<'_, '_, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.1;
        match self.0.view() {
            // Messages about types name their bases; labels have messages of their own.
            View::Labeled(_, inner) => write!(f, "{}", Show(inner, names)),
            View::Refined(refinements, inner) => {
                let base = Show(inner, names).to_string();
                match names.refined_name(refinements, &base) {
                    Some(name) => write!(f, "{name}"),
                    None => write!(f, "{base}"),
                }
            }
            View::Int => write!(f, "Int"),
            View::Fixed(int) => write!(f, "{}", int.as_str()),
            View::Measured(unit, inner) => write!(f, "{} of {unit}", Show(inner, names)),
            View::Bool => write!(f, "Bool"),
            View::Text => write!(f, "Text"),
            View::Unit => write!(f, "Unit"),
            View::Hashed => write!(f, "Hashed"),
            View::F64 => write!(f, "F64"),
            View::Decimal(scale) => write!(f, "Decimal[{scale}]"),
            View::F32 => write!(f, "F32"),
            View::List(element) => write!(f, "List[{}]", Show(element, names)),
            View::Dense(dense, element) => {
                write!(f, "{}[{}]", dense.as_str(), Show(element, names))
            }
            View::Map(key, value) => {
                write!(f, "Map[{}, {}]", Show(key, names), Show(value, names))
            }
            View::Kind(id, args) => {
                write!(f, "{}", names.kind_name(id))?;
                if !args.is_empty() {
                    let args: Vec<String> = args
                        .iter()
                        .map(|arg| Show(arg, names).to_string())
                        .collect();
                    write!(f, "[{}]", args.join(", "))?;
                }
                Ok(())
            }
            View::Coll(coll, args) => {
                write!(f, "{}", coll.as_str())?;
                if !args.is_empty() {
                    let args: Vec<String> = args
                        .iter()
                        .map(|arg| Show(arg, names).to_string())
                        .collect();
                    write!(f, "[{}]", args.join(", "))?;
                }
                Ok(())
            }
            View::Param(index) => write!(f, "{}", names.param_name(index)),
            View::Fn(params, result) => {
                let params: Vec<String> = params
                    .iter()
                    .map(|param| Show(param, names).to_string())
                    .collect();
                write!(f, "fn({}) -> {}", params.join(", "), Show(result, names))
            }
            View::Var => write!(f, "_"),
            View::Never => write!(f, "Never"),
            View::Error => write!(f, "?"),
        }
    }
}

/// A type printed with its labels, as in `secret[PII] Text` or `Option[internal Text]`, for
/// information-flow messages and the editor's hovers.
pub(crate) struct ShowLabeled<'t, 'n, T: TypeView, N: Names>(pub(crate) &'t T, pub(crate) &'n N);

impl<T: TypeView, N: Names> fmt::Display for ShowLabeled<'_, '_, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.1;
        match self.0.view() {
            View::Labeled(label, inner) => write!(f, "{label} {}", ShowLabeled(inner, names)),
            View::Refined(refinements, inner) => {
                let base = ShowLabeled(inner, names).to_string();
                match names.refined_name(refinements, &base) {
                    Some(name) => write!(f, "{name}"),
                    None => write!(f, "{base}"),
                }
            }
            View::List(element) => write!(f, "List[{}]", ShowLabeled(element, names)),
            View::Dense(dense, element) => {
                write!(f, "{}[{}]", dense.as_str(), ShowLabeled(element, names))
            }
            View::Map(key, value) => write!(
                f,
                "Map[{}, {}]",
                ShowLabeled(key, names),
                ShowLabeled(value, names)
            ),
            View::Kind(id, args) if !args.is_empty() => {
                let args: Vec<String> = args
                    .iter()
                    .map(|arg| ShowLabeled(arg, names).to_string())
                    .collect();
                write!(f, "{}[{}]", names.kind_name(id), args.join(", "))
            }
            View::Coll(coll, args) if !args.is_empty() => {
                let args: Vec<String> = args
                    .iter()
                    .map(|arg| ShowLabeled(arg, names).to_string())
                    .collect();
                write!(f, "{}[{}]", coll.as_str(), args.join(", "))
            }
            View::Fn(params, result) => {
                let params: Vec<String> = params
                    .iter()
                    .map(|param| ShowLabeled(param, names).to_string())
                    .collect();
                write!(
                    f,
                    "fn({}) -> {}",
                    params.join(", "),
                    ShowLabeled(result, names)
                )
            }
            _ => write!(f, "{}", Show(self.0, names)),
        }
    }
}
