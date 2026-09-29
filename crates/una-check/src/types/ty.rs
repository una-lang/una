//! Types of the type checker (`spec/03`, `spec/10 §1`).

use std::fmt;

use una_syntax::Span;

use super::label::Label;

/// An index into the kinds (records and sums) of the environment.
pub(crate) type KindId = usize;
/// An index into the aliases (aliases and refinements) of the environment.
pub(crate) type AliasId = usize;
/// An index into the capabilities of the environment.
pub(crate) type CapId = usize;

/// A base type: what type checking compares (`spec/03 §2`). Refinements and labels are kept
/// beside it, in [`Declared`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
    Int,
    Bool,
    Text,
    Unit,
    Hashed,
    List(Box<Ty>),
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
    /// Type equality ignores labels (DECISION D-088).
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

    /// The join of the labels of every position that holds data: the top, list elements and
    /// kind arguments. A function type holds its top label only.
    pub(crate) fn deep_label(&self) -> Label {
        match self {
            Ty::Labeled(label, inner) => label.join(&inner.deep_label()),
            Ty::List(element) => element.deep_label(),
            Ty::Kind(_, args) => args
                .iter()
                .fold(Label::public(), |label, arg| label.join(&arg.deep_label())),
            _ => Label::public(),
        }
    }

    /// `self` with every position that holds data raised by `label`.
    pub(crate) fn raise_deep(&self, label: &Label) -> Ty {
        let inner = match self.unlabeled() {
            Ty::List(element) => Ty::List(Box::new(element.raise_deep(label))),
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
            Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(Ty::erase).collect()),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function.params.iter().map(Ty::erase).collect(),
                result: function.result.erase(),
                uses: function.uses.clone(),
                fails: function.fails.as_ref().map(Ty::erase),
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
            Ty::Kind(id, kind_args) => Ty::Kind(
                *id,
                kind_args.iter().map(|arg| arg.substitute(args)).collect(),
            ),
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function.params.iter().map(|p| p.substitute(args)).collect(),
                result: function.result.substitute(args),
                uses: function.uses.clone(),
                fails: function.fails.as_ref().map(|f| f.substitute(args)),
            })),
            other => other.clone(),
        }
    }

    /// Adds the indices of the type parameters that occur in the type to `params`.
    pub(crate) fn params(&self, params: &mut Vec<usize>) {
        match self {
            Ty::Labeled(_, inner) => inner.params(params),
            Ty::Param(index) => params.push(*index),
            Ty::List(element) => element.params(params),
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
            Ty::Bool => write!(f, "Bool"),
            Ty::Text => write!(f, "Text"),
            Ty::Unit => write!(f, "Unit"),
            Ty::Hashed => write!(f, "Hashed"),
            Ty::List(element) => write!(f, "List[{}]", Show(element, names)),
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
