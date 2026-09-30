//! Instantiations of type parameters (ADR 0038 D2, `spec/05 §6`, `spec/10 §5` generic
//! instantiation): a type parameter is never instantiated with a second-class type, a function
//! type with `uses` or `fails`, or a type that contains one (U3016).

use una_diag::Code;
use una_syntax::Span;

use super::Report;
use super::authority::Pass;
use super::check::Checker;
use super::ty::Ty;
use super::unify::Vars;

/// An instantiation of the type parameters of a generic function, kind or list, recorded when
/// it is checked and judged when the declaration ends, once inference has solved its type
/// arguments.
#[derive(Clone, Debug)]
pub(crate) struct Instantiation {
    /// The generic function or kind, for messages.
    pub(crate) of: String,
    /// The names of its type parameters.
    pub(crate) params: Vec<String>,
    /// The type arguments, usually variables.
    pub(crate) args: Vec<Ty>,
    /// Each value given to it, with the declared type it flows into, written with the type
    /// parameters, and the value's own type.
    pub(crate) values: Vec<(Span, Ty, Ty)>,
    /// Where it is instantiated: the call, constructor, list or function value.
    pub(crate) at: Span,
}

impl Checker<'_, '_, '_> {
    /// Records an instantiation of `params` of `of` with `args` (in pass 2, where U3016 is
    /// reported).
    pub(crate) fn instantiation(
        &mut self,
        of: &str,
        params: &[&str],
        args: Vec<Ty>,
        values: Vec<(Span, Ty, Ty)>,
        at: Span,
    ) {
        if self.pass != Pass::Check {
            return;
        }
        self.instantiations.push(Instantiation {
            of: of.to_owned(),
            params: params.iter().map(|p| (*p).to_owned()).collect(),
            args,
            values,
            at,
        });
    }

    /// Records the argument at `span`, of type `found`, of a call of a function value whose
    /// parameter type `param` is a type argument of an instantiation, as the generic function
    /// `id` bound by `let i = id` and called as `i(note)`: a value of that instantiation, where
    /// U3016 is reported when its own type is second-class (re-review N-011).
    pub(crate) fn instantiated_by(&mut self, span: Span, param: &Ty, found: &Ty) {
        let Ty::Var(var) = param.unlabeled().clone() else {
            return;
        };
        let head = |ty: &Ty| matches!(ty.unlabeled(), Ty::Var(v) if *v == var);
        if let Some((instantiation, index)) =
            self.instantiations
                .iter_mut()
                .rev()
                .find_map(|instantiation| {
                    let index = instantiation.args.iter().position(head)?;
                    Some((instantiation, index))
                })
        {
            instantiation
                .values
                .push((span, Ty::Param(index), found.clone()));
        }
    }
}

/// U3016 for each instantiation whose solved type argument is or contains a second-class type:
/// at the first value whose declared type mentions the type parameter and whose own type is
/// second-class, else at the first value whose declared type mentions it, or at the
/// instantiation when none does (re-review N-011, DECISION D-TBD-l8). Instantiations are
/// judged in the order they were checked, inner ones first, and one whose site contains a site already reported is not reported
/// again, so that `Box(value: id(note))` yields one diagnostic, at `note`.
/// DECISION D-138: the site is the first value whose declared type mentions the type
/// parameter, and a site already reported, by U3016 or by U3006 for an escaping function
/// value, is not reported again.
pub(crate) fn second_class(
    report: &mut Report<'_>,
    vars: &Vars,
    instantiations: &[Instantiation],
    reported: &mut Vec<Span>,
) {
    for instantiation in instantiations {
        for (index, arg) in instantiation.args.iter().enumerate() {
            if !vars.resolve(arg).second_class() {
                continue;
            }
            let mentions = |declared: &Ty| {
                let mut params = Vec::new();
                declared.params(&mut params);
                params.contains(&index)
            };
            let site = instantiation
                .values
                .iter()
                .find(|(_, declared, found)| {
                    mentions(declared) && vars.resolve(found).second_class()
                })
                .or_else(|| {
                    instantiation
                        .values
                        .iter()
                        .find(|(_, declared, _)| mentions(declared))
                })
                .map_or(instantiation.at, |(span, _, _)| *span);
            if reported.iter().any(|known| contains(site, *known)) {
                continue;
            }
            reported.push(site);
            let param = instantiation.params.get(index).map_or("?", String::as_str);
            let value = report.text(site);
            report.error(
                Code::U3016,
                site,
                format!(
                    "`{value}` is, or holds, a function type with `uses` or `fails`, which is second-class, so it cannot instantiate type parameter `{param}` of `{}`; take the function as a parameter of function type instead",
                    instantiation.of
                ),
            );
        }
    }
}

/// Whether `outer` contains `inner`.
fn contains(outer: Span, inner: Span) -> bool {
    outer.file == inner.file && outer.start <= inner.start && inner.end <= outer.end
}
