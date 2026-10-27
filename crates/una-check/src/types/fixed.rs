//! Fixed-width integers of edition 0.2 (`spec/18`, ADR 0057): literals that take their type
//! from the context (U2022), integer types in clauses, and the `overflow` obligation sites.

use una_core::{FixedOp, IntType, Prim};
use una_diag::{Code, ObligationKind};
use una_project::{Edition, Profile};
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

/// The digits of `expr` when it is an integer literal, in parentheses or not: the operand of a
/// unary `-` that the Core IR negates as a literal (ADR 0109 rule 3).
pub(crate) fn plain_literal(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(digits)) => Some(digits.as_str()),
        ExprKind::Paren(inner) => plain_literal(inner),
        _ => None,
    }
}

/// Whether `expr` is a constant expression: integer literals combined by `+`, `-`, `*`, `/`,
/// `mod`, unary `-` and parentheses. It takes the unit its context expects, as one literal
/// does (ADR 0104 rule 3).
pub(crate) fn is_constant(expr: &Expr) -> bool {
    use una_syntax::ast::BinaryOp;
    match &expr.kind {
        ExprKind::Literal(Literal::Int(_)) => true,
        ExprKind::Paren(inner) => is_constant(inner),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => is_constant(operand),
        ExprKind::Binary { op, left, right } => {
            matches!(
                op,
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
            ) && is_constant(left)
                && is_constant(right)
        }
        _ => false,
    }
}

/// The least value of `Int` that stays in a machine word, `-2^62`: the tagged range that no
/// result of `Int` arithmetic leaves in a `systems` package (ADR 0109 rule 3, `spec/18 §3`).
pub(crate) const INT_LEAST: i128 = -(1 << 62);

/// The greatest value of `Int` that stays in a machine word, `2^62 - 1` (ADR 0109 rule 3).
pub(crate) const INT_GREATEST: i128 = (1 << 62) - 1;

/// Whether the integer `-digits` (when `negative`) or `digits` lies in the tagged range of
/// `Int`, [`INT_LEAST`] to [`INT_GREATEST`]. A literal too long for 128 bits lies outside it.
pub(crate) fn fits_word(negative: bool, digits: &str) -> bool {
    let Ok(magnitude) = digits.parse::<i128>() else {
        return false;
    };
    let value = if negative { -magnitude } else { magnitude };
    (INT_LEAST..=INT_GREATEST).contains(&value)
}

/// Whether `ty` is an integer type: `Int` or a fixed-width type.
pub(crate) fn is_integer(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Fixed(_))
}

impl Checker<'_, '_, '_> {
    /// Whether the expression being checked is in a clause (`where`, `requires`, `ensures`, or
    /// the predicate of `checked`), where every integer type reads as its mathematical `Int`:
    /// mixed types are allowed, literals are `Int`, and there is no `overflow` obligation and
    /// no U2022 (`spec/18 §2`, ADR 0057). The body of a `reflect` predicate is typed as
    /// executed code, mixed types included, but its arithmetic is exact
    /// ([`Self::exact_integers`]).
    pub(crate) fn in_clause(&self) -> bool {
        matches!(self.ctx.rl, Rl::Clause)
    }

    /// Whether integer arithmetic is exact where the expression being checked is: in a clause,
    /// and in the body of a `reflect` predicate, which is typed as code but computes exactly,
    /// as the verifier reads it, so it has no `overflow` obligation and no U2022
    /// (2026-10-06; `spec/18 §2`, `§3`, ADR 0057 amendment).
    pub(crate) fn exact_integers(&self) -> bool {
        matches!(self.ctx.rl, Rl::Clause | Rl::Reflect)
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
    /// fit where arithmetic is not exact, or `Int`.
    pub(crate) fn int_literal_type(
        &mut self,
        negative: bool,
        digits: &str,
        span: Span,
        expected: Option<&Ty>,
    ) -> Ty {
        // ADR 0104 rule 3: a literal takes the unit its context expects, and the magnitude
        // type, as it takes a fixed-width type.
        if let Some(Ty::Measured(unit, magnitude)) = expected.map(|e| self.vars.peel(e)) {
            self.int_literal_type(negative, digits, span, Some(&magnitude));
            // The expected type's magnitude, built by `Ty::measured`, holds no label.
            debug_assert!(matches!(*magnitude, Ty::Int | Ty::Fixed(_)));
            return Ty::Measured(unit, magnitude);
        }
        // ADR 0120 rule 12: an integer literal never takes a decimal type.
        if let Some(scale) = expected.and_then(|e| self.decimal_scale(e)) {
            self.int_literal_for_decimal(negative, digits, span, scale);
            return Ty::Decimal(scale);
        }
        // ADR 0114 rule 2.6: an integer literal never takes the type `F64`.
        if expected.is_some_and(|e| self.is_f64(e)) {
            self.int_literal_for_f64(negative, digits, span);
            return Ty::F64;
        }
        let Some(int) = self.expected_fixed(expected) else {
            // ADR 0109 rule 3: in a `systems` package, an `Int` literal outside the tagged
            // range is an `overflow` site, refuted at once; a negated literal is one literal.
            if self.int_word() && !fits_word(negative, digits) {
                self.obligation(ObligationKind::Overflow, span, None);
                self.findings.int_literals.insert(span);
            }
            return Ty::Int;
        };
        if !fits(int, negative, digits) && !self.exact_integers() {
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

    /// Records the `overflow` obligation of an operation of the fixed-width type `ty` at
    /// `span`, where integer arithmetic is not exact (`spec/18 §3`). An `Int` operation gets
    /// none here, even in a `systems` package: the element-wise operators of dense values call
    /// this one, and ADR 0120 rule 34 states their obligation for fixed-width elements only.
    pub(crate) fn overflow_site(&mut self, ty: &Ty, span: Span) {
        if self.exact_integers() {
            return;
        }
        if matches!(self.resolve(ty).magnitude(), Ty::Fixed(_)) {
            self.obligation(ObligationKind::Overflow, span, None);
        }
    }

    /// Whether `Int` arithmetic stays in a machine word where the expression being checked is:
    /// in a `systems` package, outside clauses and the bodies of `reflect` predicates, which
    /// compute exactly (ADR 0109 rule 3, `spec/18 §3`).
    pub(crate) fn int_word(&self) -> bool {
        self.env.profile == Profile::Systems && !self.exact_integers()
    }

    /// Records the `overflow` obligation of an operator or a conversion of type `ty` at `span`:
    /// on a fixed-width type as [`Self::overflow_site`] does, and on `Int`, or a measured `Int`,
    /// in a `systems` package (ADR 0109 rule 3). Called by the arithmetic operators, unary `-`
    /// and `convert()`, not by the element-wise operators of dense values.
    pub(crate) fn int_overflow_site(&mut self, ty: &Ty, span: Span) {
        if self.exact_integers() {
            return;
        }
        let int = matches!(self.resolve(ty).magnitude(), Ty::Int);
        if int && self.int_word() {
            self.obligation(ObligationKind::Overflow, span, None);
        } else {
            self.overflow_site(ty, span);
        }
    }

    /// Records the `overflow` obligation of a call of a prelude function that computes an
    /// `Int` by arithmetic, `sum` and `abs`, at the call, or at the reference when one is used
    /// as a function value, in a `systems` package (ADR 0109 rule 3). The entry points (`to_int` of a text, `parse_int`, JSON's `Number`,
    /// `Clock.now()`) and the functions bounded by a size, as `len`, record none.
    pub(crate) fn prelude_overflow_site(&mut self, name: &str, span: Span) {
        if matches!(name, "sum" | "abs") && self.int_word() {
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
        expected: Option<&Ty>,
    ) -> Option<Ty> {
        if self.env.edition < Edition::V0_2 {
            return None;
        }
        // ADR 0104 rule 4: in edition 0.3, these three are built in, ahead of a function of
        // the same name, when called with no argument.
        if self.env.edition >= Edition::V0_3
            && args.is_empty()
            && matches!(method, "measured" | "magnitude" | "convert")
        {
            return Some(self.unit_method(target, method, span, expected));
        }
        // A built-in integer method on a measured receiver (ADR 0104 rule 3). Any other name
        // resolves as on any value, so a function of the package called in method form keeps
        // working.
        if matches!(self.resolve(target).unlabeled(), Ty::Measured(..))
            && (convert_target(method).is_some() || FixedOp::from_name(method).is_some())
        {
            return Some(self.measured_int_method(target, method, args, span));
        }
        let receiver = self.resolve(target);
        let convert = convert_target(method);
        let op = FixedOp::from_name(method);
        // `x.to_int()` on a type parameter needs `Integer` (`spec/20 §1`, `§3` rule 2).
        if let (Ty::Param(_), Some(None)) = (receiver.unlabeled(), convert) {
            self.outside_rl(span, || format!("the method `{method}`"));
            self.no_args(method, args, span);
            self.needs
                .push((target.clone(), super::env::Constraint::Integer, span));
            self.resolved(span, Res::IntMethod(Prim::Convert(None)));
            return Some(Ty::Int.with_label(&self.deep(target)));
        }
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
                return Some(Ty::error());
            }
            _ => return None,
        };
        // #743: what the refinement language offers instead (`spec/03 §3`).
        let advice = if method == "to_int" {
            "state the bound on the value in its own type, such as `a <= 100`: the refinement language compares a fixed-width value with a literal directly (`spec/03 §3`); convert with `to_int()` in the body"
        } else {
            "the refinement language has literals, parameters and their fields, `.len`, `+`, `-`, `*` with a literal, `/` and `mod` by a nonzero literal, comparisons, `and`, `or`, `not`, `implies` and calls of predicates (`spec/03 §3`); compute the rest in the body"
        };
        self.outside_rl_with(
            span,
            || format!("the method `{method}`"),
            Some(super::Report::advice(advice)),
        );
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
            // ADR 0109 rule 3: `to_int()` on `I64` and `U64`, whose values can lie outside the
            // tagged range, carries an `overflow` obligation in a `systems` package.
            if target_int.is_none()
                && matches!(int, Ty::Fixed(IntType::I64 | IntType::U64))
                && self.int_word()
            {
                self.obligation(ObligationKind::Overflow, span, None);
            }
            self.resolved(span, Res::IntMethod(Prim::Convert(target_int)));
            return Some(result.with_label(&label));
        }
        let (Ty::Fixed(int), Some(op)) = (int, op) else {
            return Some(Ty::error());
        };
        if op.binary() {
            let operand = if op.shift() { Ty::Int } else { Ty::Fixed(int) };
            let param = op.label();
            match args {
                [arg] => {
                    // The argument is labeled, as every argument after the first (charter P5),
                    // or punned.
                    self.method_label(method, param, arg);
                    let what = format!("argument `{param}` of `{method}`");
                    let found = self.expr_as(&arg.value, Some(&operand), Role::Condition(&what));
                    label = label.join(&self.deep(&found));
                }
                _ => {
                    self.report.error(
                        Code::U2001,
                        span,
                        format!("`{method}` takes one argument, as in `a.{method}({param}: b)`"),
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

    /// `measured()`, `magnitude()` or `convert()` on `target` (ADR 0104 rule 4), the unit it
    /// produces taken from `expected`. Each unitless use, a missing expected unit and a
    /// conversion that is not exact or does not fit is an error, never a silent call.
    fn unit_method(&mut self, target: &Ty, method: &str, span: Span, expected: Option<&Ty>) -> Ty {
        use super::units::Unit;
        let label = self.deep(target);
        let receiver = self.resolve(target);
        // A receiver whose type already failed reports nothing more.
        if matches!(receiver.unlabeled(), Ty::Error(_) | Ty::Never) {
            self.resolved(span, Res::UnitMethod { ratio: 1 });
            return Ty::error();
        }
        let source_unit = receiver.unit().cloned();
        let magnitude = receiver.magnitude().clone();
        let expected = expected.map(|e| self.vars.peel(e));
        let expected_unit = expected.as_ref().and_then(|e| e.unit().cloned());
        let integer = matches!(magnitude, Ty::Int | Ty::Fixed(_) | Ty::Error(_));
        let measured = |unit: Unit, magnitude: Ty| {
            if unit.is_one() {
                magnitude
            } else {
                Ty::measured(unit, magnitude)
            }
        };
        let (result, ratio) = match method {
            "magnitude" => match &source_unit {
                Some(_) => (magnitude.clone(), 1),
                None => {
                    let message = if integer {
                        "`magnitude()` takes a measured value, and this one has no unit".to_owned()
                    } else {
                        format!(
                            "`magnitude()` takes a measured integer, not `{}`",
                            self.show(&receiver)
                        )
                    };
                    self.report.error(Code::U2028, span, message);
                    (Ty::error(), 1)
                }
            },
            "measured" if !integer => {
                self.report.error(
                    Code::U2028,
                    span,
                    format!(
                        "`measured()` takes an integer, not `{}`",
                        self.show(&receiver)
                    ),
                );
                (Ty::error(), 1)
            }
            "measured" => match (&source_unit, expected_unit) {
                (None, Some(unit)) if integer => (measured(unit, magnitude.clone()), 1),
                (Some(_), _) => {
                    self.report.error(
                        Code::U2030,
                        span,
                        "`measured()` takes a unitless value, and this one has a unit; use `convert()` to change units".to_owned(),
                    );
                    (Ty::error(), 1)
                }
                _ => {
                    self.report.error(
                        Code::U2030,
                        span,
                        "`measured()` takes its unit from the type expected here, and none is; annotate it, as `let d: Int of Meter = n.measured()`".to_owned(),
                    );
                    (Ty::error(), 1)
                }
            },
            _ => {
                // `convert()`: to the expected unit, or to no unit when the source is a scaled
                // dimensionless unit (ADR 0104 rule 2).
                let target_unit = match (&expected_unit, &expected) {
                    (Some(unit), _) => Some(unit.clone()),
                    (None, Some(ty)) if matches!(ty.magnitude(), Ty::Int | Ty::Fixed(_)) => {
                        Some(Unit::one())
                    }
                    _ => None,
                };
                match (&source_unit, target_unit) {
                    (None, _) => {
                        self.report.error(
                            Code::U2030,
                            span,
                            "`convert()` takes a measured value, and this one has no unit; use `measured()` to give it one".to_owned(),
                        );
                        (Ty::error(), 1)
                    }
                    (Some(_), None) => {
                        self.report.error(
                            Code::U2030,
                            span,
                            "`convert()` takes its unit from the type expected here, and none is; annotate it".to_owned(),
                        );
                        (Ty::error(), 1)
                    }
                    (Some(source), Some(target_unit)) => match source.ratio_to(&target_unit) {
                        Some(k) if fits_ratio(&magnitude, k) => {
                            (measured(target_unit, magnitude.clone()), k)
                        }
                        Some(k) => {
                            self.report.error(
                                Code::U2030,
                                span,
                                format!(
                                    "converting `{source}` to `{target_unit}` multiplies by {k}, which does not fit `{}`; widen the magnitude's integer type",
                                    self.show(&magnitude)
                                ),
                            );
                            (Ty::error(), 1)
                        }
                        None => {
                            let reason = if source.same_dimension(&target_unit) {
                                "is not by an integer ratio; write the division out with `magnitude()` and `measured()`, so its truncation shows"
                            } else {
                                "is between units of different dimensions"
                            };
                            self.report.error(
                                Code::U2030,
                                span,
                                format!("converting `{source}` to `{target_unit}` {reason}"),
                            );
                            (Ty::error(), 1)
                        }
                    },
                }
            }
        };
        // `convert()` multiplies by its ratio: on a fixed-width magnitude, and on an `Int` one
        // in a `systems` package (ADR 0109 rule 3).
        if ratio > 1 {
            self.int_overflow_site(&magnitude, span);
        }
        self.resolved(span, Res::UnitMethod { ratio });
        result.with_label(&label)
    }

    /// A built-in method of a fixed-width type on a measured receiver (ADR 0104 rule 3): the
    /// conversions keep the unit, wrapping and saturating `add` and `sub` take the receiver's
    /// unit and keep it, `mul` multiplies the units, shifts keep it with a unitless amount, and
    /// every other method needs a unitless receiver (U2028).
    fn measured_int_method(&mut self, target: &Ty, method: &str, args: &[Arg], span: Span) -> Ty {
        let receiver = self.resolve(target);
        let (Some(unit), magnitude) = (receiver.unit().cloned(), receiver.magnitude().clone())
        else {
            return Ty::error();
        };
        let label = self.deep(target);
        let keep = |ty: Ty| Ty::measured(unit.clone(), ty);
        if convert_target(method).is_some() {
            // Typed as on the magnitude, then the unit put back.
            let converted = self.int_method(&magnitude, method, args, span, None);
            return match converted {
                Some(ty) => keep(ty.magnitude().clone()).with_label(&label),
                None => Ty::error(),
            };
        }
        let Some(op) = FixedOp::from_name(method) else {
            self.report.error(
                Code::U2028,
                span,
                format!("`{method}` takes a unitless value; use `magnitude()` first"),
            );
            self.unchecked_args(args);
            return Ty::error();
        };
        // On an `Int` magnitude, as on `Int`, these are not methods (`spec/18 §5`), so that the
        // erased program stays well typed (ADR 0104 rule 5).
        if !matches!(magnitude, Ty::Fixed(_)) {
            self.report.error(
                Code::U2001,
                span,
                format!("`{method}` is a method of the fixed-width integer types, not of `Int`"),
            );
            self.unchecked_args(args);
            return Ty::error();
        }
        match op {
            FixedOp::WrappingAdd
            | FixedOp::WrappingSub
            | FixedOp::SaturatingAdd
            | FixedOp::SaturatingSub => {
                if let [arg] = args {
                    let what = format!("argument `{}` of `{method}`", op.label());
                    self.expr_as(&arg.value, Some(&receiver), Role::Condition(&what));
                }
                self.resolved(span, Res::IntMethod(Prim::Fixed(op, fixed_of(&magnitude))));
                keep(magnitude).with_label(&label)
            }
            FixedOp::WrappingMul | FixedOp::SaturatingMul => {
                let mut product = Some(unit.clone());
                if let [arg] = args {
                    let found = self.infer(&arg.value, Some(&magnitude));
                    let found = self.resolve(&found);
                    self.unify_at(
                        arg.value.span,
                        &magnitude,
                        &found.magnitude().clone(),
                        Role::Plain,
                    );
                    if let Some(other) = found.unit() {
                        product = unit.mul(other);
                    }
                }
                self.resolved(span, Res::IntMethod(Prim::Fixed(op, fixed_of(&magnitude))));
                match product {
                    Some(p) if !p.is_one() => Ty::measured(p, magnitude).with_label(&label),
                    _ => magnitude.with_label(&label),
                }
            }
            FixedOp::ShiftRight | FixedOp::WrappingShiftLeft => {
                if let [arg] = args {
                    self.expr_as(&arg.value, Some(&Ty::Int), Role::Plain);
                }
                self.obligation(ObligationKind::Precondition, span, None);
                self.resolved(span, Res::IntMethod(Prim::Fixed(op, fixed_of(&magnitude))));
                keep(magnitude).with_label(&label)
            }
            _ => {
                self.report.error(
                    Code::U2028,
                    span,
                    format!("`{method}` takes a unitless value; use `magnitude()` first"),
                );
                self.unchecked_args(args);
                Ty::error()
            }
        }
    }

    /// U2004 when the argument of a binary method has no label and is not the punned name
    /// `param`, U2005 when its label is another (`spec/04 §3`).
    pub(crate) fn method_label(&mut self, method: &str, param: &str, arg: &Arg) {
        let callee = format!("`{method}`");
        match &arg.label {
            Some(label) => {
                let written = self.name(label);
                if written != param {
                    let mut diagnostic = self.report.make(
                        Code::U2005,
                        label.span,
                        format!("{callee} has no parameter `{written}`; its argument is `{param}`"),
                    );
                    diagnostic.repairs.push(self.report.edit(
                        format!("write `{param}`"),
                        label.span,
                        param,
                    ));
                    self.report.push(diagnostic);
                }
            }
            None => {
                let punned =
                    matches!(&arg.value.kind, ExprKind::Name(ident) if self.name(ident) == param);
                if !punned {
                    let mut diagnostic = self.report.make(
                        Code::U2004,
                        arg.span,
                        format!(
                            "argument 2 of {callee} needs a label; only the first argument may go without one"
                        ),
                    );
                    let text = self.report.text(arg.span);
                    diagnostic.repairs.push(self.report.edit(
                        format!("label it `{param}`"),
                        arg.span,
                        format!("{param}: {text}"),
                    ));
                    self.report.push(diagnostic);
                }
            }
        }
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
    pub(crate) fn unchecked_args(&mut self, args: &[Arg]) {
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

/// Whether the ratio `k` of a `convert()` fits the magnitude's integer type, so that the
/// erased multiplication by the literal `k` is well typed (ADR 0104 rule 4).
fn fits_ratio(magnitude: &Ty, k: u128) -> bool {
    match magnitude {
        Ty::Fixed(int) => i128::try_from(k).is_ok_and(|k| k <= int.greatest()),
        _ => true,
    }
}

/// The fixed-width type of a magnitude, `I64` when it is none, for the primitive's operand.
fn fixed_of(magnitude: &Ty) -> IntType {
    match magnitude {
        Ty::Fixed(int) => *int,
        _ => IntType::I64,
    }
}
