//! The recursive-descent parser of `spec/02`, with the error recovery of
//! `compiler/architecture.md §5`: at most one U0101 per declaration, after which parsing resumes
//! at the next line with zero indentation that starts a declaration.

use una_diag::Code;

use crate::ast::{
    Arg, Arm, BinaryOp, Bind, Block, BlockExpr, Body, CapBinding, CapBindingValue, CapType,
    CapabilityDecl, Clause, ClauseKind, Decl, DeclKind, ElseBranch, ExampleDecl, Expectation,
    ExpectationKind, Expr, ExprKind, Field, File, FnTypeClause, FunctionDecl, Given, Handled,
    HandlerCall, HandlerDecl, HandlerOp, Handling, HandlingKind, Ident, IfBlock, Import, IsTarget,
    KindBody, KindDecl, Label, LambdaParam, LetStmt, LetValue, Level, Literal, ModuleHeader,
    ModulePath, OpClause, OpClauseKind, OpDecl, Param, Pattern, PatternKind, PolicyDecl,
    PredicateDecl, Range, RequirementDecl, RequirementSource, SignedInt, Stmt, TextLit, TextPart,
    Type, TypeCore, UnaryOp, UsesClause, Variant, VariantExpr, WhenExpr,
};
use crate::error::{SyntaxError, SyntaxRepair};
use crate::intern::Interner;
use crate::lexer::{Lexed, starts_declaration};
use crate::source::{FileId, Span};
use crate::token::{Comment, Keyword, Token, TokenKind};

/// The result of parsing one file.
#[derive(Clone, Debug)]
pub struct Parsed {
    /// The syntax tree of what parsed.
    pub file: File,
    /// The syntactic errors (U0101 and U0102), in the order they were found.
    pub errors: Vec<SyntaxError>,
}

/// Parses the tokens of the file `file`, produced by [`crate::lex`].
///
/// The parser recurses at most [`MAX_NESTING`] levels deep, which in unoptimized builds needs
/// up to about 10 MiB of stack; `una check` parses on a thread with a larger stack.
pub fn parse(lexed: &Lexed, file: FileId, interner: &Interner) -> Parsed {
    let mut parser = Parser {
        tokens: &lexed.tokens,
        comments: &lexed.comments,
        interner,
        eof: lexed
            .tokens
            .last()
            .map_or(Span::at(file, 0), |token| token.span),
        lexical: lexed.errors.iter().map(|error| error.span.start).collect(),
        pos: 0,
        depth: 0,
        last_end: None,
        unit_start: 0,
        nesting: 0,
        errors: Vec::new(),
    };
    let tree = parser.file();
    Parsed {
        file: tree,
        errors: parser.errors,
    }
}

/// A U0101 has been handled; the current declaration stops parsing.
struct Stop;

type Parse<T> = Result<T, Stop>;

/// What an `if` turned out to be once its condition was read.
enum IfForm {
    /// `if c then a else b`.
    Line(Expr),
    /// An `if` with indented blocks.
    Block(IfBlock),
}

/// The canonical order of a group of clauses, for U0102.
struct Order {
    /// The clause names, by rank.
    names: &'static [&'static str],
    /// The ranks that may appear at most once.
    unique: &'static [usize],
    /// The end of the message for a clause out of order.
    description: &'static str,
}

/// `spec/02 §3`.
const FUNCTION_CLAUSES: Order = Order {
    names: &["requires", "ensures", "uses", "fails", "derived from"],
    unique: &[3, 4],
    description: "clauses go in the order requires, ensures, uses, fails, derived from",
};

/// `spec/02 §4`, ADR 0017 decision 5.
const OPERATION_CLAUSES: Order = Order {
    names: &["requires", "ensures", "fails", "accepts"],
    unique: &[2, 3],
    description: "operation clauses go in the order requires, ensures, fails, accepts",
};

/// `spec/06 §3`.
const HANDLING_CLAUSES: Order = Order {
    names: &["if error", "if none"],
    unique: &[0, 1],
    description: "handling clauses go in the order if error, if none",
};

static EOF: TokenKind = TokenKind::Eof;

/// The deepest nesting the parser accepts, counting each nested expression, block and type and
/// each operator of a chain (ADR 0019). It bounds the recursion of every pass
/// over the tree, including the parser's own.
pub const MAX_NESTING: usize = 256;

struct Parser<'a> {
    tokens: &'a [Token],
    comments: &'a [Comment],
    interner: &'a Interner,
    /// Where the token stream ends.
    eof: Span,
    /// Where each lexical error starts.
    lexical: Vec<usize>,
    pos: usize,
    /// The `INDENT`s consumed and not yet closed, for recovery.
    depth: usize,
    /// The last consumed token that is neither layout nor inserted by the lexer.
    last_end: Option<Span>,
    /// Where the current declaration, import or module header starts.
    unit_start: usize,
    /// The nesting depth of the construct being parsed.
    nesting: usize,
    errors: Vec<SyntaxError>,
}

fn is_layout(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
    )
}

/// Whether a token can start a line expression; decides how a word reserved for a later
/// version is read (approved M1 plan, recovery rules).
fn can_start_expression(kind: &TokenKind) -> bool {
    match kind {
        TokenKind::LowerIdent(_)
        | TokenKind::UpperIdent(_)
        | TokenKind::Reserved(_)
        | TokenKind::CapRef(_)
        | TokenKind::Int(_)
        | TokenKind::TextStart
        | TokenKind::LParen
        | TokenKind::LBracket
        | TokenKind::Minus => true,
        TokenKind::Keyword(keyword) => matches!(
            keyword,
            Keyword::True
                | Keyword::False
                | Keyword::Unit
                | Keyword::Result
                | Keyword::It
                | Keyword::Error
                | Keyword::Not
                | Keyword::If
                | Keyword::Fn
                | Keyword::Checked
                | Keyword::Attempt
                | Keyword::Fail
        ),
        _ => false,
    }
}

fn comparison(kind: &TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::EqEq => Some(BinaryOp::Eq),
        TokenKind::NotEq => Some(BinaryOp::NotEq),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::Le => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::Ge => Some(BinaryOp::Ge),
        _ => None,
    }
}

fn additive(kind: &TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Plus => Some(BinaryOp::Add),
        TokenKind::Minus => Some(BinaryOp::Sub),
        _ => None,
    }
}

fn multiplicative(kind: &TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Star => Some(BinaryOp::Mul),
        TokenKind::Slash => Some(BinaryOp::Div),
        TokenKind::Keyword(Keyword::Mod) => Some(BinaryOp::Mod),
        _ => None,
    }
}

fn level(kind: &TokenKind) -> Option<Level> {
    match kind {
        TokenKind::Keyword(Keyword::Public) => Some(Level::Public),
        TokenKind::Keyword(Keyword::Internal) => Some(Level::Internal),
        TokenKind::Keyword(Keyword::Confidential) => Some(Level::Confidential),
        TokenKind::Keyword(Keyword::Secret) => Some(Level::Secret),
        TokenKind::Keyword(Keyword::Restricted) => Some(Level::Restricted),
        _ => None,
    }
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    let span = left.span.to(right.span);
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        span,
    }
}

impl<'a> Parser<'a> {
    // ----- tokens -----

    fn peek(&self) -> &'a TokenKind {
        self.peek_at(0)
    }

    fn peek_at(&self, ahead: usize) -> &'a TokenKind {
        self.tokens
            .get(self.pos + ahead)
            .map_or(&EOF, |token| &token.kind)
    }

    fn span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map_or(self.eof, |token| token.span)
    }

    /// Consumes the current token, except the end of file, and returns its span.
    fn advance(&mut self) -> Span {
        let Some(token) = self.tokens.get(self.pos) else {
            return self.eof;
        };
        match token.kind {
            TokenKind::Eof => return token.span,
            TokenKind::Indent => self.depth += 1,
            TokenKind::Dedent => self.depth = self.depth.saturating_sub(1),
            TokenKind::Newline => {}
            _ if token.span.start < token.span.end => self.last_end = Some(token.span),
            _ => {}
        }
        self.pos += 1;
        token.span
    }

    /// The span from `start` to the end of the last token consumed.
    fn since(&self, start: Span) -> Span {
        match self.last_end {
            Some(end) if end.end >= start.start => start.to(end),
            _ => start,
        }
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.peek(), TokenKind::Keyword(found) if *found == keyword)
    }

    /// Whether the current token is the lower-case word `word`: a contextual keyword
    /// (`spec/01 §5`).
    fn at_word(&self, word: &str) -> bool {
        matches!(self.peek(), TokenKind::LowerIdent(name) if self.interner.resolve(*name) == word)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        let found = self.at(kind);
        if found {
            self.advance();
        }
        found
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        let found = self.at_keyword(keyword);
        if found {
            self.advance();
        }
        found
    }

    // ----- errors -----

    /// Records a U0101 at `span` and stops the current declaration.
    fn report(&mut self, span: Span, message: String) -> Stop {
        self.report_error(SyntaxError::new(Code::U0101, span, message))
    }

    /// Records `error`, a U0101 that may carry repairs, and stops the current declaration.
    fn report_error(&mut self, error: SyntaxError) -> Stop {
        let span = error.span;
        // A lexical error earlier in the same declaration explains the parse
        // error, which the lexer's recovery (a skipped character, a snapped indentation, a
        // closed bracket, a skipped reserved word) may have caused.
        let explained = self
            .lexical
            .iter()
            .any(|start| (self.unit_start..=span.start).contains(start));
        if !explained {
            self.errors.push(error);
        }
        Stop
    }

    /// U0101 at the current token: "unexpected `<found>`, expected `<expected>`".
    fn unexpected(&mut self, expected: &str) -> Stop {
        let found = match self.peek() {
            TokenKind::Keyword(keyword) => format!("keyword `{}`", keyword.as_str()),
            kind => kind.describe(self.interner),
        };
        self.report(
            self.span(),
            format!("unexpected {found}, expected {expected}"),
        )
    }

    fn fail<T>(&mut self, span: Span, message: impl Into<String>) -> Parse<T> {
        Err(self.report(span, message.into()))
    }

    /// Enters one more level of nesting, or reports U0101 past [`MAX_NESTING`]. Callers restore
    /// `nesting` when the construct ends; after an error the unit is abandoned and the next one
    /// starts from zero.
    fn deeper(&mut self) -> Parse<()> {
        if self.nesting >= MAX_NESTING {
            return self.fail(
                self.span(),
                format!(
                    "nesting deeper than {MAX_NESTING} levels, counting each chained operator; split the code with `let` or a helper function"
                ),
            );
        }
        self.nesting += 1;
        Ok(())
    }

    fn expect(&mut self, kind: &TokenKind, expected: &str) -> Parse<Span> {
        if self.at(kind) {
            Ok(self.advance())
        } else {
            Err(self.unexpected(expected))
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword) -> Parse<Span> {
        if self.at_keyword(keyword) {
            Ok(self.advance())
        } else {
            Err(self.unexpected(&format!("`{}`", keyword.as_str())))
        }
    }

    fn expect_word(&mut self, word: &str) -> Parse<Span> {
        if self.at_word(word) {
            Ok(self.advance())
        } else {
            Err(self.unexpected(&format!("`{word}`")))
        }
    }

    fn expect_line_end(&mut self) -> Parse<Span> {
        self.expect(&TokenKind::Newline, "end of line")
    }

    /// U0102 for clauses given as (rank, keyword span) in source order: a clause ranked below
    /// one before it is out of order, and a second clause of a unique rank is repeated.
    fn check_order(&mut self, clauses: &[(usize, Span)], order: &Order) {
        let name = |rank: usize| order.names.get(rank).copied().unwrap_or("clause");
        let mut highest: Option<usize> = None;
        let mut seen: Vec<usize> = Vec::new();
        for &(rank, span) in clauses {
            if order.unique.contains(&rank) && seen.contains(&rank) {
                let advice = if name(rank) == "derived from" {
                    "list every requirement in one clause"
                } else {
                    "at most one is allowed"
                };
                self.errors.push(SyntaxError::new(
                    Code::U0102,
                    span,
                    format!("repeated `{}` clause; {advice}", name(rank)),
                ));
            } else if let Some(higher) = highest.filter(|higher| *higher > rank) {
                self.errors.push(SyntaxError::new(
                    Code::U0102,
                    span,
                    format!(
                        "`{}` clause after `{}`; {}",
                        name(rank),
                        name(higher),
                        order.description
                    ),
                ));
            }
            highest = Some(highest.map_or(rank, |higher| higher.max(rank)));
            seen.push(rank);
        }
    }

    // ----- names -----

    /// A lower-case name. A word reserved for a later version is read as a name; the lexer has
    /// reported its U0103 (approved M1 plan, recovery rules).
    fn lower_name(&mut self, expected: &str) -> Parse<Ident> {
        match self.peek() {
            TokenKind::LowerIdent(name) | TokenKind::Reserved(name) => {
                let name = *name;
                let span = self.advance();
                Ok(Ident { name, span })
            }
            _ => Err(self.unexpected(expected)),
        }
    }

    fn upper_name(&mut self, expected: &str) -> Parse<Ident> {
        match self.peek() {
            TokenKind::UpperIdent(name) => {
                let name = *name;
                let span = self.advance();
                Ok(Ident { name, span })
            }
            _ => Err(self.unexpected(expected)),
        }
    }

    fn cap_name(&mut self) -> Parse<Ident> {
        match self.peek() {
            TokenKind::CapRef(name) => {
                let name = *name;
                let span = self.advance();
                Ok(Ident { name, span })
            }
            _ => Err(self.unexpected("a capability reference such as `@log`")),
        }
    }

    // ----- recovery -----

    fn at_line_start(&self) -> bool {
        match self.pos.checked_sub(1) {
            None => true,
            Some(previous) => self
                .tokens
                .get(previous)
                .is_some_and(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Dedent)),
        }
    }

    fn at_declaration_start(&self) -> bool {
        match self.peek() {
            TokenKind::Keyword(keyword) => starts_declaration(keyword.as_str(), None),
            TokenKind::LowerIdent(name) | TokenKind::Reserved(name) => {
                let next = match self.peek_at(1) {
                    TokenKind::LParen => Some('('),
                    TokenKind::LBracket => Some('['),
                    _ => None,
                };
                starts_declaration(self.interner.resolve(*name), next)
            }
            _ => false,
        }
    }

    /// Skips to the next line with zero indentation that starts a declaration
    /// (`compiler/architecture.md §5`).
    fn sync(&mut self) {
        while !self.at(&TokenKind::Eof) {
            if self.depth == 0 && self.at_line_start() && self.at_declaration_start() {
                return;
            }
            self.advance();
        }
    }

    /// `INDENT item { item } DEDENT`.
    fn indented<T>(
        &mut self,
        expected: &str,
        mut item: impl FnMut(&mut Self) -> Parse<T>,
    ) -> Parse<Vec<T>> {
        self.expect(&TokenKind::Indent, expected)?;
        let mut items = vec![item(self)?];
        while !self.eat(&TokenKind::Dedent) {
            if self.at(&TokenKind::Eof) {
                break;
            }
            items.push(item(self)?);
        }
        Ok(items)
    }

    // ----- files and modules (spec/02 §1) -----

    fn file(&mut self) -> File {
        self.start_unit();
        let module = if self.at_keyword(Keyword::Trusted) || self.at_keyword(Keyword::Module) {
            self.module_header().ok()
        } else {
            let _ = self.unexpected("`module` or `trusted module`");
            None
        };
        if module.is_none() {
            self.sync();
        }
        let mut imports = Vec::new();
        while self.at_keyword(Keyword::Import) {
            self.start_unit();
            match self.import() {
                Ok(import) => imports.push(import),
                Err(Stop) => self.sync(),
            }
        }
        let mut decls = Vec::new();
        while !self.at(&TokenKind::Eof) {
            let first = self.pos;
            self.start_unit();
            match self.decl() {
                Ok(decl) => decls.push(decl),
                Err(Stop) => {
                    if self.pos == first {
                        self.advance();
                    }
                    self.sync();
                }
            }
        }
        File {
            module,
            imports,
            decls,
            comments: self.comments.to_vec(),
        }
    }

    /// Starts a declaration, import or module header.
    fn start_unit(&mut self) {
        self.unit_start = self.span().start;
        self.nesting = 0;
    }

    fn module_header(&mut self) -> Parse<ModuleHeader> {
        let start = self.span();
        let trusted = self.eat_keyword(Keyword::Trusted);
        self.expect_keyword(Keyword::Module)?;
        let path = self.module_path()?;
        let span = self.since(start);
        self.expect_line_end()?;
        Ok(ModuleHeader {
            trusted,
            path,
            span,
        })
    }

    fn module_path(&mut self) -> Parse<ModulePath> {
        let start = self.span();
        let mut segments = vec![self.lower_name("a module path")?];
        while self.eat(&TokenKind::Dot) {
            segments.push(self.lower_name("a module path segment")?);
        }
        Ok(ModulePath {
            segments,
            span: self.since(start),
        })
    }

    fn import(&mut self) -> Parse<Import> {
        let start = self.advance();
        let module = self.module_path()?;
        self.expect(&TokenKind::LParen, "`(` and the imported names")?;
        let mut names = vec![self.import_name()?];
        while self.eat(&TokenKind::Comma) {
            names.push(self.import_name()?);
        }
        self.expect(&TokenKind::RParen, "`,` or `)`")?;
        let span = self.since(start);
        self.expect_line_end()?;
        Ok(Import {
            module,
            names,
            span,
        })
    }

    fn import_name(&mut self) -> Parse<Ident> {
        if matches!(self.peek(), TokenKind::UpperIdent(_)) {
            self.upper_name("an imported name")
        } else {
            self.lower_name("an imported name")
        }
    }

    fn decl(&mut self) -> Parse<Decl> {
        let first = self.pos;
        let start = self.span();
        let public = self.eat_keyword(Keyword::Pub);
        let kind = match self.peek() {
            TokenKind::Keyword(Keyword::Kind) => self.kind_decl(public)?,
            TokenKind::Keyword(Keyword::Predicate) => self.predicate_decl(public)?,
            TokenKind::Keyword(Keyword::Capability) => self.capability_decl(public)?,
            TokenKind::Keyword(Keyword::Handler) => self.handler_decl(public)?,
            TokenKind::Keyword(Keyword::Policy) => self.policy_decl(public)?,
            TokenKind::Keyword(Keyword::Requirement) if !public => self.requirement_decl()?,
            TokenKind::Keyword(Keyword::Example) if !public => self.example_decl()?,
            TokenKind::LowerIdent(_) | TokenKind::Reserved(_) => self.function_decl(public)?,
            _ if public => {
                return Err(self.unexpected(
                    "a kind, predicate, function, capability, handler or policy after `pub`",
                ));
            }
            _ => return Err(self.unexpected("a declaration")),
        };
        Ok(Decl {
            docs: self.docs_before(first),
            kind,
            span: self.since(start),
        })
    }

    /// The doc comments between the token at `first` and the token before it (`spec/01 §3`,
    /// ADR 0017 decision 6). every doc comment in that gap attaches, even when
    /// blank lines or plain comments separate it from the declaration.
    fn docs_before(&self, first: usize) -> Vec<Comment> {
        let Some(token) = self.tokens.get(first) else {
            return Vec::new();
        };
        let boundary = self
            .tokens
            .iter()
            .take(first)
            .rev()
            .find(|previous| !is_layout(&previous.kind) && previous.span.start < previous.span.end)
            .map_or(0, |previous| previous.span.end);
        self.comments
            .iter()
            .filter(|comment| {
                comment.doc
                    && comment.span.start >= boundary
                    && comment.span.end <= token.span.start
            })
            .copied()
            .collect()
    }

    // ----- kinds and types (spec/02 §2) -----

    fn kind_decl(&mut self, public: bool) -> Parse<DeclKind> {
        self.advance();
        let name = self.upper_name("a kind name")?;
        let params = if self.at(&TokenKind::LBracket) {
            self.type_params()?
        } else {
            Vec::new()
        };
        let body = if self.eat(&TokenKind::Newline) {
            KindBody::Record(
                self.indented("the indented fields of the record", |parser| {
                    let field = parser.field()?;
                    parser.expect_line_end()?;
                    Ok(field)
                })?,
            )
        } else {
            self.expect(&TokenKind::Eq, "`=` or end of line")?;
            if self.line_has_bar() {
                self.sum_body()?
            } else {
                let ty = self.ty()?;
                let refinement = if self.eat_keyword(Keyword::Where) {
                    Some(Box::new(self.line_expr()?))
                } else {
                    None
                };
                self.expect_line_end()?;
                KindBody::Alias {
                    ty: Box::new(ty),
                    refinement,
                }
            }
        };
        Ok(DeclKind::Kind(KindDecl {
            public,
            name,
            params,
            body,
        }))
    }

    /// Whether the rest of the logical line has `|` at bracket depth zero, which makes a kind
    /// body a sum (`spec/02`, introduction).
    fn line_has_bar(&self) -> bool {
        let mut depth = 0usize;
        for token in self.tokens.iter().skip(self.pos) {
            match token.kind {
                TokenKind::Newline | TokenKind::Eof => return false,
                TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBracket => depth = depth.saturating_sub(1),
                TokenKind::Bar if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    fn sum_body(&mut self) -> Parse<KindBody> {
        let leading = self.eat(&TokenKind::Bar);
        let mut variants = vec![self.variant()?];
        if !leading && !self.at(&TokenKind::Bar) {
            return Err(self.unexpected("`|` and another variant"));
        }
        while self.eat(&TokenKind::Bar) {
            variants.push(self.variant()?);
        }
        self.expect_line_end()?;
        Ok(KindBody::Sum(variants))
    }

    fn variant(&mut self) -> Parse<Variant> {
        let start = self.span();
        let name = self.upper_name("a variant name")?;
        let mut fields = Vec::new();
        if self.eat(&TokenKind::LParen) {
            fields.push(self.field()?);
            while self.eat(&TokenKind::Comma) {
                fields.push(self.field()?);
            }
            self.expect(&TokenKind::RParen, "`,` or `)`")?;
        }
        Ok(Variant {
            name,
            fields,
            span: self.since(start),
        })
    }

    fn field(&mut self) -> Parse<Field> {
        let start = self.span();
        let name = self.lower_name("a field name")?;
        self.expect(&TokenKind::Colon, "`:` and the field's type")?;
        let ty = self.ty()?;
        Ok(Field {
            name,
            ty,
            span: self.since(start),
        })
    }

    fn type_params(&mut self) -> Parse<Vec<Ident>> {
        self.advance();
        let mut params = vec![self.upper_name("a type parameter")?];
        while self.eat(&TokenKind::Comma) {
            params.push(self.upper_name("a type parameter")?);
        }
        self.expect(&TokenKind::RBracket, "`,` or `]`")?;
        Ok(params)
    }

    fn ty(&mut self) -> Parse<Type> {
        let outer = self.nesting;
        self.deeper()?;
        let ty = self.ty_at_depth();
        self.nesting = outer;
        ty
    }

    fn ty_at_depth(&mut self) -> Parse<Type> {
        let start = self.span();
        let label = match level(self.peek()) {
            Some(level) => Some(self.label(level)?),
            None => None,
        };
        let core = self.type_core(label.as_ref())?;
        let range = if self.eat_keyword(Keyword::In) {
            Some(self.range()?)
        } else {
            None
        };
        Ok(Type {
            label,
            core,
            range,
            span: self.since(start),
        })
    }

    /// `level [ "[" upper_ident { "," upper_ident } "]" ]`. after a level, `[`
    /// always starts the label's categories, so a labeled list type is written
    /// `secret List[T]`.
    fn label(&mut self, level: Level) -> Parse<Label> {
        let start = self.advance();
        let mut categories = Vec::new();
        if self.eat(&TokenKind::LBracket) {
            categories.push(self.upper_name("a label category")?);
            while self.eat(&TokenKind::Comma) {
                categories.push(self.upper_name("a label category")?);
            }
            self.expect(&TokenKind::RBracket, "`,` or `]`")?;
        }
        Ok(Label {
            level,
            categories,
            span: self.since(start),
        })
    }

    fn type_core(&mut self, label: Option<&Label>) -> Parse<TypeCore> {
        match self.peek() {
            TokenKind::UpperIdent(_) => {
                let name = self.upper_name("a type")?;
                let mut args = Vec::new();
                if self.eat(&TokenKind::LBracket) {
                    args.push(self.ty()?);
                    while self.eat(&TokenKind::Comma) {
                        args.push(self.ty()?);
                    }
                    self.expect(&TokenKind::RBracket, "`,` or `]`")?;
                }
                Ok(TypeCore::Named { name, args })
            }
            TokenKind::LBracket => {
                self.advance();
                let element = self.ty()?;
                self.expect(&TokenKind::RBracket, "`]`")?;
                Ok(TypeCore::List(Box::new(element)))
            }
            TokenKind::Keyword(Keyword::Fn) => {
                self.advance();
                self.expect(&TokenKind::LParen, "`(` and the parameter types")?;
                let mut params = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    params.push(self.ty()?);
                    while self.eat(&TokenKind::Comma) {
                        params.push(self.ty()?);
                    }
                }
                self.expect(&TokenKind::RParen, "`,` or `)`")?;
                self.expect(&TokenKind::Arrow, "`->` and the result type")?;
                let result = Box::new(self.ty()?);
                let mut clauses = Vec::new();
                loop {
                    if self.at_keyword(Keyword::Uses) {
                        clauses.push(FnTypeClause::Uses(self.uses_clause()?));
                    } else if self.eat_keyword(Keyword::Fails) {
                        clauses.push(FnTypeClause::Fails(self.upper_name("an error kind")?));
                    } else {
                        break;
                    }
                }
                Ok(TypeCore::Function {
                    params,
                    result,
                    clauses,
                })
            }
            // The hint for a labeled list type written `secret [T]`.
            _ if label.is_some_and(|label| !label.categories.is_empty()) => Err(self.unexpected(
                "a type after the label; a labeled list type is written `secret List[T]`",
            )),
            _ if label.is_some() => Err(self.unexpected("a type after the label")),
            _ => Err(self.unexpected("a type")),
        }
    }

    fn range(&mut self) -> Parse<Range> {
        let start = self.span();
        let low = self.signed_int()?;
        let exclusive = if self.eat(&TokenKind::DotDotLt) {
            true
        } else {
            self.expect(&TokenKind::DotDot, "`..` or `..<`")?;
            false
        };
        let high = self.signed_int()?;
        Ok(Range {
            low,
            high,
            exclusive,
            span: self.since(start),
        })
    }

    fn signed_int(&mut self) -> Parse<SignedInt> {
        let start = self.span();
        let negative = self.eat(&TokenKind::Minus);
        let TokenKind::Int(digits) = self.peek() else {
            return Err(self.unexpected("an integer"));
        };
        let digits = digits.clone();
        self.advance();
        Ok(SignedInt {
            negative,
            digits,
            span: self.since(start),
        })
    }

    fn uses_clause(&mut self) -> Parse<UsesClause> {
        let start = self.advance();
        let cap = self.cap_name()?;
        self.expect(&TokenKind::Colon, "`:` and the capability type")?;
        let cap_type = self.cap_type()?;
        Ok(UsesClause {
            cap,
            cap_type,
            span: self.since(start),
        })
    }

    fn cap_type(&mut self) -> Parse<CapType> {
        let start = self.span();
        let name = self.upper_name("a capability name")?;
        let mut ops = Vec::new();
        if self.eat(&TokenKind::LBracket) {
            ops.push(self.lower_name("an operation name")?);
            while self.eat(&TokenKind::Comma) {
                ops.push(self.lower_name("an operation name")?);
            }
            self.expect(&TokenKind::RBracket, "`,` or `]`")?;
        }
        Ok(CapType {
            name,
            ops,
            span: self.since(start),
        })
    }

    // ----- predicates and functions (spec/02 §3) -----

    fn params(&mut self) -> Parse<Vec<Param>> {
        self.expect(&TokenKind::LParen, "`(` and the parameters")?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            params.push(self.param()?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.param()?);
            }
        }
        self.expect(&TokenKind::RParen, "`,` or `)`")?;
        Ok(params)
    }

    fn param(&mut self) -> Parse<Param> {
        let start = self.span();
        let name = self.lower_name("a parameter name")?;
        self.expect(&TokenKind::Colon, "`:` and the parameter's type")?;
        let ty = self.ty()?;
        Ok(Param {
            name,
            ty,
            span: self.since(start),
        })
    }

    fn predicate_decl(&mut self, public: bool) -> Parse<DeclKind> {
        self.advance();
        let reflect = if self.eat_keyword(Keyword::Reflect) {
            true
        } else if self.eat_keyword(Keyword::Opaque) {
            false
        } else {
            return Err(self.unexpected("`reflect` or `opaque`"));
        };
        let name = self.lower_name("a predicate name")?;
        let params = self.params()?;
        self.expect(&TokenKind::Arrow, "`->` and the result type")?;
        let result = self.ty()?;
        self.expect(&TokenKind::Eq, "`=` and the body")?;
        let body = self.inline_or_block()?;
        Ok(DeclKind::Predicate(PredicateDecl {
            public,
            reflect,
            name,
            params,
            result,
            body,
        }))
    }

    fn function_decl(&mut self, public: bool) -> Parse<DeclKind> {
        let name = self.lower_name("a function name")?;
        let type_params = if self.at(&TokenKind::LBracket) {
            self.type_params()?
        } else {
            Vec::new()
        };
        let params = self.params()?;
        self.expect(&TokenKind::Arrow, "`->` and the result type")?;
        let result = self.ty()?;
        let (clauses, body) = if self.eat(&TokenKind::Eq) {
            let expr = self.line_expr()?;
            self.expect_line_end()?;
            (Vec::new(), Body::Line(expr))
        } else {
            self.expect(&TokenKind::Newline, "`=` or end of line")?;
            let clauses = if self.at(&TokenKind::Indent) {
                self.indented("the header clauses", Self::clause)?
            } else {
                Vec::new()
            };
            let ranked: Vec<(usize, Span)> = clauses
                .iter()
                .map(|clause| {
                    let rank = match clause.kind {
                        ClauseKind::Requires(_) => 0,
                        ClauseKind::Ensures(_) => 1,
                        ClauseKind::Uses(_) => 2,
                        ClauseKind::Fails(_) => 3,
                        ClauseKind::DerivedFrom(_) => 4,
                    };
                    (rank, clause.keyword)
                })
                .collect();
            self.check_order(&ranked, &FUNCTION_CLAUSES);
            self.expect(&TokenKind::Eq, "`=` and the body")?;
            self.expect_line_end()?;
            (clauses, Body::Block(self.indented_block()?))
        };
        Ok(DeclKind::Function(FunctionDecl {
            public,
            name,
            type_params,
            params,
            result,
            clauses,
            body,
        }))
    }

    fn clause(&mut self) -> Parse<Clause> {
        let keyword = self.span();
        let kind = match self.peek() {
            TokenKind::Keyword(Keyword::Requires) => {
                self.advance();
                ClauseKind::Requires(self.line_expr()?)
            }
            TokenKind::Keyword(Keyword::Ensures) => {
                self.advance();
                ClauseKind::Ensures(self.line_expr()?)
            }
            TokenKind::Keyword(Keyword::Uses) => ClauseKind::Uses(self.uses_clause()?),
            TokenKind::Keyword(Keyword::Fails) => {
                self.advance();
                ClauseKind::Fails(self.upper_name("an error kind")?)
            }
            TokenKind::Keyword(Keyword::Derived) => {
                self.advance();
                self.expect_keyword(Keyword::From)?;
                let mut requirements = vec![self.upper_name("a requirement identifier")?];
                while self.eat(&TokenKind::Comma) {
                    requirements.push(self.upper_name("a requirement identifier")?);
                }
                ClauseKind::DerivedFrom(requirements)
            }
            _ => {
                return Err(self.unexpected(
                    "a clause (`requires`, `ensures`, `uses`, `fails` or `derived from`)",
                ));
            }
        };
        let span = self.since(keyword);
        self.expect_line_end()?;
        Ok(Clause {
            kind,
            keyword,
            span,
        })
    }

    fn inline_or_block(&mut self) -> Parse<Body> {
        if self.eat(&TokenKind::Newline) {
            Ok(Body::Block(self.indented_block()?))
        } else {
            let expr = self.line_expr()?;
            self.expect_line_end()?;
            Ok(Body::Line(expr))
        }
    }

    // ----- capabilities, handlers and policies (spec/02 §4) -----

    fn capability_decl(&mut self, public: bool) -> Parse<DeclKind> {
        self.advance();
        let name = self.upper_name("a capability name")?;
        self.expect_line_end()?;
        let ops = self.indented("the indented operations", Self::op_decl)?;
        Ok(DeclKind::Capability(CapabilityDecl { public, name, ops }))
    }

    fn op_decl(&mut self) -> Parse<OpDecl> {
        let start = self.expect_keyword(Keyword::Op)?;
        let name = self.lower_name("an operation name")?;
        let params = self.params()?;
        self.expect(&TokenKind::Arrow, "`->` and the result type")?;
        let result = self.ty()?;
        self.expect_line_end()?;
        let clauses = if self.at(&TokenKind::Indent) {
            self.indented("the operation's clauses", Self::op_clause)?
        } else {
            Vec::new()
        };
        let ranked: Vec<(usize, Span)> = clauses
            .iter()
            .map(|clause| {
                let rank = match clause.kind {
                    OpClauseKind::Requires(_) => 0,
                    OpClauseKind::Ensures(_) => 1,
                    OpClauseKind::Fails(_) => 2,
                    OpClauseKind::Accepts(_) => 3,
                };
                (rank, clause.keyword)
            })
            .collect();
        self.check_order(&ranked, &OPERATION_CLAUSES);
        Ok(OpDecl {
            name,
            params,
            result,
            clauses,
            span: self.since(start),
        })
    }

    fn op_clause(&mut self) -> Parse<OpClause> {
        let keyword = self.span();
        let kind = match self.peek() {
            TokenKind::Keyword(Keyword::Requires) => {
                self.advance();
                OpClauseKind::Requires(self.line_expr()?)
            }
            TokenKind::Keyword(Keyword::Ensures) => {
                self.advance();
                OpClauseKind::Ensures(self.line_expr()?)
            }
            TokenKind::Keyword(Keyword::Fails) => {
                self.advance();
                OpClauseKind::Fails(self.upper_name("an error kind")?)
            }
            TokenKind::Keyword(Keyword::Accepts) => {
                self.advance();
                let Some(level) = level(self.peek()) else {
                    return Err(self.unexpected("a label such as `internal`"));
                };
                OpClauseKind::Accepts(self.label(level)?)
            }
            _ => {
                return Err(self.unexpected(
                    "an operation clause (`requires`, `ensures`, `fails` or `accepts`)",
                ));
            }
        };
        let span = self.since(keyword);
        self.expect_line_end()?;
        Ok(OpClause {
            kind,
            keyword,
            span,
        })
    }

    fn handler_decl(&mut self, public: bool) -> Parse<DeclKind> {
        self.advance();
        let name = self.lower_name("a handler name")?;
        let params = self.params()?;
        self.expect_keyword(Keyword::For)?;
        let capability = self.upper_name("the capability the handler implements")?;
        self.expect_line_end()?;
        self.expect(&TokenKind::Indent, "the indented operations")?;
        let mut uses = Vec::new();
        while self.at_keyword(Keyword::Uses) {
            uses.push(self.uses_clause()?);
            self.expect_line_end()?;
        }
        let mut ops = vec![self.handler_op()?];
        while !self.eat(&TokenKind::Dedent) {
            if self.at(&TokenKind::Eof) {
                break;
            }
            ops.push(self.handler_op()?);
        }
        Ok(DeclKind::Handler(HandlerDecl {
            public,
            name,
            params,
            capability,
            uses,
            ops,
        }))
    }

    fn handler_op(&mut self) -> Parse<HandlerOp> {
        let start = self.expect_keyword(Keyword::Op)?;
        let name = self.lower_name("an operation name")?;
        self.expect(&TokenKind::LParen, "`(` and the parameter names")?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            params.push(self.lower_name("a parameter name")?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.lower_name("a parameter name")?);
            }
        }
        self.expect(&TokenKind::RParen, "`,` or `)`")?;
        self.expect(&TokenKind::Eq, "`=` and the body")?;
        let body = self.inline_or_block()?;
        Ok(HandlerOp {
            name,
            params,
            body,
            span: self.since(start),
        })
    }

    fn policy_decl(&mut self, public: bool) -> Parse<DeclKind> {
        self.advance();
        let name = self.lower_name("a policy name")?;
        self.expect_line_end()?;
        self.expect(&TokenKind::Indent, "the indented policy lines")?;
        self.expect_keyword(Keyword::From)?;
        let from = self.ty()?;
        self.expect_line_end()?;
        self.expect_word("to")?;
        let to = self.ty()?;
        self.expect_line_end()?;
        self.expect_word("via")?;
        let via = self.lower_name("the declassifying function")?;
        self.expect_line_end()?;
        self.expect_word("purpose")?;
        let purpose = self.text_literal_only()?;
        self.expect_line_end()?;
        self.expect_word("owner")?;
        let owner = self.lower_name("the owner")?;
        self.expect_line_end()?;
        self.expect(&TokenKind::Dedent, "the end of the policy")?;
        Ok(DeclKind::Policy(PolicyDecl {
            public,
            name,
            from,
            to,
            via,
            purpose,
            owner,
        }))
    }

    // ----- requirements and examples (spec/02 §5) -----

    fn requirement_decl(&mut self) -> Parse<DeclKind> {
        self.advance();
        let id = self.upper_name("a requirement identifier")?;
        self.expect_line_end()?;
        self.expect(&TokenKind::Indent, "the indented requirement lines")?;
        self.expect_word("text")?;
        let text = self.text_literal_only()?;
        self.expect_line_end()?;
        self.expect_word("source")?;
        let source = if self.at_word("human") {
            self.advance();
            RequirementSource::Human
        } else if self.eat_keyword(Keyword::Policy) {
            RequirementSource::Policy(self.text_literal_only()?)
        } else if self.at_word("regulation") {
            self.advance();
            RequirementSource::Regulation(self.text_literal_only()?)
        } else if self.at_word("generated") {
            self.advance();
            RequirementSource::Generated(self.text_literal_only()?)
        } else {
            return Err(self.unexpected("`human`, `policy`, `regulation` or `generated`"));
        };
        self.expect_line_end()?;
        self.expect_word("owner")?;
        let owner = self.lower_name("the owner")?;
        self.expect_line_end()?;
        self.expect_word("status")?;
        let active = if self.at_word("active") {
            true
        } else if self.at_word("proposed") {
            false
        } else {
            return Err(self.unexpected("`proposed` or `active`"));
        };
        let status = self.advance();
        self.expect_line_end()?;
        self.expect(&TokenKind::Dedent, "the end of the requirement")?;
        Ok(DeclKind::Requirement(RequirementDecl {
            id,
            text,
            source,
            owner,
            active,
            status,
        }))
    }

    fn example_decl(&mut self) -> Parse<DeclKind> {
        self.advance();
        let name = self.text_literal_only()?;
        self.expect_line_end()?;
        self.expect(&TokenKind::Indent, "the indented example lines")?;
        let mut givens = Vec::new();
        while self.at_keyword(Keyword::Given) {
            let start = self.advance();
            let cap = self.cap_name()?;
            self.expect(&TokenKind::Eq, "`=` and a handler")?;
            let handler = self.handler_call()?;
            let span = self.since(start);
            self.expect_line_end()?;
            givens.push(Given { cap, handler, span });
        }
        self.expect_keyword(Keyword::Do)?;
        let action = self.line_expr()?;
        self.expect_line_end()?;
        let mut expectations = vec![self.expectation()?];
        while !self.eat(&TokenKind::Dedent) {
            if self.at(&TokenKind::Eof) {
                break;
            }
            expectations.push(self.expectation()?);
        }
        Ok(DeclKind::Example(ExampleDecl {
            name,
            givens,
            action,
            expectations,
        }))
    }

    /// After `expect`, `fails`, `performed` and `not performed` are tried before a line
    /// expression (`spec/02 §5`).
    fn expectation(&mut self) -> Parse<Expectation> {
        let start = self.expect_keyword(Keyword::Expect)?;
        let kind = if self.eat_keyword(Keyword::Fails) {
            ExpectationKind::Fails(self.upper_name("a variant")?)
        } else if self.at_keyword(Keyword::Performed)
            || (self.at_keyword(Keyword::Not)
                && matches!(self.peek_at(1), TokenKind::Keyword(Keyword::Performed)))
        {
            let negated = self.eat_keyword(Keyword::Not);
            self.advance();
            let cap = self.cap_name()?;
            self.expect(&TokenKind::Dot, "`.` and the operation")?;
            let op = self.lower_name("an operation name")?;
            ExpectationKind::Performed { negated, cap, op }
        } else {
            ExpectationKind::Holds(self.line_expr()?)
        };
        let span = self.since(start);
        self.expect_line_end()?;
        Ok(Expectation { kind, span })
    }

    fn handler_call(&mut self) -> Parse<HandlerCall> {
        let start = self.span();
        let name = self.lower_name("a handler name")?;
        let args = self.call_args()?;
        Ok(HandlerCall {
            name,
            args,
            span: self.since(start),
        })
    }

    // ----- blocks and statements (spec/02 §6) -----

    /// `INDENT block DEDENT`, where `block = { statement } final`.
    fn indented_block(&mut self) -> Parse<Block> {
        let outer = self.nesting;
        self.deeper()?;
        let block = self.indented_block_at_depth();
        self.nesting = outer;
        block
    }

    fn indented_block_at_depth(&mut self) -> Parse<Block> {
        self.expect(&TokenKind::Indent, "an indented block")?;
        let start = self.span();
        let mut stmts = Vec::new();
        loop {
            let first = self.span();
            let stmt = self.statement()?;
            let last = self.eat(&TokenKind::Dedent) || self.at(&TokenKind::Eof);
            match &stmt {
                Stmt::Let(_) if last => {
                    return self.fail(
                        first,
                        "a block cannot end with `let`; its last line is its value",
                    );
                }
                Stmt::BlockExpr(expr) if !last => {
                    let construct = match expr {
                        BlockExpr::When(_) => "a `when`",
                        BlockExpr::If(_) => "an `if` with blocks",
                    };
                    return self.fail(
                        first,
                        format!(
                            "{construct} must be the last line of its block or the value of a `let`"
                        ),
                    );
                }
                _ => {}
            }
            stmts.push(stmt);
            if last {
                break;
            }
        }
        Ok(Block {
            stmts,
            span: self.since(start),
        })
    }

    fn statement(&mut self) -> Parse<Stmt> {
        match self.peek() {
            TokenKind::Keyword(Keyword::Let) => Ok(Stmt::Let(Box::new(self.let_stmt()?))),
            TokenKind::Keyword(Keyword::When) => {
                Ok(Stmt::BlockExpr(BlockExpr::When(self.when_expr()?)))
            }
            TokenKind::Keyword(Keyword::If) => match self.if_statement()? {
                IfForm::Line(expr) => Ok(Stmt::Handled(self.handled_rest(expr)?)),
                IfForm::Block(block) => Ok(Stmt::BlockExpr(BlockExpr::If(block))),
            },
            _ => {
                let expr = self.line_expr()?;
                Ok(Stmt::Handled(self.handled_rest(expr)?))
            }
        }
    }

    fn let_stmt(&mut self) -> Parse<LetStmt> {
        let start = self.advance();
        let name = self.lower_name("a name")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq, "`=` and the value")?;
        let value = match self.peek() {
            TokenKind::Keyword(Keyword::When) => {
                LetValue::BlockExpr(BlockExpr::When(self.when_expr()?))
            }
            TokenKind::Keyword(Keyword::If) => match self.if_statement()? {
                IfForm::Line(expr) => LetValue::Handled(self.handled_rest(expr)?),
                IfForm::Block(block) => LetValue::BlockExpr(BlockExpr::If(block)),
            },
            _ => {
                let expr = self.line_expr()?;
                LetValue::Handled(self.handled_rest(expr)?)
            }
        };
        Ok(LetStmt {
            name,
            ty,
            value,
            span: self.since(start),
        })
    }

    /// The rest of a `handled` line after its expression: the `with` clause, the end of the
    /// line and the indented handling clauses (`spec/02 §6`).
    fn handled_rest(&mut self, expr: Expr) -> Parse<Handled> {
        let start = expr.span;
        let mut with = Vec::new();
        if self.eat_keyword(Keyword::With) {
            with.push(self.cap_binding()?);
            while self.eat(&TokenKind::Comma) {
                with.push(self.cap_binding()?);
            }
        }
        let span = self.since(start);
        self.expect_line_end()?;
        let handlings = if self.at(&TokenKind::Indent) {
            self.indented("handling clauses", Self::handling)?
        } else {
            Vec::new()
        };
        let ranked: Vec<(usize, Span)> = handlings
            .iter()
            .map(|handling| {
                let rank = match handling.kind {
                    HandlingKind::ErrorFail(_)
                    | HandlingKind::ErrorPass
                    | HandlingKind::ErrorUse(_) => 0,
                    HandlingKind::NoneFail(_) | HandlingKind::NoneUse(_) => 1,
                };
                (rank, handling.keyword)
            })
            .collect();
        self.check_order(&ranked, &HANDLING_CLAUSES);
        Ok(Handled {
            expr,
            with,
            handlings,
            span,
        })
    }

    fn cap_binding(&mut self) -> Parse<CapBinding> {
        let start = self.span();
        let cap = self.cap_name()?;
        self.expect(&TokenKind::Eq, "`=` and a capability or handler")?;
        let value = if matches!(self.peek(), TokenKind::CapRef(_)) {
            CapBindingValue::Cap(self.cap_name()?)
        } else {
            CapBindingValue::Handler(self.handler_call()?)
        };
        Ok(CapBinding {
            cap,
            value,
            span: self.since(start),
        })
    }

    fn handling(&mut self) -> Parse<Handling> {
        let keyword = self.span();
        if !self.eat_keyword(Keyword::If) {
            return Err(self.unexpected("a handling clause (`if error` or `if none`)"));
        }
        let kind = if self.eat_keyword(Keyword::Error) {
            if self.eat_keyword(Keyword::Fail) {
                HandlingKind::ErrorFail(self.variant_expr()?)
            } else if self.eat_keyword(Keyword::Pass) {
                HandlingKind::ErrorPass
            } else if self.eat_keyword(Keyword::Use) {
                HandlingKind::ErrorUse(self.line_expr()?)
            } else {
                return Err(self.unexpected("`fail`, `pass` or `use`"));
            }
        } else if self.eat_keyword(Keyword::None) {
            if self.eat_keyword(Keyword::Fail) {
                HandlingKind::NoneFail(self.variant_expr()?)
            } else if self.eat_keyword(Keyword::Use) {
                HandlingKind::NoneUse(self.line_expr()?)
            } else {
                return Err(self.unexpected("`fail` or `use`"));
            }
        } else {
            return Err(self.unexpected("`error` or `none`"));
        };
        let span = self.since(keyword);
        self.expect_line_end()?;
        Ok(Handling {
            kind,
            keyword,
            span,
        })
    }

    fn variant_expr(&mut self) -> Parse<VariantExpr> {
        let start = self.span();
        let name = self.upper_name("a variant")?;
        let args = if self.at(&TokenKind::LParen) {
            self.call_args()?
        } else {
            Vec::new()
        };
        Ok(VariantExpr {
            name,
            args,
            span: self.since(start),
        })
    }

    fn when_expr(&mut self) -> Parse<WhenExpr> {
        let keyword = self.advance();
        let scrutinee = self.line_expr()?;
        self.expect_line_end()?;
        let arms = self.indented("the indented arms", Self::arm)?;
        Ok(WhenExpr {
            scrutinee,
            arms,
            keyword,
            span: self.since(keyword),
        })
    }

    fn arm(&mut self) -> Parse<Arm> {
        let start = self.span();
        let pattern = self.pattern()?;
        self.expect(&TokenKind::Arrow, "`->`")?;
        let body = if self.eat(&TokenKind::Newline) {
            Body::Block(self.indented_block()?)
        } else {
            let expr = self.line_expr()?;
            self.expect_line_end()?;
            Body::Line(expr)
        };
        Ok(Arm {
            pattern,
            body,
            span: self.since(start),
        })
    }

    fn pattern(&mut self) -> Parse<Pattern> {
        let start = self.span();
        let kind = if self.eat_keyword(Keyword::Is) {
            if matches!(self.peek(), TokenKind::UpperIdent(_)) {
                let name = self.upper_name("a variant")?;
                let mut binds = Vec::new();
                if self.eat(&TokenKind::LParen) {
                    binds.push(self.bind(name)?);
                    while self.eat(&TokenKind::Comma) {
                        binds.push(self.bind(name)?);
                    }
                    self.expect(&TokenKind::RParen, "`,` or `)`")?;
                }
                PatternKind::Variant { name, binds }
            } else {
                PatternKind::Literal(self.literal()?)
            }
        } else if self.eat_keyword(Keyword::In) {
            PatternKind::Range(self.range()?)
        } else if self.eat_keyword(Keyword::Else) {
            PatternKind::Else
        } else {
            return Err(self.unexpected("a pattern (`is`, `in` or `else`)"));
        };
        Ok(Pattern {
            kind,
            span: self.since(start),
        })
    }

    /// One `bind` of the variant pattern on `variant`.
    fn bind(&mut self, variant: Ident) -> Parse<Bind> {
        self.no_nested_pattern(variant)?;
        let field = self.lower_name("a field name")?;
        let local = if self.eat(&TokenKind::Colon) {
            self.no_nested_pattern(variant)?;
            Some(self.lower_name("a local name")?)
        } else {
            None
        };
        Ok(Bind { field, local })
    }

    /// U0101 for a pattern where `spec/02 §6` allows only a name in a field of a variant
    /// pattern, such as `is Some(Circle(radius))` or `is Ok(value: is 0)` (benchmark finding
    /// B-001). DECISION D-155: a variant, `is`, `in` or a literal where a field or local name
    /// is expected starts a nested pattern; the repair has no mechanical edit, because moving the
    /// arm body under a second `when` also regroups the arms that share the outer variant.
    fn no_nested_pattern(&mut self, variant: Ident) -> Parse<()> {
        let nested = matches!(
            self.peek(),
            TokenKind::UpperIdent(_)
                | TokenKind::Int(_)
                | TokenKind::TextStart
                | TokenKind::Minus
                | TokenKind::Keyword(Keyword::Is | Keyword::In | Keyword::True | Keyword::False)
        );
        if !nested {
            return Ok(());
        }
        let span = self.span();
        let outer = self.interner.resolve(variant.name).to_owned();
        let mut error = SyntaxError::new(
            Code::U0101,
            span,
            "nested patterns are not supported in v0.1",
        );
        error.repairs.push(SyntaxRepair {
            description: format!(
                "bind the field to a name, as in `is {outer}(inner)`, then match that name in a second `when` inside the arm"
            ),
            edit: None,
        });
        Err(self.report_error(error))
    }

    /// An `if` in a statement or `let`, whose form is decided after its condition: `then` makes
    /// it a line, the end of the line an `if` with blocks.
    fn if_statement(&mut self) -> Parse<IfForm> {
        let start = self.advance();
        let condition = self.line_expr()?;
        if self.eat_keyword(Keyword::Then) {
            return Ok(IfForm::Line(self.if_line_rest(start, condition)?));
        }
        self.expect(&TokenKind::Newline, "`then` or end of line")?;
        Ok(IfForm::Block(self.if_block(start, condition)?))
    }

    /// `line_expr "else" line_expr`, after `if`, the condition and `then`.
    fn if_line_rest(&mut self, start: Span, condition: Expr) -> Parse<Expr> {
        let then = self.line_expr()?;
        self.expect_keyword(Keyword::Else)?;
        let otherwise = self.line_expr()?;
        Ok(Expr {
            kind: ExprKind::IfLine {
                condition: Box::new(condition),
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            },
            span: self.since(start),
        })
    }

    /// The blocks of an `if` whose condition line has ended.
    fn if_block(&mut self, start: Span, condition: Expr) -> Parse<IfBlock> {
        let then_block = self.indented_block()?;
        self.expect_keyword(Keyword::Else)?;
        let otherwise = if self.at_keyword(Keyword::If) {
            let else_if = self.advance();
            let condition = self.line_expr()?;
            self.expect(
                &TokenKind::Newline,
                "end of line; after an `if` with blocks, `else if` has blocks too",
            )?;
            let outer = self.nesting;
            self.deeper()?;
            let nested = self.if_block(else_if, condition);
            self.nesting = outer;
            ElseBranch::If(Box::new(nested?))
        } else {
            self.expect_line_end()?;
            ElseBranch::Block(self.indented_block()?)
        };
        Ok(IfBlock {
            condition,
            then_block,
            otherwise: Box::new(otherwise),
            span: self.since(start),
        })
    }

    // ----- expressions (spec/02 §7) -----

    fn line_expr(&mut self) -> Parse<Expr> {
        let outer = self.nesting;
        self.deeper()?;
        let expr = self.line_expr_at_depth();
        self.nesting = outer;
        expr
    }

    fn line_expr_at_depth(&mut self) -> Parse<Expr> {
        match self.peek() {
            TokenKind::Keyword(Keyword::If) => {
                let start = self.advance();
                let condition = self.line_expr()?;
                self.expect_keyword(Keyword::Then)?;
                self.if_line_rest(start, condition)
            }
            TokenKind::Keyword(Keyword::Fn) => self.lambda(),
            TokenKind::Keyword(Keyword::Checked) => self.checked(),
            TokenKind::Keyword(Keyword::Attempt) => {
                let start = self.advance();
                let operand = self.postfix()?;
                Ok(Expr {
                    kind: ExprKind::Attempt(Box::new(operand)),
                    span: self.since(start),
                })
            }
            TokenKind::Keyword(Keyword::Fail) => {
                let start = self.advance();
                let variant = self.variant_expr()?;
                Ok(Expr {
                    kind: ExprKind::Fail(variant),
                    span: self.since(start),
                })
            }
            _ => self.implies_expr(),
        }
    }

    fn lambda(&mut self) -> Parse<Expr> {
        let start = self.advance();
        self.expect(&TokenKind::LParen, "`(` and the parameters")?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            params.push(self.lambda_param()?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.lambda_param()?);
            }
        }
        self.expect(&TokenKind::RParen, "`,` or `)`")?;
        self.expect(&TokenKind::Arrow, "`->` and the body")?;
        let body = self.line_expr()?;
        Ok(Expr {
            kind: ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
            span: self.since(start),
        })
    }

    fn lambda_param(&mut self) -> Parse<LambdaParam> {
        let name = self.lower_name("a parameter name")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        Ok(LambdaParam { name, ty })
    }

    fn checked(&mut self) -> Parse<Expr> {
        let start = self.advance();
        self.expect(&TokenKind::LParen, "`(`")?;
        let value = self.line_expr()?;
        self.expect(&TokenKind::Comma, "`,` and the predicate")?;
        let predicate = self.line_expr()?;
        self.expect(&TokenKind::RParen, "`)`")?;
        self.expect_keyword(Keyword::Else)?;
        self.expect_keyword(Keyword::Fail)?;
        let failure = self.variant_expr()?;
        Ok(Expr {
            kind: ExprKind::Checked {
                value: Box::new(value),
                predicate: Box::new(predicate),
                failure,
            },
            span: self.since(start),
        })
    }

    /// `or_expr [ "implies" implies_expr ]`: right associative.
    fn implies_expr(&mut self) -> Parse<Expr> {
        let left = self.or_expr()?;
        if !self.eat_keyword(Keyword::Implies) {
            return Ok(left);
        }
        let outer = self.nesting;
        self.deeper()?;
        let right = self.implies_expr();
        self.nesting = outer;
        Ok(binary(BinaryOp::Implies, left, right?))
    }

    fn or_expr(&mut self) -> Parse<Expr> {
        let outer = self.nesting;
        let mut left = self.and_expr()?;
        while self.eat_keyword(Keyword::Or) {
            self.deeper()?;
            let right = self.and_expr()?;
            left = binary(BinaryOp::Or, left, right);
        }
        self.nesting = outer;
        Ok(left)
    }

    fn and_expr(&mut self) -> Parse<Expr> {
        let outer = self.nesting;
        let mut left = self.not_expr()?;
        while self.eat_keyword(Keyword::And) {
            self.deeper()?;
            let right = self.not_expr()?;
            left = binary(BinaryOp::And, left, right);
        }
        self.nesting = outer;
        Ok(left)
    }

    fn not_expr(&mut self) -> Parse<Expr> {
        if !self.at_keyword(Keyword::Not) {
            return self.cmp_expr();
        }
        let start = self.advance();
        let outer = self.nesting;
        self.deeper()?;
        let operand = self.not_expr();
        self.nesting = outer;
        let operand = operand?;
        Ok(Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            },
            span: self.since(start),
        })
    }

    /// Comparisons, `is` and `in` do not associate: a second one is U0101 (`spec/01 §7`).
    fn cmp_expr(&mut self) -> Parse<Expr> {
        let left = self.add_expr()?;
        let expr = if let Some(op) = comparison(self.peek()) {
            self.advance();
            let right = self.add_expr()?;
            binary(op, left, right)
        } else if self.eat_keyword(Keyword::Is) {
            let target = if matches!(self.peek(), TokenKind::UpperIdent(_)) {
                IsTarget::Variant(self.upper_name("a variant")?)
            } else {
                IsTarget::Literal(self.literal()?)
            };
            let span = self.since(left.span);
            Expr {
                kind: ExprKind::Is {
                    value: Box::new(left),
                    target,
                },
                span,
            }
        } else if self.eat_keyword(Keyword::In) {
            let range = self.range()?;
            let span = left.span.to(range.span);
            Expr {
                kind: ExprKind::In {
                    value: Box::new(left),
                    range,
                },
                span,
            }
        } else {
            return Ok(left);
        };
        if comparison(self.peek()).is_some()
            || self.at_keyword(Keyword::Is)
            || self.at_keyword(Keyword::In)
        {
            return self.fail(
                self.span(),
                "comparisons cannot be chained; join them with `and`, as in `a < b and b < c`",
            );
        }
        Ok(expr)
    }

    fn add_expr(&mut self) -> Parse<Expr> {
        let outer = self.nesting;
        let mut left = self.mul_expr()?;
        while let Some(op) = additive(self.peek()) {
            self.advance();
            self.deeper()?;
            let right = self.mul_expr()?;
            left = binary(op, left, right);
        }
        self.nesting = outer;
        Ok(left)
    }

    fn mul_expr(&mut self) -> Parse<Expr> {
        let outer = self.nesting;
        let mut left = self.unary()?;
        while let Some(op) = multiplicative(self.peek()) {
            self.advance();
            self.deeper()?;
            let right = self.unary()?;
            left = binary(op, left, right);
        }
        self.nesting = outer;
        Ok(left)
    }

    fn unary(&mut self) -> Parse<Expr> {
        if !self.at(&TokenKind::Minus) {
            return self.postfix();
        }
        let start = self.advance();
        let outer = self.nesting;
        self.deeper()?;
        let operand = self.unary();
        self.nesting = outer;
        let operand = operand?;
        Ok(Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(operand),
            },
            span: self.since(start),
        })
    }

    fn postfix(&mut self) -> Parse<Expr> {
        let outer = self.nesting;
        let mut expr = self.primary()?;
        loop {
            if self.eat(&TokenKind::Dot) {
                self.deeper()?;
                let name = self.lower_name("a field or method name")?;
                expr = if self.at(&TokenKind::LParen) {
                    let args = self.call_args()?;
                    let span = self.since(expr.span);
                    Expr {
                        kind: ExprKind::MethodCall {
                            target: Box::new(expr),
                            name,
                            args,
                        },
                        span,
                    }
                } else {
                    let span = expr.span.to(name.span);
                    Expr {
                        kind: ExprKind::Field {
                            target: Box::new(expr),
                            name,
                        },
                        span,
                    }
                };
            } else if self.at(&TokenKind::LParen) {
                self.deeper()?;
                let args = self.call_args()?;
                let span = self.since(expr.span);
                expr = Expr {
                    kind: ExprKind::Call {
                        callee: Box::new(expr),
                        args,
                    },
                    span,
                };
            } else {
                self.nesting = outer;
                return Ok(expr);
            }
        }
    }

    fn call_args(&mut self) -> Parse<Vec<Arg>> {
        self.expect(&TokenKind::LParen, "`(` and the arguments")?;
        let mut args = Vec::new();
        if !self.at(&TokenKind::RParen) {
            args.push(self.arg()?);
            while self.eat(&TokenKind::Comma) {
                args.push(self.arg()?);
            }
        }
        self.expect(&TokenKind::RParen, "`,` or `)`")?;
        Ok(args)
    }

    fn arg(&mut self) -> Parse<Arg> {
        let start = self.span();
        let labeled = matches!(
            self.peek(),
            TokenKind::LowerIdent(_) | TokenKind::Reserved(_)
        ) && self.peek_at(1) == &TokenKind::Colon;
        let label = if labeled {
            let label = self.lower_name("an argument label")?;
            self.advance();
            Some(label)
        } else {
            None
        };
        let value = self.line_expr()?;
        Ok(Arg {
            label,
            value,
            span: self.since(start),
        })
    }

    fn primary(&mut self) -> Parse<Expr> {
        let start = self.span();
        let kind = match self.peek() {
            TokenKind::Int(_)
            | TokenKind::TextStart
            | TokenKind::Keyword(Keyword::True | Keyword::False | Keyword::Unit) => {
                ExprKind::Literal(self.literal()?)
            }
            TokenKind::LowerIdent(_) => ExprKind::Name(self.lower_name("a name")?),
            TokenKind::Reserved(name) => {
                // A word reserved for a later version, whose U0103 the lexer reported, is
                // skipped when an expression follows it (`return x * 2`) and read as a name
                // otherwise (approved M1 plan, recovery rules).
                if can_start_expression(self.peek_at(1)) {
                    self.advance();
                    return self.line_expr();
                }
                let name = *name;
                let span = self.advance();
                ExprKind::Name(Ident { name, span })
            }
            TokenKind::UpperIdent(_) => ExprKind::UpperName(self.upper_name("a name")?),
            TokenKind::CapRef(_) => {
                let cap = self.cap_name()?;
                if self.eat(&TokenKind::Dot) {
                    let op = self.lower_name("an operation name")?;
                    let args = self.call_args()?;
                    ExprKind::CapOp { cap, op, args }
                } else {
                    ExprKind::CapRef(cap)
                }
            }
            TokenKind::LParen => {
                self.advance();
                let inner = self.line_expr()?;
                self.expect(&TokenKind::RParen, "`)`")?;
                ExprKind::Paren(Box::new(inner))
            }
            TokenKind::LBracket => {
                self.advance();
                let mut items = Vec::new();
                if !self.at(&TokenKind::RBracket) {
                    items.push(self.line_expr()?);
                    while self.eat(&TokenKind::Comma) {
                        items.push(self.line_expr()?);
                    }
                }
                self.expect(&TokenKind::RBracket, "`,` or `]`")?;
                ExprKind::List(items)
            }
            TokenKind::Keyword(Keyword::Result) => {
                self.advance();
                ExprKind::Result
            }
            TokenKind::Keyword(Keyword::It) => {
                self.advance();
                ExprKind::It
            }
            TokenKind::Keyword(Keyword::Error) => {
                self.advance();
                ExprKind::Error
            }
            _ => return Err(self.unexpected("an expression")),
        };
        Ok(Expr {
            kind,
            span: self.since(start),
        })
    }

    fn literal(&mut self) -> Parse<Literal> {
        let literal = match self.peek() {
            TokenKind::Int(digits) => Literal::Int(digits.clone()),
            TokenKind::TextStart => return Ok(Literal::Text(self.text_literal()?)),
            TokenKind::Keyword(Keyword::True) => Literal::Bool(true),
            TokenKind::Keyword(Keyword::False) => Literal::Bool(false),
            TokenKind::Keyword(Keyword::Unit) => Literal::Unit,
            _ => return Err(self.unexpected("a literal")),
        };
        self.advance();
        Ok(literal)
    }

    fn text_literal_only(&mut self) -> Parse<TextLit> {
        if self.at(&TokenKind::TextStart) {
            self.text_literal()
        } else {
            Err(self.unexpected("a text literal"))
        }
    }

    fn text_literal(&mut self) -> Parse<TextLit> {
        let start = self.advance();
        let mut parts = Vec::new();
        loop {
            match self.peek() {
                TokenKind::TextChunk(text) => {
                    parts.push(TextPart::Text(text.clone()));
                    self.advance();
                }
                TokenKind::InterpStart => {
                    self.advance();
                    let expr = self.line_expr()?;
                    self.expect(&TokenKind::InterpEnd, "`}` closing the interpolation")?;
                    parts.push(TextPart::Interpolation(expr));
                }
                TokenKind::TextEnd => {
                    let end = self.advance();
                    return Ok(TextLit {
                        parts,
                        span: start.to(end),
                    });
                }
                _ => return Err(self.unexpected("the rest of the text literal")),
            }
        }
    }
}
