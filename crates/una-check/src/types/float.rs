//! `F64` of edition 0.3 (`spec/29`, ADR 0114): float literals and their range (U2031), the
//! methods and conversions resolved by the receiver, and the comparisons with a literal zero
//! (U2033) and the float patterns (U2032).
//!
//! The refinement language holds float literals, `nan()`, `infinity()`, the operators, the
//! comparisons, `abs()`, `sqrt()`, the IEEE comparisons, the classifications and `to_f64()` of
//! an integer; every other method of `F64` is code only, U2014 in a clause (`spec/29 §8`,
//! ADR 0114 rule 8.4).

use una_core::float::{self, Literal as FloatLiteral};
use una_core::float32::{self, Literal as Float32Literal};
use una_core::{FloatOp, FloatResult, Prim};
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Expr, ExprKind, Literal, UnaryOp};

use super::check::{Checker, Role};
use super::records::Res;
use super::ty::Ty;
use super::units::Unit;

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

/// The name of the float type `ty`, `F32` or `F64`.
fn float_name(ty: &Ty) -> &'static str {
    if matches!(ty.unlabeled(), Ty::F32) {
        "F32"
    } else {
        "F64"
    }
}

/// Whether the float literal `text` has the value zero.
pub(crate) fn float_literal_is_zero(text: &str) -> bool {
    let mantissa = text.split(['e', 'E']).next().unwrap_or("");
    !mantissa.chars().any(|c| matches!(c, '1'..='9'))
}

impl Checker<'_, '_, '_> {
    /// The type of the float literal `text` at `span`: `F32` where `expected` is an `F32`
    /// (ADR 0120 rule 6), `F64` elsewhere, after U2031 when it rounds to an infinity, or is
    /// not zero and rounds to zero (ADR 0114 rule 2.5).
    pub(crate) fn float_literal(&mut self, text: &str, span: Span, expected: Option<&Ty>) -> Ty {
        if expected.is_some_and(|e| self.is_f32(e)) {
            return self.f32_literal(text, span);
        }
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

    /// The type of the float literal `text` at `span` where an `F32` is expected: `F32`,
    /// rounded once from its decimal value, after U2031 at the binary32 bounds (ADR 0120
    /// rule 5).
    fn f32_literal(&mut self, text: &str, span: Span) -> Ty {
        match float32::literal(text) {
            Float32Literal::Value(_) => {}
            Float32Literal::Overflows => {
                let mut diagnostic = self.report.make(
                    Code::U2031,
                    span,
                    format!(
                        "the float literal `{text}` rounds to an infinity as an `F32`; the largest finite `F32` is `3.4028235e38`"
                    ),
                );
                diagnostic.repairs.push(super::Report::advice(
                    "write `infinity().to_f32()` if an infinity is meant, or write the value in range",
                ));
                self.report.push(diagnostic);
            }
            Float32Literal::Underflows => {
                let mut diagnostic = self.report.make(
                    Code::U2031,
                    span,
                    format!(
                        "the float literal `{text}` is not zero and rounds to zero as an `F32`; the smallest positive `F32` is `1.0e-45`"
                    ),
                );
                diagnostic
                    .repairs
                    .push(self.report.edit("write `0.0`", span, "0.0"));
                self.report.push(diagnostic);
            }
        }
        Ty::F32
    }

    /// Whether `ty`, resolved, is `F64` under its label.
    pub(crate) fn is_f64(&self, ty: &Ty) -> bool {
        matches!(self.resolve(ty).unlabeled(), Ty::F64)
    }

    /// Whether `ty`, resolved, is `F32` under its label and refinement.
    pub(crate) fn is_f32(&self, ty: &Ty) -> bool {
        matches!(self.resolve(ty).unlabeled(), Ty::F32)
    }

    /// The name of the float type of `a`, or else of `b`, measured or not: `F32` when either
    /// is one, `F64` otherwise.
    pub(crate) fn float_name_of(&self, a: &Ty, b: &Ty) -> &'static str {
        if matches!(self.number_of(a), Ty::F32) || matches!(self.number_of(b), Ty::F32) {
            "F32"
        } else {
            "F64"
        }
    }

    /// Whether `ty`, resolved, is `F64` or `F32`.
    pub(crate) fn is_float(&self, ty: &Ty) -> bool {
        matches!(self.resolve(ty).unlabeled(), Ty::F64 | Ty::F32)
    }

    /// The repair of a comparison of an `F64` with an `F32` that `same` reported since `mark`:
    /// convert the second operand to the first one's type (ADR 0120 rule 7).
    pub(crate) fn mixed_floats(
        &mut self,
        mark: usize,
        span: Span,
        right: &Expr,
        left_ty: &Ty,
        right_ty: &Ty,
    ) {
        let (l, r) = (self.resolve(left_ty), self.resolve(right_ty));
        let method = match (l.unlabeled(), r.unlabeled()) {
            (Ty::F64, Ty::F32) => "to_f64",
            (Ty::F32, Ty::F64) => "to_f32",
            _ => return,
        };
        let text = self.report.text(right.span).to_owned();
        let repair = self.report.edit(
            format!("convert it with `{method}()`"),
            right.span,
            format!("({text}).{method}()"),
        );
        self.report.repair_since(mark, Code::U2001, span, repair);
    }

    /// U2001 for the integer literal `digits` at `span` where `F64` is expected, with an edit
    /// to the float literal (ADR 0114 rule 2.6): an integer literal never takes the type `F64`,
    /// nor `F32` (ADR 0120 rule 6).
    pub(crate) fn int_literal_for_f64(&mut self, negative: bool, digits: &str, span: Span) {
        self.int_literal_for_float("F64", negative, digits, span);
    }

    /// [`Self::int_literal_for_f64`] where the float type `name` is expected.
    pub(crate) fn int_literal_for_float(
        &mut self,
        name: &str,
        negative: bool,
        digits: &str,
        span: Span,
    ) {
        let sign = if negative { "-" } else { "" };
        let mut diagnostic = self.report.make(
            Code::U2001,
            span,
            format!(
                "the integer literal `{sign}{digits}` is not an `{name}`; a float literal has a point"
            ),
        );
        if digits.chars().all(|c| c == '0') {
            // `0.0` would be U2033 in a comparison (ADR 0114 rule 5.5): say both.
            diagnostic.repairs.push(super::Report::advice(
                "compare with zero by `x.is_zero()` or an IEEE method such as `x.ieee_lt(other: 0.0)`; elsewhere write `0.0`",
            ));
        } else {
            diagnostic.repairs.push(self.report.edit(
                format!("write `{sign}{digits}.0`"),
                span,
                format!("{sign}{digits}.0"),
            ));
        }
        self.report.push(diagnostic);
    }

    /// U2033 when one operand of the comparison `symbol` at `span` is an `F64` and the other is
    /// a float literal zero (ADR 0114 rule 5.5), with the repairs of its form. `equality` says
    /// whether it is `==`, `!=` or `is`.
    pub(crate) fn zero_comparison(
        &mut self,
        name: &str,
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
                "`{symbol}` compares an `{name}` with a literal zero, which tells `-0.0` from `0.0` by the order of `{name}`"
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
    pub(crate) fn float_pattern(&mut self, name: &str, text: &str, span: Span) {
        if float_literal_is_zero(text) {
            let mut diagnostic = self.report.make(
                Code::U2033,
                span,
                format!(
                    "the pattern `{text}` compares an `{name}` with a literal zero, which tells `-0.0` from `0.0` by the order of `{name}`"
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
                    "the float literal `{text}` is not a pattern; a computed `{name}` rarely equals a literal"
                ),
            );
            diagnostic.repairs.push(super::Report::advice(format!(
                "compare in an `if`, with `x == {text}` or an IEEE method such as `x.ieee_eq(other: {text})`"
            )));
            self.report.push(diagnostic);
        }
    }

    /// `e.m(args)` when `e` is an `F64` or an `F32` and `m` one of its methods, or `e` an
    /// integer and `m` `to_f64()` (edition 0.3) or `to_f32()` (edition 0.5), or a conversion
    /// between the float types (ADR 0114 rules 3 and 4, ADR 0120 rules 5 and 8). `None` when it
    /// is neither, so the call resolves as before.
    pub(crate) fn float_method(
        &mut self,
        target: &Ty,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        if self.edition() < Edition::V0_3 {
            return None;
        }
        let edition_0_5 = self.edition() >= Edition::V0_5;
        let receiver = self.resolve(target);
        let label = self.deep(target);
        // A measured receiver takes its magnitude's methods, with the units of ADR 0114 rule
        // 12 (ADR 0120 rule 62): a conversion between number types keeps the unit (12.6).
        let unit = receiver.unit().cloned();
        let keep = |ty: Ty| ty.in_unit(unit.as_ref());
        let float = match receiver.magnitude() {
            // ADR 0114 rule 4.1: `n.to_f64()` on an integer, ahead of a function of that name.
            Ty::Int | Ty::Fixed(_) if method == "to_f64" && args.is_empty() => {
                self.resolved(span, Res::IntMethod(Prim::Float(FloatOp::FromInt)));
                return Some(keep(Ty::F64.with_label(&label)));
            }
            // ADR 0120 rule 8: `n.to_f32()` on an integer, the same in edition 0.5.
            Ty::Int | Ty::Fixed(_) if edition_0_5 && method == "to_f32" && args.is_empty() => {
                self.resolved(span, Res::IntMethod(Prim::Float32(FloatOp::FromInt)));
                return Some(keep(Ty::F32.with_label(&label)));
            }
            // ADR 0120 rule 8: the conversions between the float types.
            Ty::F64 if edition_0_5 && method == "to_f32" && args.is_empty() => {
                self.resolved(span, Res::IntMethod(Prim::Float(FloatOp::ToF32)));
                return Some(keep(Ty::F32.with_label(&label)));
            }
            Ty::F32 if method == "to_f64" && args.is_empty() => {
                self.resolved(span, Res::IntMethod(Prim::Float32(FloatOp::ToF64)));
                return Some(keep(Ty::F64.with_label(&label)));
            }
            Ty::F64 => Ty::F64,
            Ty::F32 => Ty::F32,
            _ => return None,
        };
        let name = float_name(&float);
        let Some(op) = FloatOp::from_method(method) else {
            return self.no_float_method(name, method, args, span);
        };
        if !op.in_refinement_language() {
            self.outside_rl(span, || format!("the method `{method}`"));
        }
        // ADR 0114 rule 12.2: `to_bits` needs a unitless receiver.
        if unit.is_some() && op == FloatOp::ToBits {
            let mut diagnostic = self.report.make(
                Code::U2028,
                span,
                format!("`{method}` takes a unitless value, and this one has a unit"),
            );
            diagnostic.repairs.push(super::Report::advice(format!(
                "write `x.magnitude().{method}()`"
            )));
            self.report.push(diagnostic);
        }
        let params = op.params();
        let mut label = label;
        // The receiver's type with its unit: what an argument in the receiver's unit is.
        let measured = keep(float.clone());
        // `fma`: the unit of `times`, then the unit of `x * times`, which `plus` and the
        // result have.
        let mut times_unit: Option<Option<Unit>> = None;
        let mut fma_unit: Option<Option<Unit>> = None;
        if args.len() == params.len() {
            for (arg, param) in args.iter().zip(params) {
                self.method_label(method, param, arg);
                let what = format!("argument `{param}` of `{method}`");
                let found = match (op, *param) {
                    // Any unit (rule 12.2): `copy_sign` reads only the sign of `from`, and the
                    // unit of `times` multiplies the receiver's.
                    (FloatOp::CopySign, _) | (FloatOp::Fma, "times") => {
                        let found = self.infer(&arg.value, Some(&float));
                        if self.number_of(&found) != float {
                            self.unify_at(arg.value.span, &float, &found, Role::Condition(&what));
                        }
                        if op == FloatOp::Fma {
                            times_unit = Some(self.unit_of(&found));
                        }
                        found
                    }
                    // `plus` is in the unit of `x * times`.
                    (FloatOp::Fma, _) => {
                        let product = match &times_unit {
                            Some(times) => self.product_unit(unit.as_ref(), times.as_ref(), span),
                            None => unit.clone(),
                        };
                        let expected = float.clone().in_unit(product.as_ref());
                        fma_unit = Some(product);
                        self.expr_as(&arg.value, Some(&expected), Role::Condition(&what))
                    }
                    // Every other argument is in the receiver's unit.
                    _ => self.expr_as(&arg.value, Some(&measured), Role::Condition(&what)),
                };
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
        let prim = if float == Ty::F32 {
            Prim::Float32(op)
        } else {
            Prim::Float(op)
        };
        self.resolved(span, Res::IntMethod(prim));
        let result = match op.result() {
            FloatResult::F64 => float,
            FloatResult::Other if name == "F32" => Ty::F64,
            FloatResult::Other => Ty::F32,
            FloatResult::Bool => Ty::Bool,
            FloatResult::U64 if name == "F32" => Ty::Fixed(una_core::IntType::U32),
            FloatResult::U64 => Ty::Fixed(una_core::IntType::U64),
            FloatResult::OptionInt => match self.env.prelude_kind("Option") {
                // `to_int_exact()` keeps the unit inside the option (rule 12.6).
                Some(id) => Ty::Kind(id, vec![keep(Ty::Int)].into()),
                None => Ty::error(),
            },
        };
        // The unit of the result (rule 12.2 and 12.3).
        let result = match (op.result(), op) {
            (FloatResult::F64 | FloatResult::Other, FloatOp::Sqrt) => {
                match self.sqrt_unit(unit.as_ref(), method, span) {
                    Some(root) => result.in_unit(root.as_ref()),
                    // U2028, reported: nothing more follows from the result.
                    None => Ty::error(),
                }
            }
            (FloatResult::F64 | FloatResult::Other, FloatOp::Fma) => {
                let product = fma_unit.unwrap_or_else(|| unit.clone());
                result.in_unit(product.as_ref())
            }
            (FloatResult::F64 | FloatResult::Other, _) => keep(result),
            _ => result,
        };
        Some(result.with_label(&label))
    }

    /// A method of `F64`'s receiver that is not one of its methods: the conversions to
    /// integers other than `to_int_exact()` are U2001 with their repair (ADR 0114 rule 4.3);
    /// any other name resolves as a function of the package (`spec/10 §5`).
    fn no_float_method(
        &mut self,
        name: &str,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
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
                "`{method}` is not a conversion of `{name}`; `to_int_exact()` gives `Some` of an integral value and `None` otherwise"
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
