//! The rewrites that need the syntax tree (ADR 0020, rules 4 to 7): one-line bodies, the
//! leading `|` of sums, and constructor arguments and import names in canonical order.

use std::collections::{BTreeMap, BTreeSet};

use una_syntax::ast::{
    Arg, Arm, Block, BlockExpr, Body, CapBindingValue, ClauseKind, DeclKind, ElseBranch,
    ExpectationKind, Expr, ExprKind, File, ForSource, Handled, HandlingKind, IsTarget, KindBody,
    LetValue, Literal, OpClauseKind, PatternKind, RequirementSource, Stmt, TextLit, TextPart,
    VariantExpr,
};
use una_syntax::{Comment, Interner, Span, Symbol, Token, TokenKind};

use crate::lines::Line;

/// Tokens printed in another order: parts separated by commas.
#[derive(Debug)]
pub(crate) struct Region {
    /// The last token of the region.
    pub(crate) last: usize,
    /// The parts, as inclusive token ranges, in canonical order.
    pub(crate) parts: Vec<(usize, usize)>,
    /// The commas between the parts, in source order.
    pub(crate) separators: Vec<usize>,
}

/// Everything the layout changes beyond spacing, indentation, blank lines and comments.
#[derive(Debug, Default)]
pub(crate) struct Rewrites {
    /// Runs of logical lines printed as one line: the first line of each run, to its last.
    pub(crate) merges: BTreeMap<usize, usize>,
    /// Tokens left out: the leading `|` of sums with two or more variants.
    pub(crate) dropped: BTreeSet<usize>,
    /// Regions printed in canonical order, by their first token.
    pub(crate) regions: BTreeMap<usize, Region>,
}

/// The constructors that the modules of a package declare, so that a file can order the
/// arguments of the constructors it imports: by module path, the fields of each record kind
/// and variant declared once in its module, and the variants of each sum kind (ADR 0020 rule 6).
#[derive(Clone, Debug, Default)]
pub struct Constructors {
    modules: BTreeMap<Vec<Symbol>, Declared>,
}

/// The constructors of one module.
#[derive(Clone, Debug, Default)]
struct Declared {
    /// The fields of each record kind and variant, by its name.
    fields: BTreeMap<Symbol, Vec<Symbol>>,
    /// The variants of each sum kind, by the kind's name.
    variants: BTreeMap<Symbol, Vec<Symbol>>,
}

impl Constructors {
    /// The constructors of `files`, the modules of one package that parsed without errors,
    /// named with one interner. A module path that two files claim is left out.
    pub fn new<'f>(files: impl IntoIterator<Item = &'f File>) -> Constructors {
        let mut modules: BTreeMap<Vec<Symbol>, Declared> = BTreeMap::new();
        let mut claimed: BTreeMap<Vec<Symbol>, usize> = BTreeMap::new();
        for file in files {
            let Some(header) = &file.module else {
                continue;
            };
            let path: Vec<Symbol> = header.path.segments.iter().map(|s| s.name).collect();
            *claimed.entry(path.clone()).or_default() += 1;
            let mut count: BTreeMap<Symbol, usize> = BTreeMap::new();
            let mut declared = Declared::default();
            for decl in &file.decls {
                let DeclKind::Kind(kind) = &decl.kind else {
                    continue;
                };
                *count.entry(kind.name.name).or_default() += 1;
                match &kind.body {
                    KindBody::Record(record) => {
                        declared.fields.insert(
                            kind.name.name,
                            record.iter().map(|field| field.name.name).collect(),
                        );
                    }
                    KindBody::Sum(variants) => {
                        declared.variants.insert(
                            kind.name.name,
                            variants.iter().map(|variant| variant.name.name).collect(),
                        );
                        for variant in variants {
                            *count.entry(variant.name.name).or_default() += 1;
                            declared.fields.insert(
                                variant.name.name,
                                variant.fields.iter().map(|field| field.name.name).collect(),
                            );
                        }
                    }
                    KindBody::Alias { .. } => {}
                }
            }
            declared
                .fields
                .retain(|name, _| count.get(name) == Some(&1));
            modules.insert(path, declared);
        }
        modules.retain(|path, _| claimed.get(path) == Some(&1));
        Constructors { modules }
    }
}

/// The rewrites of `file`, whose tokens, comments and logical lines are given, with the
/// constructors of its package.
pub(crate) fn collect(
    file: &File,
    tokens: &[Token],
    comments: &[Comment],
    lines: &[Line],
    interner: &Interner,
    package: &Constructors,
) -> Rewrites {
    let mut collector = Collector {
        index: TokenIndex::new(tokens),
        comments,
        lines,
        constructors: constructors(file, package),
        rewrites: Rewrites::default(),
    };
    for import in &file.imports {
        let mut names: Vec<(&str, Span)> = import
            .names
            .iter()
            .map(|name| (interner.resolve(name.name), name.span))
            .collect();
        let spans: Vec<Span> = names.iter().map(|(_, span)| *span).collect();
        // The names of an import are sorted bytewise.
        names.sort_by(|left, right| left.0.cmp(right.0));
        let sorted: Vec<Span> = names.iter().map(|(_, span)| *span).collect();
        collector.region(&spans, &sorted);
    }
    for decl in &file.decls {
        collector.decl(&decl.kind, decl.span);
    }
    collector.rewrites
}

/// The fields of the constructors `file` can name: the record kinds and variants it declares,
/// those it imports, and the variants of the sum kinds it imports, when each name comes from
/// exactly one of them (ADR 0020 rule 6).
fn constructors(file: &File, package: &Constructors) -> BTreeMap<Symbol, Vec<Symbol>> {
    let mut declared: BTreeMap<Symbol, usize> = BTreeMap::new();
    let mut fields: BTreeMap<Symbol, Vec<Symbol>> = BTreeMap::new();
    for import in &file.imports {
        let path: Vec<Symbol> = import.module.segments.iter().map(|s| s.name).collect();
        let module = package.modules.get(&path);
        for name in &import.names {
            *declared.entry(name.name).or_default() += 1;
            let Some(module) = module else {
                continue;
            };
            if let Some(known) = module.fields.get(&name.name) {
                fields.insert(name.name, known.clone());
            }
            for variant in module.variants.get(&name.name).into_iter().flatten() {
                *declared.entry(*variant).or_default() += 1;
                if let Some(known) = module.fields.get(variant) {
                    fields.insert(*variant, known.clone());
                }
            }
        }
    }
    for decl in &file.decls {
        let DeclKind::Kind(kind) = &decl.kind else {
            continue;
        };
        *declared.entry(kind.name.name).or_default() += 1;
        match &kind.body {
            KindBody::Record(record) => {
                fields.insert(
                    kind.name.name,
                    record.iter().map(|field| field.name.name).collect(),
                );
            }
            KindBody::Sum(variants) => {
                for variant in variants {
                    *declared.entry(variant.name.name).or_default() += 1;
                    fields.insert(
                        variant.name.name,
                        variant.fields.iter().map(|field| field.name.name).collect(),
                    );
                }
            }
            KindBody::Alias { .. } => {}
        }
    }
    fields.retain(|name, names| declared.get(name) == Some(&1) && names.len() >= 2);
    fields
}

/// Non-layout tokens by offset.
struct TokenIndex<'a> {
    tokens: &'a [Token],
    order: Vec<usize>,
}

impl<'a> TokenIndex<'a> {
    fn new(tokens: &'a [Token]) -> TokenIndex<'a> {
        let order = tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| {
                !matches!(
                    token.kind,
                    TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
                )
            })
            .map(|(index, _)| index)
            .collect();
        TokenIndex { tokens, order }
    }

    fn kind(&self, index: usize) -> Option<&TokenKind> {
        self.tokens.get(index).map(|token| &token.kind)
    }

    /// The token that starts at `offset`.
    fn starting_at(&self, offset: usize) -> Option<usize> {
        let position = self.order.partition_point(|&index| {
            self.tokens
                .get(index)
                .is_some_and(|token| token.span.start < offset)
        });
        let index = *self.order.get(position)?;
        (self.tokens.get(index)?.span.start == offset).then_some(index)
    }

    /// The token that ends at `offset`.
    fn ending_at(&self, offset: usize) -> Option<usize> {
        let position = self.order.partition_point(|&index| {
            self.tokens
                .get(index)
                .is_some_and(|token| token.span.end < offset)
        });
        let index = *self.order.get(position)?;
        (self.tokens.get(index)?.span.end == offset).then_some(index)
    }
}

struct Collector<'a> {
    index: TokenIndex<'a>,
    comments: &'a [Comment],
    lines: &'a [Line],
    constructors: BTreeMap<Symbol, Vec<Symbol>>,
    rewrites: Rewrites,
}

/// Whether a block is one plain line: a statement without `with` or handling clauses whose
/// expression is a line expression (ADR 0020 rule 4).
fn plain_line(block: &Block) -> Option<&Expr> {
    match block.stmts.as_slice() {
        [Stmt::Handled(handled)]
            if handled.with.is_empty()
                && handled.within.is_none()
                && handled.handlings.is_empty()
                && !marked(&handled.expr) =>
        {
            Some(&handled.expr)
        }
        _ => None,
    }
}

/// Whether `expr` is a call with an `edit` or `take` marker, which is legal only as a
/// statement, so its block form stays (`spec/15`, `spec/17 §3` rule 5).
fn marked(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Call { args, .. } => args.iter().any(|arg| arg.marker.is_some()),
        ExprKind::MethodCall { receiver, args, .. } => {
            receiver.is_some() || args.iter().any(|arg| arg.marker.is_some())
        }
        _ => false,
    }
}

impl Collector<'_> {
    /// Whether a comment starts in `start..end`. The formatter never moves a comment to another
    /// line, so no rewrite joins, merges or reorders across one.
    fn comment_within(&self, start: usize, end: usize) -> bool {
        let position = self
            .comments
            .partition_point(|comment| comment.span.start < start);
        self.comments
            .get(position)
            .is_some_and(|comment| comment.span.start < end)
    }

    /// Whether the logical line containing the token at `offset` has a comment inside it.
    fn line_has_comment(&self, offset: usize) -> bool {
        let Some(line) = self.line_at(offset).and_then(|line| self.lines.get(line)) else {
            return true;
        };
        let (Some(first), Some(last)) = (
            self.index.tokens.get(line.first),
            self.index.tokens.get(line.last),
        ) else {
            return true;
        };
        self.comment_within(first.span.start, last.span.end)
    }

    /// The logical line containing the token that starts at `offset`.
    fn line_at(&self, offset: usize) -> Option<usize> {
        let token = self.index.starting_at(offset)?;
        let position = self.lines.partition_point(|line| line.first <= token);
        let line = position.checked_sub(1)?;
        (self.lines.get(line)?.last >= token).then_some(line)
    }

    /// Merges the line starting at `start` with the body line that follows it, or with the `=`
    /// line and the body line when `equals_line` (the long form of a function).
    fn merge(&mut self, start: usize, body: &Expr, equals_line: bool) {
        let Some(first) = self.line_at(start) else {
            return;
        };
        let body_line = first + if equals_line { 2 } else { 1 };
        if equals_line {
            let Some(equals) = self.lines.get(first + 1) else {
                return;
            };
            if equals.first != equals.last
                || !matches!(self.index.kind(equals.first), Some(TokenKind::Eq))
            {
                return;
            }
        }
        let (Some(line), Some(body_first), Some(body_last)) = (
            self.lines.get(body_line),
            self.index.starting_at(body.span.start),
            self.index.ending_at(body.span.end),
        ) else {
            return;
        };
        let (Some(head), Some(tail)) = (
            self.lines
                .get(first)
                .and_then(|head| self.index.tokens.get(head.first)),
            self.index.tokens.get(body_last),
        ) else {
            return;
        };
        if line.first == body_first
            && line.last == body_last
            && !self.comment_within(head.span.start, tail.span.end)
        {
            self.rewrites.merges.insert(first, body_line);
        }
    }

    /// Records a region whose parts, spanned by `source` in source order, are printed in the
    /// order of `canonical`.
    fn region(&mut self, source: &[Span], canonical: &[Span]) {
        if source.len() < 2
            || source == canonical
            || source
                .first()
                .is_none_or(|first| self.line_has_comment(first.start))
        {
            return;
        }
        let range = |span: &Span| {
            Some((
                self.index.starting_at(span.start)?,
                self.index.ending_at(span.end)?,
            ))
        };
        let Some(parts) = canonical.iter().map(range).collect::<Option<Vec<_>>>() else {
            return;
        };
        let Some(ranges) = source.iter().map(range).collect::<Option<Vec<_>>>() else {
            return;
        };
        let separators: Vec<usize> = ranges
            .iter()
            .take(ranges.len() - 1)
            .map(|(_, last)| last + 1)
            .collect();
        let commas = separators
            .iter()
            .all(|&comma| matches!(self.index.kind(comma), Some(TokenKind::Comma)));
        let (Some(&(first, _)), Some(&(_, last))) = (ranges.first(), ranges.last()) else {
            return;
        };
        if commas {
            self.rewrites.regions.insert(
                first,
                Region {
                    last,
                    parts,
                    separators,
                },
            );
        }
    }

    /// Orders the arguments of a constructor by the declaration of its fields when every one
    /// is labeled or punned (`spec/04 §3.4`, ADR 0020 rule 6).
    fn constructor(&mut self, name: Symbol, args: &[Arg]) {
        let Some(fields) = self.constructors.get(&name) else {
            return;
        };
        let mut keyed = Vec::new();
        for arg in args {
            let field = match (&arg.label, &arg.value.kind) {
                (Some(label), _) => label.name,
                (None, ExprKind::Name(pun)) => pun.name,
                _ => return,
            };
            let Some(position) = fields.iter().position(|known| *known == field) else {
                return;
            };
            if keyed.iter().any(|(known, _)| *known == position) {
                return;
            }
            keyed.push((position, arg.span));
        }
        let source: Vec<Span> = keyed.iter().map(|(_, span)| *span).collect();
        keyed.sort_by_key(|(position, _)| *position);
        let canonical: Vec<Span> = keyed.iter().map(|(_, span)| *span).collect();
        self.region(&source, &canonical);
    }

    fn decl(&mut self, kind: &DeclKind, span: Span) {
        match kind {
            DeclKind::Kind(kind) => match &kind.body {
                KindBody::Alias { refinement, .. } => {
                    if let Some(refinement) = refinement {
                        self.expr(refinement);
                    }
                }
                KindBody::Sum(variants) if variants.len() >= 2 => self.drop_leading_bar(span),
                KindBody::Sum(_) | KindBody::Record(_) => {}
            },
            DeclKind::Predicate(predicate) => self.body(&predicate.body, span.start),
            DeclKind::Function(function) => {
                for clause in &function.clauses {
                    if let ClauseKind::Requires(expr) | ClauseKind::Ensures(expr) = &clause.kind {
                        self.expr(expr);
                    }
                }
                if let Body::Block(block) = &function.body {
                    if let Some(expr) = plain_line(block).filter(|_| function.clauses.is_empty()) {
                        self.merge(span.start, expr, true);
                    }
                    self.block(block);
                } else if let Body::Line(expr) = &function.body {
                    self.expr(expr);
                }
            }
            DeclKind::Capability(capability) => {
                for op in &capability.ops {
                    for clause in &op.clauses {
                        if let OpClauseKind::Requires(expr) | OpClauseKind::Ensures(expr) =
                            &clause.kind
                        {
                            self.expr(expr);
                        }
                    }
                }
            }
            DeclKind::Handler(handler) => {
                for op in &handler.ops {
                    self.body(&op.body, op.span.start);
                }
            }
            DeclKind::Policy(policy) => self.text(&policy.purpose),
            DeclKind::Requirement(requirement) => {
                self.text(&requirement.text);
                if let RequirementSource::Policy(text)
                | RequirementSource::Regulation(text)
                | RequirementSource::Generated(text) = &requirement.source
                {
                    self.text(text);
                }
            }
            DeclKind::Example(example) => {
                self.text(&example.name);
                for given in &example.givens {
                    self.args(&given.handler.args);
                }
                self.expr(&example.action);
                for expectation in &example.expectations {
                    if let ExpectationKind::Holds(expr) = &expectation.kind {
                        self.expr(expr);
                    }
                }
            }
            DeclKind::Measure(_) => {}
            DeclKind::Const(constant) => self.expr(&constant.value),
        }
    }

    /// Drops the `|` right after the `=` of the sum declared at `span` (ADR 0020 rule 5).
    fn drop_leading_bar(&mut self, span: Span) {
        let Some(line) = self
            .line_at(span.start)
            .and_then(|line| self.lines.get(line))
        else {
            return;
        };
        let equals = (line.first..=line.last)
            .find(|&index| matches!(self.index.kind(index), Some(TokenKind::Eq)));
        if let Some(bar) = equals.map(|index| index + 1)
            && matches!(self.index.kind(bar), Some(TokenKind::Bar))
        {
            self.rewrites.dropped.insert(bar);
        }
    }

    /// A body after `=` or `->` on the line starting at `start`: one plain line in a block is
    /// merged with that line (ADR 0020 rule 4).
    fn body(&mut self, body: &Body, start: usize) {
        match body {
            Body::Line(expr) => self.expr(expr),
            Body::Block(block) => {
                if let Some(expr) = plain_line(block) {
                    self.merge(start, expr, false);
                }
                self.block(block);
            }
        }
    }

    fn block(&mut self, block: &Block) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(stmt) => match &stmt.value {
                LetValue::Handled(handled) => self.handled(handled),
                LetValue::BlockExpr(expr) => self.block_expr(expr),
            },
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
                self.block(&stmt.block);
            }
            Stmt::Together(stmt) => {
                for branch in &stmt.body {
                    self.stmt(branch);
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
                if let Some(within) = &stmt.within {
                    self.expr(within);
                }
                self.block(&stmt.block);
                if let Some(on_stop) = &stmt.on_stop {
                    self.block(&on_stop.block);
                }
            }
            Stmt::OnStop(on_stop) => self.block(&on_stop.block),
            Stmt::Tasks(stmt) => {
                if let Some(bound) = &stmt.bound {
                    self.expr(bound);
                }
                self.block(&stmt.block);
            }
            Stmt::Start(stmt) => self.handled(&stmt.handled),
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
                self.args(&call.args);
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

    fn block_expr(&mut self, expr: &BlockExpr) {
        match expr {
            BlockExpr::When(when) => {
                self.expr(&when.scrutinee);
                for arm in &when.arms {
                    self.arm(arm);
                }
            }
            BlockExpr::If(block) => {
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
            BlockExpr::TogetherFor(together) => {
                self.for_source(&together.source);
                self.block(&together.block);
            }
            BlockExpr::FirstOf(first) => {
                for branch in &first.body {
                    self.stmt(branch);
                }
            }
        }
    }

    fn arm(&mut self, arm: &Arm) {
        match &arm.pattern.kind {
            PatternKind::Literal(Literal::Text(text)) => self.text(text),
            PatternKind::Variant { .. }
            | PatternKind::Literal(_)
            | PatternKind::Range(_)
            | PatternKind::Else => {}
        }
        self.body(&arm.body, arm.span.start);
    }

    fn variant(&mut self, variant: &VariantExpr) {
        self.args(&variant.args);
        self.constructor(variant.name.name, &variant.args);
    }

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
            ExprKind::Lambda { body, .. } => self.expr(body),
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
                if let IsTarget::Literal(Literal::Text(text)) = target {
                    self.text(text);
                }
            }
            ExprKind::In { value, .. } | ExprKind::Field { target: value, .. } => {
                self.expr(value);
            }
            ExprKind::MethodCall { target, args, .. } => {
                self.expr(target);
                self.args(args);
            }
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                self.args(args);
                if let ExprKind::UpperName(name) = &callee.kind {
                    self.constructor(name.name, args);
                }
            }
            ExprKind::CapOp { args, .. } => self.args(args),
            ExprKind::List(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            ExprKind::Literal(Literal::Text(text)) => self.text(text),
            ExprKind::Name(_)
            | ExprKind::UpperName(_)
            | ExprKind::CapRef(_)
            | ExprKind::Result
            | ExprKind::It
            | ExprKind::Error
            | ExprKind::Literal(_) => {}
        }
    }
}
