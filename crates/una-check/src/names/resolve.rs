//! Resolving every name of a module's bodies (ADR 0023 rules 2 to 4): U1001 for a name that no
//! scope holds, U1002 for a local that rebinds another, U1005 for local names, and U1006 for
//! undeclared requirements.

use std::collections::BTreeSet;

use una_core::IntType;
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{
    Arg, Arm, Block, BlockExpr, Body, CapBindingValue, CapType, ClauseKind, DeclKind, ElseBranch,
    ExpectationKind, Expr, ExprKind, Field, FnTypeClause, ForSource, Handled, HandlerCall,
    HandlingKind, Ident, IfBlock, IsTarget, KindBody, LetValue, Literal, OpClauseKind, Param,
    PatternKind, Stmt, TextLit, TextPart, Type, TypeCore, VariantExpr,
};

use super::{Checker, Table, prelude};

/// Resolves the bodies of the module of `table`, which has no syntax errors.
pub(super) fn module<'a>(
    checker: &mut Checker<'_, 'a>,
    table: &Table<'a>,
    requirements: &BTreeSet<&'a str>,
) {
    let mut resolver = Resolver {
        checker,
        table,
        requirements,
        locals: Vec::new(),
        types: Vec::new(),
    };
    for decl in &table.module.tree.decls {
        resolver.decl(&decl.kind);
    }
}

/// A binding of a local name.
#[derive(Clone, Copy)]
struct Local<'a> {
    name: &'a str,
    span: Span,
}

struct Resolver<'c, 'p, 'a> {
    checker: &'c mut Checker<'p, 'a>,
    table: &'c Table<'a>,
    requirements: &'c BTreeSet<&'a str>,
    /// Frames of local names, innermost last.
    locals: Vec<Vec<Local<'a>>>,
    /// Frames of type parameters, innermost last.
    types: Vec<Vec<Local<'a>>>,
}

/// The namespace a name is looked up in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    /// Functions, predicates, handlers, policies and locals.
    Lower,
    /// Kinds, variants, capabilities and type parameters.
    Upper,
}

impl<'a> Resolver<'_, '_, 'a> {
    fn name(&self, ident: &Ident) -> &'a str {
        self.checker.name(ident)
    }

    /// Whether some scope holds `name` (ADR 0023 rule 2).
    fn visible(&self, name: &str, space: Space) -> bool {
        let table = self.table;
        match space {
            Space::Lower => {
                self.locals.iter().flatten().any(|local| local.name == name)
                    || table.lower.contains_key(name)
                    || table.imported.contains_key(name)
                    || prelude::LOWER.contains(&name)
                    // Edition 0.2: `set` (`spec/17 §8`).
                    || (name == "set" && self.checker.package.settings.edition >= Edition::V0_2)
            }
            Space::Upper => {
                self.types.iter().flatten().any(|local| local.name == name)
                    || table.upper.contains_key(name)
                    || table.imported.contains_key(name)
                    || table.imported_variants.contains(name)
                    || prelude::UPPER.contains(&name)
                    // Edition 0.2: the fixed-width integer types (`spec/18 §1`).
                    || (self.checker.package.settings.edition >= Edition::V0_2
                        && IntType::from_name(name).is_some())
            }
        }
    }

    /// U1001 when no scope holds the name of `ident`; otherwise, where the name is declared,
    /// for the language server (ADR 0063).
    fn lookup(&mut self, ident: &Ident, space: Space, what: &str) {
        let name = self.name(ident);
        let frames = match space {
            Space::Lower => &self.locals,
            Space::Upper => &self.types,
        };
        let declared = frames
            .iter()
            .rev()
            .flat_map(|frame| frame.iter().rev())
            .find(|local| local.name == name)
            .map(|local| local.span)
            .or_else(|| {
                let table = match space {
                    Space::Lower => &self.table.lower,
                    Space::Upper => &self.table.upper,
                };
                table.get(name).map(|declared| declared.span)
            });
        if let Some(declared) = declared {
            self.checker.references.push((ident.span, declared));
        }
        if !self.visible(name, space) {
            self.checker.error(
                Code::U1001,
                ident.span,
                format!("cannot find {what} `{name}` in this module, its imports or the prelude"),
            );
        }
    }

    fn enter(&mut self) {
        self.locals.push(Vec::new());
    }

    fn leave(&mut self) {
        self.locals.pop();
    }

    /// Binds a local; U1002 when it rebinds a local of an enclosing scope.
    fn bind(&mut self, ident: &Ident, what: &str, check_convention: bool) {
        let name = self.name(ident);
        if check_convention {
            self.checker.lower_convention(ident, what);
        }
        if let Some(earlier) = self
            .locals
            .iter()
            .flatten()
            .find(|local| local.name == name)
            .copied()
        {
            self.checker.duplicate(
                ident.span,
                earlier.span,
                format!("`{name}` is already bound in this scope; a local cannot rebind another"),
            );
            return;
        }
        if let Some(frame) = self.locals.last_mut() {
            frame.push(Local {
                name,
                span: ident.span,
            });
        }
    }

    /// Binds type parameters in a new frame; U1002 for repeats, U1005 for conventions.
    fn type_params(&mut self, params: &[Ident]) {
        let mut frame: Vec<Local<'a>> = Vec::new();
        for param in params {
            let name = self.name(param);
            self.checker.upper_convention(param, "type parameter");
            if let Some(earlier) = frame.iter().find(|local| local.name == name).copied() {
                self.checker.duplicate(
                    param.span,
                    earlier.span,
                    format!("type parameter `{name}` is declared twice"),
                );
                continue;
            }
            frame.push(Local {
                name,
                span: param.span,
            });
        }
        self.types.push(frame);
    }

    /// Fields of a record or variant: U1002 for repeats, U1005 for conventions, and their types.
    fn fields(&mut self, fields: &[Field]) {
        let mut seen: Vec<(&'a str, Span)> = Vec::new();
        for field in fields {
            let name = self.name(&field.name);
            self.checker.lower_convention(&field.name, "field");
            match seen.iter().find(|(known, _)| *known == name) {
                Some(&(_, first)) => self.checker.duplicate(
                    field.name.span,
                    first,
                    format!("field `{name}` is declared twice"),
                ),
                None => seen.push((name, field.name.span)),
            }
            self.ty(&field.ty);
        }
    }

    fn params(&mut self, params: &[Param]) {
        for param in params {
            self.ty(&param.ty);
            self.bind(&param.name, "parameter", true);
        }
    }

    /// Operation names: U1002 for repeats; U1005 only where `convention`, since a handler's
    /// operations name those of its capability.
    fn operation_names<'i>(&mut self, names: impl Iterator<Item = &'i Ident>, convention: bool) {
        let mut seen: Vec<(&'a str, Span)> = Vec::new();
        for ident in names {
            let name = self.name(ident);
            if convention {
                self.checker.lower_convention(ident, "operation");
            }
            match seen.iter().find(|(known, _)| *known == name) {
                Some(&(_, first)) => self.checker.duplicate(
                    ident.span,
                    first,
                    format!("operation `{name}` is declared twice"),
                ),
                None => seen.push((name, ident.span)),
            }
        }
    }

    fn decl(&mut self, kind: &DeclKind) {
        match kind {
            DeclKind::Kind(kind) => {
                self.type_params(&kind.params);
                match &kind.body {
                    KindBody::Alias { ty, refinement } => {
                        self.ty(ty);
                        if let Some(refinement) = refinement {
                            self.enter();
                            self.expr(refinement);
                            self.leave();
                        }
                    }
                    KindBody::Sum(variants) => {
                        for variant in variants {
                            self.fields(&variant.fields);
                        }
                    }
                    KindBody::Record(fields) => self.fields(fields),
                }
                self.types.pop();
            }
            DeclKind::Predicate(predicate) => {
                self.enter();
                self.params(&predicate.params);
                self.ty(&predicate.result);
                self.body(&predicate.body);
                self.leave();
            }
            DeclKind::Function(function) => {
                self.type_params(&function.type_params);
                self.enter();
                self.params(&function.params);
                self.ty(&function.result);
                for clause in &function.clauses {
                    match &clause.kind {
                        ClauseKind::Requires(expr) | ClauseKind::Ensures(expr) => self.expr(expr),
                        ClauseKind::Uses(uses) => self.cap_type(&uses.cap_type),
                        ClauseKind::Fails(kind) => self.lookup(kind, Space::Upper, "error kind"),
                        // A parameter, which the type checker resolves (ADR 0062).
                        ClauseKind::UsesOf(_) | ClauseKind::FailsOf(_) => {}
                        ClauseKind::DerivedFrom(requirements) => {
                            for requirement in requirements {
                                self.requirement(requirement);
                            }
                        }
                    }
                }
                self.body(&function.body);
                self.leave();
                self.types.pop();
            }
            DeclKind::Capability(capability) => {
                self.operation_names(capability.ops.iter().map(|op| &op.name), true);
                for op in &capability.ops {
                    self.enter();
                    self.params(&op.params);
                    self.ty(&op.result);
                    for clause in &op.clauses {
                        match &clause.kind {
                            OpClauseKind::Requires(expr) | OpClauseKind::Ensures(expr) => {
                                self.expr(expr);
                            }
                            OpClauseKind::Fails(kind) => {
                                self.lookup(kind, Space::Upper, "error kind");
                            }
                            OpClauseKind::Accepts(_) => {}
                        }
                    }
                    self.leave();
                }
            }
            DeclKind::Handler(handler) => {
                self.lookup(&handler.capability, Space::Upper, "capability");
                self.enter();
                self.params(&handler.params);
                for uses in &handler.uses {
                    self.cap_type(&uses.cap_type);
                }
                self.operation_names(handler.ops.iter().map(|op| &op.name), false);
                for op in &handler.ops {
                    self.enter();
                    for param in &op.params {
                        self.bind(param, "parameter", true);
                    }
                    self.body(&op.body);
                    self.leave();
                }
                self.leave();
            }
            DeclKind::Policy(policy) => {
                self.ty(&policy.from);
                self.ty(&policy.to);
                self.lookup(&policy.via, Space::Lower, "function");
                self.text(&policy.purpose);
            }
            DeclKind::Requirement(_) => {}
            DeclKind::Example(example) => {
                self.enter();
                for given in &example.givens {
                    self.handler_call(&given.handler);
                }
                self.expr(&example.action);
                for expectation in &example.expectations {
                    match &expectation.kind {
                        // A variant after `expect fails` takes its kind from
                        // the `do` expression (M4).
                        ExpectationKind::Fails(_) | ExpectationKind::Performed { .. } => {}
                        ExpectationKind::Holds(expr) => self.expr(expr),
                    }
                }
                self.leave();
            }
        }
    }

    /// U1006: a requirement that the package does not declare (`spec/08 §1` rule 3).
    fn requirement(&mut self, ident: &Ident) {
        let name = self.name(ident);
        if !self.requirements.contains(name) {
            self.checker.error(
                Code::U1006,
                ident.span,
                format!("requirement `{name}` is not declared in this package"),
            );
        }
    }

    fn cap_type(&mut self, cap_type: &CapType) {
        self.lookup(&cap_type.name, Space::Upper, "capability");
    }

    /// A type (an unknown type name is U1001); label categories wait for M6.
    fn ty(&mut self, ty: &Type) {
        match &ty.core {
            TypeCore::Named { name, args } => {
                self.lookup(name, Space::Upper, "type");
                for arg in args {
                    self.ty(arg);
                }
            }
            TypeCore::List(element) => self.ty(element),
            TypeCore::Function {
                params,
                result,
                clauses,
            } => {
                for param in params {
                    self.ty(param);
                }
                self.ty(result);
                for clause in clauses {
                    match clause {
                        FnTypeClause::Uses(uses) => self.cap_type(&uses.cap_type),
                        FnTypeClause::Fails(kind) => self.lookup(kind, Space::Upper, "error kind"),
                    }
                }
            }
        }
    }

    fn body(&mut self, body: &Body) {
        match body {
            Body::Line(expr) => self.expr(expr),
            Body::Block(block) => self.block(block),
        }
    }

    fn block(&mut self, block: &Block) {
        self.enter();
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let(stmt) => {
                    if let Some(ty) = &stmt.ty {
                        self.ty(ty);
                    }
                    match &stmt.value {
                        LetValue::Handled(handled) => self.handled(handled),
                        LetValue::BlockExpr(expr) => self.block_expr(expr),
                    }
                    self.bind(&stmt.name, "local", true);
                }
                Stmt::Assign(stmt) => {
                    self.expr(&stmt.place);
                    match &stmt.value {
                        LetValue::Handled(handled) => self.handled(handled),
                        LetValue::BlockExpr(expr) => self.block_expr(expr),
                    }
                }
                Stmt::IfThen(stmt) => {
                    self.expr(&stmt.condition);
                    self.block(&stmt.block);
                }
                Stmt::For(stmt) => {
                    match &stmt.source {
                        ForSource::List(list) => self.expr(list),
                        ForSource::Range { low, high, .. } => {
                            self.expr(low);
                            self.expr(high);
                        }
                    }
                    // The loop variable is in scope in the body only.
                    self.enter();
                    self.bind(&stmt.var, "local", true);
                    self.block(&stmt.block);
                    self.leave();
                }
                Stmt::Handled(handled) => self.handled(handled),
                Stmt::BlockExpr(expr) => self.block_expr(expr),
            }
        }
        self.leave();
    }

    fn handled(&mut self, handled: &Handled) {
        self.expr(&handled.expr);
        for binding in &handled.with {
            if let CapBindingValue::Handler(call) = &binding.value {
                self.handler_call(call);
            }
        }
        for handling in &handled.handlings {
            match &handling.kind {
                HandlingKind::ErrorFail(variant) | HandlingKind::NoneFail(variant) => {
                    self.variant(variant);
                }
                HandlingKind::ErrorUse(expr) | HandlingKind::NoneUse(expr) => self.expr(expr),
                HandlingKind::ErrorPass => {}
            }
        }
    }

    fn handler_call(&mut self, call: &HandlerCall) {
        self.lookup(&call.name, Space::Lower, "handler");
        self.args(&call.args);
    }

    fn block_expr(&mut self, expr: &BlockExpr) {
        match expr {
            BlockExpr::When(when) => {
                self.expr(&when.scrutinee);
                for arm in &when.arms {
                    self.arm(arm);
                }
            }
            BlockExpr::If(block) => self.if_block(block),
        }
    }

    fn if_block(&mut self, block: &IfBlock) {
        let mut current = block;
        loop {
            self.expr(&current.condition);
            self.block(&current.then_block);
            match current.otherwise.as_ref() {
                ElseBranch::Block(otherwise) => {
                    self.block(otherwise);
                    break;
                }
                ElseBranch::If(next) => current = next,
            }
        }
    }

    fn arm(&mut self, arm: &Arm) {
        self.enter();
        match &arm.pattern.kind {
            // A pattern's variant takes its kind from the value tested (M4).
            PatternKind::Variant { binds, .. } => {
                for bind in binds {
                    match &bind.local {
                        Some(local) => self.bind(local, "local", true),
                        // The field's own declaration is checked for U1005.
                        None => self.bind(&bind.field, "local", false),
                    }
                }
            }
            PatternKind::Literal(Literal::Text(text)) => self.text(text),
            PatternKind::Literal(_) | PatternKind::Range(_) | PatternKind::Else => {}
        }
        self.body(&arm.body);
        self.leave();
    }

    /// A variant after `fail` takes its kind from the error kind of the
    /// function (M4); only its arguments are resolved here.
    fn variant(&mut self, variant: &VariantExpr) {
        self.args(&variant.args);
    }

    /// Argument values; labels depend on the callee's type (M4).
    fn args(&mut self, args: &[Arg]) {
        for arg in args {
            self.expr(&arg.value);
        }
    }

    fn text(&mut self, text: &TextLit) {
        for part in &text.parts {
            if let TextPart::Interpolation(expr) = part {
                self.expr(expr);
            }
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } => {
                self.expr(condition);
                self.expr(then);
                self.expr(otherwise);
            }
            ExprKind::Lambda { params, body } => {
                self.enter();
                for param in params {
                    if let Some(ty) = &param.ty {
                        self.ty(ty);
                    }
                    self.bind(&param.name, "parameter", true);
                }
                self.expr(body);
                self.leave();
            }
            ExprKind::Checked {
                value,
                predicate,
                failure,
            } => {
                self.expr(value);
                self.expr(predicate);
                self.variant(failure);
            }
            ExprKind::Attempt(operand)
            | ExprKind::Unary { operand, .. }
            | ExprKind::Old(operand)
            | ExprKind::Paren(operand) => self.expr(operand),
            ExprKind::Fail(variant) => self.variant(variant),
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Is { value, target } => {
                self.expr(value);
                match target {
                    // The variant takes its kind from the value tested (M4).
                    IsTarget::Literal(Literal::Text(text)) => self.text(text),
                    IsTarget::Variant(_) | IsTarget::Literal(_) => {}
                }
            }
            // Fields and methods depend on types (M4).
            ExprKind::In { value, .. } | ExprKind::Field { target: value, .. } => self.expr(value),
            ExprKind::MethodCall { target, args, .. } => {
                self.expr(target);
                self.args(args);
            }
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                self.args(args);
            }
            // Capability references are resolved against capability contexts (M5).
            ExprKind::CapOp { args, .. } => self.args(args),
            ExprKind::List(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            ExprKind::Name(name) => self.lookup(name, Space::Lower, "name"),
            ExprKind::UpperName(name) => self.lookup(name, Space::Upper, "name"),
            ExprKind::Literal(Literal::Text(text)) => self.text(text),
            ExprKind::CapRef(_)
            | ExprKind::Result
            | ExprKind::It
            | ExprKind::Error
            | ExprKind::Literal(_) => {}
        }
    }
}
