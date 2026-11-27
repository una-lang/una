//! Resolving every name of a module's bodies (ADR 0023 rules 2 to 4): U1001 for a name that no
//! scope holds, U1002 for a local that rebinds another, U1005 for local names, and U1006 for
//! undeclared requirements.

use std::collections::BTreeSet;

/// The names that JSON adds to the prelude in edition 0.2 (`spec/23 §1`): its kinds and their
/// variants.
pub(crate) const JSON_NAMES: &[&str] = &[
    "Json",
    "JsonNumber",
    "JsonError",
    "Null",
    "Boolean",
    "Number",
    "Decimal",
    "Str",
    "Array",
    "Object",
    "Syntax",
    "TooDeep",
    "DuplicateKey",
    "NumberTooLong",
];

/// The names that HTTP adds to the prelude in edition 0.2 (`spec/24 §1`): its capability, its
/// kinds and the variants of its failure.
pub(crate) const HTTP_NAMES: &[&str] = &[
    "Http",
    "Header",
    "Response",
    "HttpError",
    "Denied",
    "BadUrl",
    "BadRequest",
    "Unreachable",
    "TimedOut",
    "TooLarge",
    "Tls",
    "Protocol",
];

use una_core::IntType;
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{
    Arg, Arm, Block, BlockExpr, Body, CapBindingValue, CapType, ClauseKind, DeclKind, ElseBranch,
    ExpectationKind, Expr, ExprKind, Field, FnTypeClause, ForSource, Handled, HandlerCall,
    HandlingKind, Ident, IfBlock, IsTarget, KindBody, LetStmt, LetValue, Literal, OpClauseKind,
    Param, PatternKind, Stmt, TextLit, TextPart, Type, TypeCore, VariantExpr,
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
        locals: Frames::default(),
        types: Frames::default(),
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
    locals: Frames<'a>,
    /// Frames of type parameters, innermost last.
    types: Frames<'a>,
}

/// Frames of local names, innermost last, with an index from each name to where it is bound,
/// so a name resolves in expected O(1) whatever the number of locals (#326). The index is only
/// looked up, never iterated.
#[derive(Default)]
struct Frames<'a> {
    frames: Vec<Vec<Local<'a>>>,
    index: std::collections::HashMap<&'a str, Vec<Span>>,
}

impl<'a> Frames<'a> {
    fn push(&mut self, frame: Vec<Local<'a>>) {
        for local in &frame {
            self.index.entry(local.name).or_default().push(local.span);
        }
        self.frames.push(frame);
    }

    fn pop(&mut self) {
        for local in self.frames.pop().unwrap_or_default() {
            if let Some(spans) = self.index.get_mut(local.name) {
                spans.pop();
                if spans.is_empty() {
                    self.index.remove(local.name);
                }
            }
        }
    }

    /// Binds `local` in the innermost frame.
    fn bind(&mut self, local: Local<'a>) {
        if let Some(frame) = self.frames.last_mut() {
            self.index.entry(local.name).or_default().push(local.span);
            frame.push(local);
        }
    }

    /// Where the innermost binding of `name` is.
    fn find(&self, name: &str) -> Option<Span> {
        self.index.get(name).and_then(|spans| spans.last().copied())
    }

    /// The names bound in every frame, innermost frame last, in the order they were bound.
    fn names(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.frames.iter().flatten().map(|local| local.name)
    }
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

    /// The edition of the module being resolved, its package's (ADR 0122 rule 49).
    fn edition(&self) -> Edition {
        self.checker.edition(self.table.module.id)
    }

    /// Whether some scope holds `name` (ADR 0023 rule 2).
    fn visible(&self, name: &str, space: Space) -> bool {
        let table = self.table;
        match space {
            Space::Lower => {
                self.locals.find(name).is_some()
                    || table.lower.contains_key(name)
                    || table.imported.contains_key(name)
                    || prelude::LOWER.contains(&name)
                    // Edition 0.2: `set` (`spec/17 §8`), `sort` (`spec/20 §6`), `empty_map`
                    // (`spec/21 §2`), the text functions (`spec/22 §2`) and the JSON functions
                    // (`spec/23 §2`).
                    || ((matches!(
                        name,
                        "set"
                            | "sort"
                            | "empty_map"
                            | "parse_json"
                            | "to_json"
                            | "quote_json"
                            | "is_json_number"
                    ) || prelude::TEXT_0_2.contains(&name))
                        && self.edition() >= Edition::V0_2)
                    // Edition 0.3: the functions of `F64` (`spec/29`, ADR 0114 rule 3.9).
                    || (prelude::FLOAT_0_3.contains(&name)
                        && self.edition() >= Edition::V0_3)
                    // Edition 0.5: the functions of `Decimal` (`spec/29 §16`, ADR 0120), of
                    // `F32` (ADR 0120 rule 5) and of `Complex` (ADR 0120 rule 22).
                    || ((prelude::DECIMAL_0_5.contains(&name)
                        || prelude::FLOAT_0_5.contains(&name)
                        || prelude::COMPLEX_0_5.contains(&name))
                        && self.edition() >= Edition::V0_5)
                    // Edition 0.5: the constructors of dense values (`spec/29 §17`, ADR 0120
                    // rule 32).
                    || (prelude::DENSE_0_5.contains(&name)
                        && self.edition() >= Edition::V0_5)
            }
            Space::Upper => {
                self.types.find(name).is_some()
                    || table.upper.contains_key(name)
                    || table.imported.contains_key(name)
                    || table.imported_variants.contains(name)
                    || prelude::UPPER.contains(&name)
                    // Edition 0.2: the fixed-width integer types (`spec/18 §1`), maps with
                    // their entries (`spec/21 §1`), the kind of `parse_int`'s failures with its
                    // variants (`spec/22 §4`), and JSON (`spec/23 §1`).
                    || (self.edition() >= Edition::V0_2
                        && (IntType::from_name(name).is_some()
                            || matches!(
                                name,
                                "Map"
                                    | "Entry"
                                    | "ParseError"
                                    | "Empty"
                                    | "NotADigit"
                                    | "NoDigits"
                                    | "TooLong"
                            )
                            || JSON_NAMES.contains(&name)
                            || HTTP_NAMES.contains(&name)))
                    || (self.edition() >= Edition::V0_3 && name == "F64")
                    // Edition 0.4: the variants `HttpError` gains (ADR 0116 rule 12, ADR 0117
                    // rule 8).
                    || (self.edition() >= Edition::V0_4
                        && prelude::HTTP_0_4.contains(&name))
                    // Edition 0.5: `Rounding` and its variants (`spec/29 §16`, ADR 0120).
                    || (self.edition() >= Edition::V0_5
                        && prelude::ROUNDING_0_5.contains(&name))
                    // Edition 0.5: the dense types (`spec/29 §17`, ADR 0120 rule 3).
                    || (self.edition() >= Edition::V0_5
                        && matches!(name, "Array" | "Matrix" | "Tensor" | "Lu"))
                    // Edition 0.5: `F32`, the finite kinds and `Complex` (ADR 0120 rules 5, 11
                    // and 22).
                    || (self.edition() >= Edition::V0_5
                        && prelude::UPPER_0_5.contains(&name))
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
        let declared = frames.find(name).or_else(|| {
            let table = match space {
                Space::Lower => &self.table.lower,
                Space::Upper => &self.table.upper,
            };
            table.get(name).map(|declared| declared.span)
        });
        if let Some(declared) = declared {
            self.checker.references.push((ident.span, declared));
        }
        if !self.visible(name, space) && !self.checker.poisoned(self.table.module.id, name) {
            let mut diagnostic = self.checker.make(
                Code::U1001,
                ident.span,
                format!("cannot find {what} `{name}` in this module, its imports or the prelude"),
            );
            // ADR 0139 rule 7: a name that edition 0.5 has and edition 0.4 lost, first.
            if self.edition() == Edition::V0_4 && prelude::moved_to_0_5(name) {
                diagnostic
                    .repairs
                    .push(Checker::advice(prelude::moved_to_0_5_advice(name)));
            }
            // #419: the names in scope closest to it, as advice only. A typo's fix is a guess,
            // and an edit that an agent applies unread could bind another name that checks
            // (an edit comes only when the fix is unambiguous; R-1, #125).
            let near = crate::types::nearest(name, self.candidates(space));
            if !near.is_empty() {
                let listed: Vec<String> = near
                    .iter()
                    .map(|(_, candidate)| format!("`{candidate}`"))
                    .collect();
                diagnostic.repairs.push(Checker::advice(format!(
                    "names in scope close to `{name}`: {}",
                    listed.join(", ")
                )));
            }
            self.checker.diagnostics.push(diagnostic);
        }
    }

    /// Every name that [`Resolver::visible`] finds in `space` here, each scope in a fixed
    /// order, for the suggestions of U1001; only built when a name is not found.
    fn candidates(&self, space: Space) -> Vec<&'a str> {
        let table = self.table;
        let edition_0_2 = self.edition() >= Edition::V0_2;
        let lower = |name: &&str| name.starts_with(|c: char| c.is_lowercase());
        let mut names: Vec<&'a str> = Vec::new();
        match space {
            Space::Lower => {
                names.extend(self.locals.names());
                names.extend(table.lower.keys().copied());
                names.extend(table.imported.keys().copied().filter(lower));
                names.extend(prelude::LOWER);
                if edition_0_2 {
                    names.extend([
                        "set",
                        "sort",
                        "empty_map",
                        "parse_json",
                        "to_json",
                        "quote_json",
                        "is_json_number",
                    ]);
                    names.extend(prelude::TEXT_0_2);
                }
                if self.edition() >= Edition::V0_3 {
                    names.extend(prelude::FLOAT_0_3);
                }
                if self.edition() >= Edition::V0_5 {
                    names.extend(prelude::DECIMAL_0_5);
                    names.extend(prelude::DENSE_0_5);
                    names.extend(prelude::FLOAT_0_5);
                    names.extend(prelude::COMPLEX_0_5);
                }
            }
            Space::Upper => {
                names.extend(self.types.names());
                names.extend(table.upper.keys().copied());
                names.extend(table.imported.keys().copied().filter(|n| !lower(n)));
                names.extend(table.imported_variants.iter().copied());
                names.extend(prelude::UPPER);
                if edition_0_2 {
                    names.extend(IntType::ALL.iter().map(|int| int.as_str()));
                    names.extend([
                        "Map",
                        "Entry",
                        "ParseError",
                        "Empty",
                        "NotADigit",
                        "NoDigits",
                        "TooLong",
                    ]);
                    names.extend(JSON_NAMES.iter().copied());
                    names.extend(HTTP_NAMES.iter().copied());
                }
                if self.edition() >= Edition::V0_3 {
                    names.push("F64");
                }
                if self.edition() >= Edition::V0_4 {
                    names.extend(prelude::HTTP_0_4);
                }
                if self.edition() >= Edition::V0_5 {
                    names.extend(prelude::ROUNDING_0_5);
                    names.extend(["Matrix", "Tensor", "Lu"]);
                    names.extend(prelude::UPPER_0_5);
                }
            }
        }
        names
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
        if let Some(earlier) = self.locals.find(name) {
            self.checker.duplicate(
                ident.span,
                earlier,
                format!("`{name}` is already bound in this scope; a local cannot rebind another"),
            );
            return;
        }
        self.locals.bind(Local {
            name,
            span: ident.span,
        });
    }

    /// Binds type parameters in a new frame; U1002 for repeats, U1005 for conventions.
    fn type_params(&mut self, params: &[Ident]) {
        let mut frame: Vec<Local<'a>> = Vec::new();
        for param in params {
            let name = self.name(param);
            self.checker.upper_convention(param, "type parameter");
            // Edition 0.5: no declaration hides a dense type (`spec/29 §17` rule 1).
            if self.edition() >= Edition::V0_5
                && matches!(name, "Array" | "Matrix" | "Tensor" | "Lu")
            {
                let diagnostic = self.checker.make(
                    Code::U1002,
                    param.span,
                    format!(
                        "`{name}` is a built-in dense type from edition 0.5, so a type parameter cannot take its name"
                    ),
                );
                self.checker.diagnostics.push(diagnostic);
            }
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
                        ClauseKind::UsesOf(_) | ClauseKind::FailsOf(_) | ClauseKind::Diverges => {}
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
            // Unit expressions resolve in the type check, which reports an unknown unit as
            // U2029 (ADR 0104 rule 1).
            DeclKind::Requirement(_) | DeclKind::Measure(_) => {}
            // A module constant (ADR 0126 rule 2): its type, and its value with no local in
            // scope.
            DeclKind::Const(constant) => {
                self.ty(&constant.ty);
                self.enter();
                self.expr(&constant.value);
                self.leave();
            }
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
        // The conditions of its operations, over the parameters in scope (ADR 0092 rule 1).
        for condition in cap_type.conditions.iter().flatten() {
            self.expr(condition);
        }
    }

    /// A type (an unknown type name is U1001); label categories wait for M6.
    fn ty(&mut self, ty: &Type) {
        // A range bound that names a constant, edition 0.5 (ADR 0126 rule 9).
        for bound in ty.range.iter().flat_map(|range| [&range.low, &range.high]) {
            if let Some(constant) = &bound.constant {
                self.lookup(constant, Space::Lower, "constant");
            }
        }
        match &ty.core {
            TypeCore::Named { name, args } => {
                self.lookup(name, Space::Upper, "type");
                for arg in args {
                    self.ty(arg);
                }
            }
            TypeCore::List(element) => self.ty(element),
            // `Decimal[s]` is built in, edition 0.4 (ADR 0120 rule 2).
            TypeCore::Decimal { .. } => {}
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
            self.stmt(stmt);
        }
        self.leave();
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(stmt) => {
                self.let_value(stmt);
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
                self.for_source(&stmt.source);
                // The loop variable is in scope in the body only.
                self.enter();
                self.bind(&stmt.var, "local", true);
                self.block(&stmt.block);
                self.leave();
            }
            Stmt::Together(stmt) => {
                // A branch sees the names before the block and none of its siblings'; the
                // branches' names are in scope after it (`spec/19 §2` rule 1).
                for branch in &stmt.body {
                    match branch {
                        Stmt::Let(branch) => self.let_value(branch),
                        // Not a branch (U2024), resolved in a scope of its own.
                        other => {
                            self.enter();
                            self.stmt(other);
                            self.leave();
                        }
                    }
                }
                for branch in &stmt.body {
                    if let Stmt::Let(branch) = branch {
                        self.bind(&branch.name, "local", true);
                    }
                }
            }
            Stmt::Handled(handled) => self.handled(handled),
            Stmt::BlockExpr(expr) => self.block_expr(expr),
            Stmt::Stop(stmt) => {
                if let Some(condition) = &stmt.condition {
                    self.expr(condition);
                }
            }
            Stmt::Skip(_) => {}
            Stmt::Until(stmt) => {
                self.expr(&stmt.condition);
                self.expr(&stmt.measure);
                self.block(&stmt.block);
            }
            Stmt::Ever(stmt) => {
                // The deadline is evaluated once, before the first iteration (`spec/26 §7`).
                if let Some(within) = &stmt.within {
                    self.expr(within);
                }
                self.block(&stmt.block);
                if let Some(on_stop) = &stmt.on_stop {
                    self.block(&on_stop.block);
                }
            }
            Stmt::OnStop(on_stop) => self.block(&on_stop.block),
            // The bound is evaluated once, before the block (`spec/27 §7`).
            Stmt::Tasks(stmt) => {
                if let Some(bound) = &stmt.bound {
                    self.expr(bound);
                }
                self.block(&stmt.block);
            }
            // The started expression sees the names before it, in a scope of its own.
            Stmt::Start(stmt) => {
                self.enter();
                self.handled(&stmt.handled);
                self.leave();
            }
        }
    }

    /// The type and value of a `let`, before its name is bound.
    fn let_value(&mut self, stmt: &LetStmt) {
        if let Some(ty) = &stmt.ty {
            self.ty(ty);
        }
        match &stmt.value {
            LetValue::Handled(handled) => self.handled(handled),
            LetValue::BlockExpr(expr) => self.block_expr(expr),
        }
    }

    fn for_source(&mut self, source: &ForSource) {
        match source {
            ForSource::List(list) => self.expr(list),
            ForSource::Range { low, high, .. } => {
                self.expr(low);
                self.expr(high);
            }
        }
    }

    fn handled(&mut self, handled: &Handled) {
        self.expr(&handled.expr);
        for binding in &handled.with {
            if let CapBindingValue::Handler(call) = &binding.value {
                self.handler_call(call);
            }
        }
        if let Some(within) = &handled.within {
            self.expr(within);
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
            BlockExpr::TogetherFor(together) => {
                self.for_source(&together.source);
                self.enter();
                self.bind(&together.var, "local", true);
                self.block(&together.block);
                self.leave();
            }
            // Each branch sees the names before the block, in a scope of its own (ADR 0117
            // rule 6).
            BlockExpr::FirstOf(first) => {
                for branch in &first.body {
                    self.enter();
                    self.stmt(branch);
                    self.leave();
                }
            }
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
            ExprKind::Attempt { operand, within } => {
                self.expr(operand);
                if let Some(within) = within {
                    self.expr(within);
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Old(operand) | ExprKind::Paren(operand) => {
                self.expr(operand)
            }
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
