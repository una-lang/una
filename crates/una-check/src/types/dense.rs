//! Dense values of edition 0.5: `Array[T]`, `Matrix[T]`, `Tensor[T]` and `Lu[T]` (`spec/29
//! §17`, ADR 0120 rules 26 to 43).
//!
//! The methods of a dense value are resolved by its receiver to functions of the prelude whose
//! names start with `dense_`, which no name in a program reaches: their contracts are the
//! methods' `requires` and `ensures`, so the verifier proves each shape precondition at the
//! call, and both engines run them by name. A tensor's shape, index or permutation is a list
//! literal of 1 to 8 elements (U2038 otherwise), whose length selects the prelude function of
//! that rank, and whose elements are passed as its arguments. The operators `+`, `-`, unary
//! `-`, a scalar `*` and `/` are the `zip_map` or `map` of rule 34 with the element type's
//! operator as the lambda, so a fixed-width element type carries that lambda's `overflow`
//! obligation. `*` and `/` between two dense values are U2001, with both repairs (question 1).
//! In a clause, `a.len`, `m.rows`, `m.columns`, `t.rank`, `t.dim(axis: k)`, `f.size` and `get`
//! are terms of the refinement language.

use una_core::ShapeFun;
use una_diag::{Code, ObligationKind};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, BinaryOp, Convention, Expr, ExprKind, Literal};

use super::check::{Checker, Role, Site};
use super::label::Label;
use super::records::{DenseOp, Res};
use super::ty::{Dense, Shared, Ty};
use super::units::Unit;
use crate::names::prelude::DENSE_HIDDEN;

/// The element types a method of a dense value takes (ADR 0120 rule 26).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Needs {
    /// Every element type (rules 29 and 30).
    Any,
    /// `F64` or a fixed-width integer type (rules 34 to 43).
    Numeric,
    /// `F64` (rules 34 to 38: float elements).
    Float,
    /// A fixed-width integer type (rule 36: `sum_exact` and `dot_exact`).
    Integer,
}

/// The methods of dense values (`spec/29 §17`): the receiver, the method, the prelude function
/// that implements it, and the element types it takes. A method a receiver lacks, or whose
/// element type it does not take, resolves as a function of the package (`spec/10 §5`). The
/// methods of a tensor that take a list literal or a literal axis are [`Checker::tensor_method`]'s.
const METHODS: &[(Dense, &str, &str, Needs)] = &[
    (Dense::Array, "get", "dense_array_get", Needs::Any),
    (Dense::Array, "set", "dense_array_set", Needs::Any),
    (Dense::Array, "at", "dense_array_at", Needs::Any),
    (Dense::Array, "slice", "dense_array_slice", Needs::Any),
    (Dense::Array, "compact", "dense_array_compact", Needs::Any),
    (
        Dense::Array,
        "to_list",
        "dense_array_to_list",
        Needs::Numeric,
    ),
    (Dense::Array, "map", "dense_array_map", Needs::Numeric),
    (
        Dense::Array,
        "zip_map",
        "dense_array_zip_map",
        Needs::Numeric,
    ),
    (Dense::Array, "update", "dense_array_update", Needs::Numeric),
    (Dense::Array, "fold", "dense_array_fold", Needs::Numeric),
    (
        Dense::Array,
        "multiply_elements",
        "dense_array_multiply_elements",
        Needs::Float,
    ),
    (
        Dense::Array,
        "divide_elements",
        "dense_array_divide_elements",
        Needs::Float,
    ),
    (Dense::Array, "sum", "dense_array_sum", Needs::Float),
    (Dense::Array, "dot", "dense_array_dot", Needs::Float),
    (Dense::Array, "norm2", "dense_array_norm2", Needs::Float),
    (
        Dense::Array,
        "all_finite",
        "dense_array_all_finite",
        Needs::Float,
    ),
    (
        Dense::Array,
        "add_scaled",
        "dense_array_add_scaled",
        Needs::Float,
    ),
    (Dense::Array, "scale", "dense_array_scale", Needs::Float),
    (
        Dense::Array,
        "sum_exact",
        "dense_array_sum_exact",
        Needs::Integer,
    ),
    (
        Dense::Array,
        "dot_exact",
        "dense_array_dot_exact",
        Needs::Integer,
    ),
    (
        Dense::Array,
        "to_tensor",
        "dense_array_to_tensor",
        Needs::Numeric,
    ),
    (Dense::Matrix, "get", "dense_matrix_get", Needs::Numeric),
    (Dense::Matrix, "set", "dense_matrix_set", Needs::Numeric),
    (Dense::Matrix, "at", "dense_matrix_at", Needs::Numeric),
    (Dense::Matrix, "row", "dense_matrix_row", Needs::Numeric),
    (
        Dense::Matrix,
        "column",
        "dense_matrix_column",
        Needs::Numeric,
    ),
    (Dense::Matrix, "block", "dense_matrix_block", Needs::Numeric),
    (
        Dense::Matrix,
        "transpose",
        "dense_matrix_transpose",
        Needs::Numeric,
    ),
    (
        Dense::Matrix,
        "compact",
        "dense_matrix_compact",
        Needs::Numeric,
    ),
    (Dense::Matrix, "map", "dense_matrix_map", Needs::Numeric),
    (
        Dense::Matrix,
        "zip_map",
        "dense_matrix_zip_map",
        Needs::Numeric,
    ),
    (
        Dense::Matrix,
        "update",
        "dense_matrix_update",
        Needs::Numeric,
    ),
    (Dense::Matrix, "fold", "dense_matrix_fold", Needs::Numeric),
    (
        Dense::Matrix,
        "multiply_elements",
        "dense_matrix_multiply_elements",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "divide_elements",
        "dense_matrix_divide_elements",
        Needs::Float,
    ),
    (Dense::Matrix, "sum", "dense_matrix_sum", Needs::Float),
    (
        Dense::Matrix,
        "row_sums",
        "dense_matrix_row_sums",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "column_sums",
        "dense_matrix_column_sums",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "all_finite",
        "dense_matrix_all_finite",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "multiply",
        "dense_matrix_multiply",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "multiply_array",
        "dense_matrix_multiply_array",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "add_product",
        "dense_matrix_add_product",
        Needs::Float,
    ),
    (Dense::Matrix, "scale", "dense_matrix_scale", Needs::Float),
    (
        Dense::Matrix,
        "sum_exact",
        "dense_matrix_sum_exact",
        Needs::Integer,
    ),
    (
        Dense::Matrix,
        "to_tensor",
        "dense_matrix_to_tensor",
        Needs::Numeric,
    ),
    (Dense::Matrix, "lu", "dense_matrix_lu", Needs::Float),
    (Dense::Matrix, "solve", "dense_matrix_solve", Needs::Float),
    (
        Dense::Matrix,
        "solve_array",
        "dense_matrix_solve_array",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "cholesky",
        "dense_matrix_cholesky",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "solve_lower",
        "dense_matrix_solve_lower",
        Needs::Float,
    ),
    (
        Dense::Matrix,
        "solve_upper",
        "dense_matrix_solve_upper",
        Needs::Float,
    ),
    (
        Dense::Tensor,
        "compact",
        "dense_tensor_compact",
        Needs::Numeric,
    ),
    (
        Dense::Tensor,
        "to_array",
        "dense_tensor_to_array",
        Needs::Numeric,
    ),
    (
        Dense::Tensor,
        "to_matrix",
        "dense_tensor_to_matrix",
        Needs::Numeric,
    ),
    (Dense::Tensor, "map", "dense_tensor_map", Needs::Numeric),
    (
        Dense::Tensor,
        "zip_map",
        "dense_tensor_zip_map",
        Needs::Numeric,
    ),
    (
        Dense::Tensor,
        "update",
        "dense_tensor_update",
        Needs::Numeric,
    ),
    (Dense::Tensor, "fold", "dense_tensor_fold", Needs::Numeric),
    (
        Dense::Tensor,
        "multiply_elements",
        "dense_tensor_multiply_elements",
        Needs::Float,
    ),
    (
        Dense::Tensor,
        "divide_elements",
        "dense_tensor_divide_elements",
        Needs::Float,
    ),
    (Dense::Tensor, "sum", "dense_tensor_sum", Needs::Float),
    (
        Dense::Tensor,
        "all_finite",
        "dense_tensor_all_finite",
        Needs::Float,
    ),
    (
        Dense::Tensor,
        "sum_exact",
        "dense_tensor_sum_exact",
        Needs::Integer,
    ),
    (
        Dense::Tensor,
        "contract",
        "dense_tensor_contract",
        Needs::Float,
    ),
    (Dense::Tensor, "dim", "dense_tensor_dim", Needs::Numeric),
    (Dense::Lu, "solve", "dense_lu_solve", Needs::Any),
    (Dense::Lu, "factors", "dense_lu_factors", Needs::Any),
    (Dense::Lu, "pivots", "dense_lu_pivots", Needs::Any),
];

/// The functions of the prelude that take the elements of a list literal as their arguments,
/// one function per length from 1 to 8 (`spec/29 §17`, ADR 0120 rules 29, 32, 33 and 39): the
/// prefix of their names, completed by the length, and what they implement.
const RANKED: [(&str, &str); 7] = [
    ("dense_tensor_make_", "tensor"),
    ("dense_tensor_filled_", "tensor_filled"),
    ("dense_tensor_get_", "get"),
    ("dense_tensor_at_", "at"),
    ("dense_tensor_set_", "set"),
    ("dense_tensor_permute_", "permute"),
    ("dense_tensor_reshape_", "reshape"),
];

/// The most axes a tensor has (ADR 0120 rule 27).
const MAX_RANK: usize = 8;

/// The trusted functions of the prelude that compute new values of their element type rather
/// than return values they were given (ADR 0120 rules 32, 36 to 39): the `+0` that
/// `matrix_diagonal` places off the diagonal, a sum, a product, a factorization. Their results,
/// and the type arguments their calls elaborate with, keep no refinement of the element type: a
/// refinement flows through a generic call's covariant result only for values the callee takes
/// from its arguments (ADR 0103 rule 5), so `matrix_diagonal` over `I32 in 1..10` is a
/// `Matrix[I32]`. Every function the prelude adds that computes elements MUST be listed here.
pub(crate) const SYNTHESIZING: &[&str] = &[
    "matrix_diagonal",
    "dense_array_multiply_elements",
    "dense_array_divide_elements",
    "dense_array_sum",
    "dense_array_dot",
    "dense_array_norm2",
    "dense_array_add_scaled",
    "dense_array_scale",
    "dense_matrix_multiply_elements",
    "dense_matrix_divide_elements",
    "dense_matrix_sum",
    "dense_matrix_row_sums",
    "dense_matrix_column_sums",
    "dense_matrix_multiply",
    "dense_matrix_multiply_array",
    "dense_matrix_add_product",
    "dense_matrix_scale",
    "dense_matrix_lu",
    "dense_matrix_solve",
    "dense_matrix_solve_array",
    "dense_matrix_cholesky",
    "dense_matrix_solve_lower",
    "dense_matrix_solve_upper",
    "dense_lu_solve",
    "dense_lu_factors",
    "dense_tensor_multiply_elements",
    "dense_tensor_divide_elements",
    "dense_tensor_sum",
    "dense_tensor_sum_axis",
    "dense_tensor_contract",
    // `map`, `zip_map`, `update` and `fold` take their new elements from the function they
    // are given, whose result flows into the element type with its obligation, as for a
    // list; the elements of every other method are its receiver's or its arguments'.
    "dense_array_sum_exact",
    "dense_array_dot_exact",
    "dense_matrix_sum_exact",
    "dense_tensor_sum_exact",
    "dense_array_all_finite",
    "dense_matrix_all_finite",
    "dense_tensor_all_finite",
];

/// The methods of dense values whose units are a rule of their own (ADR 0120 rule 64): they
/// multiply or divide the units of their operands, or add a product to their receiver. Every
/// other method is generic in its element type, unit included.
const UNIT_KERNELS: [&str; 9] = [
    "multiply_elements",
    "divide_elements",
    "dot",
    "dot_exact",
    "add_scaled",
    "scale",
    "multiply",
    "multiply_array",
    "add_product",
];

/// `ty`, a dense type, with its element in `unit` in place of its own, labels and
/// refinements kept: `x.magnitude()` and `x.measured()` of a dense value, and the products,
/// whose representation is the same (ADR 0120 rule 64).
pub(crate) fn dense_in_unit(ty: &Ty, unit: Option<&Unit>) -> Ty {
    match ty {
        Ty::Labeled(label, inner) => {
            Ty::Labeled(label.clone(), Shared::new(dense_in_unit(inner, unit)))
        }
        Ty::Dense(dense, element) => Ty::Dense(*dense, Shared::new(element_in_unit(element, unit))),
        other => other.clone(),
    }
}

/// An element type in `unit` in place of its own, its labels and refinements kept.
fn element_in_unit(element: &Ty, unit: Option<&Unit>) -> Ty {
    match element {
        Ty::Labeled(label, inner) => {
            Ty::Labeled(label.clone(), Shared::new(element_in_unit(inner, unit)))
        }
        Ty::Refined(refinements, inner) => Ty::Refined(
            refinements.clone(),
            Shared::new(element_in_unit(inner, unit)),
        ),
        other => other.clone().in_unit(unit),
    }
}

/// Whether `def`, a function of module `module`, computes new values of its element type
/// ([`SYNTHESIZING`]).
pub(crate) fn synthesizes(prelude: usize, module: usize, name: &str) -> bool {
    module == prelude && SYNTHESIZING.contains(&name)
}

/// The method that the prelude function `name` implements, when it is one of the dense
/// methods: diagnostics name the method, never the function (`spec/29 §17`).
pub(crate) fn dense_display(name: &str) -> Option<&'static str> {
    if !name.starts_with(DENSE_HIDDEN) {
        return None;
    }
    if name == "dense_tensor_sum_axis" {
        return Some("sum");
    }
    if let Some(method) = METHODS
        .iter()
        .find(|(_, _, function, _)| *function == name)
        .map(|(_, method, ..)| *method)
    {
        return Some(method);
    }
    RANKED.iter().find_map(|(prefix, method)| {
        let rank = name.strip_prefix(prefix)?;
        rank.parse::<usize>().ok().map(|_| *method)
    })
}

/// The arguments of a call with the elements of the list literal at `position` passed in its
/// place, in order, each unlabeled (`spec/29 §17`). The checker binds them to the parameters of
/// the function of that literal's length ([`Checker::spread`]), and the elaboration reads them
/// the same way, so both see one call.
pub(crate) fn spread(args: &[Arg], position: usize) -> Vec<Arg> {
    let mut out = Vec::with_capacity(args.len() + MAX_RANK);
    for (index, arg) in args.iter().enumerate() {
        match (&arg.value.kind, index == position) {
            (ExprKind::List(items), true) => {
                out.extend(items.iter().map(|item| Arg {
                    label: None,
                    marker: None,
                    value: item.clone(),
                    span: item.span,
                }));
            }
            _ => out.push(arg.clone()),
        }
    }
    out
}

/// The value of `expr` when it is an integer literal.
fn literal_value(expr: &Expr) -> Option<u64> {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(digits)) => digits.parse().ok(),
        ExprKind::Paren(inner) => literal_value(inner),
        _ => None,
    }
}

/// Whether `ty`, a written element type with its labels and refinements, is numeric: `F64` or
/// a fixed-width integer type, or a measured version of one (ADR 0120 rules 26 and 64). An
/// error type is numeric, so one mistake gives one diagnostic.
pub(crate) fn dense_numeric(ty: &Ty) -> bool {
    match ty {
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) | Ty::Measured(_, inner) => {
            dense_numeric(inner)
        }
        Ty::F64 | Ty::Fixed(_) | Ty::Error(_) => true,
        _ => false,
    }
}

/// Whether `ty`, a written element type, is a float element type, `F64` or a refinement of
/// it: the element types of `Lu[T]` (ADR 0120 rule 38). An error type is one, so one mistake
/// gives one diagnostic.
pub(crate) fn dense_float(ty: &Ty) -> bool {
    match ty {
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) => dense_float(inner),
        Ty::F64 | Ty::Error(_) => true,
        _ => false,
    }
}

/// The class of a numeric element type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Float,
    Integer,
}

impl Needs {
    fn takes(self, class: Option<Class>) -> bool {
        match self {
            Needs::Any => true,
            Needs::Numeric => class.is_some(),
            Needs::Float => class == Some(Class::Float),
            Needs::Integer => class == Some(Class::Integer),
        }
    }
}

impl Checker<'_, '_, '_> {
    /// The class of the element type `element`, through its solved variables, labels,
    /// refinements and unit (ADR 0120 rule 64): `None` when it is not numeric.
    fn element_class(&self, element: &Ty) -> Option<Class> {
        match self.vars.peel(element).magnitude() {
            Ty::F64 => Some(Class::Float),
            Ty::Fixed(_) => Some(Class::Integer),
            _ => None,
        }
    }

    /// The dense type of `ty` and its element, through solved variables and labels.
    pub(crate) fn dense_of(&self, ty: &Ty) -> Option<(Dense, Shared<Ty>)> {
        if self.edition() < Edition::V0_5 {
            return None;
        }
        match self.vars.peel(ty) {
            Ty::Dense(dense, element) => Some((dense, element)),
            _ => None,
        }
    }

    /// `a.len` of an array, `m.rows` and `m.columns` of a matrix, `t.rank` of a tensor and
    /// `f.size` of an `Lu[T]` (ADR 0120 rules 33 and 38): an `Int` with the value's top label,
    /// which covers its shape (IF3). The prelude's contracts also read a tensor's element
    /// count, `count`, which no program names. `None` for any other field.
    pub(crate) fn dense_field(&mut self, target_ty: &Ty, field: &str, span: Span) -> Option<Ty> {
        let (dense, _) = self.dense_of(target_ty)?;
        let top = self.labeled(target_ty).top_label();
        let prelude = self.module == self.env.prelude;
        match (dense, field) {
            (Dense::Array, "len") => self.resolved(span, Res::Len),
            (Dense::Matrix, "rows") => self.resolved(span, Res::Shape(ShapeFun::Rows)),
            (Dense::Matrix, "columns") => self.resolved(span, Res::Shape(ShapeFun::Columns)),
            (Dense::Tensor, "rank") => self.resolved(span, Res::Shape(ShapeFun::Rank)),
            (Dense::Tensor, "count") if prelude => {
                self.resolved(span, Res::Shape(ShapeFun::Count));
            }
            (Dense::Lu, "size") => self.resolved(span, Res::Shape(ShapeFun::Size)),
            _ => return None,
        }
        Some(Ty::Int.with_label(&top))
    }

    /// `x.m(args)` when `x` is a dense value and `m` one of its methods (`spec/29 §17`): a call
    /// of the prelude function that implements it, with `x` as the receiver. In a clause, `get`
    /// is the element term instead. `None` when `m` is not a method of `x`, so the call
    /// resolves as before.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dense_method(
        &mut self,
        target: &Expr,
        target_ty: &Ty,
        marker: Option<Convention>,
        method: &str,
        args: &[Arg],
        span: Span,
        site: Site,
    ) -> Option<Ty> {
        let (dense, element) = self.dense_of(target_ty)?;
        let class = self.element_class(&element);
        if dense == Dense::Tensor
            && let Some(ty) = self.tensor_method(
                target, target_ty, marker, &element, method, args, span, site,
            )
        {
            return Some(ty);
        }
        let (_, _, function, needs) = METHODS
            .iter()
            .find(|(d, name, ..)| *d == dense && *name == method)
            .copied()?;
        // The prelude's own contracts name its methods over a type parameter, whose element
        // types its callers' receivers decide.
        if !needs.takes(class) && self.module != self.env.prelude {
            return None;
        }
        if method == "get" && self.in_clause() {
            return Some(self.element_term(dense, &element, target_ty, args, span));
        }
        let id = self.env.prelude_function(function)?;
        // A kernel combines the units of its operands by its own rule (ADR 0120 rule 64), so
        // the call is typed on their magnitudes, and the rule gives the result's unit.
        let kernel = UNIT_KERNELS.contains(&method);
        if kernel {
            self.unit_blind.insert(target.span, None);
            for arg in args {
                self.unit_blind.insert(arg.value.span, None);
            }
        }
        self.receiver_marker = Some((marker, target.clone()));
        let ty = self.call_function(id, Some((target.span, target_ty.clone())), args, span, site);
        self.receiver_marker = None;
        let ty = if kernel {
            self.unit_blind.remove(&target.span);
            let found: Vec<(String, Option<Ty>, Span)> = args
                .iter()
                .map(|arg| {
                    let name = match (&arg.label, &arg.value.kind) {
                        (Some(label), _) => self.name(label).to_owned(),
                        (None, ExprKind::Name(ident)) => self.name(ident).to_owned(),
                        (None, _) => String::new(),
                    };
                    let found = self.unit_blind.remove(&arg.value.span).flatten();
                    (name, found, arg.value.span)
                })
                .collect();
            self.kernel_units(method, target_ty, &found, ty, span)
        } else {
            ty
        };
        if matches!(method, "add_scaled" | "add_product") {
            self.kernel_inputs(target_ty, args);
        }
        Some(ty)
    }

    /// The methods of a tensor that read a list literal or a literal axis (ADR 0120 rules 29,
    /// 33, 36 and 39): `get`, `at` and `set` by the list literal `at:`, `permute(axes:)`,
    /// `reshape(shape:)`, `dim(axis:)`, `sum(axis:)` and `contract(with:, axis:, other:)`. A
    /// list literal selects the function of its length; anything else in its place is U2038.
    /// `None` for the other methods, which [`METHODS`] resolves.
    #[allow(clippy::too_many_arguments)]
    fn tensor_method(
        &mut self,
        target: &Expr,
        target_ty: &Ty,
        marker: Option<Convention>,
        element: &Ty,
        method: &str,
        args: &[Arg],
        span: Span,
        site: Site,
    ) -> Option<Ty> {
        let prelude = self.module == self.env.prelude;
        let class = self.element_class(element);
        let (prefix, label, what) = match method {
            "get" => ("dense_tensor_get_", "at", "the index of `get` on a tensor"),
            "at" => ("dense_tensor_at_", "at", "the index of `at` on a tensor"),
            "set" => ("dense_tensor_set_", "at", "the index of `set` on a tensor"),
            "permute" => ("dense_tensor_permute_", "axes", "the axes of `permute`"),
            "reshape" => ("dense_tensor_reshape_", "shape", "the shape of `reshape`"),
            "dim" => {
                if !prelude && !self.literal_axes(args, &["axis"], "dim") {
                    return Some(Ty::error());
                }
                if self.in_clause() {
                    return Some(self.dim_term(target_ty, args, span));
                }
                return None;
            }
            "sum" if !args.is_empty() => {
                if !prelude && class != Some(Class::Float) {
                    return None;
                }
                if !prelude && !self.literal_axes(args, &["axis"], "sum") {
                    return Some(Ty::error());
                }
                let id = self.env.prelude_function("dense_tensor_sum_axis")?;
                self.receiver_marker = Some((marker, target.clone()));
                let ty = self.call_function(
                    id,
                    Some((target.span, target_ty.clone())),
                    args,
                    span,
                    site,
                );
                self.receiver_marker = None;
                return Some(ty);
            }
            "contract" => {
                if !prelude
                    && class == Some(Class::Float)
                    && !self.literal_axes(args, &["axis", "other"], "contract")
                {
                    return Some(Ty::error());
                }
                return None;
            }
            _ => return None,
        };
        if !prelude && class.is_none() {
            return None;
        }
        let found = args.iter().enumerate().find(|(_, arg)| {
            arg.label
                .as_ref()
                .is_some_and(|written| self.name(written) == label)
        });
        let example = match label {
            "at" => "`at: [i, j]`",
            "axes" => "`axes: [1, 0]`",
            _ => "`shape: [2, 3]`",
        };
        let Some((position, items)) = found.and_then(|(position, arg)| match &arg.value.kind {
            ExprKind::List(items) if (1..=MAX_RANK).contains(&items.len()) => {
                Some((position, items))
            }
            _ => None,
        }) else {
            let (verb, things) = match label {
                "at" => ("is", "indices"),
                "axes" => ("are", "integer literals"),
                _ => ("is", "lengths"),
            };
            self.report.error(
                Code::U2038,
                span,
                format!(
                    "{what} {verb} a list literal of 1 to {MAX_RANK} {things}, written `{label}:`, as in {example}"
                ),
            );
            self.unchecked_args(args);
            return Some(Ty::error());
        };
        if method == "permute" && !prelude && !permutation(items) {
            self.report.error(
                Code::U2038,
                span,
                format!(
                    "{what} are a list literal of integer literals that is a permutation of 0 to {}, as in {example}",
                    items.len() - 1
                ),
            );
            self.unchecked_args(args);
            return Some(Ty::error());
        }
        if method == "get" && self.in_clause() {
            return Some(self.tensor_element(element, target_ty, position, items, span));
        }
        let id = self
            .env
            .prelude_function(&format!("{prefix}{}", items.len()))?;
        let spread_args = spread(args, position);
        self.spread = Some((position, items.len(), 1));
        self.spread_at(span, position);
        self.receiver_marker = Some((marker, target.clone()));
        let ty = self.call_function(
            id,
            Some((target.span, target_ty.clone())),
            &spread_args,
            span,
            site,
        );
        self.receiver_marker = None;
        self.spread = None;
        Some(ty)
    }

    /// `tensor(shape: [...], elements: xs)` and `tensor_filled(shape: [...], value: x)` (ADR
    /// 0120 rule 32): the call of the prelude function of the literal's length, with its
    /// elements as the lengths. U2038 when the shape is not a list literal of 1 to 8 elements.
    /// `None` when `id` is neither prelude function, or in the prelude itself.
    pub(crate) fn tensor_construct(
        &mut self,
        id: usize,
        args: &[Arg],
        span: Span,
        site: Site,
    ) -> Option<Ty> {
        let def = self.env.functions.get(id)?;
        if def.module != self.env.prelude || self.module == self.env.prelude {
            return None;
        }
        let (prefix, name) = match def.name {
            "tensor" => ("dense_tensor_make_", "tensor"),
            "tensor_filled" => ("dense_tensor_filled_", "tensor_filled"),
            _ => return None,
        };
        let found = args
            .iter()
            .enumerate()
            .find(|(index, arg)| match &arg.label {
                Some(label) => self.name(label) == "shape",
                None => *index == 0,
            });
        let Some((position, items)) = found.and_then(|(position, arg)| match &arg.value.kind {
            ExprKind::List(items) if (1..=MAX_RANK).contains(&items.len()) => {
                Some((position, items))
            }
            _ => None,
        }) else {
            self.report.error(
                Code::U2038,
                span,
                format!(
                    "the shape of `{name}` is a list literal of 1 to {MAX_RANK} lengths, as in `shape: [2, 3]`"
                ),
            );
            self.unchecked_args(args);
            return Some(Ty::error());
        };
        let ranked = self
            .env
            .prelude_function(&format!("{prefix}{}", items.len()))?;
        let spread_args = spread(args, position);
        self.spread = Some((position, items.len(), 0));
        self.spread_at(span, position);
        let ty = self.call_function(ranked, None, &spread_args, span, site);
        self.spread = None;
        Some(ty)
    }

    /// Whether the arguments `labels` of the tensor method `method` are integer literals
    /// below the largest rank (ADR 0120 rules 33, 36 and 39); U2038 at `span` otherwise, with
    /// the arguments checked for the diagnostics inside them.
    fn literal_axes(&mut self, args: &[Arg], labels: &[&str], method: &str) -> bool {
        for label in labels {
            let value = args
                .iter()
                .find(|arg| {
                    arg.label
                        .as_ref()
                        .is_some_and(|written| self.name(written) == *label)
                })
                .map(|arg| (arg.value.span, literal_value(&arg.value)));
            match value {
                Some((_, Some(axis))) if axis < MAX_RANK as u64 => {}
                Some((at, _)) => {
                    self.report.error(
                        Code::U2038,
                        at,
                        format!(
                            "the `{label}:` of `{method}` is an integer literal from 0 to {}, an axis of the tensor",
                            MAX_RANK - 1
                        ),
                    );
                    self.unchecked_args(args);
                    return false;
                }
                // A missing argument is reported by the call.
                None => {}
            }
        }
        true
    }

    /// `t.dim(axis: k)` in a clause (ADR 0120 rule 33): the length of axis k, an `Int` with
    /// the tensor's top label (IF3).
    fn dim_term(&mut self, target_ty: &Ty, args: &[Arg], span: Span) -> Ty {
        let top = self.labeled(target_ty).top_label();
        let position = match args {
            [arg]
                if arg
                    .label
                    .as_ref()
                    .is_some_and(|label| self.name(label) == "axis") =>
            {
                0
            }
            _ => {
                self.report.error(
                    Code::U2001,
                    span,
                    "`dim` takes one argument, its axis, as in `t.dim(axis: 0)`",
                );
                self.unchecked_args(args);
                return Ty::error();
            }
        };
        if let Some(arg) = args.first() {
            self.expr_as(&arg.value, Some(&Ty::Int), Role::Condition("an axis"));
        }
        self.resolved(span, Res::Dim(position));
        Ty::Int.with_label(&top)
    }

    /// `t.get(at: [i, ...])` in a clause: the element term of a tensor (ADR 0120 rule 33), over
    /// integer elements, with the element label joined with the top label and the indices'
    /// labels (IF3).
    fn tensor_element(
        &mut self,
        element: &Ty,
        target_ty: &Ty,
        position: usize,
        items: &[Expr],
        span: Span,
    ) -> Ty {
        if self.module != self.env.prelude {
            match self.element_class(element) {
                Some(Class::Integer) => {}
                Some(Class::Float) => self.outside_rl(span, || "an element of `F64`".to_owned()),
                None => self.outside_rl(span, || {
                    "an element of a dense value that is not an integer".to_owned()
                }),
            }
        }
        let mut label = self.labeled(target_ty).top_label();
        for item in items {
            let found = self.expr_as(item, Some(&Ty::Int), Role::Condition("an index"));
            label = label.join(&self.deep(&found));
        }
        self.resolved(span, Res::TensorElement(position));
        self.labeled(element).with_label(&label)
    }

    /// The unit of the element of `ty` when it is a dense value, or of `ty` when it is a
    /// scalar.
    fn element_unit(&self, ty: &Ty) -> Option<Unit> {
        match self.vars.peel(ty) {
            Ty::Dense(_, element) => self.unit_of(&element),
            other => other.unit().cloned(),
        }
    }

    /// The type `ty` of a call of the kernel `method` on `receiver`, typed on the magnitudes,
    /// with the units of ADR 0120 rule 64: `multiply_elements`, `multiply`, `multiply_array` and
    /// `dot` multiply the units of the receiver and `by` or `with`, `divide_elements` divides
    /// them; `add_scaled` and `add_product` add a product to their receiver, which must have
    /// its unit, and `scale` scales its receiver in place by a unitless factor (U2028
    /// otherwise, at the argument). `found` holds each argument's label, type and span.
    fn kernel_units(
        &mut self,
        method: &str,
        receiver: &Ty,
        found: &[(String, Option<Ty>, Span)],
        ty: Ty,
        span: Span,
    ) -> Ty {
        let own = self.element_unit(receiver);
        let arg = |name: &str| {
            found
                .iter()
                .find(|(label, ..)| label == name)
                .and_then(|(_, ty, _)| ty.as_ref())
        };
        // Where a unit error of the argument `name` is reported: at it, else at the call.
        let at = |name: &str| {
            found
                .iter()
                .find(|(label, ..)| label == name)
                .map_or(span, |(.., at)| *at)
        };
        let units: Vec<Option<Unit>> = ["by", "with", "x", "of", "and"]
            .iter()
            .map(|name| arg(name).and_then(|ty| self.element_unit(ty)))
            .collect();
        let [by, with, x, of, and] = units.as_slice() else {
            return ty;
        };
        if own.is_none() && units.iter().all(Option::is_none) {
            return ty;
        }
        let result = self.labeled(&ty);
        match method {
            "multiply_elements" | "multiply" | "multiply_array" => {
                let unit = self.product_unit(own.as_ref(), by.as_ref(), span);
                dense_in_unit(&result, unit.as_ref())
            }
            "divide_elements" => {
                let unit = self.quotient_unit(own.as_ref(), by.as_ref(), span);
                dense_in_unit(&result, unit.as_ref())
            }
            "dot" | "dot_exact" => {
                let unit = self.product_unit(own.as_ref(), with.as_ref(), span);
                result.in_unit(unit.as_ref())
            }
            "add_scaled" | "add_product" => {
                let (a, b, last) = if method == "add_scaled" {
                    (by, x, at("by"))
                } else {
                    (of, and, at("and"))
                };
                let product = self.product_unit(a.as_ref(), b.as_ref(), span);
                if product != own {
                    let shown = |u: &Option<Unit>| {
                        u.as_ref()
                            .map_or_else(|| "no unit".to_owned(), |u| format!("`{u}`"))
                    };
                    self.report.error(
                        Code::U2028,
                        last,
                        format!(
                            "`{method}` adds a product in {} to elements in {}, which need one unit",
                            shown(&product),
                            shown(&own)
                        ),
                    );
                }
                result
            }
            // `scale`: in place, so the receiver's type, unit included, does not change.
            _ => {
                if by.is_some() {
                    let mut diagnostic = self.report.make(
                        Code::U2028,
                        at("by"),
                        "`scale` changes its receiver in place, so its factor `by` is unitless",
                    );
                    diagnostic.repairs.push(super::Report::advice(
                        "multiply with `*` for a result in another unit, as in `let y = x * k`",
                    ));
                    self.report.push(diagnostic);
                }
                result
            }
        }
    }

    /// `a.get(index: i)` or `m.get(row: i, column: j)` in a clause: the element term (ADR
    /// 0120 rule 33), with the element label joined with the top label and the indices'
    /// labels (IF3). No float term is in the refinement language yet (`spec/29 §8`), so an
    /// element of `F64` there is U2014.
    fn element_term(
        &mut self,
        dense: Dense,
        element: &Ty,
        target_ty: &Ty,
        args: &[Arg],
        span: Span,
    ) -> Ty {
        // Over integer elements only (`spec/29 §17` rule 4): no float term is in the refinement
        // language yet (`spec/29 §8`), and the elements of other types wait for their rules.
        // The prelude's own contracts state the element equations over its type parameter.
        if self.module != self.env.prelude {
            match self.element_class(element) {
                Some(Class::Integer) => {}
                Some(Class::Float) => self.outside_rl(span, || "an element of `F64`".to_owned()),
                None => self.outside_rl(span, || {
                    "an element of a dense value that is not an integer".to_owned()
                }),
            }
        }
        let axes: &[&str] = match dense {
            Dense::Array => &["index"],
            Dense::Matrix => &["row", "column"],
            // `Tensor` reads its indices from a list literal ([`Self::tensor_element`]); `Lu`
            // has no element term.
            Dense::Tensor | Dense::Lu => &[],
        };
        let mut order = Vec::new();
        let mut label = self.labeled(target_ty).top_label();
        if args.len() == axes.len() {
            for axis in axes {
                let position = args.iter().position(|arg| {
                    arg.label
                        .as_ref()
                        .is_some_and(|label| self.name(label) == *axis)
                });
                match position {
                    Some(position) => order.push(position),
                    None => {
                        let shape = axes
                            .iter()
                            .map(|axis| format!("{axis}: {}", &axis[..1]))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.report.error(
                            Code::U2005,
                            span,
                            format!("`get` takes its indices as `get({shape})`"),
                        );
                        self.unchecked_args(args);
                        return Ty::error();
                    }
                }
            }
            for arg in args {
                let found = self.expr_as(&arg.value, Some(&Ty::Int), Role::Condition("an index"));
                label = label.join(&self.deep(&found));
            }
        } else {
            self.report.error(
                Code::U2001,
                span,
                format!(
                    "`get` takes {} index argument(s) on `{}`",
                    axes.len(),
                    dense.as_str()
                ),
            );
            self.unchecked_args(args);
            return Ty::error();
        }
        self.resolved(span, Res::DenseElement(order));
        self.labeled(element).with_label(&label)
    }

    /// IF6 for the kernels that update their receiver from other dense values: each element of
    /// the receiver depends on the inputs' shapes too (`add_product` sums over `a.columns`
    /// terms), so their top labels flow into the receiver's element position; their elements
    /// flow through the element type, as for any argument. U4001 at an input above it.
    fn kernel_inputs(&mut self, place_ty: &Ty, args: &[Arg]) {
        if !self.labels() {
            return;
        }
        let place = self.labeled(place_ty);
        let Ty::Dense(_, element) = place.unlabeled() else {
            return;
        };
        let element_label = self.labeled(element).top_label();
        for arg in args {
            let Some(found) = self.records.types.get(&arg.value.span).cloned() else {
                continue;
            };
            let found = self.labeled(&found);
            if !matches!(found.unlabeled(), Ty::Dense(..)) {
                continue;
            }
            let top = found.top_label();
            if !top.flows_to(&element_label) {
                let input = self.report.text(arg.value.span).to_owned();
                self.flow_error(
                    Code::U4001,
                    arg.value.span,
                    format!(
                        "the shape of `{input}` is labeled `{top}`, and decides the elements of a receiver labeled `{element_label}`; give the receiver's elements a `{top}` label"
                    ),
                    super::flow::Carrier::Value("this value"),
                    &top,
                    &element_label,
                );
            }
        }
    }

    /// `x op y` where `first`, of type `first_ty`, is a dense value or the scalar of a scalar
    /// `*`, and `second` is the other operand (ADR 0120 rules 34 and 42): the dense value of
    /// the result, or the error type after U2001. `dense_first` says whether `first` is the
    /// left operand as written.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dense_arithmetic(
        &mut self,
        op: BinaryOp,
        first: &Expr,
        first_ty: Ty,
        second: &Expr,
        second_ty: Option<Ty>,
        span: Span,
        swapped: bool,
    ) -> Ty {
        self.outside_rl(span, || "arithmetic on dense values".to_owned());
        let symbol = op_symbol(op);
        // The dense operand and the other one, each with its type.
        let first_dense = self.dense_of(&first_ty);
        let (second_ty, second_dense) = match second_ty {
            Some(ty) => {
                let dense = self.dense_of(&ty);
                (ty, dense)
            }
            None => {
                let hint = match (op, &first_dense) {
                    (BinaryOp::Add | BinaryOp::Sub, _) => Some(first_ty.clone()),
                    // A scalar is unitless unless it says otherwise: its unit multiplies the
                    // elements' (ADR 0120 rule 64), so the hint is the magnitude.
                    (_, Some((_, element))) if element.has_units() => Some(self.number_of(element)),
                    (_, Some((_, element))) => Some(element.as_ref().clone()),
                    _ => None,
                };
                let ty = self.infer(second, hint.as_ref());
                let dense = self.dense_of(&ty);
                (ty, dense)
            }
        };
        self.operands.push(first_ty.clone());
        self.operands.push(second_ty.clone());
        // An `Lu[T]` is a factorization, not an array of numbers: no operator applies to it.
        if matches!(first_dense, Some((Dense::Lu, _)))
            || matches!(second_dense, Some((Dense::Lu, _)))
        {
            self.report.error(
                Code::U2001,
                span,
                format!(
                    "`{symbol}` is not defined on an `Lu` value, a factorization; solve with it, or read its `factors()`"
                ),
            );
            return Ty::error();
        }
        // In source order.
        let (left, left_ty, left_dense, right, right_ty, right_dense) = if swapped {
            (
                second,
                second_ty.clone(),
                second_dense.clone(),
                first,
                first_ty.clone(),
                first_dense.clone(),
            )
        } else {
            (
                first,
                first_ty.clone(),
                first_dense.clone(),
                second,
                second_ty.clone(),
                second_dense.clone(),
            )
        };
        match (op, &left_dense, &right_dense) {
            // Question 1: no `*` and no `/` between two dense values.
            (BinaryOp::Mul | BinaryOp::Div, Some(_), Some(_)) => {
                let (a, b) = (
                    self.report.text(left.span).to_owned(),
                    self.report.text(right.span).to_owned(),
                );
                let a = parenthesized(left, a);
                let mut diagnostic = self.report.make(
                    Code::U2001,
                    span,
                    if op == BinaryOp::Mul {
                        "`*` between two dense values is not defined: the matrix product and the product element by element are different operations".to_owned()
                    } else {
                        "`/` between two dense values is not defined: the quotient element by element is a method".to_owned()
                    },
                );
                let elements = if op == BinaryOp::Mul {
                    diagnostic.repairs.push(super::Report::advice(format!(
                        "for the matrix product, write `{a}.multiply(by: {b})`"
                    )));
                    "multiply_elements"
                } else {
                    "divide_elements"
                };
                diagnostic.repairs.push(super::Report::advice(format!(
                    "for the {} element by element, write `{a}.{elements}(by: {b})`",
                    if op == BinaryOp::Mul {
                        "product"
                    } else {
                        "quotient"
                    }
                )));
                self.report.push(diagnostic);
                Ty::error()
            }
            (BinaryOp::Add | BinaryOp::Sub, Some((ld, le)), Some((rd, re))) => {
                if ld != rd {
                    self.report.error(
                        Code::U2001,
                        span,
                        format!(
                            "`{symbol}` needs two `{}` values, and the other operand is a `{}`",
                            ld.as_str(),
                            rd.as_str()
                        ),
                    );
                    return Ty::error();
                }
                if !self.numeric_operands(symbol, le, span) {
                    return Ty::error();
                }
                let element = self.element_base(le);
                let found_element = self.element_base(re);
                if !self.unify_at(
                    right.span,
                    &element,
                    &found_element,
                    Role::Condition(&format!("the operand of `{symbol}`")),
                ) {
                    return Ty::error();
                }
                let id = self.dense_function(*ld, "zip_map");
                self.obligation(ObligationKind::Precondition, span, None);
                self.overflow_site(&element, span);
                let label = self.element_label(le).join(&self.element_label(re));
                let top = self
                    .labeled(&left_ty)
                    .top_label()
                    .join(&self.labeled(&right_ty).top_label());
                let op = if op == BinaryOp::Add {
                    DenseOp::Add
                } else {
                    DenseOp::Sub
                };
                if let Some(id) = id {
                    self.resolved(span, Res::DenseOp { id, op });
                }
                Ty::Dense(*ld, Shared::new(element.with_label(&label))).with_label(&top)
            }
            // A scalar `*` on either side, and `x / k` with the scalar on the right.
            (BinaryOp::Mul, Some((dense, element)), None)
            | (BinaryOp::Mul, None, Some((dense, element)))
            | (BinaryOp::Div, Some((dense, element)), None) => {
                let scalar_first = left_dense.is_none();
                let (scalar, scalar_ty, dense_ty) = if scalar_first {
                    (left, &left_ty, &right_ty)
                } else {
                    (right, &right_ty, &left_ty)
                };
                if !self.numeric_operands(symbol, element, span) {
                    return Ty::error();
                }
                let base = self.element_base(element);
                let scalar_base = self.resolve(scalar_ty);
                // The scalar's unit multiplies the elements', or divides them (ADR 0120 rule
                // 64): the magnitudes are one type.
                let units = base.has_units() || scalar_base.has_units();
                let (element_unit, scalar_unit) =
                    (base.unit().cloned(), scalar_base.unit().cloned());
                let (base, scalar_base) = if units {
                    (base.magnitude().clone(), scalar_base.magnitude().clone())
                } else {
                    (base, scalar_base)
                };
                if !self.unify_at(
                    scalar.span,
                    &base,
                    &scalar_base,
                    Role::Condition(&format!("the scalar of `{symbol}`")),
                ) {
                    return Ty::error();
                }
                let base = if !units {
                    base
                } else if op == BinaryOp::Div {
                    let unit =
                        self.quotient_unit(element_unit.as_ref(), scalar_unit.as_ref(), span);
                    base.in_unit(unit.as_ref())
                } else {
                    let unit = self.product_unit(element_unit.as_ref(), scalar_unit.as_ref(), span);
                    base.in_unit(unit.as_ref())
                };
                self.overflow_site(&base, span);
                if op == BinaryOp::Div
                    && self.element_class(element) == Some(Class::Integer)
                    && super::expr::int_literal(scalar) != Some(true)
                {
                    self.obligation(ObligationKind::Division, span, None);
                }
                let label = self.element_label(element).join(&self.deep(scalar_ty));
                let top = self.labeled(dense_ty).top_label();
                let op = if op == BinaryOp::Div {
                    DenseOp::Divide
                } else {
                    DenseOp::Scale { scalar_first }
                };
                if let Some(id) = self.dense_function(*dense, "map") {
                    self.resolved(span, Res::DenseOp { id, op });
                }
                Ty::Dense(*dense, Shared::new(base.with_label(&label))).with_label(&top)
            }
            // Rule 42: no implicit broadcasting.
            _ => {
                let mut diagnostic = self.report.make(
                    Code::U2001,
                    span,
                    match op {
                        BinaryOp::Mod => format!("`{symbol}` is not defined on dense values"),
                        BinaryOp::Div => "`/` divides a dense value by a scalar on its right; a scalar divided by a dense value is not defined".to_owned(),
                        _ => format!(
                            "`{symbol}` needs two dense values of one shape: a scalar is never broadcast"
                        ),
                    },
                );
                let example = match (op, left_dense.is_some()) {
                    (BinaryOp::Mod, _) => "fn(e) -> e mod k".to_owned(),
                    (_, true) => format!("fn(e) -> e {symbol} k"),
                    (_, false) => format!("fn(e) -> k {symbol} e"),
                };
                diagnostic.repairs.push(super::Report::advice(format!(
                    "write the element-wise operation with `map`, as in `x.map(each: {example})`"
                )));
                self.report.push(diagnostic);
                Ty::error()
            }
        }
    }

    /// `-x` of a dense value (ADR 0120 rule 34): the `map` of the element type's negation.
    pub(crate) fn dense_negation(&mut self, found: &Ty, span: Span) -> Option<Ty> {
        let (dense, element) = self.dense_of(found)?;
        self.outside_rl(span, || "arithmetic on dense values".to_owned());
        if dense == Dense::Lu {
            self.report.error(
                Code::U2001,
                span,
                "`-` is not defined on an `Lu` value, a factorization; solve with it, or read its `factors()`",
            );
            return Some(Ty::error());
        }
        if !self.numeric_operands("-", &element, span) {
            return Some(Ty::error());
        }
        let base = self.element_base(&element);
        self.overflow_site(&base, span);
        let label = self.element_label(&element);
        let top = self.labeled(found).top_label();
        if let Some(id) = self.dense_function(dense, "map") {
            self.resolved(
                span,
                Res::DenseOp {
                    id,
                    op: DenseOp::Neg,
                },
            );
        }
        Some(Ty::Dense(dense, Shared::new(base.with_label(&label))).with_label(&top))
    }

    /// U2001 when the operator `symbol` meets a dense value whose elements are not numeric.
    fn numeric_operands(&mut self, symbol: &str, element: &Ty, span: Span) -> bool {
        if self.element_class(element).is_some() {
            return true;
        }
        if matches!(self.resolve(element), Ty::Error(_) | Ty::Never) {
            return false;
        }
        let shown = self.show(element);
        self.report.error(
            Code::U2001,
            span,
            format!(
                "`{symbol}` applies to dense values of `F64` or fixed-width integer elements, not `{shown}`"
            ),
        );
        false
    }

    /// The element type's base: its variables solved, its labels and refinements dropped. The
    /// result of an operator holds new values, which keep no refinement of the operands.
    fn element_base(&self, element: &Ty) -> Ty {
        self.resolve(element)
    }

    /// The label of an element position.
    fn element_label(&self, element: &Ty) -> Label {
        self.labeled(element).top_label()
    }

    /// The prelude function of the method `method` of `dense`.
    fn dense_function(&self, dense: Dense, method: &str) -> Option<usize> {
        let (_, _, function, _) = METHODS
            .iter()
            .find(|(d, name, ..)| *d == dense && *name == method)?;
        self.env.prelude_function(function)
    }

    /// U2002 at `span` when `ty`, the result of a call, is a matrix or a tensor whose element
    /// type is not numeric (ADR 0120 rule 26), as a constructor or `map` can make one.
    pub(crate) fn dense_result(&mut self, ty: &Ty, span: Span) {
        if self.module == self.env.prelude {
            return;
        }
        let Some((dense @ (Dense::Matrix | Dense::Tensor), element)) = self.dense_of(ty) else {
            return;
        };
        if self.element_class(&element).is_some() {
            return;
        }
        let resolved = self.resolve(&element);
        if matches!(resolved, Ty::Error(_) | Ty::Never | Ty::Var(_)) {
            return;
        }
        let shown = self.show(&resolved);
        let name = dense.as_str();
        self.report.error(
            Code::U2002,
            span,
            format!(
                "`{name}` takes a numeric element type, `F64` or one of `I8` to `U64`, and this call makes a `{name}[{shown}]`"
            ),
        );
    }

    /// The targeted repair of rule 3: `Array(...)` where an array is expected is the variant
    /// `Json.Array`, and the array is made by `array(...)`.
    pub(crate) fn json_array_repair(&self, expected: Option<&Ty>, name: &str) -> Option<String> {
        if name != "Array" {
            return None;
        }
        match self.dense_of(expected?)? {
            (Dense::Array, _) => Some(
                "`Array(...)` is the `Json` variant; did you mean `array(...)`, which makes an array of a list's elements?".to_owned(),
            ),
            _ => None,
        }
    }
}

/// Whether `items` are integer literals that are a permutation of 0 to `items.len() - 1`.
fn permutation(items: &[Expr]) -> bool {
    let mut seen = [false; MAX_RANK];
    for item in items {
        match literal_value(item)
            .and_then(|axis| usize::try_from(axis).ok())
            .filter(|axis| *axis < items.len())
            .and_then(|axis| seen.get_mut(axis))
        {
            Some(slot) if !*slot => *slot = true,
            _ => return false,
        }
    }
    true
}

/// The written form of an operator.
fn op_symbol(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "mod",
        _ => "?",
    }
}

/// `text`, the source of `expr`, in parentheses when it is not a primary.
fn parenthesized(expr: &Expr, text: String) -> String {
    match expr.kind {
        ExprKind::Name(_)
        | ExprKind::Paren(_)
        | ExprKind::Field { .. }
        | ExprKind::MethodCall { .. }
        | ExprKind::Call { .. } => text,
        _ => format!("({text})"),
    }
}
