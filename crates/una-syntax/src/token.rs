//! Tokens produced by the lexer (`spec/01`).

use crate::intern::{Interner, Symbol};
use crate::source::Span;

/// A reserved keyword (`spec/01 §5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Keyword {
    /// `accepts`
    Accepts,
    /// `and`
    And,
    /// `attempt`
    Attempt,
    /// `capability`
    Capability,
    /// `checked`
    Checked,
    /// `confidential`
    Confidential,
    /// `derived`
    Derived,
    /// `do`
    Do,
    /// `else`
    Else,
    /// `ensures`
    Ensures,
    /// `error`
    Error,
    /// `example`
    Example,
    /// `expect`
    Expect,
    /// `fail`
    Fail,
    /// `fails`
    Fails,
    /// `false`
    False,
    /// `fn`
    Fn,
    /// `for`
    For,
    /// `from`
    From,
    /// `given`
    Given,
    /// `handler`
    Handler,
    /// `if`
    If,
    /// `implies`
    Implies,
    /// `import`
    Import,
    /// `in`
    In,
    /// `internal`
    Internal,
    /// `is`
    Is,
    /// `it`
    It,
    /// `kind`
    Kind,
    /// `let`
    Let,
    /// `mod`
    Mod,
    /// `module`
    Module,
    /// `none`
    None,
    /// `not`
    Not,
    /// `op`
    Op,
    /// `opaque`
    Opaque,
    /// `or`
    Or,
    /// `pass`
    Pass,
    /// `performed`
    Performed,
    /// `policy`
    Policy,
    /// `predicate`
    Predicate,
    /// `pub`
    Pub,
    /// `public`
    Public,
    /// `reflect`
    Reflect,
    /// `requirement`
    Requirement,
    /// `requires`
    Requires,
    /// `restricted`
    Restricted,
    /// `result`
    Result,
    /// `secret`
    Secret,
    /// `then`
    Then,
    /// `true`
    True,
    /// `trusted`
    Trusted,
    /// `unit`
    Unit,
    /// `use`
    Use,
    /// `uses`
    Uses,
    /// `when`
    When,
    /// `where`
    Where,
    /// `with`
    With,
    /// `edit`, from edition 0.2 (`spec/17`).
    Edit,
    /// `take`, from edition 0.2 (`spec/17`).
    Take,
    /// `old`, from edition 0.2 (`spec/17`).
    Old,
    /// `together`, from edition 0.2 (`spec/19`).
    Together,
    /// `test`, from edition 0.3: the keyword of the test declaration (`spec/08 §2`, ADR 0105).
    Test,
}

impl Keyword {
    /// Every reserved keyword, in the order of `spec/01 §5`.
    pub const ALL: [Keyword; 58] = [
        Keyword::Accepts,
        Keyword::And,
        Keyword::Attempt,
        Keyword::Capability,
        Keyword::Checked,
        Keyword::Confidential,
        Keyword::Derived,
        Keyword::Do,
        Keyword::Else,
        Keyword::Ensures,
        Keyword::Error,
        Keyword::Example,
        Keyword::Expect,
        Keyword::Fail,
        Keyword::Fails,
        Keyword::False,
        Keyword::Fn,
        Keyword::For,
        Keyword::From,
        Keyword::Given,
        Keyword::Handler,
        Keyword::If,
        Keyword::Implies,
        Keyword::Import,
        Keyword::In,
        Keyword::Internal,
        Keyword::Is,
        Keyword::It,
        Keyword::Kind,
        Keyword::Let,
        Keyword::Mod,
        Keyword::Module,
        Keyword::None,
        Keyword::Not,
        Keyword::Op,
        Keyword::Opaque,
        Keyword::Or,
        Keyword::Pass,
        Keyword::Performed,
        Keyword::Policy,
        Keyword::Predicate,
        Keyword::Pub,
        Keyword::Public,
        Keyword::Reflect,
        Keyword::Requirement,
        Keyword::Requires,
        Keyword::Restricted,
        Keyword::Result,
        Keyword::Secret,
        Keyword::Then,
        Keyword::True,
        Keyword::Trusted,
        Keyword::Unit,
        Keyword::Use,
        Keyword::Uses,
        Keyword::When,
        Keyword::Where,
        Keyword::With,
    ];

    /// The keyword as written.
    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::Accepts => "accepts",
            Keyword::And => "and",
            Keyword::Attempt => "attempt",
            Keyword::Capability => "capability",
            Keyword::Checked => "checked",
            Keyword::Confidential => "confidential",
            Keyword::Derived => "derived",
            Keyword::Do => "do",
            Keyword::Else => "else",
            Keyword::Ensures => "ensures",
            Keyword::Error => "error",
            Keyword::Example => "example",
            Keyword::Expect => "expect",
            Keyword::Fail => "fail",
            Keyword::Fails => "fails",
            Keyword::False => "false",
            Keyword::Fn => "fn",
            Keyword::For => "for",
            Keyword::From => "from",
            Keyword::Given => "given",
            Keyword::Handler => "handler",
            Keyword::If => "if",
            Keyword::Implies => "implies",
            Keyword::Import => "import",
            Keyword::In => "in",
            Keyword::Internal => "internal",
            Keyword::Is => "is",
            Keyword::It => "it",
            Keyword::Kind => "kind",
            Keyword::Let => "let",
            Keyword::Mod => "mod",
            Keyword::Module => "module",
            Keyword::None => "none",
            Keyword::Not => "not",
            Keyword::Op => "op",
            Keyword::Opaque => "opaque",
            Keyword::Or => "or",
            Keyword::Pass => "pass",
            Keyword::Performed => "performed",
            Keyword::Policy => "policy",
            Keyword::Predicate => "predicate",
            Keyword::Pub => "pub",
            Keyword::Public => "public",
            Keyword::Reflect => "reflect",
            Keyword::Requirement => "requirement",
            Keyword::Requires => "requires",
            Keyword::Restricted => "restricted",
            Keyword::Result => "result",
            Keyword::Secret => "secret",
            Keyword::Then => "then",
            Keyword::True => "true",
            Keyword::Trusted => "trusted",
            Keyword::Unit => "unit",
            Keyword::Use => "use",
            Keyword::Uses => "uses",
            Keyword::When => "when",
            Keyword::Where => "where",
            Keyword::With => "with",
            Keyword::Edit => "edit",
            Keyword::Take => "take",
            Keyword::Old => "old",
            Keyword::Together => "together",
            Keyword::Test => "test",
        }
    }

    /// The keyword spelled `word`, if any.
    pub fn from_word(word: &str) -> Option<Keyword> {
        Keyword::ALL
            .into_iter()
            .find(|keyword| keyword.as_str() == word)
    }

    /// The keywords that edition 0.2 adds to [`Keyword::ALL`] (`spec/17 §9`, ADR 0056;
    /// `spec/19 §9`, ADR 0065).
    pub const EDITION_0_2: [Keyword; 4] = [
        Keyword::Edit,
        Keyword::Take,
        Keyword::Old,
        Keyword::Together,
    ];

    /// The keywords that edition 0.3 adds to those of edition 0.2 (`spec/01 §5`, ADR 0105
    /// rule 2): `test`, the keyword of the test declaration.
    pub const EDITION_0_3: [Keyword; 1] = [Keyword::Test];

    /// The keyword `word` is under the keyword set `set`.
    pub fn from_word_in(word: &str, set: KeywordSet) -> Option<Keyword> {
        Keyword::from_word(word)
            .or_else(|| {
                (set >= KeywordSet::V0_2)
                    .then(|| {
                        Keyword::EDITION_0_2
                            .into_iter()
                            .find(|keyword| keyword.as_str() == word)
                    })
                    .flatten()
            })
            .or_else(|| {
                (set >= KeywordSet::V0_3)
                    .then(|| {
                        Keyword::EDITION_0_3
                            .into_iter()
                            .find(|keyword| keyword.as_str() == word)
                    })
                    .flatten()
            })
    }
}

/// The keywords and reserved words of a package's edition (`spec/01 §5`, `spec/17 §9`).
/// Editions are cumulative (`spec/13 §7`): a later set holds every keyword of an earlier one,
/// so the sets are ordered, oldest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeywordSet {
    /// Edition 0.1: the tables of `spec/01 §5`.
    #[default]
    V0_1,
    /// Edition 0.2: `edit`, `take`, `old` and `together` are keywords, and the rows of
    /// `spec/17 §9` and `spec/19 §9` apply.
    V0_2,
    /// Edition 0.3: the keywords of edition 0.2 (`spec/13 §7`), and `test` (ADR 0105).
    V0_3,
    /// Edition 0.4: the keywords of edition 0.3, with `diverges` and `within` reserved
    /// (`spec/26 §1`, ADR 0116 rule 0), and `simd` no longer reserved (ADR 0120 rule 1).
    V0_4,
}

/// The kind of a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// A reserved keyword.
    Keyword(Keyword),
    /// A `lower_ident` that is not a keyword; contextual keywords are identifiers here.
    LowerIdent(Symbol),
    /// An `upper_ident`.
    UpperIdent(Symbol),
    /// A word reserved for a later version (U0103), read as a name for recovery.
    Reserved(Symbol),
    /// A capability reference `@name`.
    CapRef(Symbol),
    /// An integer literal: its decimal digits, without underscores or leading zeros.
    Int(String),
    /// A float literal of edition 0.3, as written, or its accepted form after U0007
    /// (`spec/29`).
    Float(String),
    /// A version, as written, in the project files of edition 0.4 only (ADR 0122 rule 4):
    /// `1.4`, `0.1.0` or `1.0.0-rc.1`. Module sources never produce it.
    Version(String),
    /// The opening quote of a text literal.
    TextStart,
    /// A run of literal text, with escapes decoded.
    TextChunk(String),
    /// The `{` that starts an interpolation.
    InterpStart,
    /// The `}` that ends an interpolation.
    InterpEnd,
    /// The closing quote of a text literal; empty when it is missing.
    TextEnd,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// `->`
    Arrow,
    /// `=`
    Eq,
    /// `|`
    Bar,
    /// `.`
    Dot,
    /// `..`
    DotDot,
    /// `..<`
    DotDotLt,
    /// `==`
    EqEq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// The end of a logical line.
    Newline,
    /// One indentation level deeper.
    Indent,
    /// One indentation level shallower.
    Dedent,
    /// The end of the file.
    Eof,
}

impl TokenKind {
    /// How the token is named in messages, for example `` `pub` `` or `end of line`.
    pub fn describe(&self, interner: &Interner) -> String {
        match self {
            TokenKind::Keyword(keyword) => format!("`{}`", keyword.as_str()),
            TokenKind::LowerIdent(name)
            | TokenKind::UpperIdent(name)
            | TokenKind::Reserved(name) => {
                format!("`{}`", interner.resolve(*name))
            }
            TokenKind::CapRef(name) => format!("`@{}`", interner.resolve(*name)),
            TokenKind::Int(digits) | TokenKind::Float(digits) => format!("`{digits}`"),
            TokenKind::Version(text) => format!("version `{text}`"),
            TokenKind::TextStart | TokenKind::TextChunk(_) | TokenKind::TextEnd => {
                "text literal".to_owned()
            }
            TokenKind::InterpStart => "`{`".to_owned(),
            TokenKind::InterpEnd => "`}`".to_owned(),
            TokenKind::Newline => "end of line".to_owned(),
            TokenKind::Indent => "indentation".to_owned(),
            TokenKind::Dedent => "end of block".to_owned(),
            TokenKind::Eof => "end of file".to_owned(),
            punctuation => format!("`{}`", punctuation.symbol()),
        }
    }

    /// The spelling of a punctuation token; empty for other tokens.
    pub fn symbol(&self) -> &'static str {
        match self {
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::Comma => ",",
            TokenKind::Colon => ":",
            TokenKind::Arrow => "->",
            TokenKind::Eq => "=",
            TokenKind::Bar => "|",
            TokenKind::Dot => ".",
            TokenKind::DotDot => "..",
            TokenKind::DotDotLt => "..<",
            TokenKind::EqEq => "==",
            TokenKind::NotEq => "!=",
            TokenKind::Lt => "<",
            TokenKind::Le => "<=",
            TokenKind::Gt => ">",
            TokenKind::Ge => ">=",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            _ => "",
        }
    }
}

/// A token and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// Where it is; empty for tokens the lexer inserts during recovery.
    pub span: Span,
}

/// A comment, kept as trivia for the formatter (`spec/01 §3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comment {
    /// The comment, from `--` to the end of its line.
    pub span: Span,
    /// Whether it is a doc comment (`---`).
    pub doc: bool,
}
