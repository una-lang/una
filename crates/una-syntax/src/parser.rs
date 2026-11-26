//! The recursive-descent parser of `spec/02`, with the error recovery of
//! `compiler/architecture.md §5`: at most one U0101 per declaration, after which parsing resumes
//! at the next line with zero indentation that starts a declaration.

use una_diag::Code;

use crate::ast::{
    Arg, Arm, AssignStmt, BinaryOp, Bind, Block, BlockExpr, Body, CapBinding, CapBindingValue,
    CapType, CapabilityDecl, Clause, ClauseKind, ConstDecl, Convention, Decl, DeclKind, ElseBranch, EverStmt,
    ExampleDecl, Expectation, ExpectationKind, Expr, ExprKind, Field, File, FirstOf, FnTypeClause,
    ForSource, ForStmt, FunctionDecl, Given, Handled, HandlerCall, HandlerDecl, HandlerOp,
    Handling, HandlingKind, Ident, IfBlock, IfThenStmt, Import, IsTarget, KindBody, KindDecl,
    Label, LambdaParam, LetStmt, LetValue, Level, Literal, MeasureDecl, ModuleHeader, ModulePath,
    OnStop, OpClause, OpClauseKind, OpDecl, Param, Pattern, PatternKind, PolicyDecl, PredicateDecl,
    Range, RequirementDecl, RequirementSource, SignedInt, StartStmt, Stmt, StopAfter, StopStmt,
    TasksStmt, TextLit, TextPart, TogetherFor, TogetherStmt, Type, TypeCore, UnaryOp, UnitExpr,
    UntilStmt, UsesClause, Variant, VariantExpr, WhenExpr,
};
use crate::error::{SyntaxError, SyntaxRepair};
use crate::intern::{Interner, Symbol};
use crate::lexer::{Lexed, starts_declaration};
use crate::source::{FileId, Span};
use crate::token::{Comment, Keyword, KeywordSet, Token, TokenKind};

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
    let mut parsed = parse_plain(lexed, file, interner);
    // The exact rewrites of foreign syntax (ADR 0096 A).
    crate::rewrite::attach(&lexed.text, file, lexed.keywords, &mut parsed.errors);
    if lexed.keywords < KeywordSet::V0_5 {
        binding_words_help(&lexed.text, &mut parsed.errors);
    }
    parsed
}

/// ADR 0137 rule 1: before edition 0.5, a U0101 on a line that starts with `const` and a
/// lower-case name keeps its diagnostic and gains the help that names the edition of the
/// binding words. (`var` there is U0103, whose help the lexer adds.) O(1) per error after the
/// line is found, O(length of the line).
fn binding_words_help(text: &str, errors: &mut [SyntaxError]) {
    for error in errors.iter_mut() {
        if error.code != Code::U0101 {
            continue;
        }
        let at = error.span.start.min(text.len());
        let start = text
            .get(..at)
            .and_then(|t| t.rfind('\n'))
            .map_or(0, |i| i + 1);
        let line = text.get(start..).unwrap_or("");
        let line = line.split('\n').next().unwrap_or(line);
        let content = line.trim_start_matches(' ');
        let binding = content.strip_prefix("const ").is_some_and(|rest| {
            rest.trim_start_matches(' ')
                .starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        });
        if binding
            && !error
                .repairs
                .iter()
                .any(|repair| repair.description == crate::keywords::BINDING_WORDS_HELP)
        {
            error.repairs.push(SyntaxRepair {
                description: crate::keywords::BINDING_WORDS_HELP.to_owned(),
                edit: None,
            });
        }
    }
}

/// [`parse`] without the rewrites of ADR 0096, which check their candidates with it.
pub(crate) fn parse_plain(lexed: &Lexed, file: FileId, interner: &Interner) -> Parsed {
    let mut parser = Parser {
        tokens: &lexed.tokens,
        text: &lexed.text,
        comments: &lexed.comments,
        interner,
        eof: lexed
            .tokens
            .last()
            .map_or(Span::at(file, 0), |token| token.span),
        // U7007 of `let` in edition 0.5 explains no parse error: the binding parses.
        lexical: lexed
            .errors
            .iter()
            .filter(|error| error.code != Code::U7007)
            .map(|error| error.span.start)
            .collect(),
        pos: 0,
        depth: 0,
        last_end: None,
        unit_start: 0,
        nesting: 0,
        errors: Vec::new(),
        v0_2: lexed.keywords >= KeywordSet::V0_2,
        v0_3: lexed.keywords >= KeywordSet::V0_3,
        keywords: lexed.keywords,
        v0_4: lexed.keywords >= KeywordSet::V0_4,
        in_body: false,
        from_label: lexed.from_label,
        with_label: lexed.with_label,
        and_label: lexed.and_label,
        error_label: lexed.error_label,
    };
    let tree = parser.file();
    Parsed {
        file: tree,
        errors: parser.errors,
    }
}

/// One `uses` clause on its own, `@b: C[op where cond, ...]`, as a lock entry of `una.toml`
/// writes it (ADR 0092 rule 8, as amended). `text` is the clause, and `place` gives where its
/// bytes `start..end` are written in `file`, whose text is `written`, so that every span is a
/// span of that file even where the clause's string has escapes (#727). Its operations may
/// carry conditions, as in edition `0.3`.
pub fn parse_uses_clause(
    text: &str,
    file: FileId,
    place: &dyn Fn(usize, usize) -> (usize, usize),
    written: &str,
    interner: &mut Interner,
) -> Result<UsesClause, Vec<SyntaxError>> {
    let shift = |span: Span| {
        let (start, end) = place(span.start, span.end);
        Span::new(file, start, end)
    };
    let source = crate::source::SourceFile::new("una.toml", text.as_bytes());
    let mut lexed = crate::lexer::lex_in(&source, file, interner, KeywordSet::V0_3);
    if !lexed.errors.is_empty() {
        return Err(lexed
            .errors
            .into_iter()
            .map(|error| SyntaxError::new(error.code, shift(error.span), error.message))
            .collect());
    }
    for token in &mut lexed.tokens {
        token.span = shift(token.span);
    }
    // The parser reads the text of a span for some messages and repairs, so it reads the file.
    lexed.text = written.to_owned();
    let mut parser = Parser {
        tokens: &lexed.tokens,
        text: &lexed.text,
        comments: &lexed.comments,
        interner: &*interner,
        eof: lexed
            .tokens
            .last()
            .map_or(shift(Span::at(file, 0)), |token| token.span),
        lexical: Vec::new(),
        pos: 0,
        depth: 0,
        last_end: None,
        unit_start: 0,
        nesting: 0,
        errors: Vec::new(),
        v0_2: true,
        v0_3: true,
        keywords: KeywordSet::V0_3,
        v0_4: true,
        in_body: false,
        from_label: None,
        with_label: None,
        and_label: None,
        error_label: None,
    };
    match parser.lone_uses_clause() {
        Ok(clause) if parser.errors.is_empty() => Ok(clause),
        _ => Err(parser.errors),
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
    /// An `if` with a block and no `else`, in edition 0.2.
    Then(IfThenStmt),
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
    names: &[
        "requires",
        "ensures",
        "uses",
        "fails",
        "fails of",
        "diverges",
        "derived from",
    ],
    unique: &[3, 4, 5, 6],
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
    /// The text of the file.
    text: &'a str,
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
    /// Whether the constructs of edition 0.2 parse (`spec/17 §1`).
    v0_2: bool,
    /// Whether the constructs of edition 0.3 parse (`spec/13 §7`).
    v0_3: bool,
    /// The keyword set of the package's edition, for the declaration starts.
    keywords: KeywordSet,
    /// Whether the constructs of edition 0.4 parse (`spec/26 §1`).
    v0_4: bool,
    /// Whether a function body or a handler operation body is being parsed, where a
    /// declaration keyword bound as a name is renamed by an edit (ADR 0096, amendment of
    /// 2026-10-15).
    in_body: bool,
    /// Edition 0.3: the symbol of the argument label `from` (`spec/29 §3`), when the file has
    /// one.
    from_label: Option<Symbol>,
    /// Edition 0.4: the symbols of the argument labels `with` and `and` (`spec/29 §17`), when
    /// the file has them.
    with_label: Option<Symbol>,
    /// See [`Parser::with_label`].
    and_label: Option<Symbol>,
    /// Edition 0.4: the symbol of `error` as the field of `Failed(error: x)` (`spec/27 §5`).
    error_label: Option<Symbol>,
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
        | TokenKind::Float(_)
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
    /// Whether the token after the current one is the lower-case word `word`.
    fn next_is_word(&self, word: &str) -> bool {
        matches!(self.peek_at(1), TokenKind::LowerIdent(name) if self.interner.resolve(*name) == word)
    }

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
        let message = self.unexpected_message(expected);
        self.report(self.span(), message)
    }

    /// "unexpected …, expected …", or, when the token is part of a number written as Una does
    /// not write numbers, what that number is (#12, review R-045).
    fn unexpected_message(&self, expected: &str) -> String {
        if let Some(literal) = self.literal_hint() {
            return literal;
        }
        let found = match self.peek() {
            TokenKind::Keyword(keyword) => format!("keyword `{}`", keyword.as_str()),
            kind => kind.describe(self.interner),
        };
        format!("unexpected {found}, expected {expected}")
    }

    /// When the current token continues an integer literal with no space between them, as
    /// `x1F` does in `0x1F`, `e5` in `1e5` and `14` in `3.14`: the message naming that literal.
    /// D-016 keeps such a token a token of its own, and U0101 at it.
    fn literal_hint(&self) -> Option<String> {
        let at = |offset: usize| {
            self.pos
                .checked_sub(offset)
                .and_then(|i| self.tokens.get(i))
        };
        let current = self.tokens.get(self.pos)?;
        let touches = |a: &Token, b: &Token| a.span.end == b.span.start;
        let is_name = |token: &Token| {
            matches!(
                token.kind,
                TokenKind::LowerIdent(_) | TokenKind::UpperIdent(_) | TokenKind::Reserved(_)
            )
        };
        let text = |token: &Token| {
            self.text
                .get(token.span.start..token.span.end)
                .unwrap_or("")
        };
        // Where the literal starts: the integer the current token touches, through a `.` for a
        // fraction.
        let (start, what) = match (at(2), at(1)) {
            (Some(int), Some(dot))
                if matches!(int.kind, TokenKind::Int(_))
                    && matches!(dot.kind, TokenKind::Dot)
                    && touches(int, dot)
                    && touches(dot, current)
                    && matches!(current.kind, TokenKind::Int(_)) =>
            {
                (int, "a fractional literal")
            }
            (_, Some(int))
                if matches!(int.kind, TokenKind::Int(_))
                    && touches(int, current)
                    && is_name(current) =>
            {
                let digits = text(int);
                let rest = text(current);
                let mut chars = rest.chars();
                let first = chars.next()?;
                let base = match first.to_ascii_lowercase() {
                    'x' if digits == "0" => "a hexadecimal literal",
                    'b' if digits == "0" => "a binary literal",
                    'o' if digits == "0" => "an octal literal",
                    'e' if chars.next().is_some_and(|c| c.is_ascii_digit()) => {
                        "a literal with an exponent"
                    }
                    _ => return None,
                };
                (int, base)
            }
            _ => return None,
        };
        // The literal runs on through every token that touches the one before it and could
        // be part of a number: digits, names, `.`.
        let mut end = self.pos;
        while let (Some(this), Some(next)) = (self.tokens.get(end), self.tokens.get(end + 1)) {
            let numeric = matches!(next.kind, TokenKind::Int(_) | TokenKind::Dot) || is_name(next);
            if !touches(this, next) || !numeric {
                break;
            }
            end += 1;
        }
        let last = self.tokens.get(end)?;
        let literal = self.text.get(start.span.start..last.span.end)?;
        Some(format!(
            "`{literal}` is {what}; Una's integer literals are decimal"
        ))
    }

    /// [`Parser::unexpected`], with `advice` as a repair without an edit (issue #75).
    fn unexpected_with(&mut self, expected: &str, advice: impl Into<String>) -> Stop {
        let message = self.unexpected_message(expected);
        let mut error = SyntaxError::new(Code::U0101, self.span(), message);
        error.repairs.push(SyntaxRepair {
            description: advice.into(),
            edit: None,
        });
        self.report_error(error)
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

    /// The keywords that cannot stand in a function's body, clauses or types, so that within a
    /// binding's scope every token of one of them is a use of the binding: a binding named with
    /// one is renamed by an exact edit (ADR 0096, amendment of 2026-10-15).
    const DECLARATION_KEYWORDS: [&'static str; 20] = [
        "accepts",
        "capability",
        "do",
        "example",
        "expect",
        "given",
        "handler",
        "import",
        "kind",
        "module",
        "op",
        "opaque",
        "performed",
        "policy",
        "predicate",
        "pub",
        "reflect",
        "requirement",
        "test",
        "trusted",
    ];

    /// The word of the test declaration in the package's edition: `test` from edition 0.3,
    /// `example` before (ADR 0105), for the messages that name it.
    fn test_word(&self) -> &'static str {
        if self.v0_3 { "test" } else { "example" }
    }

    /// A lower-case name. A word reserved for a later version is read as a name; the lexer has
    /// reported its U0103 (approved M1 plan, recovery rules).
    fn lower_name(&mut self, expected: &str) -> Parse<Ident> {
        match self.peek() {
            TokenKind::LowerIdent(name) | TokenKind::Reserved(name) => {
                let name = *name;
                let span = self.advance();
                Ok(Ident { name, span })
            }
            TokenKind::Keyword(keyword) => {
                let word = keyword.as_str();
                // #743: a reserved keyword where a module path segment or an operation is named
                // says what the rule is, with a name to use instead. Renaming one changes other
                // files or declarations (a module's file and imports, an operation's every use),
                // so the repair is advice (ADR 0096 rule 2(a)). `test`, the keyword of edition
                // 0.3, says the same where a function, a field, a handler or a predicate is
                // named (ADR 0105 rule 4): renaming those reaches every caller, constructor or
                // import.
                let module_path = matches!(expected, "a module path" | "a module path segment")
                    && *keyword == Keyword::Test;
                let role = match expected {
                    "a module path" | "a module path segment" => {
                        Some(("a module path segment", "module"))
                    }
                    "an operation name" => Some(("an operation name", "op")),
                    "a function name" if *keyword == Keyword::Test => {
                        Some(("a function name", "fn"))
                    }
                    "a field name" if *keyword == Keyword::Test => Some(("a field name", "field")),
                    "a handler name" if *keyword == Keyword::Test => {
                        Some(("a handler name", "handler"))
                    }
                    "a predicate name" if *keyword == Keyword::Test => {
                        Some(("a predicate name", "predicate"))
                    }
                    _ => None,
                };
                match role {
                    Some((role, suffix)) => {
                        let fresh = self.fresh_name(&format!("{word}_{suffix}"), &[]);
                        let mut error = SyntaxError::new(
                            Code::U0101,
                            self.span(),
                            format!("`{word}` is a reserved keyword, so it cannot be {role}"),
                        );
                        let description = if module_path {
                            format!(
                                "choose another name, such as `{fresh}`, and rename the file or directory to match"
                            )
                        } else {
                            format!("choose another name, such as `{fresh}`")
                        };
                        error.repairs.push(SyntaxRepair {
                            description,
                            edit: None,
                        });
                        Err(self.report_error(error))
                    }
                    None => Err(self.unexpected(expected)),
                }
            }
            _ => Err(self.unexpected(expected)),
        }
    }

    /// The name a binding introduces: a `let`, a parameter, a loop variable or a pattern's
    /// local name. A reserved keyword there is U0101 saying what the word is reserved for
    /// (#206), with a new name. For a `let` of `result`, the repair is one edit that renames
    /// the binding and its uses in its block: in a body, `result` names nothing else, since it
    /// is legal only in `ensures` (U2012) and neither lambdas nor nested blocks have clauses.
    /// `it` also names the value of a `checked` predicate or a `where` clause in a body, so
    /// its repair is advice. A declaration keyword bound inside a function body or a handler
    /// operation body is renamed through its scope, since none can stand there as a keyword
    /// (ADR 0096, amendment of 2026-10-15); every other repair is advice.
    fn binding_name(&mut self, expected: &str, binding: Binding) -> Parse<Ident> {
        let local = binding == Binding::Let;
        let TokenKind::Keyword(keyword) = *self.peek() else {
            return self.lower_name(expected);
        };
        let word = keyword.as_str();
        let (meaning, base) = match keyword {
            Keyword::Result => (
                Some("names a function's result in `ensures`"),
                "outcome".to_owned(),
            ),
            Keyword::It => (
                Some("names the value that a `where` clause or a `checked` predicate constrains"),
                "item".to_owned(),
            ),
            Keyword::Error => (
                Some("names the failure in an `if error` handling clause"),
                "problem".to_owned(),
            ),
            _ => (None, format!("{word}_value")),
        };
        let message = match meaning {
            Some(meaning) => {
                format!("`{word}` is reserved: it {meaning}, so it cannot name a binding")
            }
            None => format!("`{word}` is a reserved keyword, so it cannot name a binding"),
        };
        let fresh = self.fresh_name(&base, &[]);
        let span = self.span();
        // A declaration keyword bound inside a function or handler-operation body is renamed
        // through its scope (ADR 0096, amendment of 2026-10-15); a function's parameter is not,
        // since its callers name it.
        if Self::DECLARATION_KEYWORDS.contains(&word)
            && (self.in_body || binding == Binding::HandlerOp)
            && binding != Binding::Param
            && binding != Binding::Loop
            && let Some(renames) = self.keyword_renames(keyword, &fresh, binding)
        {
            let end = renames.last().map_or(span.end, |(last, _)| last.end);
            let mut replacement = String::new();
            let mut at = span.start;
            for (range, name) in &renames {
                replacement.push_str(self.text.get(at..range.start).unwrap_or(""));
                replacement.push_str(name);
                at = range.end;
            }
            let mut error = SyntaxError::new(Code::U0101, span, message);
            error.repairs.push(SyntaxRepair {
                description: format!("rename the binding and its uses to `{fresh}`"),
                edit: Some((
                    Span {
                        file: span.file,
                        start: span.start,
                        end,
                    },
                    replacement,
                )),
            });
            return Err(self.report_error(error));
        }
        let repair = match self.result_renames(&base) {
            Some(renames) if local && keyword == Keyword::Result => {
                let end = renames.last().map_or(span.end, |(last, _)| last.end);
                let mut replacement = String::new();
                let mut at = span.start;
                for (range, name) in &renames {
                    replacement.push_str(self.text.get(at..range.start).unwrap_or(""));
                    replacement.push_str(name);
                    at = range.end;
                }
                let first = renames
                    .first()
                    .map_or(fresh.as_str(), |(_, name)| name.as_str());
                SyntaxRepair {
                    description: format!("rename the binding and its uses to `{first}`"),
                    edit: Some((
                        Span {
                            file: span.file,
                            start: span.start,
                            end,
                        },
                        replacement,
                    )),
                }
            }
            _ => SyntaxRepair {
                description: format!("choose another name, such as `{fresh}`"),
                edit: None,
            },
        };
        let mut error = SyntaxError::new(Code::U0101, span, message);
        error.repairs.push(repair);
        Err(self.report_error(error))
    }

    /// The renames of `result` from the current token, a `let` binding of it, to the end of its
    /// scope (#206, #257): each `let result` gets a name of its own, from `base`, and each use
    /// the name of the binding it reads. A binding's own value reads the binding before it, so
    /// a new name takes effect at the end of its line, and a nested block's names end with the
    /// block. The scope is the block of the `let`, extended past a `together` block, whose
    /// tasks' bindings are in scope after it (`spec/19`).
    fn result_renames(&self, base: &str) -> Option<Vec<(Span, String)>> {
        let tokens = self.tokens.get(self.pos..)?;
        let mut chosen: Vec<String> = Vec::new();
        let mut renames = Vec::new();
        // The name in effect at each open block, innermost last; the pending switch to a new
        // binding's name, with the depth whose end of line makes it effective.
        let mut names: Vec<Option<String>> = vec![None];
        let mut pending: Option<(usize, String)> = None;
        let mut depth = 0usize;
        // The blocks closed past so far, when the scope extends past `together` blocks.
        let mut level = self.pos;
        for (offset, token) in tokens.iter().enumerate() {
            let at = self.pos + offset;
            match token.kind {
                TokenKind::Indent => {
                    depth += 1;
                    let current = names.last().cloned().flatten();
                    names.push(current);
                }
                TokenKind::Dedent if depth == 0 => {
                    // The end of the scope, unless the block was a `together` block.
                    if !self.opens_together(level, at) {
                        break;
                    }
                    level = at;
                }
                TokenKind::Dedent => {
                    depth -= 1;
                    names.pop();
                }
                TokenKind::Newline => {
                    if let Some((line_depth, name)) = pending.take_if(|(d, _)| *d == depth) {
                        let _ = line_depth;
                        if let Some(slot) = names.last_mut() {
                            *slot = Some(name);
                        }
                    }
                }
                TokenKind::Eof => break,
                TokenKind::Keyword(Keyword::Result) => {
                    let binds = offset == 0 || self.binds_at(at);
                    if binds {
                        let name = self.fresh_name(base, &chosen);
                        chosen.push(name.clone());
                        renames.push((token.span, name.clone()));
                        pending = Some((depth, name));
                    } else if let Some(Some(name)) = names.last() {
                        renames.push((token.span, name.clone()));
                    } else if let Some((_, name)) = &pending {
                        // A use on the line of the first binding, after it.
                        renames.push((token.span, name.clone()));
                    }
                }
                _ => {}
            }
        }
        Some(renames)
    }

    /// The renames of a binding named with the declaration keyword `keyword`, from the current
    /// token, to `name`, through its scope (ADR 0096, amendment of 2026-10-15): a `let`'s block
    /// (past `together` blocks), or the body of the lambda, the `when` arm or the handler
    /// operation that binds it. A use inside an interpolation hole is a token too, so it is
    /// renamed; a name after `.` (a field or a method) and an argument label (`op:`, a callee's
    /// parameter) are left as they are. `None` when the scope cannot be read with certainty,
    /// such as a second binding of the same word in it: then the repair is advice.
    fn keyword_renames(
        &self,
        keyword: Keyword,
        name: &str,
        binding: Binding,
    ) -> Option<Vec<(Span, String)>> {
        let tokens = self.tokens.get(self.pos..)?;
        let mut renames = vec![(tokens.first()?.span, name.to_owned())];
        let kind = |at: usize| self.tokens.get(at).map(|t| &t.kind);
        // Brackets opened since the binding: a lambda's, an arm's or an operation's parameter
        // list closes at -1.
        let mut brackets: i64 = 0;
        // Indentation since the scope's body began.
        let mut blocks: usize = 0;
        // Whether the body has begun, and whether it is a block (else one line).
        let mut in_body = binding == Binding::Let;
        let mut block_body = binding == Binding::Let;
        let mut level = self.pos;
        for (offset, token) in tokens.iter().enumerate().skip(1) {
            let at = self.pos + offset;
            match &token.kind {
                TokenKind::LParen | TokenKind::LBracket => brackets += 1,
                TokenKind::RParen | TokenKind::RBracket => {
                    brackets -= 1;
                    if brackets < 0 && !in_body {
                        // The parameter list (or the pattern) has closed: the body follows its
                        // `->` (a lambda, a `when` arm) or `=` (a handler operation) at once.
                        let opens = match binding {
                            Binding::HandlerOp => TokenKind::Eq,
                            _ => TokenKind::Arrow,
                        };
                        if kind(at + 1) != Some(&opens) {
                            return None;
                        }
                        in_body = true;
                        brackets = 0;
                        block_body = matches!(kind(at + 2), Some(TokenKind::Newline))
                            && matches!(kind(at + 3), Some(TokenKind::Indent));
                        // A lambda's body is one line (`line_expr`).
                        if binding == Binding::Lambda && block_body {
                            return None;
                        }
                    } else if in_body && binding == Binding::Lambda && brackets < 0 {
                        // The lambda's body ends where the call around it closes.
                        break;
                    }
                }
                TokenKind::Comma if in_body && binding == Binding::Lambda && brackets <= 0 => {
                    break;
                }
                TokenKind::Indent if in_body => blocks += 1,
                TokenKind::Dedent if in_body && blocks > 0 => {
                    blocks -= 1;
                    // The block of a block body has closed.
                    if blocks == 0 && block_body && binding != Binding::Let {
                        break;
                    }
                }
                TokenKind::Dedent if binding == Binding::Let => {
                    // The end of the `let`'s block, unless it was a `together` block.
                    if !self.opens_together(level, at) {
                        break;
                    }
                    level = at;
                }
                TokenKind::Dedent => break,
                TokenKind::Newline if in_body && !block_body && blocks == 0 && brackets <= 0 => {
                    break;
                }
                TokenKind::Newline if !in_body => return None,
                TokenKind::Eof => break,
                TokenKind::Keyword(k) if *k == keyword => {
                    if self.binds_at(at) {
                        return None;
                    }
                    let after_dot = at.checked_sub(1).and_then(kind) == Some(&TokenKind::Dot);
                    let label = matches!(kind(at + 1), Some(TokenKind::Colon));
                    if after_dot || label {
                        continue;
                    }
                    if !in_body {
                        // Another parameter of the same list with the same word.
                        return None;
                    }
                    renames.push((token.span, name.to_owned()));
                }
                _ => {}
            }
        }
        if !in_body {
            return None;
        }
        Some(renames)
    }

    /// Whether the token at `at` is the name of a binding statement: `let` or `let edit`, or
    /// `const` or `var` in edition 0.5.
    fn binds_at(&self, at: usize) -> bool {
        let before = |back: usize| at.checked_sub(back).and_then(|i| self.tokens.get(i));
        match before(1).map(|t| &t.kind) {
            Some(TokenKind::Keyword(Keyword::Let | Keyword::Const | Keyword::Var)) => true,
            Some(TokenKind::Keyword(Keyword::Edit)) => {
                matches!(
                    before(2).map(|t| &t.kind),
                    Some(TokenKind::Keyword(Keyword::Let))
                )
            }
            _ => false,
        }
    }

    /// Whether the block that the `Dedent` at `close` ends, and that holds the token at `from`,
    /// was opened by `together`: its `Indent` follows the keyword and the end of its line.
    fn opens_together(&self, from: usize, close: usize) -> bool {
        let _ = close;
        // From a `Dedent`, the block it closes is passed over first.
        let mut depth = usize::from(matches!(
            self.tokens.get(from).map(|t| &t.kind),
            Some(TokenKind::Dedent)
        ));
        let mut index = from;
        while index > 0 {
            index -= 1;
            match self.tokens.get(index).map(|t| &t.kind) {
                Some(TokenKind::Dedent) => depth += 1,
                Some(TokenKind::Indent) if depth == 0 => {
                    let mut before = index;
                    while before > 0 {
                        before -= 1;
                        match self.tokens.get(before).map(|t| &t.kind) {
                            Some(TokenKind::Newline) => continue,
                            Some(TokenKind::Keyword(Keyword::Together)) => return true,
                            _ => return false,
                        }
                    }
                    return false;
                }
                Some(TokenKind::Indent) => depth -= 1,
                _ => {}
            }
        }
        false
    }

    /// `base`, or `base` followed by the first number from 2 that makes it a name the file
    /// does not use and that is not in `taken`.
    fn fresh_name(&self, base: &str, taken: &[String]) -> String {
        let used = |name: &str| {
            taken.iter().any(|t| t == name)
                || self.tokens.iter().any(|token| {
                    matches!(token.kind, TokenKind::LowerIdent(symbol) if self.interner.resolve(symbol) == name)
                })
        };
        if !used(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base}{n}"))
            .find(|name| !used(name))
            .unwrap_or_else(|| base.to_owned())
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
            TokenKind::Keyword(keyword) => {
                starts_declaration(keyword.as_str(), None, self.keywords)
            }
            TokenKind::LowerIdent(name) | TokenKind::Reserved(name) => {
                let next = match self.peek_at(1) {
                    TokenKind::LParen => Some('('),
                    TokenKind::LBracket => Some('['),
                    _ => None,
                };
                starts_declaration(self.interner.resolve(*name), next, self.keywords)
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
            // Edition 0.3 writes the test declaration `test` (ADR 0105 rule 1); `example` there
            // has its U0103 from the lexer and is read the same way (rule 3). A `test` followed
            // by `(` or `[` is a function named with the keyword (rule 4).
            TokenKind::Keyword(Keyword::Test)
                if !public
                    && !matches!(self.peek_at(1), TokenKind::LParen | TokenKind::LBracket) =>
            {
                self.example_decl()?
            }
            TokenKind::Keyword(Keyword::Test)
                if matches!(self.peek_at(1), TokenKind::LParen | TokenKind::LBracket) =>
            {
                self.function_decl(public)?
            }
            // ADR 0104 rule 1: `measure` is the keyword only at a declaration's start, before
            // an upper identifier, in edition 0.3; `measure(x)` stays a function.
            TokenKind::LowerIdent(_)
                if self.v0_3
                    && self.at_word("measure")
                    && matches!(self.peek_at(1), TokenKind::UpperIdent(_)) =>
            {
                self.measure_decl(public)?
            }
            TokenKind::LowerIdent(_) | TokenKind::Reserved(_) => self.function_decl(public)?,
            // Edition 0.5: a module constant (ADR 0126 rule 1). `let` there has its U7007 from
            // the lexer, with the rewrite to `const`, and is read the same way.
            TokenKind::Keyword(Keyword::Const) => self.constant_decl(public)?,
            TokenKind::Keyword(Keyword::Let) if self.keywords >= KeywordSet::V0_5 => {
                if matches!(self.peek_at(1), TokenKind::Keyword(Keyword::Edit)) {
                    let span = self.span();
                    return self.fail(
                        span,
                        "a module declares no `var`; a module value is a `const`",
                    );
                }
                self.constant_decl(public)?
            }
            // Edition 0.5: a module has no mutable state (ADR 0137 rule 3).
            TokenKind::Keyword(Keyword::Var) => {
                let span = self.span();
                return self.fail(
                    span,
                    "a module declares no `var`; a module value is a `const`",
                );
            }
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

    /// Edition 0.4: the keyword `error` as a field name, when `followed_by_colon` is false or
    /// it is followed by `:`, which only `Failed(error: x)` of `std.time` and code that names
    /// that field use (`spec/27 §5`, ADR 0117 rule 5). It never names a binding.
    fn error_field(&mut self, followed_by_colon: bool) -> Option<Ident> {
        let name = self.error_label?;
        if !self.v0_4
            || self.peek() != &TokenKind::Keyword(Keyword::Error)
            || (followed_by_colon && self.peek_at(1) != &TokenKind::Colon)
        {
            return None;
        }
        let span = self.advance();
        Some(Ident { name, span })
    }

    fn field(&mut self) -> Parse<Field> {
        let start = self.span();
        let name = match self.error_field(true) {
            Some(name) => name,
            None => self.lower_name("a field name")?,
        };
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

    /// The type parameters of a function, each with its constraint, as `[T: Ord]`; a
    /// constraint only in edition 0.2 (`spec/20 §2`).
    fn constrained_params(&mut self) -> Parse<(Vec<Ident>, Vec<Option<Ident>>)> {
        self.advance();
        let mut params = Vec::new();
        let mut constraints = Vec::new();
        loop {
            params.push(self.upper_name("a type parameter")?);
            let constraint = if self.v0_2 && self.eat(&TokenKind::Colon) {
                Some(self.upper_name("a constraint: `Eq`, `Ord` or `Integer`")?)
            } else {
                None
            };
            constraints.push(constraint);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBracket, "`,` or `]`")?;
        Ok((params, constraints))
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
        // ADR 0104 rule 2: a unit after `of`, in edition 0.3. The checker refuses it on a type
        // that is not an integer (U2029).
        let unit = if self.v0_3 && self.at_word("of") {
            self.advance();
            Some(self.unit_expr()?)
        } else {
            None
        };
        let range = if self.eat_keyword(Keyword::In) {
            Some(self.type_range()?)
        } else {
            None
        };
        Ok(Type {
            label,
            core,
            unit,
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
                // Edition 0.5: `Decimal[s]`, whose argument is a scale, an integer literal
                // (ADR 0120 rule 2, moved to edition 0.5 by ADR 0139 rule 6). Any other argument
                // parses as a type, which the checker refuses (U2037).
                if self.keywords >= KeywordSet::V0_5
                    && self.interner.resolve(name.name) == "Decimal"
                    && self.at(&TokenKind::LBracket)
                    && let TokenKind::Int(digits) = self.peek_at(1)
                {
                    let scale = digits.clone();
                    self.advance();
                    let scale_span = self.advance();
                    self.expect(&TokenKind::RBracket, "`]`")?;
                    return Ok(TypeCore::Decimal {
                        name,
                        scale,
                        scale_span,
                    });
                }
                // ADR 0139 rule 7: in edition 0.4 a scale is the syntax error edition 0.3
                // gives it, with the advice to raise the edition (#1743).
                if self.keywords == KeywordSet::V0_4
                    && self.interner.resolve(name.name) == "Decimal"
                    && self.at(&TokenKind::LBracket)
                    && matches!(self.peek_at(1), TokenKind::Int(_))
                {
                    self.advance();
                    return Err(self.unexpected_with(
                        "a type",
                        "`Decimal[s]` exists from edition 0.5 (ADR 0139): write `edition 0.5` in ita.una to use it",
                    ));
                }
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
                        clauses.push(FnTypeClause::Uses(self.uses_clause(false)?));
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

    /// The range of a type: in edition 0.5, a bound may name a module constant,
    /// `signed_int | lower_ident` (ADR 0126 rule 9).
    fn type_range(&mut self) -> Parse<Range> {
        let start = self.span();
        let low = self.type_bound()?;
        let exclusive = if self.eat(&TokenKind::DotDotLt) {
            true
        } else {
            self.expect(&TokenKind::DotDot, "`..` or `..<`")?;
            false
        };
        let high = self.type_bound()?;
        Ok(Range {
            low,
            high,
            exclusive,
            span: self.since(start),
        })
    }

    fn type_bound(&mut self) -> Parse<SignedInt> {
        if self.keywords >= KeywordSet::V0_5 && matches!(self.peek(), TokenKind::LowerIdent(_)) {
            let ident = self.lower_name("a constant")?;
            return Ok(SignedInt {
                negative: false,
                digits: String::new(),
                span: ident.span,
                constant: Some(ident),
            });
        }
        self.signed_int()
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
            constant: None,
        })
    }

    /// A `uses` clause. `conditions`: whether its operations may carry `where` and a condition,
    /// in the `uses` of a function or a handler in edition 0.3 (ADR 0092 rule 1). Elsewhere, and
    /// in a function type, `where` there is the error it was before (U0101).
    fn uses_clause(&mut self, conditions: bool) -> Parse<UsesClause> {
        let start = self.advance();
        let cap = self.cap_name()?;
        self.expect(&TokenKind::Colon, "`:` and the capability type")?;
        let cap_type = self.cap_type(conditions)?;
        Ok(UsesClause {
            cap,
            cap_type,
            span: self.since(start),
        })
    }

    /// A `uses` clause without its keyword, alone up to the end of the text.
    fn lone_uses_clause(&mut self) -> Parse<UsesClause> {
        let start = self.span();
        let cap = self.cap_name()?;
        self.expect(&TokenKind::Colon, "`:` and the capability type")?;
        let cap_type = self.cap_type(true)?;
        let span = self.since(start);
        while self.eat(&TokenKind::Newline) {}
        self.expect(&TokenKind::Eof, "the end of the clause")?;
        Ok(UsesClause {
            cap,
            cap_type,
            span,
        })
    }

    /// `header` says whether the clause is a function's or a handler's header's, which may
    /// condition and label its operations in edition 0.3; a function type's never does.
    fn cap_type(&mut self, header: bool) -> Parse<CapType> {
        let conditions = header && self.v0_3;
        let start = self.span();
        let name = self.upper_name("a capability name")?;
        let mut ops = Vec::new();
        let mut written = Vec::new();
        let mut gives = Vec::new();
        if self.eat(&TokenKind::LBracket) {
            loop {
                ops.push(self.lower_name("an operation name")?);
                // ADR 0094 rule 1: `gives` and a label, before the condition, in the `uses` of
                // a function's or a handler's header in edition 0.3. The word is a contextual
                // keyword here only (rule 6): before edition 0.3 it is U0103, and in a function
                // type, which carries no operation labels, U0101.
                gives.push(if self.at_word("gives") {
                    let span = self.span();
                    if !header {
                        return self.fail(
                            span,
                            "`gives` is not allowed in a function type's `uses`, which carries no operation labels",
                        );
                    }
                    if !self.v0_3 {
                        return Err(self.report_error(SyntaxError::new(
                            Code::U0103,
                            span,
                            "`gives` is a keyword of edition 0.3 (operation labels)",
                        )));
                    }
                    self.advance();
                    let Some(level) = level(self.peek()) else {
                        return Err(self.unexpected("a label after `gives`"));
                    };
                    Some(self.label(level)?)
                } else {
                    None
                });
                // An operation's condition ends at the first `,` or `]` outside brackets and
                // parentheses, which no expression holds at its top level (ADR 0092 rule 1).
                written.push(if conditions && self.eat_keyword(Keyword::Where) {
                    Some(self.line_expr()?)
                } else {
                    None
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            let expected = if conditions {
                "`,`, `gives`, `where` or `]`"
            } else {
                "`,` or `]`"
            };
            self.expect(&TokenKind::RBracket, expected)?;
        }
        Ok(CapType {
            name,
            ops,
            conditions: written,
            gives,
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
        let convention = self.convention();
        let name = self.binding_name("a parameter name", Binding::Param)?;
        self.expect(&TokenKind::Colon, "`:` and the parameter's type")?;
        let ty = self.ty()?;
        Ok(Param {
            convention,
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
        let (type_params, constraints) = if self.at(&TokenKind::LBracket) {
            self.constrained_params()?
        } else {
            (Vec::new(), Vec::new())
        };
        let params = self.params()?;
        self.expect(&TokenKind::Arrow, "`->` and the result type")?;
        let result = self.ty()?;
        let (clauses, body) = if self.eat(&TokenKind::Eq) {
            let expr = self.body_of(Self::line_expr)?;
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
                        ClauseKind::Uses(_) | ClauseKind::UsesOf(_) => 2,
                        ClauseKind::Fails(_) => 3,
                        // A header has at most one `fails` clause of each form (ADR 0062).
                        ClauseKind::FailsOf(_) => 4,
                        ClauseKind::Diverges => 5,
                        ClauseKind::DerivedFrom(_) => 6,
                    };
                    (rank, clause.keyword)
                })
                .collect();
            self.check_order(&ranked, &FUNCTION_CLAUSES);
            self.expect(&TokenKind::Eq, "`=` and the body")?;
            self.expect_line_end()?;
            (clauses, Body::Block(self.body_of(Self::indented_block)?))
        };
        Ok(DeclKind::Function(FunctionDecl {
            public,
            name,
            type_params,
            constraints,
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
            // Edition 0.2: `uses of p` and `fails of p` (ADR 0062). `of` is no keyword; it is
            // a word right after `uses` or `fails`.
            TokenKind::Keyword(Keyword::Uses) if self.v0_2 && self.next_is_word("of") => {
                self.advance();
                self.advance();
                ClauseKind::UsesOf(self.lower_name("a parameter of function type")?)
            }
            TokenKind::Keyword(Keyword::Uses) => ClauseKind::Uses(self.uses_clause(true)?),
            TokenKind::Keyword(Keyword::Fails) if self.v0_2 && self.next_is_word("of") => {
                self.advance();
                self.advance();
                ClauseKind::FailsOf(self.lower_name("a parameter of function type")?)
            }
            TokenKind::Keyword(Keyword::Fails) => {
                self.advance();
                ClauseKind::Fails(self.upper_name("an error kind")?)
            }
            // Edition 0.4: `diverges` (`spec/26 §5`), a reserved word elsewhere.
            TokenKind::LowerIdent(word)
                if self.v0_4 && self.interner.resolve(*word) == "diverges" =>
            {
                self.advance();
                ClauseKind::Diverges
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
            uses.push(self.uses_clause(true)?);
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

    /// `parse` run as a function body or a handler operation body.
    fn body_of<T>(&mut self, parse: fn(&mut Self) -> Parse<T>) -> Parse<T> {
        let outer = std::mem::replace(&mut self.in_body, true);
        let parsed = parse(self);
        self.in_body = outer;
        parsed
    }

    fn handler_op(&mut self) -> Parse<HandlerOp> {
        let start = self.expect_keyword(Keyword::Op)?;
        let name = self.lower_name("an operation name")?;
        self.expect(&TokenKind::LParen, "`(` and the parameter names")?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            params.push(self.binding_name("a parameter name", Binding::HandlerOp)?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.binding_name("a parameter name", Binding::HandlerOp)?);
            }
        }
        self.expect(&TokenKind::RParen, "`,` or `)`")?;
        self.expect(&TokenKind::Eq, "`=` and the body")?;
        let body = self.body_of(Self::inline_or_block)?;
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

    /// `[ "pub" ] "measure" upper_ident [ "=" unit_expr ]` (ADR 0104 rule 1).
    fn measure_decl(&mut self, public: bool) -> Parse<DeclKind> {
        self.advance();
        let name = self.upper_name("a unit's name")?;
        let unit = if self.eat(&TokenKind::Eq) {
            Some(self.unit_expr()?)
        } else {
            None
        };
        self.expect_line_end()?;
        Ok(DeclKind::Measure(MeasureDecl { public, name, unit }))
    }

    /// `[ "pub" ] "const" lower_ident ":" type "=" line_expr NEWLINE`, edition 0.5 (ADR 0126
    /// rule 1). The type is required: without it, U0101 at `=`.
    fn constant_decl(&mut self, public: bool) -> Parse<DeclKind> {
        self.advance();
        let name = self.binding_name("a constant's name", Binding::Let)?;
        if matches!(self.peek(), TokenKind::Eq) {
            let span = self.span();
            let written = self.interner.resolve(name.name).to_owned();
            return self.fail(
                span,
                format!(
                    "a module constant writes its type: `const {written}: Type = value` (ADR 0126 rule 1)"
                ),
            );
        }
        self.expect(&TokenKind::Colon, "`:` and the constant's type")?;
        let ty = self.ty()?;
        self.expect(&TokenKind::Eq, "`=` and the constant's value")?;
        let value = self.line_expr()?;
        self.expect_line_end()?;
        Ok(DeclKind::Const(ConstDecl {
            public,
            name,
            ty,
            value,
        }))
    }

    /// `unit_expr = unit_term { ( "*" | "/" ) unit_term }`, left to right (ADR 0104 rule 2).
    fn unit_expr(&mut self) -> Parse<UnitExpr> {
        let mut left = self.unit_term()?;
        loop {
            if self.eat(&TokenKind::Star) {
                left = UnitExpr::Mul(Box::new(left), Box::new(self.unit_term()?));
            } else if self.eat(&TokenKind::Slash) {
                left = UnitExpr::Div(Box::new(left), Box::new(self.unit_term()?));
            } else {
                return Ok(left);
            }
        }
    }

    /// `unit_term = upper_ident | "(" unit_expr ")" | integer_literal`.
    fn unit_term(&mut self) -> Parse<UnitExpr> {
        match self.peek() {
            TokenKind::UpperIdent(_) => Ok(UnitExpr::Name(self.upper_name("a unit")?)),
            TokenKind::Int(digits) => {
                let digits = digits.clone();
                let span = self.advance();
                Ok(UnitExpr::Factor { digits, span })
            }
            TokenKind::LParen => {
                let start = self.advance();
                let outer = self.nesting;
                self.deeper()?;
                let inner = self.unit_expr();
                self.nesting = outer;
                let inner = inner?;
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(UnitExpr::Paren(Box::new(inner), self.since(start)))
            }
            _ => Err(self.unexpected("a unit, a factor or `(`")),
        }
    }

    /// `example_decl`, or `test_decl` in edition 0.3 (`spec/02 §5`, ADR 0105): the same
    /// body under either keyword.
    fn example_decl(&mut self) -> Parse<DeclKind> {
        self.advance();
        let name = self.text_literal_only()?;
        self.expect_line_end()?;
        let lines = if self.v0_3 {
            "the indented test lines"
        } else {
            "the indented example lines"
        };
        self.expect(&TokenKind::Indent, lines)?;
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
        let mut stop_after = None;
        while !self.eat(&TokenKind::Dedent) {
            if self.at(&TokenKind::Eof) {
                break;
            }
            // Edition 0.4: the last line MAY be `stop after N` (ADR 0116 rule 19).
            if self.v0_4 && self.at_word("stop") && self.next_is_word("after") {
                stop_after = Some(self.stop_after()?);
                if !self.eat(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
                    let span = self.span();
                    let message = format!(
                        "`stop after` must be the last line of its {}",
                        self.test_word()
                    );
                    return self.fail(span, message);
                }
                break;
            }
            expectations.push(self.expectation()?);
        }
        Ok(DeclKind::Example(ExampleDecl {
            name,
            givens,
            action,
            expectations,
            stop_after,
        }))
    }

    /// `"stop" "after" integer_literal NEWLINE`, `N` from 1 to 2^63 - 1 (ADR 0116 rule 19).
    fn stop_after(&mut self) -> Parse<StopAfter> {
        let start = self.advance();
        self.advance();
        let TokenKind::Int(digits) = self.peek() else {
            let span = self.span();
            return self.fail(span, "`stop after` needs an integer literal of at least 1");
        };
        let iterations = digits
            .replace('_', "")
            .parse::<u64>()
            .ok()
            .filter(|n| (1..=i64::MAX.unsigned_abs()).contains(n));
        let literal = self.span();
        let Some(iterations) = iterations else {
            return self.fail(
                literal,
                "`stop after` needs an integer literal from 1 to 9223372036854775807",
            );
        };
        self.advance();
        let span = self.since(start);
        self.expect_line_end()?;
        Ok(StopAfter { iterations, span })
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
                // A body with a statement other than `let` is U2024, which the checker reports
                // wherever that statement stands, its last line included (`spec/19 §1` rule
                // 3, #928).
                Stmt::Together(together)
                    if last && together.body.iter().all(|s| matches!(s, Stmt::Let(_))) =>
                {
                    return self.fail(
                        first,
                        "a block cannot end with `together`; its last line is its value",
                    );
                }
                Stmt::BlockExpr(expr) if !last => {
                    let construct = match expr {
                        BlockExpr::When(_) => "a `when`",
                        BlockExpr::If(_) => "an `if` with blocks",
                        BlockExpr::TogetherFor(_) => "a `together for`",
                        BlockExpr::FirstOf(_) => "a `first of`",
                    };
                    // Edition 0.2 adds assignments, whose value may be a `block_expr` too
                    // (`spec/17 §1`, `spec/19 §1` rule 2).
                    let values = if self.v0_2 {
                        "the value of a `let` or of an assignment"
                    } else {
                        "the value of a `let`"
                    };
                    return self.fail(
                        first,
                        format!("{construct} must be the last line of its block or {values}"),
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
            TokenKind::Keyword(Keyword::Let | Keyword::Const | Keyword::Var) => {
                Ok(Stmt::Let(Box::new(self.let_stmt()?)))
            }
            TokenKind::Keyword(Keyword::When) => {
                Ok(Stmt::BlockExpr(BlockExpr::When(self.when_expr()?)))
            }
            // Edition 0.4: `for until c decreases m` (`spec/26 §4`); `for until in xs` is a loop
            // over `xs` with the variable `until`.
            TokenKind::Keyword(Keyword::For) if self.v0_4 && self.for_word("until") => {
                self.until_stmt()
            }
            // Edition 0.4: `for ever` (`spec/26 §5`); `for ever in xs` is a loop over `xs`.
            TokenKind::Keyword(Keyword::For) if self.v0_4 && self.for_word("ever") => {
                self.ever_stmt()
            }
            // Edition 0.4: `first of` alone on its line, a race (`spec/27 §6`); elsewhere
            // `first` is a name.
            TokenKind::LowerIdent(_) if self.at_first_of() => Ok(Stmt::BlockExpr(
                BlockExpr::FirstOf(Box::new(self.first_of()?)),
            )),
            // `on stop` after a loop that is not `for ever`, for U2034.
            TokenKind::LowerIdent(_) if self.v0_4 && self.at_on_stop() => {
                Ok(Stmt::OnStop(Box::new(self.on_stop()?)))
            }
            // Edition 0.4: `tasks` and `start` (`spec/27 §7`); elsewhere both words are names.
            TokenKind::LowerIdent(_) if self.at_tasks() => {
                Ok(Stmt::Tasks(Box::new(self.tasks_stmt()?)))
            }
            TokenKind::LowerIdent(_) if self.at_start() => {
                Ok(Stmt::Start(Box::new(self.start_stmt()?)))
            }
            // Edition 0.2: `for x in source` (`spec/17 §1`).
            TokenKind::Keyword(Keyword::For) if self.v0_2 => {
                Ok(Stmt::For(Box::new(self.for_stmt()?)))
            }
            // Edition 0.2: `together` with branches, or `together for` (`spec/19 §1`).
            TokenKind::Keyword(Keyword::Together) => {
                if matches!(self.peek_at(1), TokenKind::Keyword(Keyword::For)) {
                    Ok(Stmt::BlockExpr(BlockExpr::TogetherFor(Box::new(
                        self.together_for()?,
                    ))))
                } else {
                    Ok(Stmt::Together(Box::new(self.together_stmt()?)))
                }
            }
            TokenKind::Keyword(Keyword::If) => match self.if_statement()? {
                IfForm::Line(expr) => Ok(Stmt::Handled(self.handled_rest(expr)?)),
                IfForm::Block(block) => Ok(Stmt::BlockExpr(BlockExpr::If(block))),
                IfForm::Then(stmt) => Ok(Stmt::IfThen(Box::new(stmt))),
            },
            // Edition 0.4: `stop [when c]` and `skip` (`spec/26 §3`). Elsewhere the words are
            // identifiers.
            TokenKind::LowerIdent(word) if self.v0_4 && self.exit_word(*word).is_some() => {
                self.exit_stmt()
            }
            _ => {
                let expr = self.line_expr()?;
                // Edition 0.2: `place = value` (`spec/17 §1`).
                if self.v0_2 && self.at(&TokenKind::Eq) {
                    self.advance();
                    let value = self.let_value()?;
                    let span = expr.span.to(self.last_end.unwrap_or(expr.span));
                    return Ok(Stmt::Assign(Box::new(AssignStmt {
                        place: expr,
                        value,
                        span,
                    })));
                }
                Ok(Stmt::Handled(self.handled_rest(expr)?))
            }
        }
    }

    /// Whether the `for` at the cursor is followed by `word` used as a keyword: not followed
    /// by `in`, which makes it the loop variable (ADR 0116 rule 5).
    fn for_word(&self, word: &str) -> bool {
        matches!(self.peek_at(1), TokenKind::LowerIdent(next) if self.interner.resolve(*next) == word)
            && !matches!(self.peek_at(2), TokenKind::Keyword(Keyword::In))
    }

    /// Whether the cursor is at the word `within` of a deadline (`spec/26 §7`, `spec/27 §5`),
    /// which the lexer gives as a name only where a deadline may stand, in edition 0.4.
    fn at_within(&self) -> bool {
        self.v0_4
            && matches!(self.peek(), TokenKind::LowerIdent(word) if self.interner.resolve(*word) == "within")
    }

    /// Edition 0.4: whether the cursor is at a `tasks` statement (`spec/27 §7`, ADR 0117 rule
    /// 7): the word `tasks` followed by the end of its line and an indented block, or by `at`.
    /// Elsewhere `tasks` is a name, so a block may still end with it.
    fn at_tasks(&self) -> bool {
        self.v0_4
            && matches!(self.peek(), TokenKind::LowerIdent(word) if self.interner.resolve(*word) == "tasks")
            && (matches!(
                (self.peek_at(1), self.peek_at(2)),
                (TokenKind::Newline, TokenKind::Indent)
            ) || matches!(self.peek_at(1), TokenKind::LowerIdent(at) if self.interner.resolve(*at) == "at"))
    }

    /// Edition 0.4: whether the cursor is at a `start` statement (`spec/27 §7`): the word
    /// `start` followed by a name, a capability reference or `attempt`, which begin its
    /// expression. Such a line is no expression of any edition, so `start(x)`, `start = v`,
    /// `start.m()` and a final `start` stay names. It parses outside a `tasks` block too, where
    /// the checker reports it (U2035).
    fn at_start(&self) -> bool {
        self.v0_4
            && matches!(self.peek(), TokenKind::LowerIdent(word) if self.interner.resolve(*word) == "start")
            && matches!(
                self.peek_at(1),
                TokenKind::LowerIdent(_)
                    | TokenKind::UpperIdent(_)
                    | TokenKind::CapRef(_)
                    | TokenKind::Keyword(Keyword::Attempt)
            )
    }

    /// `"tasks" [ "at" "most" add_expr "at" "once" ] NEWLINE INDENT block DEDENT` (`spec/27
    /// §7`).
    fn tasks_stmt(&mut self) -> Parse<TasksStmt> {
        let keyword = self.advance();
        let bound = if self.at_word("at") {
            self.advance();
            self.expect_word("most")?;
            let bound = self.add_expr()?;
            self.expect_word("at")?;
            self.expect_word("once")?;
            Some(bound)
        } else {
            None
        };
        self.expect(&TokenKind::Newline, "end of line after `tasks`")?;
        let block = self.indented_block()?;
        Ok(TasksStmt {
            bound,
            block,
            keyword,
            span: self.since(keyword),
        })
    }

    /// `"start" line_expr [ "within" add_expr ] NEWLINE [ INDENT handling { handling } DEDENT ]`
    /// (`spec/27 §7`); a `with` clause parses too, for U2035.
    fn start_stmt(&mut self) -> Parse<StartStmt> {
        let keyword = self.advance();
        let expr = self.line_expr()?;
        let handled = self.handled_rest(expr)?;
        Ok(StartStmt {
            handled,
            keyword,
            span: self.since(keyword),
        })
    }

    /// Whether the cursor is at `on stop` alone on its line.
    fn at_on_stop(&self) -> bool {
        matches!(self.peek(), TokenKind::LowerIdent(on) if self.interner.resolve(*on) == "on")
            && matches!(self.peek_at(1), TokenKind::LowerIdent(stop) if self.interner.resolve(*stop) == "stop")
            && matches!(self.peek_at(2), TokenKind::Newline)
    }

    /// `"on" "stop" NEWLINE INDENT block DEDENT` (`spec/26 §5`).
    fn on_stop(&mut self) -> Parse<OnStop> {
        let start = self.advance();
        let stop = self.advance();
        let keyword = start.to(stop);
        self.expect(&TokenKind::Newline, "end of line")?;
        let block = self.indented_block()?;
        Ok(OnStop {
            block,
            keyword,
            span: self.since(start),
        })
    }

    /// `"for" "ever" [ "within" add_expr ] NEWLINE INDENT block DEDENT [ "on" "stop" NEWLINE
    /// INDENT block DEDENT ]` (`spec/26 §5`, §7, ADR 0116 rules 5, 10 and 11).
    fn ever_stmt(&mut self) -> Parse<Stmt> {
        let start = self.advance();
        self.advance();
        // Edition 0.4: `within d`, the deadline of each iteration (`spec/26 §7`).
        let within = match self.peek() {
            TokenKind::LowerIdent(word) if self.interner.resolve(*word) == "within" => {
                self.advance();
                Some(self.add_expr()?)
            }
            _ => None,
        };
        self.expect(&TokenKind::Newline, "end of line")?;
        let block = self.indented_block()?;
        let on_stop = if self.at_on_stop() {
            Some(self.on_stop()?)
        } else {
            None
        };
        Ok(Stmt::Ever(Box::new(EverStmt {
            within,
            block,
            on_stop,
            span: self.since(start),
        })))
    }

    /// `"for" "until" line_expr "decreases" add_expr NEWLINE INDENT block DEDENT` (`spec/26
    /// §4`, ADR 0116 rule 7). Without `decreases` it is U0101.
    fn until_stmt(&mut self) -> Parse<Stmt> {
        let start = self.advance();
        self.advance();
        let condition = self.line_expr()?;
        let decreases = match self.peek() {
            TokenKind::LowerIdent(word) if self.interner.resolve(*word) == "decreases" => {
                self.advance()
            }
            _ => {
                let span = self.span();
                return self.fail(span, "`for until` needs a `decreases` measure");
            }
        };
        let measure = self.add_expr()?;
        self.expect(&TokenKind::Newline, "end of line")?;
        let block = self.indented_block()?;
        Ok(Stmt::Until(Box::new(UntilStmt {
            condition,
            decreases,
            measure,
            block,
            span: self.since(start),
        })))
    }

    /// Whether `word`, the first word of a statement, starts `stop` (`Some(true)`) or `skip`
    /// (`Some(false)`): the word alone on its line, or followed by `when`.
    fn exit_word(&self, word: Symbol) -> Option<bool> {
        let stop = match self.interner.resolve(word) {
            "stop" => true,
            "skip" => false,
            _ => return None,
        };
        matches!(
            self.peek_at(1),
            TokenKind::Newline
                | TokenKind::Dedent
                | TokenKind::Eof
                | TokenKind::Keyword(Keyword::When)
        )
        .then_some(stop)
    }

    /// `"stop" [ "when" line_expr ] NEWLINE` or `"skip" NEWLINE`, in edition 0.4 (`spec/26 §3`,
    /// ADR 0116 rules 5 and 6). `skip` takes no `when` (U0101).
    fn exit_stmt(&mut self) -> Parse<Stmt> {
        let stop = match self.peek() {
            TokenKind::LowerIdent(word) => self.exit_word(*word) == Some(true),
            _ => false,
        };
        let keyword = self.advance();
        if !stop {
            if self.at_keyword(Keyword::When) {
                let span = self.span();
                return self.fail(span, "`skip` takes no `when`; write `if c` around `skip`");
            }
            self.expect_line_end()?;
            return Ok(Stmt::Skip(keyword));
        }
        let condition = if self.eat_keyword(Keyword::When) {
            Some(self.line_expr()?)
        } else {
            None
        };
        let span = self.since(keyword);
        self.expect_line_end()?;
        Ok(Stmt::Stop(Box::new(StopStmt {
            condition,
            keyword,
            span,
        })))
    }

    /// The value of a `let` or an assignment: a `when`, an `if`, a `together for`, or a
    /// handled line.
    fn let_value(&mut self) -> Parse<LetValue> {
        Ok(match self.peek() {
            TokenKind::Keyword(Keyword::Together) => {
                if matches!(self.peek_at(1), TokenKind::Keyword(Keyword::For)) {
                    LetValue::BlockExpr(BlockExpr::TogetherFor(Box::new(self.together_for()?)))
                } else {
                    // `spec/19 §1` rule 4: branches make a statement, not a value.
                    let span = self.span();
                    return Err(self.report(
                        span,
                        "`together` with branches is a statement; write `together for` to build a list"
                            .to_owned(),
                    ));
                }
            }
            TokenKind::Keyword(Keyword::When) => {
                LetValue::BlockExpr(BlockExpr::When(self.when_expr()?))
            }
            // Edition 0.4: `first of`, a race (`spec/27 §6`).
            TokenKind::LowerIdent(_) if self.at_first_of() => {
                LetValue::BlockExpr(BlockExpr::FirstOf(Box::new(self.first_of()?)))
            }
            TokenKind::Keyword(Keyword::If) => match self.if_statement()? {
                IfForm::Line(expr) => LetValue::Handled(self.handled_rest(expr)?),
                IfForm::Block(block) => LetValue::BlockExpr(BlockExpr::If(block)),
                IfForm::Then(stmt) => {
                    return Err(self.report(
                        stmt.span,
                        "an `if` that gives a value needs `else`".to_owned(),
                    ));
                }
            },
            _ => {
                let expr = self.line_expr()?;
                LetValue::Handled(self.handled_rest(expr)?)
            }
        })
    }

    /// A parameter convention or call-site marker, `edit` or `take`, in edition 0.2.
    fn convention(&mut self) -> Option<Convention> {
        if self.eat_keyword(Keyword::Edit) {
            Some(Convention::Edit)
        } else if self.eat_keyword(Keyword::Take) {
            Some(Convention::Take)
        } else {
            None
        }
    }

    /// A binding statement: `let` or `let edit` up to edition 0.4, `const` or `var` in edition
    /// 0.5 (`spec/02 §6`, ADR 0137 rule 2). The lexer reports a `let` of edition 0.5 (U7007),
    /// and the parser reads it as in edition 0.4.
    fn let_stmt(&mut self) -> Parse<LetStmt> {
        let word = self.peek().clone();
        let start = self.advance();
        let editable = match word {
            TokenKind::Keyword(Keyword::Const | Keyword::Var) => {
                if self.at_keyword(Keyword::Edit) {
                    let span = self.span();
                    let written = if word == TokenKind::Keyword(Keyword::Var) {
                        "var"
                    } else {
                        "const"
                    };
                    return self.fail(
                        span,
                        format!(
                            "a binding is `const` or `var`, without `edit`: write `var` for a binding that is reassigned, not `{written} edit`; `edit` marks parameters and arguments"
                        ),
                    );
                }
                word == TokenKind::Keyword(Keyword::Var)
            }
            _ => self.eat_keyword(Keyword::Edit),
        };
        let name = self.binding_name("a name", Binding::Let)?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq, "`=` and the value")?;
        let value = self.let_value()?;
        Ok(LetStmt {
            editable,
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
        // Edition 0.4: `within d`, the deadline of the expression (`spec/27 §5`). An `attempt`
        // with its own deadline takes no second one (U0101).
        let within = if self.at_within() {
            if let ExprKind::Attempt {
                within: Some(_), ..
            } = &expr.kind
            {
                let span = self.span();
                return self.fail(
                    span,
                    "this `attempt` has its deadline already; write one `within`, right after the attempted call",
                );
            }
            self.advance();
            Some(self.add_expr()?)
        } else {
            None
        };
        let span = self.since(start);
        if self.at_keyword(Keyword::If)
            && matches!(
                self.peek_at(1),
                TokenKind::Keyword(Keyword::Error | Keyword::None)
            )
        {
            // The clauses are reported with their repair, and the body goes on after the line,
            // so that every such line of a body is reported in one check (#268).
            let _ = self.same_line_handling(span);
            let mut nesting = 0usize;
            loop {
                match self.peek() {
                    TokenKind::Newline | TokenKind::Eof if nesting == 0 => break,
                    TokenKind::Eof => break,
                    TokenKind::LParen | TokenKind::LBracket => nesting += 1,
                    TokenKind::RParen | TokenKind::RBracket => {
                        nesting = nesting.saturating_sub(1);
                    }
                    _ => {}
                }
                self.advance();
            }
        }
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
            within,
            handlings,
            span,
        })
    }

    /// U0101 at a handling clause written on the line of its expression, which ends at
    /// `expr` (#211). The repair moves the clause to its own line, one level deeper than the
    /// line of the expression: each open block is two spaces in canonical form.
    fn same_line_handling(&mut self, expr: Span) -> Stop {
        let clause = self.span();
        let mut error = SyntaxError::new(
            Code::U0101,
            clause,
            "a handling clause goes on its own line, indented under the expression it handles",
        );
        // Every clause on the rest of the line goes to a line of its own, each one level deeper
        // than the expression's line, so that the edit leaves no clause on another's line
        // (#257).
        let indent = format!("\n{}", "  ".repeat(self.depth + 1));
        let mut starts = Vec::new();
        let mut end = clause.end;
        let mut nesting = 0usize;
        for (offset, token) in self
            .tokens
            .get(self.pos..)
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            match token.kind {
                TokenKind::Newline | TokenKind::Eof if nesting == 0 => break,
                TokenKind::LParen | TokenKind::LBracket => nesting += 1,
                TokenKind::RParen | TokenKind::RBracket => nesting = nesting.saturating_sub(1),
                TokenKind::Keyword(Keyword::If) if nesting == 0 => {
                    let next = self.tokens.get(self.pos + offset + 1).map(|t| &t.kind);
                    if matches!(
                        next,
                        Some(TokenKind::Keyword(Keyword::Error | Keyword::None))
                    ) {
                        starts.push(token.span.start);
                    }
                }
                _ => {}
            }
            if token.span.end > token.span.start {
                end = end.max(token.span.end);
            }
        }
        let mut replacement = String::new();
        for (i, start) in starts.iter().enumerate() {
            let stop = starts.get(i + 1).copied().unwrap_or(end);
            let text = self.text.get(*start..stop).unwrap_or("").trim_end();
            replacement.push_str(&indent);
            replacement.push_str(text);
        }
        let gap = Span {
            file: clause.file,
            start: expr.end,
            end,
        };
        error.repairs.push(SyntaxRepair {
            description: if starts.len() > 1 {
                "move each handling clause to its own line".to_owned()
            } else {
                "move the clause to its own line".to_owned()
            },
            edit: Some((gap, replacement)),
        });
        self.report_error(error)
    }

    fn cap_binding(&mut self) -> Parse<CapBinding> {
        let start = self.span();
        let cap = self.cap_name()?;
        self.expect(&TokenKind::Eq, "`=` and a capability or handler")?;
        let value = if matches!(self.peek(), TokenKind::CapRef(_)) {
            let target = self.cap_name()?;
            // A narrowed rebinding, `@y[ops]`, in edition 0.3 (ADR 0093 rule 4). Before it, the
            // `[` is the error it always was.
            if self.v0_3 && self.eat(&TokenKind::LBracket) {
                let mut ops = vec![self.lower_name("an operation name")?];
                while self.eat(&TokenKind::Comma) {
                    ops.push(self.lower_name("an operation name")?);
                }
                self.expect(&TokenKind::RBracket, "`,` or `]`")?;
                CapBindingValue::Narrowed { target, ops }
            } else {
                CapBindingValue::Cap(target)
            }
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
        if matches!(self.peek(), TokenKind::LowerIdent(_)) {
            return Err(self.unexpected_with(
                "a variant",
                "`fail` takes a variant of the error kind, as in `fail Broke`, not a stored value; fail where the error is found, or pass a call's failure on with `if error pass`",
            ));
        }
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
        let field = match self.error_field(true) {
            Some(field) => field,
            None => self.lower_name("a field name")?,
        };
        let local = if self.eat(&TokenKind::Colon) {
            self.no_nested_pattern(variant)?;
            Some(self.binding_name("a local name", Binding::Pattern)?)
        } else {
            None
        };
        Ok(Bind { field, local })
    }

    /// U0101 for a pattern where `spec/02 §6` allows only a name in a field of a variant
    /// pattern, such as `is Some(Circle(radius))` or `is Ok(value: is 0)` (benchmark finding
    /// B-001). D-155: a variant, `is`, `in` or a literal where a field or local name
    /// is expected starts a nested pattern; the repair has no mechanical edit, because moving the
    /// arm body under a second `when` also regroups the arms that share the outer variant. A
    /// literal is named as such, with a second `when` or a comparison in the arm as its repair
    /// (issue #73).
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
        // Issue #73 (UI-001): a literal is named as such. A second `when` matches it with a
        // literal arm and `else` (`spec/02 §6`), except a negative number, which a pattern
        // cannot write; a comparison in the arm works for every literal.
        let literal = match (self.peek(), self.peek_at(1)) {
            (TokenKind::Int(digits), _) => Some((digits.clone(), true)),
            (TokenKind::Minus, TokenKind::Int(digits)) => Some((format!("-{digits}"), false)),
            (TokenKind::Minus, _) => Some(("-...".to_owned(), false)),
            (TokenKind::Keyword(Keyword::True), _) => Some(("true".to_owned(), true)),
            (TokenKind::Keyword(Keyword::False), _) => Some(("false".to_owned(), true)),
            (TokenKind::TextStart, _) => Some(("\"...\"".to_owned(), true)),
            _ => None,
        };
        let bind = format!("bind the field to a name, as in `is {outer}(inner)`");
        let (message, descriptions) = match literal {
            Some((value, in_when)) => {
                let compare = format!(
                    "{bind}, then compare that name inside the arm, as in `if inner == {value} then ... else ...`"
                );
                let descriptions = if in_when {
                    vec![
                        format!(
                            "{bind}, then match that name in a second `when` inside the arm, with `is {value} -> ...` and `else -> ...`"
                        ),
                        compare,
                    ]
                } else {
                    vec![compare]
                };
                ("literal patterns are not supported in v0.1", descriptions)
            }
            None => (
                "nested patterns are not supported in v0.1",
                vec![format!(
                    "{bind}, then match that name in a second `when` inside the arm"
                )],
            ),
        };
        let mut error = SyntaxError::new(Code::U0101, span, message);
        for description in descriptions {
            error.repairs.push(SyntaxRepair {
                description,
                edit: None,
            });
        }
        Err(self.report_error(error))
    }

    /// `together` and its indented body, whose statements the checker examines (`spec/19 §1`).
    fn together_stmt(&mut self) -> Parse<TogetherStmt> {
        let keyword = self.advance();
        self.expect(&TokenKind::Newline, "end of line after `together`")?;
        let outer = self.nesting;
        self.deeper()?;
        let body = self.indented("the indented branches", Self::statement);
        self.nesting = outer;
        Ok(TogetherStmt {
            body: body?,
            keyword,
            span: self.since(keyword),
        })
    }

    /// Edition 0.4: whether the cursor is at `first of` followed by the end of its line, which
    /// opens a race (`spec/27 §6`, ADR 0117 rule 6). Elsewhere `first` is a name, so the
    /// prelude's `first` stays a method.
    fn at_first_of(&self) -> bool {
        self.v0_4
            && matches!(self.peek(), TokenKind::LowerIdent(word) if self.interner.resolve(*word) == "first")
            && matches!(self.peek_at(1), TokenKind::LowerIdent(word) if self.interner.resolve(*word) == "of")
            && matches!(self.peek_at(2), TokenKind::Newline)
    }

    /// `first of` and its indented branches, whose statements the checker examines (`spec/27
    /// §6`).
    fn first_of(&mut self) -> Parse<FirstOf> {
        let start = self.advance();
        let of = self.advance();
        let keyword = start.to(of);
        self.expect(&TokenKind::Newline, "end of line after `first of`")?;
        let outer = self.nesting;
        self.deeper()?;
        let body = self.indented("the indented branches", Self::statement);
        self.nesting = outer;
        Ok(FirstOf {
            body: body?,
            keyword,
            span: self.since(start),
        })
    }

    /// `together for x in xs` or `together for i in a..<b` with its block (`spec/19 §1`).
    fn together_for(&mut self) -> Parse<TogetherFor> {
        let keyword = self.advance();
        let for_stmt = self.for_stmt()?;
        Ok(TogetherFor {
            var: for_stmt.var,
            source: for_stmt.source,
            block: for_stmt.block,
            keyword,
            span: self.since(keyword),
        })
    }

    /// `for x in xs` or `for i in a..<b` with its block (`spec/17 §1`).
    fn for_stmt(&mut self) -> Parse<ForStmt> {
        let start = self.advance();
        let var = self.binding_name("the loop variable", Binding::Loop)?;
        self.expect_keyword(Keyword::In)?;
        let low = self.line_expr()?;
        let exclusive = if self.eat(&TokenKind::DotDotLt) {
            Some(true)
        } else if self.eat(&TokenKind::DotDot) {
            Some(false)
        } else {
            None
        };
        let source = match exclusive {
            Some(exclusive) => {
                let high = self.add_expr()?;
                ForSource::Range {
                    low,
                    high,
                    exclusive,
                }
            }
            None => ForSource::List(low),
        };
        self.expect(&TokenKind::Newline, "end of line")?;
        let block = self.indented_block()?;
        Ok(ForStmt {
            var,
            source,
            block,
            span: self.since(start),
        })
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
        // Edition 0.2: an `if` with a block and no `else` is a `Unit` statement.
        if self.v0_2 {
            let then_block = self.indented_block()?;
            if !self.at_keyword(Keyword::Else) {
                return Ok(IfForm::Then(IfThenStmt {
                    condition,
                    block: then_block,
                    span: self.since(start),
                }));
            }
            return Ok(IfForm::Block(
                self.if_block_else(start, condition, then_block)?,
            ));
        }
        Ok(IfForm::Block(self.if_block(start, condition)?))
    }

    /// `line_expr "else" line_expr`, after `if`, the condition and `then`.
    fn if_line_rest(&mut self, start: Span, condition: Expr) -> Parse<Expr> {
        let then = self.line_expr()?;
        if !self.at_keyword(Keyword::Else) {
            return Err(self.unexpected_with(
                "`else`",
                "an `if` with `then` needs `else`; to fail early, make the rest of the body its `else` branch, as in `if x < 0 then fail Broke else x`, or use the block form, whose `else` block holds the rest",
            ));
        }
        self.advance();
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
        self.if_block_else(start, condition, then_block)
    }

    /// The `else` part of an `if` with blocks, after its then-block.
    fn if_block_else(&mut self, start: Span, condition: Expr, then_block: Block) -> Parse<IfBlock> {
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
                // Edition 0.4: `attempt e within d` takes the deadline, so the `Result` holds the
                // time-out (`spec/27 §5`).
                let within = if self.at_within() {
                    self.advance();
                    Some(Box::new(self.add_expr()?))
                } else {
                    None
                };
                Ok(Expr {
                    kind: ExprKind::Attempt {
                        operand: Box::new(operand),
                        within,
                    },
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
        let name = self.binding_name("a parameter name", Binding::Lambda)?;
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
        // Edition 0.2: `edit` before a method call marks its receiver (`spec/17 §1`).
        if self.at_keyword(Keyword::Edit) {
            let start = self.advance();
            let mut expr = self.postfix()?;
            return match &mut expr.kind {
                ExprKind::MethodCall { receiver, .. } if receiver.is_none() => {
                    *receiver = Some(Convention::Edit);
                    expr.span = start.to(expr.span);
                    Ok(expr)
                }
                kind => {
                    let mut error = SyntaxError::new(
                        Code::U0101,
                        start,
                        "`edit` marks an argument, as in `f(edit x)`, or the receiver of a method call, as in `edit total.record(e)`",
                    );
                    // Before a call, the edit moves the marker to its first argument, as a
                    // receiver's marker stands for the first parameter (#403, D-8).
                    if let ExprKind::Call { args, .. } = kind
                        && let Some(first) = args.first()
                        && first.marker.is_none()
                    {
                        let at = first.span.start;
                        let replacement = format!(
                            "{}edit {}",
                            self.text.get(expr.span.start..at).unwrap_or(""),
                            self.text.get(at..expr.span.end).unwrap_or("")
                        );
                        error = error.with_edit(
                            "mark the first argument instead",
                            start.to(expr.span),
                            replacement,
                        );
                    }
                    Err(self.report_error(error))
                }
            };
        }
        let outer = self.nesting;
        let mut expr = self.primary()?;
        loop {
            if self.eat(&TokenKind::Dot) {
                self.deeper()?;
                let name = match self.error_field(false) {
                    Some(name) => name,
                    None => self.lower_name("a field or method name")?,
                };
                expr = if self.at(&TokenKind::LParen) {
                    let args = self.call_args()?;
                    let span = self.since(expr.span);
                    Expr {
                        kind: ExprKind::MethodCall {
                            receiver: None,
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
        // Edition 0.3: `from:` labels an argument, as in `x.copy_sign(from: y)` (`spec/29 §3`);
        // only here does the keyword `from` serve as a name.
        let from = match self.from_label {
            Some(symbol)
                if self.v0_3
                    && self.peek() == &TokenKind::Keyword(Keyword::From)
                    && self.peek_at(1) == &TokenKind::Colon =>
            {
                Some(symbol)
            }
            _ => None,
        };
        // Edition 0.5: `with:` and `and:` label the arguments of the dense methods
        // (`spec/29 §17`, ADR 0120); the lexer interns them only from edition 0.5.
        let from = from.or_else(|| match (self.peek(), self.peek_at(1)) {
            (TokenKind::Keyword(Keyword::With), TokenKind::Colon) => self.with_label,
            (TokenKind::Keyword(Keyword::And), TokenKind::Colon) => self.and_label,
            _ => None,
        });
        // ADR 0139 rule 7: in edition 0.4 `with:` and `and:` are the syntax error edition 0.3
        // gives them, with the advice to raise the edition (#1743).
        if from.is_none()
            && self.keywords == KeywordSet::V0_4
            && self.peek_at(1) == &TokenKind::Colon
            && let TokenKind::Keyword(keyword @ (Keyword::With | Keyword::And)) = self.peek()
        {
            let advice = format!(
                "the label `{}:` exists from edition 0.5 (ADR 0139): write `edition 0.5` in ita.una to use it",
                keyword.as_str()
            );
            return Err(self.unexpected_with("an expression", advice));
        }
        let labeled = matches!(
            self.peek(),
            TokenKind::LowerIdent(_) | TokenKind::Reserved(_)
        ) && self.peek_at(1) == &TokenKind::Colon;
        let label = if let Some(name) = from {
            let span = self.advance();
            self.advance();
            Some(Ident { name, span })
        } else if let Some(label) = self.error_field(true) {
            self.advance();
            Some(label)
        } else if labeled {
            let label = self.lower_name("an argument label")?;
            self.advance();
            Some(label)
        } else {
            None
        };
        // Edition 0.2: `edit place` or `take name` (`spec/17 §3`).
        let marker = self.convention();
        let value = if marker.is_some() {
            self.postfix()?
        } else {
            self.line_expr()?
        };
        Ok(Arg {
            label,
            marker,
            value,
            span: self.since(start),
        })
    }

    fn primary(&mut self) -> Parse<Expr> {
        let start = self.span();
        let kind = match self.peek() {
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::TextStart
            | TokenKind::Keyword(Keyword::True | Keyword::False | Keyword::Unit) => {
                ExprKind::Literal(self.literal()?)
            }
            TokenKind::LowerIdent(_) => ExprKind::Name(self.lower_name("a name")?),
            // Edition 0.2: `old(e)` (`spec/17 §3`).
            TokenKind::Keyword(Keyword::Old) => {
                self.advance();
                self.expect(&TokenKind::LParen, "`(` after `old`")?;
                let inner = self.line_expr()?;
                self.expect(&TokenKind::RParen, "`)`")?;
                ExprKind::Old(Box::new(inner))
            }
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
            TokenKind::Keyword(Keyword::When) => {
                return Err(self.unexpected_with(
                    "an expression",
                    "`when` must start its own line: write the long form, with `=` on its own line and the `when` indented below it, or bind it with `let`",
                ));
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
            TokenKind::Float(text) => Literal::Float(text.clone()),
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

/// Where a name is bound, for its repair when it is a reserved keyword (#206, #743).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    /// A `let` (or `let edit`) binding.
    Let,
    /// A parameter of a function, a predicate or a policy: callers name it.
    Param,
    /// A parameter of a handler operation.
    HandlerOp,
    /// A lambda's parameter.
    Lambda,
    /// A local name a pattern binds.
    Pattern,
    /// A loop variable.
    Loop,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Body, DeclKind, Stmt};
    use crate::source::SourceDb;

    fn parsed(text: &str, keywords: KeywordSet) -> (Lexed, Parsed) {
        let mut sources = SourceDb::new();
        let id = sources.add("m.una", text.as_bytes());
        let mut interner = Interner::new();
        let lexed = crate::lexer::lex_in(sources.file(id), id, &mut interner, keywords);
        let parsed = parse(&lexed, id, &interner);
        (lexed, parsed)
    }

    #[test]
    fn const_and_var_are_the_binding_statements_of_edition_0_5() {
        // ADR 0137 rule 2: `const x = e` is the `let x = e` of edition 0.4, and `var x = e` its
        // `let edit x = e`; the tree is the same.
        let text = "module m\n\nf() -> Int\n=\n  const a = 1\n  var b: Int = a\n  b\n";
        let (lexed, parsed) = parsed(text, KeywordSet::V0_5);
        assert!(lexed.errors.is_empty(), "{:?}", lexed.errors);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let DeclKind::Function(function) = &parsed.file.decls[0].kind else {
            panic!("a function");
        };
        let Body::Block(block) = &function.body else {
            panic!("a block");
        };
        let editable: Vec<(bool, bool)> = block
            .stmts
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::Let(binding) => Some((binding.editable, binding.ty.is_some())),
                _ => None,
            })
            .collect();
        assert_eq!(editable, [(false, false), (true, true)]);
    }

    #[test]
    fn const_gains_the_help_of_edition_0_5_before_it() {
        // ADR 0137 rule 1: in editions 0.1 to 0.4, `const x = 1` keeps its U0101 and gains the
        // help; another U0101 does not.
        let text = "module m\n\nf() -> Int\n=\n  const x = 1\n  x\n\ng() -> Int\n=\n  1 +\n";
        for keywords in [KeywordSet::V0_1, KeywordSet::V0_3, KeywordSet::V0_4] {
            let (_, parsed) = parsed(text, keywords);
            let helped: Vec<bool> = parsed
                .errors
                .iter()
                .map(|error| {
                    error
                        .repairs
                        .iter()
                        .any(|r| r.description == crate::keywords::BINDING_WORDS_HELP)
                })
                .collect();
            assert_eq!(helped, [true, false], "{keywords:?}: {:?}", parsed.errors);
            assert!(parsed.errors.iter().all(|e| e.code == Code::U0101));
        }
    }
}
