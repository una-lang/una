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
    /// `const`, from edition 0.5: an immutable binding (`spec/02 §6`, ADR 0137 rule 1).
    Const,
    /// `var`, from edition 0.5: a binding that can be assigned (`spec/17 §2`, ADR 0137 rule 1).
    Var,
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
            Keyword::Const => "const",
            Keyword::Var => "var",
        }
    }

    /// The keyword of [`Keyword::ALL`] spelled `word`, if any. A `match` on the word, which
    /// the compiler lowers to a switch on its length and a few comparisons, instead of a scan
    /// of the table with a comparison per keyword: the lexer asks this for every lowercase
    /// word, most of them names (#1894). A test checks it against the table.
    pub fn from_word(word: &str) -> Option<Keyword> {
        Some(match word {
            "accepts" => Keyword::Accepts,
            "and" => Keyword::And,
            "attempt" => Keyword::Attempt,
            "capability" => Keyword::Capability,
            "checked" => Keyword::Checked,
            "confidential" => Keyword::Confidential,
            "derived" => Keyword::Derived,
            "do" => Keyword::Do,
            "else" => Keyword::Else,
            "ensures" => Keyword::Ensures,
            "error" => Keyword::Error,
            "example" => Keyword::Example,
            "expect" => Keyword::Expect,
            "fail" => Keyword::Fail,
            "fails" => Keyword::Fails,
            "false" => Keyword::False,
            "fn" => Keyword::Fn,
            "for" => Keyword::For,
            "from" => Keyword::From,
            "given" => Keyword::Given,
            "handler" => Keyword::Handler,
            "if" => Keyword::If,
            "implies" => Keyword::Implies,
            "import" => Keyword::Import,
            "in" => Keyword::In,
            "internal" => Keyword::Internal,
            "is" => Keyword::Is,
            "it" => Keyword::It,
            "kind" => Keyword::Kind,
            "let" => Keyword::Let,
            "mod" => Keyword::Mod,
            "module" => Keyword::Module,
            "none" => Keyword::None,
            "not" => Keyword::Not,
            "op" => Keyword::Op,
            "opaque" => Keyword::Opaque,
            "or" => Keyword::Or,
            "pass" => Keyword::Pass,
            "performed" => Keyword::Performed,
            "policy" => Keyword::Policy,
            "predicate" => Keyword::Predicate,
            "pub" => Keyword::Pub,
            "public" => Keyword::Public,
            "reflect" => Keyword::Reflect,
            "requirement" => Keyword::Requirement,
            "requires" => Keyword::Requires,
            "restricted" => Keyword::Restricted,
            "result" => Keyword::Result,
            "secret" => Keyword::Secret,
            "then" => Keyword::Then,
            "true" => Keyword::True,
            "trusted" => Keyword::Trusted,
            "unit" => Keyword::Unit,
            "use" => Keyword::Use,
            "uses" => Keyword::Uses,
            "when" => Keyword::When,
            "where" => Keyword::Where,
            "with" => Keyword::With,
            _ => return None,
        })
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

    /// The keywords that edition 0.5 adds to those of edition 0.4 (`spec/01 §5`, ADR 0137 rule
    /// 1): `const` and `var`, the binding words. `let` stays a keyword token there, so that the
    /// parser reads the binding, and the lexer reports each one with U7007 and its rewrite.
    pub const EDITION_0_5: [Keyword; 2] = [Keyword::Const, Keyword::Var];

    /// The keywords of [`Keyword::ALL`] that edition 0.4 removes: `example`, an identifier there
    /// (ADR 0105, amendment of 2026-11-05).
    pub const REMOVED_IN_0_4: [Keyword; 1] = [Keyword::Example];

    /// The keyword `word` is under the keyword set `set`.
    pub fn from_word_in(word: &str, set: KeywordSet) -> Option<Keyword> {
        Keyword::from_word(word)
            .filter(|keyword| {
                !(set >= KeywordSet::V0_4 && Keyword::REMOVED_IN_0_4.contains(keyword))
            })
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
            .or_else(|| {
                (set >= KeywordSet::V0_5)
                    .then(|| {
                        Keyword::EDITION_0_5
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
    /// (`spec/26 §1`, ADR 0116 rule 0), and `example` an identifier (ADR 0105, amendment of 2026-11-05).
    V0_4,
    /// Edition 0.5: the keywords of edition 0.4, with `simd` no longer reserved and `with` and
    /// `and` argument labels of the dense methods (ADR 0120 rule 1, which ADR 0139 rule 6
    /// moves here from edition 0.4); `const` and `var` keywords, and `let` reserved with U7007
    /// (ADR 0137 rule 1).
    V0_5,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_word_is_the_table_of_all_keywords() {
        // #1894: `from_word` matches on the word; it must agree with a scan of `ALL` on every
        // keyword, on the keywords of later editions (not in `ALL`, so `None`), and on words one
        // character away from a keyword.
        let scan = |word: &str| Keyword::ALL.into_iter().find(|k| k.as_str() == word);
        let mut words: Vec<String> = Vec::new();
        for keyword in Keyword::ALL
            .iter()
            .chain(&Keyword::EDITION_0_2)
            .chain(&Keyword::EDITION_0_3)
            .chain(&Keyword::EDITION_0_5)
        {
            let text = keyword.as_str();
            words.push(text.to_owned());
            words.push(format!("{text}s"));
            words.push(format!("_{text}"));
            words.push(text[..text.len() - 1].to_owned());
            words.push(text.to_ascii_uppercase());
        }
        words.extend(["", "_", "x", "the_table"].map(str::to_owned));
        for word in &words {
            assert_eq!(Keyword::from_word(word), scan(word), "{word:?}");
        }
        for keyword in Keyword::ALL {
            assert_eq!(Keyword::from_word(keyword.as_str()), Some(keyword));
        }
        for keyword in Keyword::EDITION_0_2
            .iter()
            .chain(&Keyword::EDITION_0_3)
            .chain(&Keyword::EDITION_0_5)
        {
            assert_eq!(Keyword::from_word(keyword.as_str()), None, "{keyword:?}");
        }
    }

    #[test]
    fn const_and_var_are_keywords_of_edition_0_5_only() {
        // ADR 0137 rule 1: `const` and `var` are keywords in edition 0.5, and `let` stays the
        // keyword token the lexer reports with U7007; in editions 0.1 to 0.4 nothing changes.
        for set in [
            KeywordSet::V0_1,
            KeywordSet::V0_2,
            KeywordSet::V0_3,
            KeywordSet::V0_4,
        ] {
            assert_eq!(Keyword::from_word_in("const", set), None);
            assert_eq!(Keyword::from_word_in("var", set), None);
            assert_eq!(Keyword::from_word_in("let", set), Some(Keyword::Let));
        }
        assert_eq!(
            Keyword::from_word_in("const", KeywordSet::V0_5),
            Some(Keyword::Const)
        );
        assert_eq!(
            Keyword::from_word_in("var", KeywordSet::V0_5),
            Some(Keyword::Var)
        );
        assert_eq!(
            Keyword::from_word_in("let", KeywordSet::V0_5),
            Some(Keyword::Let)
        );
    }

    #[test]
    fn example_is_a_keyword_through_edition_0_3_and_an_identifier_from_0_4() {
        // ADR 0105 rule 2 and its amendment of 2026-11-05.
        for set in [KeywordSet::V0_1, KeywordSet::V0_2, KeywordSet::V0_3] {
            assert_eq!(
                Keyword::from_word_in("example", set),
                Some(Keyword::Example)
            );
        }
        assert_eq!(Keyword::from_word_in("example", KeywordSet::V0_4), None);
        // `test` stays the keyword of the test declaration in edition 0.4.
        assert_eq!(
            Keyword::from_word_in("test", KeywordSet::V0_4),
            Some(Keyword::Test)
        );
        assert_eq!(Keyword::from_word_in("test", KeywordSet::V0_2), None);
        // The other keywords of edition 0.1 are unchanged in edition 0.4.
        for keyword in Keyword::ALL {
            if keyword != Keyword::Example {
                assert_eq!(
                    Keyword::from_word_in(keyword.as_str(), KeywordSet::V0_4),
                    Some(keyword)
                );
            }
        }
    }
}
