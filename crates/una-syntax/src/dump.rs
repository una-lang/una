//! A compact, span-free rendering of the syntax tree as S-expressions, for snapshot tests and
//! for comparing trees that differ only in layout (the formatter's tests, M2).

use crate::ast::{
    Arg, Arm, BinaryOp, Block, BlockExpr, Body, CapBinding, CapBindingValue, CapType, ClauseKind,
    Decl, DeclKind, ElseBranch, ExpectationKind, Expr, ExprKind, Field, File, FnTypeClause,
    ForSource, Handled, HandlerCall, HandlingKind, Ident, IfBlock, IsTarget, KindBody, Label,
    LetValue, Level, Literal, OpClauseKind, Param, PatternKind, Range, RequirementSource,
    SignedInt, Stmt, TextLit, TextPart, Type, TypeCore, UnaryOp, UnitExpr, UsesClause, VariantExpr,
};
use crate::intern::Interner;

/// The width past which a node is split over several lines.
const WIDTH: usize = 100;

/// Renders `file` as S-expressions, one declaration per top-level form. Doc comments are
/// shown with their text, which is read from `text`, the file's source.
pub fn dump(file: &File, interner: &Interner, text: &str) -> String {
    let dumper = Dumper { interner, text };
    let mut forms = Vec::new();
    if let Some(module) = &file.module {
        let mut header = vec![atom("module")];
        if module.trusted {
            header.push(atom("trusted"));
        }
        header.push(atom(dumper.path(&module.path.segments)));
        forms.push(Node::List(header));
    }
    for import in &file.imports {
        let mut items = vec![atom("import"), atom(dumper.path(&import.module.segments))];
        items.extend(import.names.iter().map(|name| atom(dumper.name(name))));
        forms.push(Node::List(items));
    }
    forms.extend(file.decls.iter().map(|decl| dumper.decl(decl)));
    let mut out = String::new();
    for form in &forms {
        render(form, 0, &mut out);
        out.push('\n');
    }
    out
}

enum Node {
    Atom(String),
    List(Vec<Node>),
}

fn atom(text: impl Into<String>) -> Node {
    Node::Atom(text.into())
}

fn list(head: &str, rest: impl IntoIterator<Item = Node>) -> Node {
    let mut items = vec![atom(head)];
    items.extend(rest);
    Node::List(items)
}

fn flat(node: &Node) -> String {
    match node {
        Node::Atom(text) => text.clone(),
        Node::List(items) => {
            let inner: Vec<String> = items.iter().map(flat).collect();
            format!("({})", inner.join(" "))
        }
    }
}

/// Writes `node` at `indent`: on one line when it fits, otherwise with its leading atoms on the
/// first line and each other child on its own line, two spaces deeper.
fn render(node: &Node, indent: usize, out: &mut String) {
    let one_line = flat(node);
    let Node::List(items) = node else {
        out.push_str(&one_line);
        return;
    };
    if indent + one_line.len() <= WIDTH {
        out.push_str(&one_line);
        return;
    }
    let leading = items
        .iter()
        .take_while(|item| matches!(item, Node::Atom(_)))
        .count()
        .max(1);
    out.push('(');
    let head: Vec<String> = items.iter().take(leading).map(flat).collect();
    out.push_str(&head.join(" "));
    for item in items.iter().skip(leading) {
        out.push('\n');
        out.push_str(&" ".repeat(indent + 2));
        render(item, indent + 2, out);
    }
    out.push(')');
}

struct Dumper<'a> {
    interner: &'a Interner,
    text: &'a str,
}

impl Dumper<'_> {
    fn name(&self, ident: &Ident) -> String {
        self.interner.resolve(ident.name).to_owned()
    }

    fn path(&self, segments: &[Ident]) -> String {
        let names: Vec<String> = segments.iter().map(|segment| self.name(segment)).collect();
        names.join(".")
    }

    fn decl(&self, decl: &Decl) -> Node {
        let mut items = match &decl.kind {
            DeclKind::Kind(kind) => {
                let mut items = vec![atom("kind")];
                if kind.public {
                    items.push(atom("pub"));
                }
                items.push(atom(self.name(&kind.name)));
                if !kind.params.is_empty() {
                    items.push(self.type_params(&kind.params));
                }
                items.push(match &kind.body {
                    KindBody::Alias { ty, refinement } => {
                        let mut alias = vec![atom("alias"), self.ty(ty)];
                        if let Some(refinement) = refinement {
                            alias.push(list("where", [self.expr(refinement)]));
                        }
                        Node::List(alias)
                    }
                    KindBody::Sum(variants) => list(
                        "sum",
                        variants.iter().map(|variant| {
                            if variant.fields.is_empty() {
                                atom(self.name(&variant.name))
                            } else {
                                let mut items = vec![atom(self.name(&variant.name))];
                                items.extend(variant.fields.iter().map(|field| self.field(field)));
                                Node::List(items)
                            }
                        }),
                    ),
                    KindBody::Record(fields) => {
                        list("record", fields.iter().map(|field| self.field(field)))
                    }
                });
                items
            }
            DeclKind::Predicate(predicate) => {
                let mut items = vec![atom("predicate")];
                if predicate.public {
                    items.push(atom("pub"));
                }
                items.push(atom(if predicate.reflect {
                    "reflect"
                } else {
                    "opaque"
                }));
                items.push(atom(self.name(&predicate.name)));
                items.push(self.params(&predicate.params));
                items.push(self.ty(&predicate.result));
                items.push(self.body(&predicate.body));
                items
            }
            DeclKind::Function(function) => {
                let mut items = vec![atom("fn")];
                if function.public {
                    items.push(atom("pub"));
                }
                items.push(atom(self.name(&function.name)));
                if !function.type_params.is_empty() {
                    let names: Vec<String> = function
                        .type_params
                        .iter()
                        .zip(&function.constraints)
                        .map(|(param, constraint)| match constraint {
                            Some(constraint) => {
                                format!("{}: {}", self.name(param), self.name(constraint))
                            }
                            None => self.name(param),
                        })
                        .collect();
                    items.push(atom(format!("[{}]", names.join(", "))));
                }
                items.push(self.params(&function.params));
                items.push(self.ty(&function.result));
                for clause in &function.clauses {
                    items.push(match &clause.kind {
                        ClauseKind::Requires(expr) => list("requires", [self.expr(expr)]),
                        ClauseKind::Ensures(expr) => list("ensures", [self.expr(expr)]),
                        ClauseKind::Uses(uses) => self.uses(uses),
                        ClauseKind::Fails(kind) => list("fails", [atom(self.name(kind))]),
                        ClauseKind::UsesOf(param) => list("uses-of", [atom(self.name(param))]),
                        ClauseKind::FailsOf(param) => list("fails-of", [atom(self.name(param))]),
                        ClauseKind::Diverges => atom("diverges"),
                        ClauseKind::DerivedFrom(requirements) => list(
                            "derived-from",
                            requirements.iter().map(|id| atom(self.name(id))),
                        ),
                    });
                }
                items.push(self.body(&function.body));
                items
            }
            DeclKind::Capability(capability) => {
                let mut items = vec![atom("capability")];
                if capability.public {
                    items.push(atom("pub"));
                }
                items.push(atom(self.name(&capability.name)));
                for op in &capability.ops {
                    let mut op_items = vec![
                        atom("op"),
                        atom(self.name(&op.name)),
                        self.params(&op.params),
                        self.ty(&op.result),
                    ];
                    op_items.extend(op.clauses.iter().map(|clause| match &clause.kind {
                        OpClauseKind::Requires(expr) => list("requires", [self.expr(expr)]),
                        OpClauseKind::Ensures(expr) => list("ensures", [self.expr(expr)]),
                        OpClauseKind::Fails(kind) => list("fails", [atom(self.name(kind))]),
                        OpClauseKind::Accepts(label) => list("accepts", [atom(self.label(label))]),
                    }));
                    items.push(Node::List(op_items));
                }
                items
            }
            DeclKind::Handler(handler) => {
                let mut items = vec![atom("handler")];
                if handler.public {
                    items.push(atom("pub"));
                }
                items.push(atom(self.name(&handler.name)));
                items.push(self.params(&handler.params));
                items.push(list("for", [atom(self.name(&handler.capability))]));
                items.extend(handler.uses.iter().map(|uses| self.uses(uses)));
                for op in &handler.ops {
                    items.push(list(
                        "op",
                        [
                            atom(self.name(&op.name)),
                            Node::List(op.params.iter().map(|p| atom(self.name(p))).collect()),
                            self.body(&op.body),
                        ],
                    ));
                }
                items
            }
            DeclKind::Policy(policy) => {
                let mut items = vec![atom("policy")];
                if policy.public {
                    items.push(atom("pub"));
                }
                items.push(atom(self.name(&policy.name)));
                items.push(list("from", [self.ty(&policy.from)]));
                items.push(list("to", [self.ty(&policy.to)]));
                items.push(list("via", [atom(self.name(&policy.via))]));
                items.push(list("purpose", [self.text(&policy.purpose)]));
                items.push(list("owner", [atom(self.name(&policy.owner))]));
                items
            }
            DeclKind::Requirement(requirement) => {
                let source = match &requirement.source {
                    RequirementSource::Human => list("source", [atom("human")]),
                    RequirementSource::Policy(document) => {
                        list("source", [atom("policy"), self.text(document)])
                    }
                    RequirementSource::Regulation(document) => {
                        list("source", [atom("regulation"), self.text(document)])
                    }
                    RequirementSource::Generated(task) => {
                        list("source", [atom("generated"), self.text(task)])
                    }
                };
                vec![
                    atom("requirement"),
                    atom(self.name(&requirement.id)),
                    list("text", [self.text(&requirement.text)]),
                    source,
                    list("owner", [atom(self.name(&requirement.owner))]),
                    list(
                        "status",
                        [atom(if requirement.active {
                            "active"
                        } else {
                            "proposed"
                        })],
                    ),
                ]
            }
            DeclKind::Example(example) => {
                let mut items = vec![atom("example"), self.text(&example.name)];
                for given in &example.givens {
                    items.push(list(
                        "given",
                        [
                            atom(format!("@{}", self.name(&given.cap))),
                            self.handler_call(&given.handler),
                        ],
                    ));
                }
                items.push(list("do", [self.expr(&example.action)]));
                for expectation in &example.expectations {
                    items.push(match &expectation.kind {
                        ExpectationKind::Fails(variant) => {
                            list("expect-fails", [atom(self.name(variant))])
                        }
                        ExpectationKind::Performed { negated, cap, op } => list(
                            if *negated {
                                "expect-not-performed"
                            } else {
                                "expect-performed"
                            },
                            [atom(format!("@{}.{}", self.name(cap), self.name(op)))],
                        ),
                        ExpectationKind::Holds(expr) => list("expect", [self.expr(expr)]),
                    });
                }
                if let Some(stop_after) = &example.stop_after {
                    items.push(list(
                        "stop-after",
                        [atom(stop_after.iterations.to_string())],
                    ));
                }
                items
            }
            DeclKind::Measure(measure) => {
                let mut items = vec![atom("measure"), atom(self.name(&measure.name))];
                if measure.public {
                    items.insert(1, atom("pub"));
                }
                if let Some(unit) = &measure.unit {
                    items.push(atom(format!("{{{}}}", self.unit_text(unit))));
                }
                items
            }
            DeclKind::Const(constant) => {
                let mut items = vec![atom("const"), atom(self.name(&constant.name))];
                if constant.public {
                    items.insert(1, atom("pub"));
                }
                items.push(self.ty(&constant.ty));
                items.push(self.expr(&constant.value));
                items
            }
        };
        let docs = decl.docs.iter().map(|comment| {
            let text = self
                .text
                .get(comment.span.start..comment.span.end)
                .unwrap_or("");
            list("doc", [atom(format!("{text:?}"))])
        });
        let docs: Vec<Node> = docs.collect();
        let at = items
            .iter()
            .take_while(|item| matches!(item, Node::Atom(_)))
            .count();
        let tail = items.split_off(at);
        items.extend(docs);
        items.extend(tail);
        Node::List(items)
    }

    fn type_params(&self, params: &[Ident]) -> Node {
        let names: Vec<String> = params.iter().map(|param| self.name(param)).collect();
        atom(format!("[{}]", names.join(", ")))
    }

    fn params(&self, params: &[Param]) -> Node {
        Node::List(
            params
                .iter()
                .map(|param| {
                    let mut items = Vec::new();
                    if let Some(convention) = param.convention {
                        items.push(atom(convention.as_str()));
                    }
                    items.push(atom(self.name(&param.name)));
                    items.push(self.ty(&param.ty));
                    Node::List(items)
                })
                .collect(),
        )
    }

    fn field(&self, field: &Field) -> Node {
        Node::List(vec![atom(self.name(&field.name)), self.ty(&field.ty)])
    }

    fn uses(&self, uses: &UsesClause) -> Node {
        list(
            "uses",
            [
                atom(format!("@{}", self.name(&uses.cap))),
                atom(format!("{{{}}}", self.cap_type(&uses.cap_type))),
            ],
        )
    }

    fn cap_type(&self, cap_type: &CapType) -> String {
        let name = self.name(&cap_type.name);
        if cap_type.ops.is_empty() {
            name
        } else {
            // An operation's label (ADR 0094) and condition (ADR 0092) are shown as their
            // source text.
            let ops: Vec<String> = cap_type
                .ops
                .iter()
                .enumerate()
                .map(|(index, op)| {
                    let mut text = self.name(op);
                    if let Some(Some(label)) = cap_type.gives.get(index) {
                        text.push_str(" gives ");
                        text.push_str(
                            self.text
                                .get(label.span.start..label.span.end)
                                .unwrap_or_default(),
                        );
                    }
                    if let Some(Some(condition)) = cap_type.conditions.get(index) {
                        text.push_str(" where ");
                        text.push_str(
                            self.text
                                .get(condition.span.start..condition.span.end)
                                .unwrap_or_default(),
                        );
                    }
                    text
                })
                .collect();
            format!("{name}[{}]", ops.join(", "))
        }
    }

    /// A type as `{source spelling}`.
    /// The range of a type, a bound that names a module constant shown by its name, edition
    /// 0.5 (ADR 0126 rule 9).
    fn type_range_text(&self, range: &Range) -> String {
        let bound = |value: &SignedInt| match &value.constant {
            Some(constant) => self.name(constant),
            None => signed_text(value),
        };
        let operator = if range.exclusive { "..<" } else { ".." };
        format!("{}{operator}{}", bound(&range.low), bound(&range.high))
    }

    fn ty(&self, ty: &Type) -> Node {
        atom(format!("{{{}}}", self.type_text(ty)))
    }

    fn type_text(&self, ty: &Type) -> String {
        let mut text = String::new();
        if let Some(label) = &ty.label {
            text.push_str(&self.label(label));
            text.push(' ');
        }
        text.push_str(&match &ty.core {
            TypeCore::Named { name, args } => {
                if args.is_empty() {
                    self.name(name)
                } else {
                    let args: Vec<String> = args.iter().map(|arg| self.type_text(arg)).collect();
                    format!("{}[{}]", self.name(name), args.join(", "))
                }
            }
            TypeCore::List(element) => format!("[{}]", self.type_text(element)),
            TypeCore::Decimal { scale, .. } => format!("Decimal[{scale}]"),
            TypeCore::Function {
                params,
                result,
                clauses,
            } => {
                let params: Vec<String> =
                    params.iter().map(|param| self.type_text(param)).collect();
                let mut text = format!("fn({}) -> {}", params.join(", "), self.type_text(result));
                for clause in clauses {
                    match clause {
                        FnTypeClause::Uses(uses) => {
                            text.push_str(&format!(
                                " uses @{}: {}",
                                self.name(&uses.cap),
                                self.cap_type(&uses.cap_type)
                            ));
                        }
                        FnTypeClause::Fails(kind) => {
                            text.push_str(&format!(" fails {}", self.name(kind)));
                        }
                    }
                }
                text
            }
        });
        if let Some(unit) = &ty.unit {
            text.push_str(&format!(" of {}", self.unit_text(unit)));
        }
        if let Some(range) = &ty.range {
            text.push_str(&format!(" in {}", self.type_range_text(range)));
        }
        text
    }

    /// A unit expression as written, fully parenthesized (ADR 0104 rule 2).
    fn unit_text(&self, unit: &UnitExpr) -> String {
        match unit {
            UnitExpr::Name(name) => self.name(name),
            UnitExpr::Factor { digits, .. } => digits.clone(),
            UnitExpr::Mul(a, b) => format!("({} * {})", self.unit_text(a), self.unit_text(b)),
            UnitExpr::Div(a, b) => format!("({} / {})", self.unit_text(a), self.unit_text(b)),
            UnitExpr::Paren(inner, _) => self.unit_text(inner),
        }
    }

    fn label(&self, label: &Label) -> String {
        let level = match label.level {
            Level::Public => "public",
            Level::Internal => "internal",
            Level::Confidential => "confidential",
            Level::Secret => "secret",
            Level::Restricted => "restricted",
        };
        if label.categories.is_empty() {
            level.to_owned()
        } else {
            let categories: Vec<String> = label
                .categories
                .iter()
                .map(|category| self.name(category))
                .collect();
            format!("{level}[{}]", categories.join(", "))
        }
    }

    fn body(&self, body: &Body) -> Node {
        match body {
            Body::Line(expr) => self.expr(expr),
            Body::Block(block) => self.block(block),
        }
    }

    fn block(&self, block: &Block) -> Node {
        list("block", block.stmts.iter().map(|stmt| self.stmt(stmt)))
    }

    fn stmt(&self, stmt: &Stmt) -> Node {
        match stmt {
            Stmt::Let(stmt) => {
                let mut items = vec![atom(if stmt.editable { "let-edit" } else { "let" })];
                items.push(atom(self.name(&stmt.name)));
                if let Some(ty) = &stmt.ty {
                    items.push(self.ty(ty));
                }
                items.push(match &stmt.value {
                    LetValue::Handled(handled) => self.handled(handled),
                    LetValue::BlockExpr(expr) => self.block_expr(expr),
                });
                Node::List(items)
            }
            Stmt::Assign(stmt) => list(
                "assign",
                [
                    self.expr(&stmt.place),
                    match &stmt.value {
                        LetValue::Handled(handled) => self.handled(handled),
                        LetValue::BlockExpr(expr) => self.block_expr(expr),
                    },
                ],
            ),
            Stmt::IfThen(stmt) => list(
                "if-then",
                [self.expr(&stmt.condition), self.block(&stmt.block)],
            ),
            Stmt::For(stmt) => list(
                "for",
                [
                    atom(self.name(&stmt.var)),
                    self.for_source(&stmt.source),
                    self.block(&stmt.block),
                ],
            ),
            Stmt::Together(stmt) => list("together", stmt.body.iter().map(|s| self.stmt(s))),
            Stmt::Handled(handled) => self.handled(handled),
            Stmt::BlockExpr(expr) => self.block_expr(expr),
            Stmt::Stop(stmt) => match &stmt.condition {
                Some(condition) => list("stop-when", [self.expr(condition)]),
                None => atom("stop"),
            },
            Stmt::Skip(_) => atom("skip"),
            Stmt::Ever(stmt) => {
                let mut items = vec![atom("for-ever")];
                if let Some(within) = &stmt.within {
                    items.push(list("within", [self.expr(within)]));
                }
                items.push(self.block(&stmt.block));
                if let Some(on_stop) = &stmt.on_stop {
                    items.push(list("on-stop", [self.block(&on_stop.block)]));
                }
                Node::List(items)
            }
            Stmt::OnStop(on_stop) => list("on-stop", [self.block(&on_stop.block)]),
            Stmt::Until(stmt) => list(
                "for-until",
                [
                    self.expr(&stmt.condition),
                    self.expr(&stmt.measure),
                    self.block(&stmt.block),
                ],
            ),
            Stmt::Tasks(stmt) => {
                let mut items = vec![atom("tasks")];
                if let Some(bound) = &stmt.bound {
                    items.push(list("at-most", [self.expr(bound)]));
                }
                items.push(self.block(&stmt.block));
                Node::List(items)
            }
            Stmt::Start(stmt) => list("start", [self.handled(&stmt.handled)]),
        }
    }

    fn handled(&self, handled: &Handled) -> Node {
        if handled.with.is_empty() && handled.within.is_none() && handled.handlings.is_empty() {
            return self.expr(&handled.expr);
        }
        let mut items = vec![atom("handled"), self.expr(&handled.expr)];
        if !handled.with.is_empty() {
            items.push(list(
                "with",
                handled.with.iter().map(|binding| self.cap_binding(binding)),
            ));
        }
        if let Some(within) = &handled.within {
            items.push(list("within", [self.expr(within)]));
        }
        for handling in &handled.handlings {
            items.push(match &handling.kind {
                HandlingKind::ErrorFail(variant) => list("if-error-fail", [self.variant(variant)]),
                HandlingKind::ErrorPass => list("if-error-pass", []),
                HandlingKind::ErrorUse(expr) => list("if-error-use", [self.expr(expr)]),
                HandlingKind::NoneFail(variant) => list("if-none-fail", [self.variant(variant)]),
                HandlingKind::NoneUse(expr) => list("if-none-use", [self.expr(expr)]),
            });
        }
        Node::List(items)
    }

    fn cap_binding(&self, binding: &CapBinding) -> Node {
        let value = match &binding.value {
            CapBindingValue::Cap(cap) => atom(format!("@{}", self.name(cap))),
            CapBindingValue::Narrowed { target, ops } => atom(format!(
                "@{}[{}]",
                self.name(target),
                ops.iter()
                    .map(|op| self.name(op))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            CapBindingValue::Handler(call) => self.handler_call(call),
        };
        list("=", [atom(format!("@{}", self.name(&binding.cap))), value])
    }

    fn handler_call(&self, call: &HandlerCall) -> Node {
        let mut items = vec![atom("call"), atom(self.name(&call.name))];
        items.extend(call.args.iter().map(|arg| self.arg(arg)));
        Node::List(items)
    }

    fn block_expr(&self, expr: &BlockExpr) -> Node {
        match expr {
            BlockExpr::When(when) => {
                let mut items = vec![atom("when"), self.expr(&when.scrutinee)];
                items.extend(when.arms.iter().map(|arm| self.arm(arm)));
                Node::List(items)
            }
            BlockExpr::If(block) => self.if_block(block),
            BlockExpr::TogetherFor(together) => list(
                "together-for",
                [
                    atom(self.name(&together.var)),
                    self.for_source(&together.source),
                    self.block(&together.block),
                ],
            ),
            BlockExpr::FirstOf(first) => list("first-of", first.body.iter().map(|s| self.stmt(s))),
        }
    }

    fn for_source(&self, source: &ForSource) -> Node {
        match source {
            ForSource::List(list) => self.expr(list),
            ForSource::Range {
                low,
                high,
                exclusive,
            } => list(
                if *exclusive { "range..<" } else { "range.." },
                [self.expr(low), self.expr(high)],
            ),
        }
    }

    fn if_block(&self, block: &IfBlock) -> Node {
        let otherwise = match block.otherwise.as_ref() {
            ElseBranch::Block(otherwise) => self.block(otherwise),
            ElseBranch::If(otherwise) => self.if_block(otherwise),
        };
        list(
            "if",
            [
                self.expr(&block.condition),
                self.block(&block.then_block),
                otherwise,
            ],
        )
    }

    fn arm(&self, arm: &Arm) -> Node {
        let pattern = match &arm.pattern.kind {
            PatternKind::Variant { name, binds } => {
                let mut items = vec![atom("is"), atom(self.name(name))];
                items.extend(binds.iter().map(|bind| match &bind.local {
                    Some(local) => atom(format!("{}:{}", self.name(&bind.field), self.name(local))),
                    None => atom(self.name(&bind.field)),
                }));
                Node::List(items)
            }
            PatternKind::Literal(literal) => list("is", [self.literal(literal)]),
            PatternKind::Range(range) => list("in", [atom(range_text(range))]),
            PatternKind::Else => atom("else"),
        };
        list("arm", [pattern, self.body(&arm.body)])
    }

    fn variant(&self, variant: &VariantExpr) -> Node {
        if variant.args.is_empty() {
            return atom(self.name(&variant.name));
        }
        let mut items = vec![atom(self.name(&variant.name))];
        items.extend(variant.args.iter().map(|arg| self.arg(arg)));
        Node::List(items)
    }

    fn arg(&self, arg: &Arg) -> Node {
        let value = match arg.marker {
            Some(marker) => list(marker.as_str(), [self.expr(&arg.value)]),
            None => self.expr(&arg.value),
        };
        match &arg.label {
            Some(label) => Node::List(vec![atom(format!("{}:", self.name(label))), value]),
            None => value,
        }
    }

    fn text(&self, text: &TextLit) -> Node {
        match text.parts.as_slice() {
            [] => atom("\"\""),
            [TextPart::Text(chunk)] => atom(format!("{chunk:?}")),
            parts => list(
                "text",
                parts.iter().map(|part| match part {
                    TextPart::Text(chunk) => atom(format!("{chunk:?}")),
                    TextPart::Interpolation(expr) => self.expr(expr),
                }),
            ),
        }
    }

    fn literal(&self, literal: &Literal) -> Node {
        match literal {
            Literal::Int(digits) | Literal::Float(digits) => atom(digits.clone()),
            Literal::Text(text) => self.text(text),
            Literal::Bool(value) => atom(if *value { "true" } else { "false" }),
            Literal::Unit => atom("unit"),
        }
    }

    fn expr(&self, expr: &Expr) -> Node {
        match &expr.kind {
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } => list(
                "if",
                [self.expr(condition), self.expr(then), self.expr(otherwise)],
            ),
            ExprKind::Lambda { params, body } => {
                let params = params.iter().map(|param| match &param.ty {
                    Some(ty) => Node::List(vec![atom(self.name(&param.name)), self.ty(ty)]),
                    None => atom(self.name(&param.name)),
                });
                list("fn", [Node::List(params.collect()), self.expr(body)])
            }
            ExprKind::Checked {
                value,
                predicate,
                failure,
            } => list(
                "checked",
                [
                    self.expr(value),
                    self.expr(predicate),
                    list("fail", [self.variant(failure)]),
                ],
            ),
            ExprKind::Attempt {
                operand,
                within: None,
            } => list("attempt", [self.expr(operand)]),
            ExprKind::Attempt {
                operand,
                within: Some(within),
            } => list(
                "attempt",
                [self.expr(operand), list("within", [self.expr(within)])],
            ),
            ExprKind::Fail(variant) => list("fail", [self.variant(variant)]),
            ExprKind::Binary { op, left, right } => {
                list(binary_text(*op), [self.expr(left), self.expr(right)])
            }
            ExprKind::Unary { op, operand } => list(
                match op {
                    UnaryOp::Neg => "neg",
                    UnaryOp::Not => "not",
                },
                [self.expr(operand)],
            ),
            ExprKind::Is { value, target } => {
                let target = match target {
                    IsTarget::Variant(variant) => atom(self.name(variant)),
                    IsTarget::Literal(literal) => self.literal(literal),
                };
                list("is", [self.expr(value), target])
            }
            ExprKind::In { value, range } => {
                list("in", [self.expr(value), atom(range_text(range))])
            }
            ExprKind::Field { target, name } => {
                list(".", [self.expr(target), atom(self.name(name))])
            }
            ExprKind::MethodCall {
                receiver,
                target,
                name,
                args,
            } => {
                let target = match receiver {
                    Some(marker) => list(marker.as_str(), [self.expr(target)]),
                    None => self.expr(target),
                };
                let mut items = vec![atom("method"), target, atom(self.name(name))];
                items.extend(args.iter().map(|arg| self.arg(arg)));
                Node::List(items)
            }
            ExprKind::Call { callee, args } => {
                let mut items = vec![atom("call"), self.expr(callee)];
                items.extend(args.iter().map(|arg| self.arg(arg)));
                Node::List(items)
            }
            ExprKind::Name(name) | ExprKind::UpperName(name) => atom(self.name(name)),
            ExprKind::CapOp { cap, op, args } => {
                let mut items = vec![atom(format!("@{}.{}", self.name(cap), self.name(op)))];
                items.extend(args.iter().map(|arg| self.arg(arg)));
                Node::List(items)
            }
            ExprKind::CapRef(cap) => atom(format!("@{}", self.name(cap))),
            ExprKind::Paren(inner) => list("paren", [self.expr(inner)]),
            ExprKind::List(items) => list("list", items.iter().map(|item| self.expr(item))),
            ExprKind::Old(inner) => list("old", [self.expr(inner)]),
            ExprKind::Result => atom("result"),
            ExprKind::It => atom("it"),
            ExprKind::Error => atom("error"),
            ExprKind::Literal(literal) => self.literal(literal),
        }
    }
}

fn binary_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Implies => "implies",
        BinaryOp::Or => "or",
        BinaryOp::And => "and",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "mod",
    }
}

fn signed_text(value: &SignedInt) -> String {
    if value.negative {
        format!("-{}", value.digits)
    } else {
        value.digits.clone()
    }
}

fn range_text(range: &Range) -> String {
    let operator = if range.exclusive { "..<" } else { ".." };
    format!(
        "{}{operator}{}",
        signed_text(&range.low),
        signed_text(&range.high)
    )
}
