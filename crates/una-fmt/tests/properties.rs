//! Formatter properties over every `.una` file of the repository (`compiler/testing.md §1`,
//! M2 acceptance): idempotence, round trip modulo spans and the formatter's own rewrites,
//! comments preserved, and canonical examples.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use una_syntax::ast::{
    Arg, Block, BlockExpr, Body, CapBindingValue, ClauseKind, DeclKind, ElseBranch,
    ExpectationKind, Expr, ExprKind, File, ForSource, HandlingKind, IfBlock, IsTarget, KindBody,
    LetValue, Literal, OpClauseKind, PatternKind, RequirementSource, Stmt, TextLit, TextPart,
    VariantExpr,
};
use una_syntax::{Interner, KeywordSet, SourceDb, dump, lex_in, parse};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.una` file of the repository outside `target/` and hidden directories, sorted.
fn una_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if path.is_dir() {
                walk(&path, files);
            } else if name.ends_with(".una") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&repository(), &mut files);
    files.sort();
    files
}

/// A parsed text: its tree dump after `normalize`, its comments, and its canonical form; `None`
/// when it has syntax errors.
struct Parsed {
    tree: String,
    comments: Vec<String>,
    canonical: String,
}

/// The keywords of the edition of the package that holds `path`, by its nearest `una.toml`
/// (`spec/13 §7`); a single file is edition 0.1.
fn keywords_for(path: &Path) -> KeywordSet {
    // The search stops at the repository root: no manifest above the checkout gives an edition
    // to the repository's files (#1560).
    let repository = repository();
    for dir in path
        .ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(&repository))
    {
        // `ita.una` exists from edition 0.4 only (ADR 0122), so it names that edition or a later
        // one for the wrapping formatter (#1175); it comes first, as the package reads its
        // manifests.
        if let Ok(text) = std::fs::read_to_string(dir.join("ita.una")) {
            return if text.lines().any(|line| line.trim() == "edition 0.5") {
                KeywordSet::V0_5
            } else {
                KeywordSet::V0_4
            };
        }
        let manifest = dir.join("una.toml");
        if manifest.is_file() {
            let text = std::fs::read_to_string(manifest).unwrap_or_default();
            return if text.contains("edition = \"0.3\"") {
                KeywordSet::V0_3
            } else if text.contains("edition = \"0.2\"") {
                KeywordSet::V0_2
            } else {
                KeywordSet::V0_1
            };
        }
    }
    KeywordSet::V0_1
}

fn parse_text(bytes: &[u8], keywords: KeywordSet) -> Option<Parsed> {
    let mut db = SourceDb::new();
    let id = db.add("t.una", bytes);
    let mut interner = Interner::new();
    let lexed = lex_in(db.file(id), id, &mut interner, keywords);
    let parsed = parse(&lexed, id, &interner);
    if !lexed.errors.is_empty() || !parsed.errors.is_empty() {
        return None;
    }
    let text = db.file(id).text();
    let canonical = una_fmt::format(db.file(id), &lexed, &parsed.file, &interner);
    let mut file = parsed.file;
    normalize(&mut file, &interner);
    let mut comments: Vec<String> = lexed
        .comments
        .iter()
        .map(|comment| {
            text[comment.span.start..comment.span.end]
                .trim_end()
                .to_owned()
        })
        .collect();
    comments.sort();
    Some(Parsed {
        tree: dump(&file, &interner, text),
        comments,
        canonical,
    })
}

#[test]
fn every_file_is_formatted_idempotently_and_keeps_its_tree_and_comments() {
    let mut formatted = 0;
    let mut skipped = 0;
    for path in una_files() {
        let name = path
            .strip_prefix(repository())
            .unwrap()
            .display()
            .to_string();
        let keywords = keywords_for(&path);
        let Some(original) = parse_text(&std::fs::read(&path).unwrap(), keywords) else {
            skipped += 1;
            continue;
        };
        let again = parse_text(original.canonical.as_bytes(), keywords)
            .unwrap_or_else(|| panic!("{name}: the canonical form does not parse"));
        assert_eq!(
            again.canonical, original.canonical,
            "{name}: not idempotent"
        );
        assert_eq!(again.tree, original.tree, "{name}: the tree changed");
        assert_eq!(
            again.comments, original.comments,
            "{name}: comments changed"
        );
        if name.starts_with("examples/") {
            let text = std::fs::read_to_string(&path).unwrap();
            assert_eq!(original.canonical, text, "{name}: not canonical");
        }
        formatted += 1;
    }
    assert!(
        formatted >= 10 && skipped >= 10,
        "{formatted} formatted, {skipped} skipped"
    );
}

// ----- normalization: the rewrites of ADR 0020 rules 4, 6 and 7, applied to both trees, so
// the comparison holds whether or not the formatter applied them -----

fn normalize(file: &mut File, interner: &Interner) {
    let name = |symbol| interner.resolve(symbol).to_owned();
    for import in &mut file.imports {
        import.names.sort_by_key(|ident| name(ident.name));
    }
    file.imports.sort_by_key(|import| {
        import
            .module
            .segments
            .iter()
            .map(|segment| name(segment.name))
            .collect::<Vec<_>>()
            .join(".")
    });
    let mut normalizer = Normalizer { interner };
    for decl in &mut file.decls {
        normalizer.decl(&mut decl.kind);
    }
}

struct Normalizer<'a> {
    interner: &'a Interner,
}

impl Normalizer<'_> {
    fn decl(&mut self, kind: &mut DeclKind) {
        match kind {
            DeclKind::Kind(kind) => {
                if let KindBody::Alias {
                    refinement: Some(refinement),
                    ..
                } = &mut kind.body
                {
                    self.expr(refinement);
                }
            }
            DeclKind::Predicate(predicate) => self.body(&mut predicate.body),
            DeclKind::Function(function) => {
                for clause in &mut function.clauses {
                    if let ClauseKind::Requires(expr) | ClauseKind::Ensures(expr) = &mut clause.kind
                    {
                        self.expr(expr);
                    }
                }
                self.body(&mut function.body);
            }
            DeclKind::Capability(capability) => {
                for op in &mut capability.ops {
                    for clause in &mut op.clauses {
                        if let OpClauseKind::Requires(expr) | OpClauseKind::Ensures(expr) =
                            &mut clause.kind
                        {
                            self.expr(expr);
                        }
                    }
                }
            }
            DeclKind::Handler(handler) => {
                for op in &mut handler.ops {
                    self.body(&mut op.body);
                }
            }
            DeclKind::Const(constant) => self.expr(&mut constant.value),
            DeclKind::Policy(policy) => self.text(&mut policy.purpose),
            DeclKind::Requirement(requirement) => {
                self.text(&mut requirement.text);
                if let RequirementSource::Policy(text)
                | RequirementSource::Regulation(text)
                | RequirementSource::Generated(text) = &mut requirement.source
                {
                    self.text(text);
                }
            }
            DeclKind::Example(example) => {
                self.text(&mut example.name);
                for given in &mut example.givens {
                    self.args(&mut given.handler.args);
                }
                self.expr(&mut example.action);
                for expectation in &mut example.expectations {
                    if let ExpectationKind::Holds(expr) = &mut expectation.kind {
                        self.expr(expr);
                    }
                }
            }
            DeclKind::Measure(_) => {}
        }
    }

    fn body(&mut self, body: &mut Body) {
        if let Body::Block(block) = body
            && let [Stmt::Handled(handled)] = block.stmts.as_slice()
            && handled.with.is_empty()
            && handled.within.is_none()
            && handled.handlings.is_empty()
        {
            *body = Body::Line(handled.expr.clone());
        }
        match body {
            Body::Line(expr) => self.expr(expr),
            Body::Block(block) => self.block(block),
        }
    }

    fn block(&mut self, block: &mut Block) {
        for stmt in &mut block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Let(stmt) => match &mut stmt.value {
                LetValue::Handled(handled) => {
                    self.expr(&mut handled.expr);
                    self.handlings(handled);
                }
                LetValue::BlockExpr(expr) => self.block_expr(expr),
            },
            Stmt::Assign(stmt) => {
                self.expr(&mut stmt.place);
                match &mut stmt.value {
                    LetValue::Handled(handled) => {
                        self.expr(&mut handled.expr);
                        self.handlings(handled);
                    }
                    LetValue::BlockExpr(expr) => self.block_expr(expr),
                }
            }
            Stmt::IfThen(stmt) => {
                self.expr(&mut stmt.condition);
                self.block(&mut stmt.block);
            }
            Stmt::For(stmt) => {
                self.source(&mut stmt.source);
                self.block(&mut stmt.block);
            }
            Stmt::Together(stmt) => {
                for branch in &mut stmt.body {
                    self.stmt(branch);
                }
            }
            Stmt::Handled(handled) => {
                self.expr(&mut handled.expr);
                self.handlings(handled);
            }
            Stmt::BlockExpr(expr) => self.block_expr(expr),
            Stmt::Stop(stmt) => {
                if let Some(condition) = &mut stmt.condition {
                    self.expr(condition);
                }
            }
            Stmt::Skip(_) => {}
            Stmt::Until(stmt) => {
                self.expr(&mut stmt.condition);
                self.expr(&mut stmt.measure);
                self.block(&mut stmt.block);
            }
            Stmt::Ever(stmt) => {
                if let Some(within) = &mut stmt.within {
                    self.expr(within);
                }
                self.block(&mut stmt.block);
                if let Some(on_stop) = &mut stmt.on_stop {
                    self.block(&mut on_stop.block);
                }
            }
            Stmt::OnStop(on_stop) => self.block(&mut on_stop.block),
            Stmt::Tasks(stmt) => {
                if let Some(bound) = &mut stmt.bound {
                    self.expr(bound);
                }
                self.block(&mut stmt.block);
            }
            Stmt::Start(stmt) => {
                self.expr(&mut stmt.handled.expr);
                self.handlings(&mut stmt.handled);
            }
        }
    }

    fn source(&mut self, source: &mut ForSource) {
        match source {
            ForSource::List(list) => self.expr(list),
            ForSource::Range { low, high, .. } => {
                self.expr(low);
                self.expr(high);
            }
        }
    }

    fn handlings(&mut self, handled: &mut una_syntax::ast::Handled) {
        for binding in &mut handled.with {
            if let CapBindingValue::Handler(call) = &mut binding.value {
                self.args(&mut call.args);
            }
        }
        if let Some(within) = &mut handled.within {
            self.expr(within);
        }
        for handling in &mut handled.handlings {
            match &mut handling.kind {
                HandlingKind::ErrorFail(variant) | HandlingKind::NoneFail(variant) => {
                    self.variant(variant);
                }
                HandlingKind::ErrorUse(expr) | HandlingKind::NoneUse(expr) => self.expr(expr),
                HandlingKind::ErrorPass => {}
            }
        }
    }

    fn block_expr(&mut self, expr: &mut BlockExpr) {
        match expr {
            BlockExpr::When(when) => {
                self.expr(&mut when.scrutinee);
                for arm in &mut when.arms {
                    if let PatternKind::Literal(Literal::Text(text)) = &mut arm.pattern.kind {
                        self.text(text);
                    }
                    self.body(&mut arm.body);
                }
            }
            BlockExpr::If(block) => self.if_block(block),
            BlockExpr::TogetherFor(together) => {
                self.source(&mut together.source);
                self.block(&mut together.block);
            }
            BlockExpr::FirstOf(first) => {
                for branch in &mut first.body {
                    self.stmt(branch);
                }
            }
        }
    }

    fn if_block(&mut self, block: &mut IfBlock) {
        self.expr(&mut block.condition);
        self.block(&mut block.then_block);
        match block.otherwise.as_mut() {
            ElseBranch::Block(otherwise) => self.block(otherwise),
            ElseBranch::If(next) => self.if_block(next),
        }
    }

    fn variant(&mut self, variant: &mut VariantExpr) {
        self.args(&mut variant.args);
        self.labeled(&mut variant.args);
    }

    fn args(&mut self, args: &mut [Arg]) {
        for arg in args {
            self.expr(&mut arg.value);
        }
    }

    /// Sorts arguments that are all labeled or punned, by label.
    fn labeled(&mut self, args: &mut [Arg]) {
        let label = |arg: &Arg| match (&arg.label, &arg.value.kind) {
            (Some(label), _) => Some(self.interner.resolve(label.name).to_owned()),
            (None, ExprKind::Name(pun)) => Some(self.interner.resolve(pun.name).to_owned()),
            _ => None,
        };
        if args.iter().all(|arg| label(arg).is_some()) {
            args.sort_by_key(label);
        }
    }

    fn text(&mut self, text: &mut TextLit) {
        for part in &mut text.parts {
            if let TextPart::Interpolation(expr) = part {
                self.expr(expr);
            }
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        match &mut expr.kind {
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
            ExprKind::Old(operand) => self.expr(operand),
            ExprKind::Attempt { operand, within } => {
                self.expr(operand);
                if let Some(within) = within {
                    self.expr(within);
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => self.expr(operand),
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
                if matches!(callee.kind, ExprKind::UpperName(_)) {
                    self.labeled(args);
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
