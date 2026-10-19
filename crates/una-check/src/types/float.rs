//! `F64` of edition 0.3 (`spec/29`, ADR 0114): float literals and their range (U2031), the
//! methods and conversions resolved by the receiver, and the comparisons with a literal zero
//! (U2033) and the float patterns (U2032).
//!
//! In this step, no `F64` term enters the refinement language: a float literal, an operator or
//! a method of `F64` in a clause is U2014, so the verifier never reads a float, and every float
//! value is a fresh binder of an uninterpreted sort to it (`spec/29 §8`).

use una_core::float::{self, Literal as FloatLiteral};
use una_core::{FloatOp, FloatResult, Prim};
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Expr, ExprKind, Literal, UnaryOp};

use super::check::{Checker, Role};
use super::records::Res;
use super::ty::Ty;

/// Whether `expr` is a float literal of value zero, with or without unary `-` and
/// parentheses: `0.0`, `-0.0`, `0.000` (ADR 0114 rule 5.5).
pub(crate) fn is_zero_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Literal(Literal::Float(text)) => float_literal_is_zero(text),
        ExprKind::Paren(inner) => is_zero_literal(inner),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => is_zero_literal(operand),
        _ => false,
    }
}

/// Whether the float literal `text` has the value zero.
pub(crate) fn float_literal_is_zero(text: &str) -> bool {
    let mantissa = text.split(['e', 'E']).next().unwrap_or("");
    !mantissa.chars().any(|c| matches!(c, '1'..='9'))
}

impl Checker<'_, '_, '_> {
    /// The type of the float literal `text` at `span`: `F64`, after U2031 when it rounds to
    /// an infinity, or is not zero and rounds to zero (ADR 0114 rule 2.5).
    pub(crate) fn float_literal(&mut self, text: &str, span: Span) -> Ty {
        self.outside_rl(span, || "a float literal".to_owned());
        match float::literal(text) {
            FloatLiteral::Value(_) => {}
            FloatLiteral::Overflows => {
                let mut diagnostic = self.report.make(
                    Code::U2031,
                    span,
                    format!(
                        "the float literal `{text}` rounds to an infinity; the largest finite `F64` is `1.7976931348623157e308`"
                    ),
                );
                diagnostic.repairs.push(super::Report::advice(
                    "write `infinity()` if an infinity is meant, or write the value in range",
                ));
                self.report.push(diagnostic);
            }
            FloatLiteral::Underflows => {
                let mut diagnostic = self.report.make(
                    Code::U2031,
                    span,
                    format!(
                        "the float literal `{text}` is not zero and rounds to zero; the smallest positive `F64` is `5.0e-324`"
                    ),
                );
                diagnostic
                    .repairs
                    .push(self.report.edit("write `0.0`", span, "0.0"));
                self.report.push(diagnostic);
            }
        }
        Ty::F64
    }

    /// Whether `ty`, resolved, is `F64` under its label.
    pub(crate) fn is_f64(&self, ty: &Ty) -> bool {
        matches!(self.resolve(ty).unlabeled(), Ty::F64)
    }

    /// U2001 for the integer literal `digits` at `span` where `F64` is expected, with an edit
    /// to the float literal (ADR 0114 rule 2.6): an integer literal never takes the type `F64`.
    pub(crate) fn int_literal_for_f64(&mut self, negative: bool, digits: &str, span: Span) {
        let sign = if negative { "-" } else { "" };
        let mut diagnostic = self.report.make(
            Code::U2001,
            span,
            format!(
                "the integer literal `{sign}{digits}` is not an `F64`; a float literal has a point"
            ),
        );
        diagnostic.repairs.push(self.report.edit(
            format!("write `{sign}{digits}.0`"),
            span,
            format!("{sign}{digits}.0"),
        ));
        self.report.push(diagnostic);
    }

    /// U2033 when one operand of the comparison `symbol` at `span` is an `F64` and the other is
    /// a float literal zero (ADR 0114 rule 5.5), with the repairs of its form. `equality` says
    /// whether it is `==`, `!=` or `is`.
    pub(crate) fn zero_comparison(
        &mut self,
        symbol: &str,
        left: &Expr,
        right: &Expr,
        span: Span,
        equality: bool,
    ) {
        let (value, zero) = if is_zero_literal(right) {
            (left, right)
        } else if is_zero_literal(left) {
            (right, left)
        } else {
            return;
        };
        let negative_zero = matches!(
            &zero.kind,
            ExprKind::Unary {
                op: UnaryOp::Neg,
                ..
            }
        );
        let x = self.report.text(value.span).to_owned();
        let x = if matches!(
            value.kind,
            ExprKind::Name(_) | ExprKind::Paren(_) | ExprKind::Field { .. }
        ) || matches!(value.kind, ExprKind::MethodCall { .. })
        {
            x
        } else {
            format!("({x})")
        };
        let mut diagnostic = self.report.make(
            Code::U2033,
            span,
            format!(
                "`{symbol}` compares an `F64` with a literal zero, which tells `-0.0` from `0.0` by the order of `F64`"
            ),
        );
        if equality {
            let negate = if symbol == "!=" { "not " } else { "" };
            diagnostic.repairs.push(self.report.edit(
                format!("test both zeros with `{negate}{x}.is_zero()`"),
                span,
                format!("{negate}{x}.is_zero()"),
            ));
            let one = if negative_zero {
                "is_negative_zero"
            } else {
                "is_positive_zero"
            };
            diagnostic.repairs.push(super::Report::advice(format!(
                "test one zero with `{negate}{x}.{one}()`"
            )));
        } else {
            // The zero on the right: `x < 0.0`; on the left, `0.0 < x` reads as `x > 0.0`.
            let zero_on_right = std::ptr::eq(zero, right);
            let method = match (symbol, zero_on_right) {
                ("<", true) | (">", false) => "ieee_lt",
                ("<=", true) | (">=", false) => "ieee_le",
                (">", true) | ("<", false) => "ieee_gt",
                _ => "ieee_ge",
            };
            diagnostic.repairs.push(self.report.edit(
                format!(
                    "compare as IEEE does, `-0.0` equal to `0.0`, with `{x}.{method}(other: 0.0)`"
                ),
                span,
                format!("{x}.{method}(other: 0.0)"),
            ));
            diagnostic.repairs.push(super::Report::advice(format!(
                "test the sign bit with `{x}.is_sign_negative()` or `not {x}.is_sign_negative()`"
            )));
        }
        self.report.push(diagnostic);
    }

    /// U2032 for a float literal other than zero as a pattern (`is` or a `when` arm) at `span`
    /// (ADR 0114 rule 5.6); a literal zero is U2033 (rule 5.5).
    pub(crate) fn float_pattern(&mut self, text: &str, span: Span) {
        if float_literal_is_zero(text) {
            let mut diagnostic = self.report.make(
                Code::U2033,
                span,
                format!(
                    "the pattern `{text}` compares an `F64` with a literal zero, which tells `-0.0` from `0.0` by the order of `F64`"
                ),
            );
            diagnostic.repairs.push(super::Report::advice(
                "test both zeros with an `if` on `x.is_zero()`, or one zero with `x.is_positive_zero()` or `x.is_negative_zero()`",
            ));
            self.report.push(diagnostic);
        } else {
            let mut diagnostic = self.report.make(
                Code::U2032,
                span,
                format!(
                    "the float literal `{text}` is not a pattern; a computed `F64` rarely equals a literal"
                ),
            );
            diagnostic.repairs.push(super::Report::advice(format!(
                "compare in an `if`, with `x == {text}` or an IEEE method such as `x.ieee_eq(other: {text})`"
            )));
            self.report.push(diagnostic);
        }
    }

    /// `e.m(args)` when `e` is an `F64` and `m` a method of `F64`, or `e` an integer and `m`
    /// `to_f64()`, in edition 0.3 (ADR 0114 rules 3 and 4). `None` when it is neither, so the
    /// call resolves as before.
    pub(crate) fn float_method(
        &mut self,
        target: &Ty,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        if self.env.edition < Edition::V0_3 {
            return None;
        }
        let receiver = self.resolve(target);
        let label = self.deep(target);
        match receiver.unlabeled() {
            // ADR 0114 rule 4.1: `n.to_f64()` on an integer, ahead of a function of that name.
            Ty::Int | Ty::Fixed(_) if method == "to_f64" && args.is_empty() => {
                self.outside_rl(span, || format!("the method `{method}`"));
                self.resolved(span, Res::IntMethod(Prim::Float(FloatOp::FromInt)));
                Some(Ty::F64.with_label(&label))
            }
            Ty::F64 => {
                let Some(op) = FloatOp::from_method(method) else {
                    return self.no_float_method(method, args, span);
                };
                self.outside_rl(span, || format!("the method `{method}`"));
                let params = op.params();
                let mut label = label;
                if args.len() == params.len() {
                    for (arg, param) in args.iter().zip(params) {
                        self.method_label(method, param, arg);
                        let what = format!("argument `{param}` of `{method}`");
                        let found =
                            self.expr_as(&arg.value, Some(&Ty::F64), Role::Condition(&what));
                        label = label.join(&self.deep(&found));
                    }
                } else {
                    let shape = params
                        .iter()
                        .map(|param| format!("{param}: y"))
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
                }
                self.resolved(span, Res::IntMethod(Prim::Float(op)));
                let result = match op.result() {
                    FloatResult::F64 => Ty::F64,
                    FloatResult::Bool => Ty::Bool,
                    FloatResult::U64 => Ty::Fixed(una_core::IntType::U64),
                    FloatResult::OptionInt => match self.env.prelude_kind("Option") {
                        Some(id) => Ty::Kind(id, vec![Ty::Int].into()),
                        None => Ty::error(),
                    },
                };
                Some(result.with_label(&label))
            }
            _ => None,
        }
    }

    /// A method of `F64`'s receiver that is not one of its methods: the conversions to
    /// integers other than `to_int_exact()` are U2001 with their repair (ADR 0114 rule 4.3);
    /// any other name resolves as a function of the package (`spec/10 §5`).
    fn no_float_method(&mut self, method: &str, args: &[Arg], span: Span) -> Option<Ty> {
        let conversion = method == "to_int"
            || method
                .strip_prefix("to_")
                .is_some_and(|t| una_core::IntType::from_name(&t.to_ascii_uppercase()).is_some());
        if !conversion {
            return None;
        }
        let mut diagnostic = self.report.make(
            Code::U2001,
            span,
            format!(
                "`{method}` is not a conversion of `F64`; `to_int_exact()` gives `Some` of an integral value and `None` otherwise"
            ),
        );
        diagnostic.repairs.push(super::Report::advice(
            "write `x.to_int_exact()`, after `floor()`, `ceil()`, `trunc()` or a rounding when the value may have a fraction",
        ));
        self.report.push(diagnostic);
        self.unchecked_args(args);
        Some(Ty::error())
    }
}
