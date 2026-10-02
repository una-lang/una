//! Fixed-width integers of edition 0.2 (`spec/18`, ADR 0057): literals that take their type
//! from the context (U2022), integer types in clauses, and the `overflow` obligation sites.

use una_core::{FixedOp, IntType, Prim};
use una_diag::{Code, ObligationKind};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Expr, ExprKind, Literal, Range, UnaryOp};

use super::check::{Checker, Rl, Role};
use super::records::Res;
use super::ty::Ty;

/// Whether the integer `-digits` (when `negative`) or `digits` lies in `int`'s range. A literal
/// too long for 128 bits lies outside every range.
pub(crate) fn fits(int: IntType, negative: bool, digits: &str) -> bool {
    let Ok(magnitude) = digits.parse::<i128>() else {
        return false;
    };
    let value = if negative { -magnitude } else { magnitude };
    int.least() <= value && value <= int.greatest()
}

/// The digits of `expr` and whether it is negated, when it is an integer literal, a negated
/// one, or one in parentheses.
pub(crate) fn literal_value(expr: &Expr) -> Option<(bool, &str)> {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(digits)) => Some((false, digits.as_str())),
        ExprKind::Paren(inner) => literal_value(inner),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => literal_value(operand).map(|(negative, digits)| (!negative, digits)),
        _ => None,
    }
}

/// Whether `ty` is an integer type: `Int` or a fixed-width type.
pub(crate) fn is_integer(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Fixed(_))
}

impl Checker<'_, '_, '_> {
    /// Whether the expression being checked is in a clause (`where`, `requires`, `ensures`, or
    /// the predicate of `checked`), where every integer type reads as its mathematical `Int`:
    /// mixed types are allowed, literals are `Int`, and there is no `overflow` obligation and
    /// no U2022 (`spec/18 §2`, ADR 0057). The body of a `reflect` predicate runs, so it follows
    /// the rules of executed code.
    pub(crate) fn in_clause(&self) -> bool {
        matches!(self.ctx.rl, Rl::Clause)
    }

    /// The fixed-width type that `expected` resolves to now, outside clauses.
    pub(crate) fn expected_fixed(&self, expected: Option<&Ty>) -> Option<IntType> {
        if self.in_clause() {
            return None;
        }
        match self.resolve(expected?) {
            Ty::Fixed(int) => Some(int),
            _ => None,
        }
    }

    /// The type of the integer literal `-digits` or `digits` at `span` where `expected` is
    /// expected: the fixed-width type it resolves to, after U2022 when the literal does not
    /// fit, or `Int`.
    pub(crate) fn int_literal_type(
        &mut self,
        negative: bool,
        digits: &str,
        span: Span,
        expected: Option<&Ty>,
    ) -> Ty {
        let Some(int) = self.expected_fixed(expected) else {
            return Ty::Int;
        };
        if !fits(int, negative, digits) {
            let sign = if negative { "-" } else { "" };
            self.report.error(
                Code::U2022,
                span,
                format!(
                    "the literal `{sign}{digits}` does not fit `{}`, which holds `{}` to `{}`",
                    int.as_str(),
                    int.least(),
                    int.greatest()
                ),
            );
        }
        Ty::Fixed(int)
    }

    /// U2022 at each bound of `range` that does not fit `int` (`spec/18 §2`).
    pub(crate) fn range_bounds(&mut self, range: &Range, int: IntType) {
        for bound in [&range.low, &range.high] {
            self.int_literal_type(
                bound.negative,
                &bound.digits,
                bound.span,
                Some(&Ty::Fixed(int)),
            );
        }
    }

    /// Records the `overflow` obligation of an operation of type `ty` at `span`, outside
    /// clauses (`spec/18 §3`).
    pub(crate) fn overflow_site(&mut self, ty: &Ty, span: Span) {
        if self.in_clause() {
            return;
        }
        if matches!(self.resolve(ty), Ty::Fixed(_)) {
            self.obligation(ObligationKind::Overflow, span, None);
        }
    }
}

impl Checker<'_, '_, '_> {
    /// `e.m(args)` when `m` is a built-in method of `e`'s integer type in edition 0.2: a
    /// conversion (`spec/18 §4`), or a wrapping, saturating or bit method of a fixed-width
    /// type (`§5`). `None` when it is not one, so the call resolves as before.
    pub(crate) fn int_method(
        &mut self,
        target: &Ty,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        if self.env.edition < Edition::V0_2 {
            return None;
        }
        let receiver = self.resolve(target);
        let convert = convert_target(method);
        let op = FixedOp::from_name(method);
        let int = match (&receiver, convert, op) {
            (Ty::Int | Ty::Fixed(_), Some(_), _) | (Ty::Fixed(_), _, Some(_)) => receiver.clone(),
            (Ty::Int, None, Some(_)) => {
                self.report.error(
                    Code::U2001,
                    span,
                    format!(
                        "`{method}` is a method of the fixed-width integer types, not of `Int`"
                    ),
                );
                self.unchecked_args(args);
                return Some(Ty::Error);
            }
            _ => return None,
        };
        self.outside_rl(span, || format!("the method `{method}`"));
        let mut label = self.deep(target);
        if let Some(target_int) = convert {
            self.no_args(method, args, span);
            let result = target_int.map_or(Ty::Int, Ty::Fixed);
            // A narrowing conversion carries an `overflow` obligation (`spec/18 §4` rule 4).
            let narrowing = match (&int, target_int) {
                (_, None) => false,
                (Ty::Fixed(source), Some(target_int)) => !source.widens_to(target_int),
                _ => true,
            };
            if narrowing {
                self.overflow_site(&result, span);
            }
            self.resolved(span, Res::IntMethod(Prim::Convert(target_int)));
            return Some(result.with_label(&label));
        }
        let (Ty::Fixed(int), Some(op)) = (int, op) else {
            return Some(Ty::Error);
        };
        if op.binary() {
            let operand = if op.shift() { Ty::Int } else { Ty::Fixed(int) };
            match args {
                [arg] if arg.label.is_none() => {
                    let what = format!("the argument of `{method}`");
                    let found = self.expr_as(&arg.value, Some(&operand), Role::Condition(&what));
                    label = label.join(&self.deep(&found));
                }
                _ => {
                    self.report.error(
                        Code::U2001,
                        span,
                        format!(
                            "`{method}` takes one argument, without a label, as in `a.{method}(b)`"
                        ),
                    );
                    self.unchecked_args(args);
                }
            }
            if op.shift() {
                // `requires 0 <= n and n < width` (`spec/18 §5`).
                self.obligation(ObligationKind::Precondition, span, None);
            }
        } else {
            self.no_args(method, args, span);
        }
        self.resolved(span, Res::IntMethod(Prim::Fixed(op, int)));
        Some(Ty::Fixed(int).with_label(&label))
    }

    /// U2001 when a method that takes no argument has some.
    fn no_args(&mut self, method: &str, args: &[Arg], span: Span) {
        if !args.is_empty() {
            self.report.error(
                Code::U2001,
                span,
                format!("`{method}` takes no argument, as in `x.{method}()`"),
            );
            self.unchecked_args(args);
        }
    }

    /// Checks arguments that belong to no parameter, for the diagnostics inside them.
    fn unchecked_args(&mut self, args: &[Arg]) {
        for arg in args {
            self.expr(&arg.value, None);
        }
    }
}

/// The target of the conversion `method`: `Some(None)` for `to_int`, `Some(Some(t))` for
/// `to_<t>`, `None` when it is no conversion.
fn convert_target(method: &str) -> Option<Option<IntType>> {
    if method == "to_int" {
        return Some(None);
    }
    let name = method.strip_prefix("to_")?.to_ascii_uppercase();
    IntType::from_name(&name).map(Some)
}
