//! `Complex[F64]` and `Complex[F32]` of edition 0.5 (`spec/29 §19`, ADR 0120 rules 22 to 25):
//! the prelude's record kinds `Complex_F64` and `Complex_F32`, which a program writes
//! `Complex[F64]` and `Complex[F32]` and builds with `complex(re: x, im: y)`, whose arguments'
//! type picks the kind. The operators `+`, `-`, `*`, `/` and unary `-`, and the methods
//! `conj()`, `scale(by:)` and `norm_sqr()`, elaborate to the operations of the components, in
//! the order of rule 23; `==` and `!=` are the record's structural equality, which compares the
//! components by identity.
//!
//! In the refinement language a complex value has its fields `re` and `im`, `complex(re:, im:)`
//! and `==` (ADR 0120 V3); its arithmetic and its methods are code only (U2014). Checking each
//! construct is O(1) beyond its operands.

use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, BinaryOp, Expr};

use super::check::{Checker, Role};
use super::label::Label;
use super::records::Res;
use super::ty::{KindId, Ty};
use super::units::Unit;

/// The component type of a complex type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Width {
    /// `Complex[F64]`.
    F64,
    /// `Complex[F32]`.
    F32,
}

impl Width {
    /// The component type.
    pub(crate) fn component(self) -> Ty {
        match self {
            Width::F64 => Ty::F64,
            Width::F32 => Ty::F32,
        }
    }

    /// The prelude's record kind of this type.
    fn kind_name(self) -> &'static str {
        match self {
            Width::F64 => una_core::complex::COMPLEX_F64,
            Width::F32 => una_core::complex::COMPLEX_F32,
        }
    }

    /// The type as a program writes it.
    fn written(self) -> &'static str {
        match self {
            Width::F64 => "Complex[F64]",
            Width::F32 => "Complex[F32]",
        }
    }

    /// The width whose component type is `ty`, unlabeled.
    fn of(ty: &Ty) -> Option<Width> {
        match ty {
            Ty::F64 => Some(Width::F64),
            Ty::F32 => Some(Width::F32),
            _ => None,
        }
    }

    /// The conversion of a float into this width's component type.
    fn conversion(self) -> &'static str {
        match self {
            Width::F64 => "to_f64",
            Width::F32 => "to_f32",
        }
    }
}

/// A method of a complex value, code only (ADR 0120 rule 23).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComplexMethod {
    /// `x.conj()`: `complex(re: a, im: -b)`.
    Conj,
    /// `x.scale(by: k)`: `complex(re: a * k, im: b * k)`, `k` of the component type.
    Scale,
    /// `x.norm_sqr()`: `a * a + b * b`.
    NormSqr,
}

/// The parameters of `complex`, in order.
const PARAMS: [&str; 2] = ["re", "im"];

/// The parameters of `conj()` and `norm_sqr()`.
const NO_PARAMS: &[&str] = &[];

/// The parameters of `scale(by:)`.
const SCALE_PARAMS: &[&str] = &["by"];

impl Checker<'_, '_, '_> {
    /// The width of the kind `id` when it is a complex kind of the prelude.
    pub(crate) fn complex_kind(&self, id: KindId) -> Option<Width> {
        let kind = self.env.kinds.get(id)?;
        if kind.module != self.env.prelude {
            return None;
        }
        match kind.name {
            una_core::complex::COMPLEX_F64 => Some(Width::F64),
            una_core::complex::COMPLEX_F32 => Some(Width::F32),
            _ => None,
        }
    }

    /// The width of `ty`, resolved, when it is a complex type under its label.
    pub(crate) fn complex_width(&self, ty: &Ty) -> Option<Width> {
        match self.resolve(ty).unlabeled() {
            Ty::Kind(id, _) => self.complex_kind(*id),
            _ => None,
        }
    }

    /// Whether `ty`, resolved, is a complex type.
    pub(crate) fn is_complex(&self, ty: &Ty) -> bool {
        self.complex_width(ty).is_some()
    }

    /// The type `Complex[F64]` or `Complex[F32]`.
    pub(crate) fn complex_ty(&self, width: Width) -> Ty {
        match self.env.prelude_kind(width.kind_name()) {
            Some(id) => Ty::Kind(id, Vec::new().into()),
            None => Ty::error(),
        }
    }

    /// `complex(re: x, im: y)`, or `x.complex(im: y)` with the receiver, already typed, binding
    /// `re`, when `id` is the prelude's `complex` in edition 0.5 (ADR 0120 rule 22): both
    /// arguments are `F64`, or both `F32` (U2001 otherwise), and the result is `Complex[F64]` or
    /// `Complex[F32]`, labeled with both arguments' labels (IF1). A complex `expected` type gives
    /// its component type to the arguments, so float literals take it. The call is a
    /// construction, in the refinement language (V3). `None` for any other function, a
    /// package's own `complex` included, so the call resolves as declared.
    pub(crate) fn complex_call(
        &mut self,
        id: usize,
        receiver: Option<(&Expr, &Ty)>,
        args: &[Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Option<Ty> {
        if self.edition() < Edition::V0_5 {
            return None;
        }
        let def = self.env.functions.get(id)?;
        if def.module != self.env.prelude || def.name != una_core::complex::CONSTRUCTOR {
            return None;
        }
        let callee = "`complex`";
        let offset = usize::from(receiver.is_some());
        // The parameter of each argument, as for a function (`spec/04 §3`, ADR 0049): the
        // receiver or the first argument binds `re`; the others are labeled or punned.
        let mut given = [receiver.is_some(), false];
        for (index, arg) in args.iter().enumerate() {
            let target = match &arg.label {
                Some(label) => PARAMS.iter().position(|name| *name == self.name(label)),
                None if index + offset == 0 => Some(0),
                None => self.punned(arg, &PARAMS),
            };
            if let Some(slot) = target.and_then(|target| given.get_mut(target)) {
                *slot = true;
            }
        }
        let open: Vec<&str> = PARAMS
            .iter()
            .zip(given)
            .filter(|(_, given)| !given)
            .map(|(name, _)| *name)
            .collect();
        let mut taken = [receiver.is_some(), false];
        let mut problem = false;
        let mut values: Vec<(&Expr, Option<usize>, Option<Ty>)> = receiver
            .map(|(expr, ty)| (expr, Some(0), Some(ty.clone())))
            .into_iter()
            .collect();
        for (index, arg) in args.iter().enumerate() {
            let position = index + offset;
            let target = if let Some(label) = &arg.label {
                let written = self.name(label);
                let found = PARAMS.iter().position(|name| *name == written);
                if found.is_none() {
                    self.unknown_label(callee, &open, label);
                }
                found
            } else if position == 0 {
                Some(0)
            } else if let Some(target) = self.punned(arg, &PARAMS) {
                Some(target)
            } else {
                self.missing_label(callee, &PARAMS, arg, position);
                None
            };
            let target = match target {
                Some(target) if taken.get(target) == Some(&true) => {
                    let name = PARAMS.get(target).copied().unwrap_or("?");
                    self.report.error(
                        Code::U2006,
                        arg.span,
                        format!("parameter `{name}` of {callee} is given twice"),
                    );
                    None
                }
                other => other,
            };
            if let Some(slot) = target.and_then(|target| taken.get_mut(target)) {
                *slot = true;
            }
            problem |= target.is_none();
            values.push((&arg.value, target, None));
        }
        self.ordered(span, values.iter().map(|(_, target, _)| *target).collect());
        // `re` first, so that `im` takes its type; arguments of no parameter last.
        let mut sorted: Vec<&(&Expr, Option<usize>, Option<Ty>)> = values.iter().collect();
        sorted.sort_by_key(|(_, target, _)| target.unwrap_or(usize::MAX));
        let mut width = expected.and_then(|e| self.complex_width(&self.number_of(e)));
        // A measured complex type expected gives its unit to the components (ADR 0120 rule
        // 64), which share one unit, the result's.
        let expected_unit = expected.and_then(|e| self.unit_of(e));
        let mut units: Vec<(Span, Option<Unit>)> = Vec::new();
        let mut label = Label::public();
        for (value, target, typed) in sorted {
            let Some(target) = *target else {
                self.expr(value, None);
                continue;
            };
            let param = PARAMS.get(target).copied().unwrap_or("?");
            let mark = self.report.count();
            let found = match typed {
                Some(typed) => typed.clone(),
                None => {
                    let hint = width.map(|w| w.component().in_unit(expected_unit.as_ref()));
                    self.infer(value, hint.as_ref())
                }
            };
            label = label.join(&self.deep(&found));
            let resolved = self.resolve(&found);
            units.push((value.span, resolved.unit().cloned()));
            match (width, Width::of(resolved.magnitude())) {
                (None, Some(own)) => width = Some(own),
                (Some(w), Some(own)) if w != own => {
                    let component = w.component();
                    self.unify_at(
                        value.span,
                        &component,
                        &found,
                        Role::Argument { param, callee },
                    );
                    // ADR 0120 rule 7: the repair converts this argument to the other's type.
                    let method = w.conversion();
                    let text = self.report.text(value.span).to_owned();
                    let repair = self.report.edit(
                        format!("convert it with `{method}()`"),
                        value.span,
                        format!("({text}).{method}()"),
                    );
                    self.report
                        .repair_since(mark, Code::U2001, value.span, repair);
                }
                (_, Some(_)) => {}
                (_, None) => {
                    if !matches!(resolved.unlabeled(), Ty::Error(_) | Ty::Never) {
                        let shown = self.show(&found);
                        self.report.error(
                            Code::U2001,
                            value.span,
                            format!(
                                "argument `{param}` of {callee} must be `F64` or `F32`, found `{shown}`"
                            ),
                        );
                    }
                }
            }
        }
        if !problem {
            let missing: Vec<String> = PARAMS
                .iter()
                .zip(taken)
                .filter(|(_, taken)| !taken)
                .map(|(name, _)| format!("`{name}`"))
                .collect();
            if !missing.is_empty() {
                let what = if missing.len() == 1 {
                    "argument"
                } else {
                    "arguments"
                };
                self.report.error(
                    Code::U2001,
                    span,
                    format!("{callee} is missing {what} {}", missing.join(", ")),
                );
            }
        }
        let width = width.unwrap_or(Width::F64);
        let kind = self.env.prelude_kind(width.kind_name())?;
        self.resolved(
            span,
            Res::Ctor {
                kind,
                variant: None,
            },
        );
        // One unit for both components (ADR 0120 rule 64): the result's.
        let unit = match units.as_slice() {
            [(_, a), (at, b), ..] if a != b => {
                let shown = |u: &Option<Unit>| {
                    u.as_ref()
                        .map_or_else(|| "no unit".to_owned(), |u| format!("`{u}`"))
                };
                self.report.error(
                    Code::U2028,
                    *at,
                    format!(
                        "both components of {callee} need one unit, and these have {} and {}",
                        shown(a),
                        shown(b)
                    ),
                );
                a.clone()
            }
            [(_, a), ..] => a.clone(),
            [] => None,
        };
        Some(
            Ty::Kind(kind, Vec::new().into())
                .in_unit(unit.as_ref())
                .with_label(&label),
        )
    }

    /// `x.conj()`, `x.scale(by: k)` and `x.norm_sqr()` on a complex `x` in edition 0.5 (ADR
    /// 0120 rule 23), code only: the result joins the labels of the receiver and of `k`. `None`
    /// when `target` is not complex or `method` is none of them, so the call resolves as a
    /// function of the package.
    pub(crate) fn complex_method(
        &mut self,
        target: &Ty,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        if self.edition() < Edition::V0_5 {
            return None;
        }
        // A measured receiver (ADR 0120 rule 64): `conj` keeps its unit, `scale(by:)`
        // multiplies it by the scalar's, and `norm_sqr()` squares it.
        let width = self.complex_width(&self.number_of(target))?;
        let unit = self.unit_of(target);
        let mut by_unit = None;
        let (op, params) = match method {
            "conj" => (ComplexMethod::Conj, NO_PARAMS),
            "scale" => (ComplexMethod::Scale, SCALE_PARAMS),
            "norm_sqr" => (ComplexMethod::NormSqr, NO_PARAMS),
            _ => return None,
        };
        self.outside_rl(span, || {
            format!("the method `{method}` of `{}`", width.written())
        });
        let mut label = self.deep(target);
        if args.len() == params.len() {
            for (arg, param) in args.iter().zip(params) {
                self.method_label(method, param, arg);
                let what = format!("argument `{param}` of `{method}`");
                let component = width.component();
                // The scalar may have any unit.
                let found = self.infer(&arg.value, Some(&component));
                if self.number_of(&found) != component {
                    self.unify_at(arg.value.span, &component, &found, Role::Condition(&what));
                }
                by_unit = self.unit_of(&found);
                label = label.join(&self.deep(&found));
            }
        } else {
            let shape = params
                .iter()
                .map(|param| format!("{param}: k"))
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
        self.resolved(span, Res::Complex(op));
        let result = match op {
            ComplexMethod::NormSqr => {
                let square = self.product_unit(unit.as_ref(), unit.as_ref(), span);
                width.component().in_unit(square.as_ref())
            }
            ComplexMethod::Conj => self.complex_ty(width).in_unit(unit.as_ref()),
            ComplexMethod::Scale => {
                let product = self.product_unit(unit.as_ref(), by_unit.as_ref(), span);
                self.complex_ty(width).in_unit(product.as_ref())
            }
        };
        Some(result.with_label(&label))
    }

    /// The second operand and the type of an arithmetic operator whose first operand is a
    /// complex value of `width` (ADR 0120 rule 23): both are of one complex type, and the result
    /// is that type, code only. A real operand is U2001, with a repair to `scale(by:)` for a
    /// product and to `complex(re: x, im: 0.0)` otherwise; `mod` is U2001.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complex_arithmetic(
        &mut self,
        width: Width,
        first: &Expr,
        first_ty: Ty,
        second: &Expr,
        operand: &str,
        span: Span,
        op: BinaryOp,
    ) -> Ty {
        let complex = self.complex_ty(width);
        self.outside_rl(span, || format!("arithmetic on `{}`", width.written()));
        let mark = self.report.count();
        let second_ty = self.infer(second, Some(&complex));
        // The units of the operands are compared by the caller (ADR 0120 rule 64).
        let second_number = self.number_of(&second_ty);
        if self.complex_width(&second_number) != Some(width) {
            self.unify_at(second.span, &complex, &second_ty, Role::Condition(operand));
            if matches!(second_number, Ty::F64 | Ty::F32) {
                self.real_operand_repair(mark, second.span, op, first, second, span);
            }
        }
        self.operands.push(first_ty);
        self.operands.push(second_ty);
        if op == BinaryOp::Mod {
            self.report.error(
                Code::U2001,
                span,
                format!("`mod` is not defined on `{}`", width.written()),
            );
        }
        complex
    }

    /// The type of an arithmetic operator whose first operand `real` is a real value and whose
    /// second operand `complex`, already typed, is a complex value of `width` (ADR 0120 rule
    /// 23): U2001 at the real operand, with the repairs of `real_operand_repair`, as when the
    /// complex operand comes first. The result is the complex type, so that the mistake is
    /// reported once and not again against an expected complex type; `mod` is U2001.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn real_complex_arithmetic(
        &mut self,
        width: Width,
        real: &Expr,
        real_ty: Ty,
        complex: &Expr,
        complex_ty: Ty,
        operand: &str,
        span: Span,
        op: BinaryOp,
    ) -> Ty {
        let ty = self.complex_ty(width);
        let mark = self.report.count();
        self.unify_at(real.span, &ty, &real_ty, Role::Condition(operand));
        self.real_operand_repair(mark, real.span, op, complex, real, span);
        self.operands.push(real_ty);
        self.operands.push(complex_ty);
        if op == BinaryOp::Mod {
            self.report.error(
                Code::U2001,
                span,
                format!("`mod` is not defined on `{}`", width.written()),
            );
        }
        ty
    }

    /// The repair of the U2001 reported at `at` since `mark` for the operator `op`, at `span`,
    /// between the complex operand `complex` and the real operand `real` (ADR 0120 rule 23):
    /// `complex.scale(by: real)` for `*`, `complex(re: real, im: 0.0)` otherwise.
    pub(crate) fn real_operand_repair(
        &mut self,
        mark: usize,
        at: Span,
        op: BinaryOp,
        complex: &Expr,
        real: &Expr,
        span: Span,
    ) {
        let real_text = self.report.text(real.span).to_owned();
        let repair = if op == BinaryOp::Mul {
            let complex_text = self.report.text(complex.span).to_owned();
            self.report.edit(
                "scale the complex value by the real one with `scale(by:)`",
                span,
                format!("({complex_text}).scale(by: {real_text})"),
            )
        } else {
            self.report.edit(
                "make the real operand a complex value with a zero imaginary part",
                real.span,
                format!("complex(re: {real_text}, im: 0.0)"),
            )
        };
        self.report.repair_since(mark, Code::U2001, at, repair);
    }

    /// U2001 when the prelude's `complex` is used as a value in edition 0.5: its result's type
    /// is chosen by its arguments' type at each call (ADR 0120 rule 22), which a function value
    /// cannot carry.
    pub(crate) fn complex_function_value(&mut self, id: usize, span: Span) {
        if self.edition() < Edition::V0_5 {
            return;
        }
        let Some(def) = self.env.functions.get(id) else {
            return;
        };
        if def.module != self.env.prelude || def.name != una_core::complex::CONSTRUCTOR {
            return;
        }
        let mut diagnostic = self.report.make(
            Code::U2001,
            span,
            "`complex` builds a `Complex[F64]` or a `Complex[F32]` by its arguments' type at each call, so it is not a function value",
        );
        diagnostic.repairs.push(super::Report::advice(
            "write a lambda that calls it, such as `fn(x: F64, y: F64) -> complex(re: x, im: y)`",
        ));
        self.report.push(diagnostic);
    }
}
