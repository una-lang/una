//! `Decimal[s]` of edition 0.5 (`spec/29 §16`, ADR 0120 rules 12 to 18): literals typed by
//! their context and never rounded (U2037), the operators and their scales, the methods with
//! literal scales and written roundings, the prelude functions whose result's scale is their
//! `scale:` literal, and the conversions.
//!
//! A decimal value is its coefficient in the refinement language and in the verifier (V2), so
//! literals, `+`, `-`, unary `-`, comparisons and `*` by a literal are terms; the methods are
//! code only (U2014 in a clause), with the obligations they carry: `division` for `divide`
//! and `remainder`, a `precondition` for `rescale_exact`.

use una_core::decimal::{self, Literal as DecimalLiteral, MAX_SCALE};
use una_core::{DecimalOp, Prim};
use una_diag::{Code, ObligationKind};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, BinaryOp, Expr, ExprKind, Literal, UnaryOp};

use super::check::{Checker, Role};
use super::fixed;
use super::label::Label;
use super::records::Res;
use super::ty::Ty;
use super::units::Unit;

/// Whether `expr` is a float literal, with or without unary `-` and parentheses.
pub(crate) fn is_float_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Literal(Literal::Float(_)) => true,
        ExprKind::Paren(inner) => is_float_literal(inner),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => is_float_literal(operand),
        _ => false,
    }
}

/// The text of the float literal of `expr`, under unary `-` and parentheses.
pub(crate) fn float_text(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Literal(Literal::Float(text)) => Some(text),
        ExprKind::Paren(inner) => float_text(inner),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => float_text(operand),
        _ => None,
    }
}

/// Whether `expr` is a nonzero decimal literal: a divisor that needs no `division` obligation.
fn nonzero_float_literal(expr: &Expr) -> bool {
    float_text(expr).is_some_and(|text| !super::float::float_literal_is_zero(text))
}

/// The scale of an integer literal from 0 to 38, the value of a `scale:` argument.
fn scale_literal(expr: &Expr) -> Option<u8> {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(digits)) => {
            digits.parse::<u8>().ok().filter(|s| *s <= MAX_SCALE)
        }
        ExprKind::Paren(inner) => scale_literal(inner),
        _ => None,
    }
}

/// The parameters of a method of `Decimal[s]` (ADR 0120 rule 15), in order, and whether the
/// method is one.
fn method_params(method: &str) -> Option<&'static [&'static str]> {
    Some(match method {
        "widen" | "rescale_exact" => &["scale"],
        "rescale" => &["scale", "rounding"],
        "divide" => &["by", "scale", "rounding"],
        "remainder" => &["by", "scale"],
        "to_int" => &["rounding"],
        "to_f64" | "abs" | "coefficient" => &[],
        _ => return None,
    })
}

impl Checker<'_, '_, '_> {
    /// The scale of `ty`, resolved, when it is a `Decimal[s]` under its label.
    pub(crate) fn decimal_scale(&self, ty: &Ty) -> Option<u8> {
        match self.resolve(ty).unlabeled() {
            Ty::Decimal(scale) => Some(*scale),
            _ => None,
        }
    }

    /// Whether `ty`, resolved, is a `Decimal[s]`.
    pub(crate) fn is_decimal(&self, ty: &Ty) -> bool {
        self.decimal_scale(ty).is_some()
    }

    /// The float literal `text` at `span` where `Decimal[scale]` is expected (ADR 0120 rule
    /// 12): its exact value, padded, never rounded. A nonzero digit beyond the scale, or a
    /// coefficient of more than 4,300 digits, is U2037.
    pub(crate) fn decimal_literal(&mut self, text: &str, scale: u8, span: Span) -> Ty {
        match decimal::literal(text, scale) {
            DecimalLiteral::Value(_) => {}
            DecimalLiteral::Inexact => {
                let mut diagnostic = self.report.make(
                    Code::U2037,
                    span,
                    format!(
                        "the literal `{text}` has a nonzero digit beyond the scale of `Decimal[{scale}]`, and a decimal literal is never rounded"
                    ),
                );
                let own = decimal::literal_scale(text).unwrap_or(MAX_SCALE);
                diagnostic.repairs.push(super::Report::advice(format!(
                    "give the literal its own type, as in `let fee: Decimal[{own}] = {text}`, or write at most {scale} digits after the point"
                )));
                self.report.push(diagnostic);
            }
            DecimalLiteral::TooLong => {
                self.report.error(
                    Code::U2037,
                    span,
                    format!(
                        "the literal `{text}` has more than {} digits at the scale of `Decimal[{scale}]`, the language's digit cap",
                        una_core::MAX_INT_DIGITS
                    ),
                );
            }
        }
        Ty::Decimal(scale)
    }

    /// U2001 for the integer literal `digits` at `span` where `Decimal[scale]` is expected,
    /// with an edit that adds `.0` (ADR 0120 rule 12): the point says that a number is not an
    /// integer.
    pub(crate) fn int_literal_for_decimal(
        &mut self,
        negative: bool,
        digits: &str,
        span: Span,
        scale: u8,
    ) {
        let sign = if negative { "-" } else { "" };
        let mut diagnostic = self.report.make(
            Code::U2001,
            span,
            format!(
                "the integer literal `{sign}{digits}` is not a `Decimal[{scale}]`; a decimal literal has a point"
            ),
        );
        diagnostic.repairs.push(self.report.edit(
            format!("write `{sign}{digits}.0`"),
            span,
            format!("{sign}{digits}.0"),
        ));
        self.report.push(diagnostic);
    }

    /// The type of a float literal in a comparison with a decimal (ADR 0120 rule 12): the
    /// scale of its own digits, since a comparison is exact across scales. A literal whose own
    /// scale is above 38 is U2037.
    pub(crate) fn comparison_literal(&mut self, literal: &Expr, other: &Ty) -> Ty {
        if self.edition() >= Edition::V0_5
            && self.is_decimal(&self.number_of(other))
            && let Some(text) = float_text(literal)
        {
            match decimal::literal_scale(text) {
                // A measured decimal gives the literal its unit (ADR 0120 rule 63).
                Some(own) => {
                    let ty = Ty::Decimal(own).in_unit(self.unit_of(other).as_ref());
                    return self.infer(literal, Some(&ty));
                }
                None => {
                    self.report.error(
                        Code::U2037,
                        literal.span,
                        format!(
                            "the literal `{text}` has more than 38 digits after the point, beyond the largest scale of a decimal"
                        ),
                    );
                    return Ty::error();
                }
            }
        }
        self.infer(literal, Some(other))
    }

    /// An arithmetic operator `op` whose first operand, `first`, is a `Decimal[a]` (ADR 0120
    /// rule 13). The second operand, when a float literal, takes the scale `a` and must fit
    /// it; otherwise it must be a decimal of any scale. `+` and `-` give the larger scale, `*`
    /// the sum of the scales, at most 38; `/` and `mod` are U2001 with their repairs to
    /// `divide` and `remainder`, and an integer or a float operand is U2001 with a repair that
    /// converts it.
    pub(crate) fn decimal_arithmetic(
        &mut self,
        first_ty: Ty,
        a: u8,
        first_literal: bool,
        second: &Expr,
        op: BinaryOp,
        span: Span,
    ) -> Ty {
        let mark = self.report.count();
        // Two literal factors each take the scale of their own digits.
        let own = (first_literal && op == BinaryOp::Mul)
            .then(|| float_text(second).and_then(decimal::literal_scale))
            .flatten();
        let second_ty = if let Some(own) = own {
            self.infer(second, Some(&Ty::Decimal(own)))
        } else if is_float_literal(second) || fixed::is_constant(second) {
            // A float literal takes the scale `a`; an integer literal is U2001 with the edit
            // that adds `.0` (ADR 0120 rule 12).
            self.infer(second, Some(&Ty::Decimal(a)))
        } else {
            self.infer(second, None)
        };
        self.operands.push(first_ty);
        self.operands.push(second_ty.clone());
        let resolved = self.resolve(&second_ty);
        // The units of the operands are compared by the caller (ADR 0120 rule 63).
        let b = match resolved.magnitude() {
            Ty::Decimal(b) => *b,
            Ty::Error(_) | Ty::Never => return Ty::Decimal(a),
            found => {
                let found = found.clone();
                self.unify_at(
                    second.span,
                    &Ty::Decimal(a),
                    &found,
                    Role::Condition(&format!("an operand of `{}`", symbol(op))),
                );
                let text = self.report.text(second.span).to_owned();
                if fixed::is_integer(&found) {
                    let repair = self.report.edit(
                        "convert the integer with `to_decimal()`, exactly",
                        second.span,
                        format!("({text}).to_decimal()"),
                    );
                    self.report
                        .repair_since(mark, Code::U2001, second.span, repair);
                } else if matches!(found, Ty::F64) {
                    let repair = super::Report::advice(format!(
                        "convert the float with `({text}).to_decimal(scale: {a}, rounding: r)`, naming one of the seven `Rounding` modes as `r`; it gives an `Option[Decimal[{a}]]`, `None` for NaN and the infinities, so match its `Some` before the arithmetic"
                    ));
                    self.report
                        .repair_since(mark, Code::U2001, second.span, repair);
                }
                return Ty::Decimal(a);
            }
        };
        match op {
            BinaryOp::Add | BinaryOp::Sub => Ty::Decimal(a.max(b)),
            BinaryOp::Mul => {
                let scale = u16::from(a) + u16::from(b);
                if scale > u16::from(MAX_SCALE) {
                    let mut diagnostic = self.report.make(
                        Code::U2037,
                        span,
                        format!(
                            "the product of a `Decimal[{a}]` and a `Decimal[{b}]` has the scale {scale}, above 38"
                        ),
                    );
                    diagnostic.repairs.push(super::Report::advice(
                        "rescale an operand first, with `rescale(scale:, rounding:)`",
                    ));
                    self.report.push(diagnostic);
                    return Ty::error();
                }
                Ty::Decimal(a + b)
            }
            BinaryOp::Div => {
                let mut diagnostic = self.report.make(
                    Code::U2001,
                    span,
                    "`/` is not defined on `Decimal`; a quotient names its scale and its rounding",
                );
                diagnostic.repairs.push(super::Report::advice(format!(
                    "write `x.divide(by: y, scale: {a}, rounding: r)`, with the scale the quotient needs and, as `r`, the `Rounding` mode it needs: `HalfEven`, `HalfAwayFromZero`, `HalfTowardZero`, `TowardZero`, `AwayFromZero`, `TowardPositive` or `TowardNegative`"
                )));
                self.report.push(diagnostic);
                Ty::Decimal(a)
            }
            _ => {
                let mut diagnostic = self.report.make(
                    Code::U2001,
                    span,
                    "`mod` is not defined on `Decimal`; `x.remainder(by: y, scale: s)` is the remainder of the quotient truncated at scale s",
                );
                diagnostic.repairs.push(super::Report::advice(
                    "write `x.remainder(by: y, scale: 0)`, whose result has the sign of `x`",
                ));
                self.report.push(diagnostic);
                Ty::Decimal(a)
            }
        }
    }

    /// The scale of the argument `arg`, labeled `scale`, of `method`: an integer literal from
    /// 0 to 38, else U2037.
    fn scale_arg(&mut self, method: &str, arg: &Arg) -> Option<u8> {
        self.method_label(method, "scale", arg);
        let scale = scale_literal(&arg.value);
        self.infer(&arg.value, Some(&Ty::Int));
        if scale.is_none() {
            self.report.error(
                Code::U2037,
                arg.value.span,
                format!(
                    "the scale of `{method}` is an integer literal from 0 to 38, as in `scale: 2`"
                ),
            );
        }
        scale
    }

    /// The argument `rounding` of `method`, a `Rounding`, and its label.
    fn rounding_arg(&mut self, method: &str, arg: &Arg) -> Label {
        self.method_label(method, "rounding", arg);
        let rounding = match self.env.prelude_kind("Rounding") {
            Some(id) => Ty::Kind(id, Vec::new().into()),
            None => Ty::error(),
        };
        let what = format!("argument `rounding` of `{method}`");
        let found = self.expr_as(&arg.value, Some(&rounding), Role::Condition(&what));
        self.deep(&found)
    }

    /// The argument `by` of `method`, a decimal of any scale (a float literal takes the scale
    /// of its own digits), its scale, its label and its unit (ADR 0120 rule 63).
    fn by_arg(&mut self, method: &str, arg: &Arg) -> (Option<u8>, Label, Option<Unit>) {
        self.method_label(method, "by", arg);
        let found = match float_text(&arg.value).map(decimal::literal_scale) {
            Some(Some(own)) => self.infer(&arg.value, Some(&Ty::Decimal(own))),
            Some(None) => {
                self.report.error(
                    Code::U2037,
                    arg.value.span,
                    format!(
                        "the literal `{}` has more than 38 digits after the point, beyond the largest scale of a decimal",
                        float_text(&arg.value).unwrap_or_default()
                    ),
                );
                self.infer(&arg.value, None);
                return (None, Label::public(), None);
            }
            None => self.infer(&arg.value, None),
        };
        let label = self.deep(&found);
        let unit = self.unit_of(&found);
        match self.resolve(&found).magnitude() {
            Ty::Decimal(b) => (Some(*b), label, unit),
            Ty::Error(_) | Ty::Never => (None, label, unit),
            other => {
                let shown = self.show(other);
                let mut diagnostic = self.report.make(
                    Code::U2001,
                    arg.value.span,
                    format!("argument `by` of `{method}` must be a `Decimal`, found `{shown}`"),
                );
                if fixed::is_integer(other) {
                    let text = self.report.text(arg.value.span).to_owned();
                    diagnostic.repairs.push(self.report.edit(
                        "convert the integer with `to_decimal()`, exactly",
                        arg.value.span,
                        format!("({text}).to_decimal()"),
                    ));
                }
                self.report.push(diagnostic);
                (None, label, unit)
            }
        }
    }

    /// `e.m(args)` when `e` is a `Decimal[a]` and `m` one of its methods, or `e` an integer or
    /// an `F64` and `m` is `to_decimal`, in edition 0.5 (ADR 0120 rule 15). `None` when it is
    /// neither, so the call resolves as before.
    pub(crate) fn decimal_method(
        &mut self,
        target: &Ty,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        if self.edition() < Edition::V0_5 {
            return None;
        }
        let receiver = self.resolve(target);
        let mut label = self.deep(target);
        // A measured receiver keeps its unit through a conversion between number types (ADR
        // 0114 rule 12.6), and takes the units of ADR 0120 rule 63.
        let unit = receiver.unit().cloned();
        match receiver.magnitude() {
            // `n.to_decimal()` on an integer: exact, `Decimal[0]`.
            Ty::Int | Ty::Fixed(_) if method == "to_decimal" => {
                self.outside_rl(span, || format!("the method `{method}`"));
                if !args.is_empty() {
                    self.report.error(
                        Code::U2001,
                        span,
                        "`to_decimal` of an integer takes no argument, as in `n.to_decimal()`",
                    );
                    self.unchecked_args(args);
                }
                self.resolved(span, Res::IntMethod(Prim::Decimal(DecimalOp::FromInt)));
                Some(Ty::Decimal(0).in_unit(unit.as_ref()).with_label(&label))
            }
            // `x.to_decimal(scale: s, rounding: r)` on an `F64`: `None` for NaN and the
            // infinities, else its exact binary value rounded once.
            Ty::F64 if method == "to_decimal" => {
                self.outside_rl(span, || format!("the method `{method}`"));
                let [scale, rounding] = args else {
                    self.report.error(
                        Code::U2001,
                        span,
                        "`to_decimal` of an `F64` takes `scale:` and `rounding:`, as in `x.to_decimal(scale: 2, rounding: HalfEven)`",
                    );
                    self.unchecked_args(args);
                    return Some(Ty::error());
                };
                let s = self.scale_arg(method, scale);
                label = label.join(&self.rounding_arg(method, rounding));
                let Some(s) = s else {
                    return Some(Ty::error());
                };
                self.resolved(span, Res::IntMethod(Prim::Decimal(DecimalOp::FromF64(s))));
                let decimal = Ty::Decimal(s).in_unit(unit.as_ref()).with_label(&label);
                Some(match self.env.prelude_kind("Option") {
                    Some(id) => Ty::Kind(id, vec![decimal].into()).with_label(&label),
                    None => Ty::error(),
                })
            }
            Ty::Decimal(a) => {
                let a = *a;
                let params = method_params(method)?;
                // `coefficient()` is the decimal's own term (ADR 0120 V2); the other methods
                // are code only.
                if method != "coefficient" {
                    self.outside_rl(span, || format!("the method `{method}`"));
                }
                if args.len() != params.len() {
                    let shape = params
                        .iter()
                        .map(|param| format!("{param}: {param}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.report.error(
                        Code::U2001,
                        span,
                        format!(
                            "`{method}` takes {} argument(s), as in `x.{method}({shape})`",
                            params.len()
                        ),
                    );
                    self.unchecked_args(args);
                    return Some(Ty::error());
                }
                let result =
                    self.decimal_method_args(a, method, args, span, &mut label, unit.as_ref())?;
                Some(result.with_label(&label))
            }
            _ => None,
        }
    }

    /// The arguments and the result of the method `method` of a `Decimal[a]` in `unit`, whose
    /// arity is checked; `None` when a scale is not a literal, after its U2037. The units of
    /// ADR 0120 rule 63: `widen`, `rescale`, `rescale_exact`, `abs`, `to_int` and `to_f64`
    /// keep the receiver's unit, `divide` divides it by the unit of `by`, `remainder` needs
    /// one unit on both and keeps it, and `coefficient` needs a unitless receiver (U2028).
    fn decimal_method_args(
        &mut self,
        a: u8,
        method: &str,
        args: &[Arg],
        span: Span,
        label: &mut Label,
        unit: Option<&Unit>,
    ) -> Option<Ty> {
        let keep = |ty: Ty| ty.in_unit(unit);
        let resolve = |this: &mut Self, op: DecimalOp| {
            this.resolved(span, Res::IntMethod(Prim::Decimal(op)));
        };
        Some(match (method, args) {
            ("widen", [scale]) => {
                let Some(s) = self.scale_arg(method, scale) else {
                    return Some(Ty::error());
                };
                if s < a {
                    let mut diagnostic = self.report.make(
                        Code::U2037,
                        scale.value.span,
                        format!(
                            "`widen` adds digits, and `Decimal[{s}]` has fewer than `Decimal[{a}]`"
                        ),
                    );
                    diagnostic.repairs.push(super::Report::advice(format!(
                        "drop digits with `rescale(scale: {s}, rounding: r)`, naming the `Rounding` mode `r` that the value needs"
                    )));
                    self.report.push(diagnostic);
                    return Some(Ty::error());
                }
                resolve(self, DecimalOp::Widen(a, s));
                keep(Ty::Decimal(s))
            }
            ("rescale" | "rescale_exact", [scale, rest @ ..]) => {
                let s = self.scale_arg(method, scale);
                if let [rounding] = rest {
                    *label = label.join(&self.rounding_arg(method, rounding));
                }
                let Some(s) = s else {
                    return Some(Ty::error());
                };
                if s >= a {
                    let mut diagnostic = self.report.make(
                        Code::U2037,
                        scale.value.span,
                        format!(
                            "`{method}` drops digits, and `Decimal[{s}]` has no fewer than `Decimal[{a}]`"
                        ),
                    );
                    diagnostic.repairs.push(super::Report::advice(format!(
                        "add digits with `widen(scale: {s})`, which is exact"
                    )));
                    self.report.push(diagnostic);
                    return Some(Ty::error());
                }
                if method == "rescale" {
                    resolve(self, DecimalOp::Rescale(a, s));
                } else {
                    // `requires` that no nonzero digit is dropped (V2).
                    self.obligation(ObligationKind::Precondition, span, None);
                    resolve(self, DecimalOp::RescaleExact(a, s));
                }
                keep(Ty::Decimal(s))
            }
            ("divide" | "remainder", [by, scale, rest @ ..]) => {
                let (b, by_label, by_unit) = self.by_arg(method, by);
                *label = label.join(&by_label);
                let s = self.scale_arg(method, scale);
                if let [rounding] = rest {
                    *label = label.join(&self.rounding_arg(method, rounding));
                }
                if !nonzero_float_literal(&by.value) {
                    self.obligation(ObligationKind::Division, span, None);
                }
                let (Some(b), Some(s)) = (b, s) else {
                    return Some(Ty::error());
                };
                if method == "divide" {
                    resolve(self, DecimalOp::Divide(a, b, s));
                    let quotient = self.quotient_unit(unit, by_unit.as_ref(), span);
                    Ty::Decimal(s).in_unit(quotient.as_ref())
                } else {
                    if unit != by_unit.as_ref() {
                        let shown = |u: Option<&Unit>| {
                            u.map_or_else(|| "no unit".to_owned(), |u| format!("`{u}`"))
                        };
                        self.report.error(
                            Code::U2028,
                            by.value.span,
                            format!(
                                "`remainder` needs one unit on both operands, and these have {} and {}",
                                shown(unit),
                                shown(by_unit.as_ref())
                            ),
                        );
                    }
                    let t = u16::from(a).max(u16::from(b) + u16::from(s));
                    if t > u16::from(MAX_SCALE) {
                        self.report.error(
                            Code::U2037,
                            span,
                            format!(
                                "the remainder of a `Decimal[{a}]` by a `Decimal[{b}]` at scale {s} has the scale {t}, above 38"
                            ),
                        );
                        return Some(Ty::error());
                    }
                    resolve(self, DecimalOp::Remainder(a, b, s));
                    keep(Ty::Decimal(decimal::remainder_scale(a, b, s)))
                }
            }
            ("to_int", [rounding]) => {
                *label = label.join(&self.rounding_arg(method, rounding));
                resolve(self, DecimalOp::ToInt(a));
                keep(Ty::Int)
            }
            ("to_f64", []) => {
                resolve(self, DecimalOp::ToF64(a));
                keep(Ty::F64)
            }
            ("abs", []) => {
                resolve(self, DecimalOp::Abs);
                keep(Ty::Decimal(a))
            }
            ("coefficient", []) => {
                if unit.is_some() {
                    let mut diagnostic = self.report.make(
                        Code::U2028,
                        span,
                        "`coefficient` takes a unitless value, and this one has a unit",
                    );
                    diagnostic
                        .repairs
                        .push(super::Report::advice("write `x.magnitude().coefficient()`"));
                    self.report.push(diagnostic);
                }
                resolve(self, DecimalOp::Coefficient);
                Ty::Int
            }
            _ => return Some(Ty::error()),
        })
    }

    /// The scale of the prelude function call `name(args)` whose result's scale is its
    /// `scale:` literal (`parse_decimal`, `as_decimal`), after `call_function` checked it as
    /// declared at scale 0: the result `ty` at that scale (ADR 0120 rules 16 and 17).
    pub(crate) fn decimal_function_result(
        &mut self,
        name: &str,
        method: bool,
        args: &[Arg],
        span: Span,
        ty: Ty,
    ) -> Ty {
        // The argument that binds the parameter `scale`: the one labeled `scale`, else the one
        // at its position, written, punned or unlabeled. Whichever it is, it must be a
        // literal, so a variable never sets the scale at run time.
        let position = if method { 0 } else { 1 };
        let scale = args
            .iter()
            .find(|arg| arg.label.as_ref().is_some_and(|l| self.name(l) == "scale"))
            .or_else(|| args.get(position));
        let Some(scale) = scale else {
            self.report.error(
                Code::U2037,
                span,
                format!("`{name}` takes the scale of its result as a literal, as in `scale: 2`"),
            );
            return Ty::error();
        };
        let Some(s) = scale_literal(&scale.value) else {
            self.report.error(
                Code::U2037,
                scale.value.span,
                format!(
                    "the scale of `{name}` is an integer literal from 0 to 38, as in `scale: 2`"
                ),
            );
            return Ty::error();
        };
        let label = self.deep(&ty);
        let decimal = Ty::Decimal(s).with_label(&label);
        if name == "as_decimal" {
            match self.env.prelude_kind("Option") {
                Some(id) => Ty::Kind(id, vec![decimal].into()).with_label(&label),
                None => Ty::error(),
            }
        } else {
            decimal
        }
    }

    /// `json_decimal(x)` after `call_function` checked it: `x` must be a `Decimal[s]` (ADR
    /// 0120 rule 17), else U2001.
    pub(crate) fn json_decimal_arg(&mut self, value: Span, span: Span) {
        let found = self.records.types.get(&value).cloned();
        if let Some(found) = found
            && !self.is_decimal(&found)
            && !matches!(self.resolve(&found).unlabeled(), Ty::Error(_) | Ty::Never)
        {
            let shown = self.show(&found);
            self.report.error(
                Code::U2001,
                span,
                format!("`json_decimal` writes a `Decimal`, not `{shown}`"),
            );
        }
    }
}

/// The symbol of an arithmetic operator.
fn symbol(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        _ => "mod",
    }
}

impl Checker<'_, '_, '_> {
    /// The type of a call of the function `id`, `ty` as `call_function` checked it: for the
    /// prelude's `parse_decimal` and `as_decimal` in edition 0.5, at the scale of their
    /// `scale:` literal, and for `json_decimal`, after the check that its argument is a decimal
    /// (ADR 0120 rules 16 and 17). A package's function of one of these names is its own.
    pub(crate) fn decimal_call(
        &mut self,
        id: usize,
        receiver: Option<Span>,
        args: &[Arg],
        span: Span,
        ty: Ty,
    ) -> Ty {
        if self.edition() < Edition::V0_5 {
            return ty;
        }
        let Some(def) = self.env.functions.get(id) else {
            return ty;
        };
        if def.module != self.env.prelude {
            return ty;
        }
        match def.name {
            name @ ("parse_decimal" | "as_decimal") => {
                self.outside_rl(span, || format!("a call of `{name}`"));
                self.decimal_function_result(name, receiver.is_some(), args, span, ty)
            }
            "json_decimal" => {
                let value = receiver.or_else(|| args.first().map(|arg| arg.value.span));
                if let Some(value) = value {
                    self.json_decimal_arg(value, span);
                }
                ty
            }
            _ => ty,
        }
    }
}

impl Checker<'_, '_, '_> {
    /// U2037 when the prelude's `parse_decimal`, `as_decimal` or `json_decimal` is used as a
    /// value in edition 0.5: the scale of their result is the literal of each call (ADR 0120
    /// rules 16 and 17), which a function value cannot carry.
    pub(crate) fn decimal_function_value(&mut self, id: usize, span: Span) {
        if self.edition() < Edition::V0_5 {
            return;
        }
        let Some(def) = self.env.functions.get(id) else {
            return;
        };
        if def.module != self.env.prelude
            || !matches!(def.name, "parse_decimal" | "as_decimal" | "json_decimal")
        {
            return;
        }
        let name = def.name;
        let mut diagnostic = self.report.make(
            Code::U2037,
            span,
            format!(
                "`{name}` takes the scale of its result from each call, so it is not a function value"
            ),
        );
        diagnostic.repairs.push(super::Report::advice(format!(
            "write a lambda that calls it, such as `fn(t) -> {name}(t, scale: 2)`"
        )));
        self.report.push(diagnostic);
    }
}
