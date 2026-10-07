//! Types of the type checker (`spec/03`, `spec/10 §1`).

use std::fmt;

use una_syntax::Span;

use una_core::IntType;

use super::label::Label;

/// An index into the kinds (records and sums) of the environment.
pub(crate) type KindId = usize;
/// An index into the aliases (aliases and refinements) of the environment.
pub(crate) type AliasId = usize;
/// An index into the capabilities of the environment.
pub(crate) type CapId = usize;

/// What the label rules need to know of each record and sum kind, from the types of its fields
/// and variant fields (re-review N-001, ADR 0046).
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
    /// Whether flows tell `ℓ*_T` from `ℓ_T`: in both editions since #354 (edition 0.1 read
    /// `T*` as `T` before, which let annotations drop a decision).
    pub(crate) deep_distinct: bool,
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
                s.clone()
            } else {
                format!("{s}*")
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
            deep_distinct: false,
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
            Ty::List(element) => self.written(element),
            Ty::Map(key, value) => self.written(key).join(&self.written(value)),
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
            Ty::Labeled(_, inner) => self.occurrences(inner, in_fn, out),
            Ty::Param(index) => {
                if in_fn {
                    out.push(*index);
                }
            }
            Ty::List(element) => self.occurrences(element, in_fn, out),
            Ty::Map(key, value) => {
                self.occurrences(key, in_fn, out);
                self.occurrences(value, in_fn, out);
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

/// A base type: what type checking compares (`spec/03 §2`). Refinements and labels are kept
/// beside it, in [`Declared`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
    Int,
    /// A fixed-width integer type of edition 0.2 (`spec/18`).
    Fixed(IntType),
    Bool,
    Text,
    Unit,
    Hashed,
    List(Box<Ty>),
    /// `Map[K, V]`, keyed by an `Ord` type, from edition 0.2 (`spec/21 §1`). Its keys are part
    /// of its shape, so a read takes the labels of the key position with the top label (§3).
    Map(Box<Ty>, Box<Ty>),
    /// A record or sum kind and its type arguments; nominal (`spec/03 §2.1`).
    Kind(KindId, Vec<Ty>),
    /// The type parameter with this index in the declaration being checked.
    Param(usize),
    Fn(Box<FnTy>),
    /// A unification variable.
    Var(usize),
    /// The type of `fail`, below every type (ADR 0024 rule 2).
    Never,
    /// The type of something that failed to type; compatible with everything, so one mistake
    /// yields one diagnostic.
    Error,
    /// A type whose top position carries a label above `public` (`spec/03 §5`, `spec/09 §2`).
    /// Type equality ignores labels.
    Labeled(Label, Box<Ty>),
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
    /// DECISION D-150: ADR 0037 does not say how branches join; this join never assumes a
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
        Declared::plain(Ty::Error)
    }

    /// Whether a value flowing into this type is a refinement obligation (`spec/12 §3`).
    pub(crate) fn is_refined(&self) -> bool {
        !self.refinements.is_empty()
    }
}

impl Ty {
    /// `self` with its top label raised by `label`; `Error` and `Never` carry no label.
    pub(crate) fn with_label(self, label: &Label) -> Ty {
        if label.is_public() {
            return self;
        }
        match self {
            Ty::Labeled(own, inner) => Ty::Labeled(own.join(label), inner),
            Ty::Error | Ty::Never => self,
            other => Ty::Labeled(label.clone(), Box::new(other)),
        }
    }

    /// The label of the top position, `public` when it has none.
    pub(crate) fn top_label(&self) -> Label {
        match self {
            Ty::Labeled(label, _) => label.clone(),
            _ => Label::public(),
        }
    }

    /// The type without its top label.
    pub(crate) fn unlabeled(&self) -> &Ty {
        match self {
            Ty::Labeled(_, inner) => inner,
            other => other,
        }
    }

    /// The join of the labels of every position that holds data: the top, list elements, kind
    /// arguments and the labels that the fields and variant fields of a kind declare,
    /// instantiated and through the kinds they contain (`spec/07 §7` rule 3, re-review N-001).
    /// A function type holds its top label only.
    pub(crate) fn deep_label(&self, kinds: &KindLabels) -> Label {
        match self {
            Ty::Labeled(label, inner) if kinds.deep_symbols => {
                starred(label).join(&inner.deep_label(kinds))
            }
            Ty::Labeled(label, inner) => label.join(&inner.deep_label(kinds)),
            Ty::List(element) => element.deep_label(kinds),
            Ty::Map(key, value) => key.deep_label(kinds).join(&value.deep_label(kinds)),
            Ty::Kind(id, args) => args.iter().fold(kinds.own(*id), |label, arg| {
                label.join(&arg.deep_label(kinds))
            }),
            _ => Label::public(),
        }
    }

    /// [`Ty::deep_label`] without the labels that kinds' fields declare: what Core IR records
    /// for the arguments of an operation and interpolated values, since the runtime redacts
    /// the fields of a kind value by their own declared labels (`spec/11 §5`).
    /// D-165: traces keep redacting field by field, as before N-001.
    pub(crate) fn argument_label(&self) -> Label {
        self.deep_label(&KindLabels::default())
    }

    /// The join of every label of the type, at every position: [`Ty::deep_label`] and the
    /// labels of the parameters, results and failures of its function types. An argument of an
    /// invariant kind parameter counts every label in an operation's sink, since data may flow
    /// both ways there ([`Ty::sent_label`], ADR 0052, which supersedes D-164).
    pub(crate) fn every_label(&self, kinds: &KindLabels) -> Label {
        match self {
            Ty::Labeled(label, inner) => label.join(&inner.every_label(kinds)),
            Ty::List(element) => element.every_label(kinds),
            Ty::Map(key, value) => key.every_label(kinds).join(&value.every_label(kinds)),
            Ty::Kind(id, args) => args.iter().fold(kinds.own(*id), |label, arg| {
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
            Ty::List(element) => Ty::List(Box::new(element.op_parameter(positive, top, kinds))),
            Ty::Map(key, value) => Ty::Map(
                Box::new(key.op_parameter(positive, top, kinds)),
                Box::new(value.op_parameter(positive, top, kinds)),
            ),
            Ty::Kind(id, args) => Ty::Kind(
                *id,
                args.iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        if kinds.is_invariant(*id, index) {
                            arg.clone()
                        } else {
                            arg.op_parameter(positive, top, kinds)
                        }
                    })
                    .collect(),
            ),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function
                    .params
                    .iter()
                    .map(|p| p.op_parameter(!positive, top, kinds))
                    .collect(),
                result: function.result.op_parameter(positive, top, kinds),
                uses: function.uses.clone(),
                fails: function.fails.clone(),
                refinements: function.refinements.clone(),
            })),
            other => other.clone(),
        };
        if positive {
            inner.with_label(top)
        } else {
            inner.with_label(&self.top_label())
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
            Ty::List(element) => element.sent_label(positive, kinds),
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
            Ty::List(element) => Ty::List(Box::new(element.raise_deep(label))),
            Ty::Map(key, value) => Ty::Map(
                Box::new(key.raise_deep(label)),
                Box::new(value.raise_deep(label)),
            ),
            Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(|a| a.raise_deep(label)).collect()),
            other => other.clone(),
        };
        inner.with_label(&self.top_label().join(label))
    }

    /// `self` without any label.
    pub(crate) fn erase(&self) -> Ty {
        match self {
            Ty::Labeled(_, inner) => inner.erase(),
            Ty::List(element) => Ty::List(Box::new(element.erase())),
            Ty::Map(key, value) => Ty::Map(Box::new(key.erase()), Box::new(value.erase())),
            Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(Ty::erase).collect()),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function.params.iter().map(Ty::erase).collect(),
                result: function.result.erase(),
                uses: function.uses.clone(),
                fails: function.fails.as_ref().map(Ty::erase),
                refinements: function.refinements.clone(),
            })),
            other => other.clone(),
        }
    }

    /// Replaces each type parameter by the matching argument.
    pub(crate) fn substitute(&self, args: &[Ty]) -> Ty {
        match self {
            Ty::Labeled(label, inner) => inner.substitute(args).with_label(label),
            Ty::Param(index) => args.get(*index).cloned().unwrap_or(Ty::Error),
            Ty::List(element) => Ty::List(Box::new(element.substitute(args))),
            Ty::Map(key, value) => Ty::Map(
                Box::new(key.substitute(args)),
                Box::new(value.substitute(args)),
            ),
            Ty::Kind(id, kind_args) => Ty::Kind(
                *id,
                kind_args.iter().map(|arg| arg.substitute(args)).collect(),
            ),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function.params.iter().map(|p| p.substitute(args)).collect(),
                result: function.result.substitute(args),
                uses: function.uses.clone(),
                fails: function.fails.as_ref().map(|f| f.substitute(args)),
                refinements: function.refinements.clone(),
            })),
            other => other.clone(),
        }
    }

    /// `self` with every label, at every position, replaced by `f` of it.
    pub(crate) fn map_labels(&self, f: &dyn Fn(&Label) -> Label) -> Ty {
        match self {
            Ty::Labeled(label, inner) => inner.map_labels(f).with_label(&f(label)),
            Ty::List(element) => Ty::List(Box::new(element.map_labels(f))),
            Ty::Map(key, value) => {
                Ty::Map(Box::new(key.map_labels(f)), Box::new(value.map_labels(f)))
            }
            Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(|a| a.map_labels(f)).collect()),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function.params.iter().map(|p| p.map_labels(f)).collect(),
                result: function.result.map_labels(f),
                uses: function.uses.clone(),
                fails: function.fails.clone(),
                refinements: function.refinements.clone(),
            })),
            other => other.clone(),
        }
    }

    /// `self` with the value of each type parameter labeled with its symbolic label `ℓ_T`
    /// (`spec/07 §7` rule 8, ADR 0038 D3.2): the type of a value in the body of a generic
    /// function whose type parameters are `names`.
    pub(crate) fn symbolic(&self, names: &[String]) -> Ty {
        match self {
            Ty::Labeled(label, inner) => inner.symbolic(names).with_label(label),
            Ty::Param(index) => match names.get(*index) {
                Some(name) => Ty::Param(*index).with_label(&Label::symbol(name.clone())),
                None => Ty::Param(*index),
            },
            Ty::List(element) => Ty::List(Box::new(element.symbolic(names))),
            // A key of type `T` carries `ℓ*_T` (`spec/21 §3` rule 5): it is a scalar of `T`'s
            // argument, whose every label is its top.
            Ty::Map(key, value) => Ty::Map(
                Box::new(key.symbolic(names).map_labels(&starred)),
                Box::new(value.symbolic(names)),
            ),
            Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(|a| a.symbolic(names)).collect()),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function.params.iter().map(|p| p.symbolic(names)).collect(),
                result: function.result.symbolic(names),
                uses: function.uses.clone(),
                fails: function.fails.clone(),
                refinements: function.refinements.clone(),
            })),
            other => other.clone(),
        }
    }

    /// Whether the type is second-class or contains a second-class type: a function type with
    /// `uses` or `fails` at any position (`spec/05 §6`, ADR 0038 D2).
    /// DECISION D-139: a function type whose parameter or result holds a second-class type
    /// holds one too, as `fn(fn() -> Unit uses @log: Log[write]) -> Unit` does.
    pub(crate) fn second_class(&self) -> bool {
        match self {
            Ty::Labeled(_, inner) => inner.second_class(),
            Ty::List(element) => element.second_class(),
            Ty::Map(key, value) => key.second_class() || value.second_class(),
            Ty::Kind(_, args) => args.iter().any(Ty::second_class),
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
            Ty::Labeled(_, inner) => inner.params(params),
            Ty::Param(index) => params.push(*index),
            Ty::List(element) => element.params(params),
            Ty::Map(key, value) => {
                key.params(params);
                value.params(params);
            }
            Ty::Kind(_, args) => args.iter().for_each(|arg| arg.params(params)),
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
}

/// A type printed as written in Una, for messages.
pub(crate) struct Show<'t, 'n, N: Names>(pub(crate) &'t Ty, pub(crate) &'n N);

impl<N: Names> fmt::Display for Show<'_, '_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.1;
        match self.0 {
            // Messages about types name their bases; labels have messages of their own.
            Ty::Labeled(_, inner) => write!(f, "{}", Show(inner, names)),
            Ty::Int => write!(f, "Int"),
            Ty::Fixed(int) => write!(f, "{}", int.as_str()),
            Ty::Bool => write!(f, "Bool"),
            Ty::Text => write!(f, "Text"),
            Ty::Unit => write!(f, "Unit"),
            Ty::Hashed => write!(f, "Hashed"),
            Ty::List(element) => write!(f, "List[{}]", Show(element, names)),
            Ty::Map(key, value) => {
                write!(f, "Map[{}, {}]", Show(key, names), Show(value, names))
            }
            Ty::Kind(id, args) => {
                write!(f, "{}", names.kind_name(*id))?;
                if !args.is_empty() {
                    let args: Vec<String> = args
                        .iter()
                        .map(|arg| Show(arg, names).to_string())
                        .collect();
                    write!(f, "[{}]", args.join(", "))?;
                }
                Ok(())
            }
            Ty::Param(index) => write!(f, "{}", names.param_name(*index)),
            Ty::Fn(function) => {
                let params: Vec<String> = function
                    .params
                    .iter()
                    .map(|param| Show(param, names).to_string())
                    .collect();
                write!(
                    f,
                    "fn({}) -> {}",
                    params.join(", "),
                    Show(&function.result, names)
                )
            }
            Ty::Var(_) => write!(f, "_"),
            Ty::Never => write!(f, "Never"),
            Ty::Error => write!(f, "?"),
        }
    }
}

/// A type printed with its labels, as in `secret[PII] Text` or `Option[internal Text]`, for
/// information-flow messages.
pub(crate) struct ShowLabeled<'t, 'n, N: Names>(pub(crate) &'t Ty, pub(crate) &'n N);

impl<N: Names> fmt::Display for ShowLabeled<'_, '_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.1;
        match self.0 {
            Ty::Labeled(label, inner) => write!(f, "{label} {}", ShowLabeled(inner, names)),
            Ty::List(element) => write!(f, "List[{}]", ShowLabeled(element, names)),
            Ty::Map(key, value) => write!(
                f,
                "Map[{}, {}]",
                ShowLabeled(key, names),
                ShowLabeled(value, names)
            ),
            Ty::Kind(id, args) if !args.is_empty() => {
                let args: Vec<String> = args
                    .iter()
                    .map(|arg| ShowLabeled(arg, names).to_string())
                    .collect();
                write!(f, "{}[{}]", names.kind_name(*id), args.join(", "))
            }
            Ty::Fn(function) => {
                let params: Vec<String> = function
                    .params
                    .iter()
                    .map(|param| ShowLabeled(param, names).to_string())
                    .collect();
                write!(
                    f,
                    "fn({}) -> {}",
                    params.join(", "),
                    ShowLabeled(&function.result, names)
                )
            }
            other => write!(f, "{}", Show(other, names)),
        }
    }
}
