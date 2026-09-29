//! Logical lines (`spec/01 §2`) and the source lines they come from.

use una_syntax::{Token, TokenKind};

/// A logical line: the tokens between two line breaks, lines continued inside brackets
/// included.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Line {
    /// The index of its first token.
    pub(crate) first: usize,
    /// The index of its last token.
    pub(crate) last: usize,
    /// Its indentation level.
    pub(crate) depth: usize,
}

/// The logical lines of a token stream, in order.
pub(crate) fn logical_lines(tokens: &[Token]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<Line> = None;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Indent => depth += 1,
            TokenKind::Dedent => depth = depth.saturating_sub(1),
            TokenKind::Newline | TokenKind::Eof => lines.extend(current.take()),
            _ => match current.as_mut() {
                Some(line) => line.last = index,
                None => {
                    current = Some(Line {
                        first: index,
                        last: index,
                        depth,
                    });
                }
            },
        }
    }
    lines.extend(current);
    lines
}

/// Offsets and blank lines of the source text, by line.
pub(crate) struct SourceLines<'a> {
    text: &'a str,
    starts: &'a [usize],
}

impl<'a> SourceLines<'a> {
    pub(crate) fn new(text: &'a str, starts: &'a [usize]) -> SourceLines<'a> {
        SourceLines { text, starts }
    }

    /// The 0-based line of `offset`.
    pub(crate) fn line(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next.saturating_sub(1),
        }
    }

    /// The column of `offset` in bytes from the start of its line.
    pub(crate) fn column(&self, offset: usize) -> usize {
        let start = self.starts.get(self.line(offset)).copied().unwrap_or(0);
        offset.saturating_sub(start)
    }

    fn is_blank(&self, line: usize) -> bool {
        let Some(&start) = self.starts.get(line) else {
            return false;
        };
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text
            .get(start..end)
            .is_some_and(|content| content.trim().is_empty())
    }

    /// How many blank lines come right before `line`.
    pub(crate) fn blank_before(&self, line: usize) -> usize {
        (0..line)
            .rev()
            .take_while(|&previous| self.is_blank(previous))
            .count()
    }
}
