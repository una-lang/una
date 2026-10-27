//! Spacing between the tokens of a line (ADR 0020, rule 3).

use una_syntax::{Keyword, TokenKind};

/// Whether a token ends an operand, which makes a following `-` binary.
fn ends_operand(kind: &TokenKind) -> bool {
    match kind {
        TokenKind::LowerIdent(_)
        | TokenKind::UpperIdent(_)
        | TokenKind::Reserved(_)
        | TokenKind::CapRef(_)
        | TokenKind::Int(_)
        | TokenKind::Float(_)
        | TokenKind::TextEnd
        | TokenKind::RParen
        | TokenKind::RBracket => true,
        TokenKind::Keyword(keyword) => matches!(
            keyword,
            Keyword::True
                | Keyword::False
                | Keyword::Unit
                | Keyword::Result
                | Keyword::It
                | Keyword::Error
        ),
        _ => false,
    }
}

fn is_level(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Keyword(
            Keyword::Public
                | Keyword::Internal
                | Keyword::Confidential
                | Keyword::Secret
                | Keyword::Restricted
        )
    )
}

/// The spacing context of one line.
pub(crate) struct Spacer {
    /// Whether the line is an import, which keeps a space before its `(`.
    import: bool,
    /// Whether the previous token is a unary `-`.
    unary: bool,
}

impl Spacer {
    /// The context of a line starting with `first`.
    pub(crate) fn new(first: &TokenKind) -> Spacer {
        Spacer {
            import: matches!(first, TokenKind::Keyword(Keyword::Import)),
            unary: matches!(first, TokenKind::Minus),
        }
    }

    /// Whether one space separates `previous` from `next`, the next token of the line.
    pub(crate) fn space(&mut self, previous: &TokenKind, next: &TokenKind) -> bool {
        let space = self.decide(previous, next);
        self.unary = matches!(next, TokenKind::Minus) && !ends_operand(previous);
        space
    }

    fn decide(&self, previous: &TokenKind, next: &TokenKind) -> bool {
        use TokenKind as T;
        match (previous, next) {
            // Inside a text literal, outside its interpolations.
            (T::TextStart | T::TextChunk(_), _) | (_, T::TextChunk(_) | T::TextEnd) => false,
            (T::InterpEnd, T::InterpStart) | (_, T::InterpStart) => false,
            (T::InterpStart, _) | (_, T::InterpEnd) => false,
            (T::LParen | T::LBracket | T::Dot, _) => false,
            (_, T::RParen | T::RBracket | T::Comma | T::Colon | T::Dot) => false,
            (T::DotDot | T::DotDotLt, _) | (_, T::DotDot | T::DotDotLt) => false,
            (T::Minus, T::Minus) => true,
            (T::Minus, _) if self.unary => false,
            (T::LowerIdent(_), T::LParen) if self.import => true,
            (
                T::LowerIdent(_)
                | T::UpperIdent(_)
                | T::Reserved(_)
                | T::RParen
                | T::RBracket
                | T::Keyword(Keyword::Fn | Keyword::Checked | Keyword::Old),
                T::LParen,
            ) => false,
            (T::LowerIdent(_) | T::UpperIdent(_) | T::Reserved(_), T::LBracket) => false,
            // A narrowed rebinding, `@x = @y[ops]` (edition 0.3).
            (T::CapRef(_), T::LBracket) => false,
            (level, T::LBracket) if is_level(level) => false,
            _ => true,
        }
    }
}
