//! Refined flows (ADR 0103 rule 4, ADR 0037 rule 2): where a value of one type flowing into
//! another compares refinements, and whether the flow is a `refinement` obligation. One rule
//! for the type checker's pending sites and the verifier's obligations, so the two agree site
//! for site (#615), over Core IR types.

use std::collections::BTreeMap;

use crate::ir::{Base, Decl, PRELUDE_MODULE, Program, Refinement, Type};

/// How a nested position compares in a flow (ADR 0103 rule 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variance {
    /// The target's refinements are proved of the value.
    Covariant,
    /// The value's refinements are proved of the target's.
    Contravariant,
    /// Both.
    Invariant,
}

impl Variance {
    /// The variance of a position of variance `inner` inside one of variance `self`.
    pub fn then(self, inner: Variance) -> Variance {
        match (self, inner) {
            (Variance::Invariant, _) | (_, Variance::Invariant) => Variance::Invariant,
            (Variance::Covariant, inner) => inner,
            (Variance::Contravariant, Variance::Covariant) => Variance::Contravariant,
            (Variance::Contravariant, Variance::Contravariant) => Variance::Covariant,
        }
    }

    /// The variance of a function's parameter inside a position of variance `self`.
    pub fn flipped(self) -> Variance {
        self.then(Variance::Contravariant)
    }
}

/// A pair of nested positions of a flow: its path (`element`, `arg 2`, as `spec/13 §5` writes
/// them), the source's type and the target's there, and its variance.
#[derive(Clone, Debug)]
pub struct Position {
    /// The path from the top, as `element.arg 2`.
    pub path: String,
    /// The type of the value at this position.
    pub from: Type,
    /// The type of the target at this position.
    pub to: Type,
    /// How the position compares.
    pub variance: Variance,
}

/// The kinds of a program, for the variance of their type arguments.
pub struct Kinds<'p> {
    kinds: BTreeMap<(&'p str, &'p str), KindShape<'p>>,
}

/// A kind's type parameters and the types of every field of its variants.
struct KindShape<'p> {
    params: &'p [String],
    fields: Vec<&'p Type>,
}

impl<'p> Kinds<'p> {
    /// The data declarations of `program`'s modules.
    pub fn new(program: &'p Program) -> Kinds<'p> {
        let mut kinds = BTreeMap::new();
        for module in &program.modules {
            for decl in &module.decls {
                if let Decl::Data {
                    name,
                    params,
                    variants,
                    ..
                } = decl
                {
                    let fields = variants
                        .iter()
                        .flat_map(|variant| variant.fields.iter().map(|(_, ty)| ty))
                        .collect();
                    kinds.insert(
                        (module.path.as_str(), name.as_str()),
                        KindShape {
                            params: params.as_slice(),
                            fields,
                        },
                    );
                }
            }
        }
        Kinds { kinds }
    }

    /// The variance of argument `index` of the type `module.name` (ADR 0103 rule 4): the
    /// arguments of the prelude's kinds are covariant, and so is a parameter of a user kind
    /// that no field holds inside a function type or under another user kind's argument; any
    /// other is invariant, which only adds obligations (ADR 0046).
    pub fn arg_variance(&self, module: &str, name: &str, index: usize) -> Variance {
        if module == PRELUDE_MODULE {
            return Variance::Covariant;
        }
        let Some(kind) = self.kinds.get(&(module, name)) else {
            return Variance::Invariant;
        };
        let Some(param) = kind.params.get(index) else {
            return Variance::Invariant;
        };
        if kind.fields.iter().all(|ty| data_only(ty, param, false)) {
            Variance::Covariant
        } else {
            Variance::Invariant
        }
    }

    /// The pairs of nested positions of `source` and `target` that have the same shape, with
    /// their paths and variances. A position's own refinements are compared; the parameters
    /// and the result of a function type, and the components of a tuple, are not positions
    /// themselves (their own refinements are [`fn_flow_pending`]'s, a binder's or a
    /// result's), only what is nested in them. Linear in the size of the types.
    pub fn nested_positions(
        &self,
        source: &Type,
        target: &Type,
        variance: Variance,
        path: &str,
        out: &mut Vec<Position>,
    ) {
        match (source, target) {
            (
                Type::Base {
                    base:
                        Base::Named {
                            module,
                            name,
                            args: from,
                        },
                    ..
                },
                Type::Base {
                    base:
                        Base::Named {
                            module: to_module,
                            name: to_name,
                            args: to,
                        },
                    ..
                },
            ) if module == to_module && name == to_name && from.len() == to.len() => {
                for (index, (a, b)) in from.iter().zip(to).enumerate() {
                    let own = if module == PRELUDE_MODULE && name == "List" {
                        "element".to_owned()
                    } else {
                        format!("arg {}", index + 1)
                    };
                    let here = if path.is_empty() {
                        own
                    } else {
                        format!("{path}.{own}")
                    };
                    let variance = variance.then(self.arg_variance(module, name, index));
                    out.push(Position {
                        path: here.clone(),
                        from: a.clone(),
                        to: b.clone(),
                        variance,
                    });
                    self.nested_positions(a, b, variance, &here, out);
                }
            }
            // A tuple, as an `edit` call's results (`spec/17 §4`): each component covariant.
            // A component's own refinements are its binder's, an `edit` rebind's or the
            // result's, proved with their facts; only the positions nested in it compare here.
            (
                Type::Base {
                    base: Base::Tuple(from),
                    ..
                },
                Type::Base {
                    base: Base::Tuple(to),
                    ..
                },
            ) if from.len() == to.len() => {
                for (index, (a, b)) in from.iter().zip(to).enumerate() {
                    let here = format!("{path}.component {}", index + 1);
                    self.nested_positions(a, b, variance, here.trim_start_matches('.'), out);
                }
            }
            (
                Type::Fn {
                    params: from,
                    result: from_result,
                    ..
                },
                Type::Fn {
                    params: to,
                    result: to_result,
                    ..
                },
            ) if from.len() == to.len() => {
                for (index, (a, b)) in from.iter().zip(to).enumerate() {
                    let here = format!("{path}.param {}", index + 1);
                    self.nested_positions(
                        a,
                        b,
                        variance.flipped(),
                        here.trim_start_matches('.'),
                        out,
                    );
                }
                let here = format!("{path}.result");
                self.nested_positions(
                    from_result,
                    to_result,
                    variance,
                    here.trim_start_matches('.'),
                    out,
                );
            }
            _ => {}
        }
    }

    /// Whether a value of type `source` flowing into `target` is a `refinement` obligation
    /// (`spec/12 §3`, ADR 0103 rule 4, ADR 0037 rule 2): `target` has refinements at its top;
    /// or at a nested position, by its variance, one side has a refinement the other does not
    /// carry; or, as a function type, a parameter of the value has a refinement the target's
    /// parameter does not carry, or the target's result one the value's result does not.
    pub fn pending(&self, source: &Type, target: &Type) -> bool {
        if !refinements(target).is_empty() || fn_flow_pending(source, target) {
            return true;
        }
        let mut positions = Vec::new();
        self.nested_positions(source, target, Variance::Covariant, "", &mut positions);
        positions.iter().any(|position| {
            let covariant = || !missing(&position.from, &position.to).is_empty();
            let contravariant = || !missing(&position.to, &position.from).is_empty();
            match position.variance {
                Variance::Covariant => covariant(),
                Variance::Contravariant => contravariant(),
                Variance::Invariant => covariant() || contravariant(),
            }
        })
    }
}

/// Whether a function value of type `source` flowing into the function type `target` has a
/// refinement to prove at a parameter or the result (ADR 0037 rule 2): a refinement of a
/// parameter of `source` that the same parameter of `target` does not carry, or one of
/// `target`'s result that `source`'s result does not, through nested function types.
pub fn fn_flow_pending(source: &Type, target: &Type) -> bool {
    match (source, target) {
        (
            Type::Fn {
                params: from,
                result: from_result,
                ..
            },
            Type::Fn {
                params: to,
                result: to_result,
                ..
            },
        ) if from.len() == to.len() => {
            from.iter().zip(to).any(|(a, b)| {
                refinements(a)
                    .iter()
                    .any(|goal| !refinements(b).contains(goal))
                    || fn_flow_pending(b, a)
            }) || refinements(to_result)
                .iter()
                .any(|goal| !refinements(from_result).contains(goal))
                || fn_flow_pending(from_result, to_result)
        }
        _ => false,
    }
}

/// The refinements of `to` that `from` does not carry, compared by their predicate.
pub fn missing<'t>(from: &Type, to: &'t Type) -> Vec<&'t Refinement> {
    refinements(to)
        .iter()
        .filter(|goal| {
            !refinements(from)
                .iter()
                .any(|held| held.clause.term == goal.clause.term)
        })
        .collect()
}

/// The refinements at the top of `ty`.
pub fn refinements(ty: &Type) -> &[Refinement] {
    match ty {
        Type::Base { refinements, .. } => refinements,
        Type::Fn { .. } => &[],
    }
}

/// Whether the type parameter `param` occurs in `ty` only as data: never inside a function
/// type, nor in an argument of a user kind (`in_kind`).
fn data_only(ty: &Type, param: &str, in_kind: bool) -> bool {
    match ty {
        Type::Base {
            base: Base::Param(name),
            ..
        } => name != param || !in_kind,
        Type::Base {
            base: Base::Named { module, args, .. },
            ..
        } => {
            let nested = in_kind || module != PRELUDE_MODULE;
            args.iter().all(|arg| data_only(arg, param, nested))
        }
        Type::Base { .. } => true,
        Type::Fn { .. } => !mentions_param(ty, param),
    }
}

/// Whether the type parameter `param` occurs anywhere in `ty`.
fn mentions_param(ty: &Type, param: &str) -> bool {
    match ty {
        Type::Base {
            base: Base::Param(name),
            ..
        } => name == param,
        Type::Base {
            base: Base::Named { args, .. },
            ..
        } => args.iter().any(|arg| mentions_param(arg, param)),
        Type::Base {
            base: Base::Tuple(types),
            ..
        } => types.iter().any(|ty| mentions_param(ty, param)),
        Type::Base { .. } => false,
        Type::Fn { params, result, .. } => {
            params.iter().any(|ty| mentions_param(ty, param)) || mentions_param(result, param)
        }
    }
}
