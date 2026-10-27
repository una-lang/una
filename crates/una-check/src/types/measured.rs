//! Units of measure on the number types beyond the integers: `F64` from edition 0.3 (ADR 0114
//! rule 12), and `F32`, `Decimal[s]`, `Complex[F64]`, `Complex[F32]` and the elements of dense
//! values in edition 0.4 (ADR 0120 rules 61 to 65, `spec/25 §9`).
//!
//! Each number type's own rules type its operations on the magnitude; this module adds the
//! units on top: one unit for `+`, `-` and the comparisons, units multiplied and divided by
//! `*`, `/` and the products, and the unit each method keeps, squares, halves or refuses.
//! Units are erased before verification and code generation (`spec/25 §7`), so nothing here
//! reaches Core IR but the literal ratio of a `convert()`. Each rule is O(1) beyond its
//! operands, but for the normal form of a unit, which is linear in its base units.

use una_diag::Code;
use una_syntax::Span;
use una_syntax::ast::{BinaryOp, Expr, ExprKind, Literal, UnaryOp};

use super::check::Checker;
use super::expr::Combine;
use super::ty::Ty;
use super::units::Unit;

/// Whether `expr` is a constant expression whose unit is open: integer or float literals
/// combined by `+`, `-`, `*`, `/`, `mod`, unary `-` and parentheses. It takes the unit its
/// context expects (ADR 0104 rule 3, ADR 0114 rule 12.4).
pub(crate) fn is_unit_constant(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(_) | Literal::Float(_)) => true,
        ExprKind::Paren(inner) => is_unit_constant(inner),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => is_unit_constant(operand),
        ExprKind::Binary { op, left, right } => {
            matches!(
                op,
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
            ) && is_unit_constant(left)
                && is_unit_constant(right)
        }
        _ => false,
    }
}

/// Whether two magnitudes are of one number type, a decimal's scale aside: what an operator's
/// own rules accepted, so that its units are compared and not a mistake already reported.
fn one_number_type(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::F64, Ty::F64) | (Ty::F32, Ty::F32) | (Ty::Decimal(_), Ty::Decimal(_)) => true,
        (Ty::Kind(x, xs), Ty::Kind(y, ys)) => x == y && xs.is_empty() && ys.is_empty(),
        _ => false,
    }
}

/// The unit written for a diagnostic, qualified when `alike` (two modules' `Meter`).
fn shown(unit: &Option<Unit>, alike: bool) -> String {
    match unit {
        Some(u) if alike => format!("`{}`", u.qualified()),
        Some(u) => format!("`{u}`"),
        None => "no unit".to_owned(),
    }
}

impl Checker<'_, '_, '_> {
    /// The unit and the magnitude of `expected` when it is a measured type, through its
    /// solved variables and labels.
    pub(crate) fn expected_unit(&self, expected: Option<&Ty>) -> Option<(Unit, Ty)> {
        match self.vars.peel(expected?) {
            Ty::Measured(unit, magnitude) => Some(((*unit).clone(), (*magnitude).clone())),
            _ => None,
        }
    }

    /// The magnitude of `ty`, resolved: its number type without its label and its unit.
    pub(crate) fn number_of(&self, ty: &Ty) -> Ty {
        self.resolve(ty).magnitude().clone()
    }

    /// The unit of `ty`, resolved.
    pub(crate) fn unit_of(&self, ty: &Ty) -> Option<Unit> {
        self.resolve(ty).unit().cloned()
    }

    /// The unit of the result of an arithmetic operator from its operands' units, in source
    /// order (ADR 0104 rule 3): `None` for an operand is a constant, whose unit is open, and
    /// `Some(None)` a unitless operand. `+`, `-` and `mod` need one unit (U2028 otherwise);
    /// `*` and `/` multiply and divide, a constant counting as unitless, and a constant
    /// expression as a whole takes `expected_unit`.
    pub(crate) fn combine_units(
        &mut self,
        combine: Combine,
        units: (Option<Option<Unit>>, Option<Option<Unit>>),
        both_constant: bool,
        expected_unit: Option<Unit>,
        operand: &str,
        span: Span,
    ) -> Option<Unit> {
        match combine {
            Combine::Same => match units {
                (Some(a), Some(b)) => {
                    if a != b {
                        // Two units that print alike, as two modules' `Meter`, print qualified.
                        let alike = a.as_ref().map(ToString::to_string)
                            == b.as_ref().map(ToString::to_string);
                        self.report.error(
                            Code::U2028,
                            span,
                            format!(
                                "{operand} needs one unit on both sides, and these have {} and {}",
                                shown(&a, alike),
                                shown(&b, alike)
                            ),
                        );
                    }
                    a
                }
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => expected_unit,
            },
            Combine::Mul | Combine::Div => {
                let (a, b) = (units.0.flatten(), units.1.flatten());
                let one = Unit::one();
                let (a, b) = (a.unwrap_or_else(|| one.clone()), b.unwrap_or(one));
                let product = if matches!(combine, Combine::Mul) {
                    a.mul(&b)
                } else {
                    a.div(&b)
                };
                if product.is_none() {
                    self.report.error(
                        Code::U2029,
                        span,
                        "the unit of this product or quotient has a scale too large".to_owned(),
                    );
                }
                if both_constant {
                    // A constant expression as a whole takes the expected unit.
                    expected_unit
                } else {
                    product
                }
            }
        }
    }

    /// `ty`, the type an operator's own rules gave `first op second` on their magnitudes, with
    /// the unit of the result (ADR 0114 rule 12.2, ADR 0120 rules 62 to 64). `mark` is where
    /// the operands' types start in `self.operands`, which those rules pushed, the first
    /// typed operand first; `swapped` says that it is the right one as written. A dense result
    /// carries its units in its element type already. Operands that are not of one number
    /// type are a mistake already reported: the result keeps the first one's unit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn operator_units(
        &mut self,
        ty: Ty,
        combine: Combine,
        (first, second): (&Expr, &Expr),
        mark: usize,
        swapped: bool,
        expected: Option<&Ty>,
        operand: &str,
        span: Span,
    ) -> Ty {
        let (Some(first_ty), Some(second_ty)) = (
            self.operands.get(mark).cloned(),
            self.operands.get(mark + 1).cloned(),
        ) else {
            return ty;
        };
        let (first_top, second_top) = (self.vars.peel(&first_ty), self.vars.peel(&second_ty));
        let expected_unit = self.expected_unit(expected).map(|(unit, _)| unit);
        // A program without units meets none of this.
        if !first_top.has_units() && !second_top.has_units() && expected_unit.is_none() {
            return ty;
        }
        if matches!(
            self.vars.peel(&ty),
            Ty::Dense(..) | Ty::Error(_) | Ty::Never
        ) {
            return ty;
        }
        if !one_number_type(first_top.magnitude(), second_top.magnitude()) {
            return ty.in_unit(first_top.unit());
        }
        let unit_of = |expr: &Expr, top: &Ty| -> Option<Option<Unit>> {
            if is_unit_constant(expr) {
                None
            } else {
                Some(top.unit().cloned())
            }
        };
        // In source order, `left` then `right`: `1.0 / t` is `1 / Second`.
        let units = if swapped {
            (unit_of(second, &second_top), unit_of(first, &first_top))
        } else {
            (unit_of(first, &first_top), unit_of(second, &second_top))
        };
        let both_constant = is_unit_constant(first) && is_unit_constant(second);
        let unit = self.combine_units(combine, units, both_constant, expected_unit, operand, span);
        ty.in_unit(unit.as_ref())
    }

    /// The product of two units, either of which may be none; U2029 at `span` when its scale
    /// is too large, and then no unit, after the diagnostic.
    pub(crate) fn product_unit(
        &mut self,
        a: Option<&Unit>,
        b: Option<&Unit>,
        span: Span,
    ) -> Option<Unit> {
        let one = Unit::one();
        let product = a.unwrap_or(&one).mul(b.unwrap_or(&one));
        if product.is_none() {
            self.report.error(
                Code::U2029,
                span,
                "the unit of this product has a scale too large".to_owned(),
            );
        }
        product.filter(|unit| !unit.is_one())
    }

    /// The quotient of two units, either of which may be none; U2029 at `span` when its
    /// scale is too large.
    pub(crate) fn quotient_unit(
        &mut self,
        a: Option<&Unit>,
        b: Option<&Unit>,
        span: Span,
    ) -> Option<Unit> {
        let one = Unit::one();
        let quotient = a.unwrap_or(&one).div(b.unwrap_or(&one));
        if quotient.is_none() {
            self.report.error(
                Code::U2029,
                span,
                "the unit of this quotient has a scale too large".to_owned(),
            );
        }
        quotient.filter(|unit| !unit.is_one())
    }

    /// The unit of the square root of a value in `unit`, by `method` at `span` (ADR 0114 rule
    /// 12.3, ADR 0120 rule 64): half of each exponent, `Some(None)` for no unit. `None` after
    /// U2028 when an exponent is odd or the scale is not a square, with the repair that writes
    /// the conversion out.
    pub(crate) fn sqrt_unit(
        &mut self,
        unit: Option<&Unit>,
        method: &str,
        span: Span,
    ) -> Option<Option<Unit>> {
        let Some(unit) = unit else {
            return Some(None);
        };
        if let Some(root) = unit.sqrt() {
            return Some((!root.is_one()).then_some(root));
        }
        let mut diagnostic = self.report.make(
            Code::U2028,
            span,
            format!(
                "`{method}` halves each exponent of the unit, and `{unit}` has an odd exponent or a scale that is not a square"
            ),
        );
        diagnostic.repairs.push(super::Report::advice(format!(
            "write the conversion out, as in `x.magnitude().{method}().measured()`, with the unit of the result in its type"
        )));
        self.report.push(diagnostic);
        None
    }

    /// U2028 at `span` when the two operands of a comparison of decimals, which the rules of
    /// decimals compare across scales, have different units (ADR 0120 rule 63).
    pub(crate) fn same_unit(&mut self, span: Span, left: &Ty, right: &Ty) {
        let (a, b) = (self.unit_of(left), self.unit_of(right));
        if a != b {
            let alike = a.as_ref().map(ToString::to_string) == b.as_ref().map(ToString::to_string);
            self.report.error(
                Code::U2028,
                span,
                format!(
                    "a comparison needs one unit on both sides, and these have {} and {}",
                    shown(&a, alike),
                    shown(&b, alike)
                ),
            );
        }
    }
}
