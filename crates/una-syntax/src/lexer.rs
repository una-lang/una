//! The lexer (`spec/01`): tokens, layout (`NEWLINE`, `INDENT`, `DEDENT`), comments as trivia,
//! and the lexical errors U0001 to U0007, U0103 and bracket balance (U0101).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;

use crate::error::{SyntaxError, SyntaxRepair};
use crate::intern::Interner;
use crate::keywords::reserved_for_later;
use crate::source::{FileId, SourceFile, Span};
use crate::token::{Comment, Keyword, Token, TokenKind};

/// The result of lexing one file.
#[derive(Clone, Debug, Default)]
pub struct Lexed {
    /// The tokens, ending with [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Every comment, in source order.
    pub comments: Vec<Comment>,
    /// The lexical errors, in the order they were found.
    pub errors: Vec<SyntaxError>,
}

/// Lexes `source`, which is the file `file` of its database.
pub fn lex(source: &SourceFile, file: FileId, interner: &mut Interner) -> Lexed {
    let mut lexer = Lexer {
        source,
        text: source.text(),
        file,
        interner,
        pos: 0,
        out: Lexed::default(),
        indents: vec![0],
        brackets: Vec::new(),
        line_has_tokens: false,
        renames: Vec::new(),
    };
    lexer.run();
    lexer.name_renames();
    lexer.out
}

/// Keywords that start a declaration at column 1 (`compiler/architecture.md §5`).
const DECLARATION_KEYWORDS: [&str; 11] = [
    "capability",
    "example",
    "handler",
    "import",
    "kind",
    "module",
    "policy",
    "predicate",
    "pub",
    "requirement",
    "trusted",
];

/// Whether a lower-case word at column 1 starts a declaration: a declaration keyword, or a
/// function name followed by `(`, or by `[` for a generic function.
pub(crate) fn starts_declaration(word: &str, next: Option<char>) -> bool {
    DECLARATION_KEYWORDS.contains(&word) || matches!(next, Some('(' | '['))
}

/// Why a character is not allowed anywhere, including text literals and comments (ADR 0017).
fn forbidden(source: &SourceFile, offset: usize, c: char) -> Option<String> {
    if source.is_invalid_sequence(offset) {
        return Some("invalid UTF-8 byte sequence".to_owned());
    }
    if c == '\r' {
        return Some("carriage return not followed by a line feed".to_owned());
    }
    let code = u32::from(c);
    if (c < ' ' && c != '\t' && c != '\n') || c == '\u{7f}' {
        return Some(format!(
            "control character U+{code:04X} is not allowed; text literals can write it as `\\u{{{code:X}}}`"
        ));
    }
    if matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
        return Some(format!(
            "bidirectional control character U+{code:04X} is not allowed; text literals can write it as `\\u{{{code:X}}}`"
        ));
    }
    None
}

/// The hint of a character that cannot start a token (ADR 0017, decision 3).
fn invalid_character_hint(c: char) -> Option<&'static str> {
    match c {
        ';' => Some("statements end at line breaks"),
        '!' => Some("use `not`"),
        '&' => Some("use `and`"),
        '\'' => Some("text literals use double quotes"),
        '_' => Some("names start with a letter"),
        _ => None,
    }
}

/// Whether an integer literal as written is malformed (ADR 0017, decision 1).
fn is_malformed_integer(raw: &str) -> bool {
    let digits: String = raw.chars().filter(|c| *c != '_').collect();
    raw.starts_with('_')
        || raw.ends_with('_')
        || raw.contains("__")
        || (digits.len() > 1 && digits.starts_with('0'))
}

/// The canonical spelling of an integer literal: no leading or trailing underscore, no doubled
/// underscore, no leading zero.
fn canonical_integer(raw: &str) -> String {
    let mut collapsed = String::new();
    for c in raw.trim_matches('_').chars() {
        if !(c == '_' && collapsed.ends_with('_')) {
            collapsed.push(c);
        }
    }
    let mut digits = collapsed.as_str();
    while digits.len() > 1 {
        let Some(rest) = digits.strip_prefix('0') else {
            break;
        };
        digits = rest.strip_prefix('_').unwrap_or(rest);
    }
    if digits.is_empty() {
        "0".to_owned()
    } else {
        digits.to_owned()
    }
}

/// The leading whitespace of a line.
struct Indentation {
    /// Its width, counting a tab as two spaces for recovery.
    width: usize,
    /// The offset of its first tab.
    first_tab: Option<usize>,
    /// The offset just past it.
    end: usize,
}

#[derive(Clone, Copy)]
struct OpenBracket {
    square: bool,
    span: Span,
}

enum Interpolation {
    Continues,
    EndsLiteral,
}

struct Lexer<'a> {
    source: &'a SourceFile,
    text: &'a str,
    file: FileId,
    interner: &'a mut Interner,
    pos: usize,
    out: Lexed,
    indents: Vec<usize>,
    brackets: Vec<OpenBracket>,
    line_has_tokens: bool,
    /// The U0103 errors that get a rename edit once the file is read: the error's index, the
    /// word, and whether this occurrence names a kind or capability.
    renames: Vec<(usize, String, bool)>,
}

impl Lexer<'_> {
    fn run(&mut self) {
        if self.text.starts_with('\u{feff}') {
            let end = '\u{feff}'.len_utf8();
            self.error(
                Code::U0001,
                0,
                end,
                "byte order mark at the start of the file",
            );
            self.pos = end;
        }
        while self.pos < self.text.len() {
            self.line();
        }
        self.finish();
    }

    fn peek(&self) -> Option<char> {
        self.peek_at(self.pos)
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.text.get(offset..).and_then(|rest| rest.chars().next())
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.file, start, end)
    }

    fn slice(&self, start: usize, end: usize) -> &str {
        self.text.get(start..end).unwrap_or("")
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        if !matches!(
            kind,
            TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
        ) {
            self.line_has_tokens = true;
        }
        let span = self.span(start, end);
        self.out.tokens.push(Token { kind, span });
    }

    fn error(&mut self, code: Code, start: usize, end: usize, message: impl Into<String>) {
        let span = self.span(start, end);
        self.out.errors.push(SyntaxError::new(code, span, message));
    }

    /// U0103 for `word`, reserved with `message`, from `start` to the current position. A word
    /// reserved for a later version gets a rename repair, chosen after the whole file is read
    /// ([`Lexer::name_renames`], issue #75, #105); a word naming a construct Una writes
    /// differently says what to write instead. On a `module` or `import` line the repair is
    /// advice, since the file must move with the module (U1008).
    fn reserved(&mut self, word: &str, message: &str, start: usize) {
        let span = self.span(start, self.pos);
        let mut error = SyntaxError::new(Code::U0103, span, format!("`{word}` is {message}"));
        if message.starts_with("reserved for") {
            if matches!(
                self.line_head(),
                Some(TokenKind::Keyword(Keyword::Module | Keyword::Import))
            ) {
                error.repairs.push(SyntaxRepair {
                    description: format!(
                        "rename the module so that no part of its path is `{word}`, and move its file to match"
                    ),
                    edit: None,
                });
            } else {
                // After `kind` or `capability`, the name must start with an upper-case letter.
                let upper = matches!(
                    self.out.tokens.last().map(|t| &t.kind),
                    Some(TokenKind::Keyword(Keyword::Kind | Keyword::Capability))
                );
                self.renames
                    .push((self.out.errors.len(), word.to_owned(), upper));
            }
        }
        self.out.errors.push(error);
    }

    /// The first token of the current line, if any.
    fn line_head(&self) -> Option<&TokenKind> {
        let mut head = None;
        for token in self.out.tokens.iter().rev() {
            match token.kind {
                TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent => break,
                ref kind => head = Some(kind),
            }
        }
        head
    }

    /// The rename edits of the U0103 errors of [`Lexer::reserved`]: one name per word for the
    /// whole file, so applying every edit keeps the program consistent, and one that no name
    /// of the file already has (V12-002). It is capitalized when any occurrence is a kind or
    /// capability name: `Service`, else `the_table`; a clash adds a number, `the_table_2`.
    fn name_renames(&mut self) {
        let used: BTreeSet<String> = self
            .out
            .tokens
            .iter()
            .filter_map(|token| match token.kind {
                TokenKind::LowerIdent(name)
                | TokenKind::UpperIdent(name)
                | TokenKind::Reserved(name)
                | TokenKind::CapRef(name) => Some(self.interner.resolve(name).to_owned()),
                _ => None,
            })
            .collect();
        let mut chosen: BTreeMap<String, String> = BTreeMap::new();
        for (_, word, _) in &self.renames {
            if chosen.contains_key(word) {
                continue;
            }
            let upper = self.renames.iter().any(|(_, w, upper)| w == word && *upper);
            let base = if upper {
                let mut chars = word.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_ascii_uppercase().to_string() + chars.as_str()
                })
            } else {
                format!("the_{word}")
            };
            let taken = |name: &str| used.contains(name) || chosen.values().any(|v| v == name);
            let mut name = base.clone();
            let mut number = 2;
            while taken(&name) {
                name = if upper {
                    format!("{base}{number}")
                } else {
                    format!("{base}_{number}")
                };
                number += 1;
            }
            chosen.insert(word.clone(), name);
        }
        for (index, word, _) in std::mem::take(&mut self.renames) {
            let (Some(name), Some(error)) = (chosen.get(&word), self.out.errors.get_mut(index))
            else {
                continue;
            };
            error.repairs.push(SyntaxRepair {
                description: format!("rename it to `{name}`"),
                edit: Some((error.span, name.clone())),
            });
        }
    }

    fn consume_while(&mut self, accept: impl Fn(char) -> bool) {
        while let Some(c) = self.peek() {
            if !accept(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    fn skip_blanks(&mut self) {
        self.consume_while(|c| c == ' ' || c == '\t');
    }

    /// Lexes one physical line, from its start to just past its line feed.
    fn line(&mut self) {
        let line_start = self.pos;
        let indentation = self.indentation();
        match self.peek() {
            None => return,
            Some('\n') => {
                self.pos += 1;
                return;
            }
            Some(_) => {}
        }
        if self
            .text
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with("--"))
        {
            self.tab_outside_layout(line_start, &indentation);
            self.comment();
            self.end_of_line();
            return;
        }
        if self.brackets.is_empty() {
            self.layout(line_start, &indentation);
        } else if indentation.width == 0 && self.at_declaration_start() {
            self.close_brackets(line_start, "before the next declaration");
            self.layout(line_start, &indentation);
        } else {
            self.tab_outside_layout(line_start, &indentation);
        }
        loop {
            self.skip_blanks();
            match self.peek() {
                None | Some('\n') => break,
                Some(_) => self.token(),
            }
        }
        self.end_of_line();
    }

    fn indentation(&mut self) -> Indentation {
        let mut width = 0;
        let mut first_tab = None;
        while let Some(c) = self.peek() {
            match c {
                ' ' => width += 1,
                '\t' => {
                    first_tab.get_or_insert(self.pos);
                    // A tab counts two columns when choosing the recovery level.
                    width += 2;
                }
                _ => break,
            }
            self.pos += 1;
        }
        Indentation {
            width,
            first_tab,
            end: self.pos,
        }
    }

    /// U0002 for a tab leading a line that produces no layout: a comment-only line, or a line
    /// joined inside brackets. Rule 1 of `spec/01 §2` applies to every line.
    fn tab_outside_layout(&mut self, line_start: usize, indentation: &Indentation) {
        if let Some(tab) = indentation.first_tab {
            let error = SyntaxError::new(
                Code::U0002,
                self.span(tab, tab + 1),
                "tab in indentation; indentation uses spaces, two per level",
            )
            .with_edit(
                "replace the tabs with spaces",
                self.span(line_start, indentation.end),
                " ".repeat(indentation.width),
            );
            self.out.errors.push(error);
        }
    }

    fn end_of_line(&mut self) {
        if self.peek() == Some('\n') {
            if self.brackets.is_empty() && self.line_has_tokens {
                self.push(TokenKind::Newline, self.pos, self.pos + 1);
                self.line_has_tokens = false;
            }
            self.pos += 1;
        }
    }

    fn at_declaration_start(&self) -> bool {
        let rest = self.text.get(self.pos..).unwrap_or("");
        let length = rest
            .bytes()
            .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        let word = rest.get(..length).unwrap_or("");
        word.starts_with(|c: char| c.is_ascii_lowercase())
            && starts_declaration(word, rest.get(length..).and_then(|r| r.chars().next()))
    }

    /// Applies the indentation algorithm of `spec/01 §2` to a logical line. A tab reports only
    /// U0002 and an invalid width reports U0003; both continue at the nearest valid level, the
    /// deeper one on a tie (approved M1 plan, recovery rules).
    fn layout(&mut self, line_start: usize, indentation: &Indentation) {
        let top = self.indents.last().copied().unwrap_or(0);
        let mut levels = self.indents.clone();
        levels.push(top + 2);
        let width = indentation.width;
        let mut ranked = levels.clone();
        ranked.sort_by_key(|level| (level.abs_diff(width), Reverse(*level)));
        ranked.dedup();
        let nearest = ranked.first().copied().unwrap_or(top);
        let whole = self.span(line_start, indentation.end);
        let level = if let Some(tab) = indentation.first_tab {
            let error = SyntaxError::new(
                Code::U0002,
                self.span(tab, tab + 1),
                "tab in indentation; indentation uses spaces, two per level",
            )
            .with_edit(indent_description(nearest), whole, " ".repeat(nearest));
            self.out.errors.push(error);
            nearest
        } else if levels.contains(&width) {
            width
        } else {
            let mut expected: Vec<usize> = levels.clone();
            expected.sort_unstable();
            let mut error = SyntaxError::new(
                Code::U0003,
                whole,
                format!(
                    "inconsistent indentation of {}; expected {}",
                    spaces(width),
                    list(&expected)
                ),
            );
            for level in ranked.iter().take(2) {
                error = error.with_edit(indent_description(*level), whole, " ".repeat(*level));
            }
            self.out.errors.push(error);
            nearest
        };
        if level > top {
            self.indents.push(level);
            self.push(TokenKind::Indent, line_start, indentation.end);
        } else {
            while self.indents.last().is_some_and(|open| *open > level) {
                self.indents.pop();
                self.push(TokenKind::Dedent, indentation.end, indentation.end);
            }
        }
    }

    fn finish(&mut self) {
        let end = self.text.len();
        if !self.brackets.is_empty() {
            self.close_brackets(end, "before the end of the file");
        }
        if self.line_has_tokens {
            self.push(TokenKind::Newline, end, end);
            self.line_has_tokens = false;
        }
        while self.indents.len() > 1 {
            self.indents.pop();
            self.push(TokenKind::Dedent, end, end);
        }
        self.push(TokenKind::Eof, end, end);
    }

    /// Reports every open bracket (U0101, `spec/01 §2`) and closes it with an empty token, so
    /// the parser does not report it again; then ends the logical line.
    fn close_brackets(&mut self, at: usize, context: &str) {
        while let Some(open) = self.brackets.pop() {
            let (symbol, closer) = if open.square {
                ("[", TokenKind::RBracket)
            } else {
                ("(", TokenKind::RParen)
            };
            self.out.errors.push(SyntaxError::new(
                Code::U0101,
                open.span,
                format!("unclosed `{symbol}` {context}"),
            ));
            self.push(closer, at, at);
        }
        if self.line_has_tokens {
            self.push(TokenKind::Newline, at, at);
            self.line_has_tokens = false;
        }
    }

    fn token(&mut self) {
        let start = self.pos;
        let Some(c) = self.peek() else {
            return;
        };
        let next = self.peek_at(start + c.len_utf8());
        match c {
            '-' if next == Some('-') => self.comment(),
            '-' if next == Some('>') => self.punctuation(TokenKind::Arrow, 2),
            '-' => self.punctuation(TokenKind::Minus, 1),
            '"' => self.text_literal(),
            '0'..='9' => self.integer(),
            '_' if next.is_some_and(|n| n.is_ascii_digit()) => self.integer(),
            'a'..='z' => self.lower_word(),
            'A'..='Z' => self.upper_word(),
            '@' if next.is_some_and(|n| n.is_ascii_lowercase()) => self.capability_reference(),
            '(' | '[' => {
                let square = c == '[';
                let span = self.span(start, start + 1);
                self.brackets.push(OpenBracket { square, span });
                let kind = if square {
                    TokenKind::LBracket
                } else {
                    TokenKind::LParen
                };
                self.punctuation(kind, 1);
            }
            ')' | ']' => self.closer(c == ']'),
            ',' => self.punctuation(TokenKind::Comma, 1),
            ':' => self.punctuation(TokenKind::Colon, 1),
            '|' => self.punctuation(TokenKind::Bar, 1),
            '+' => self.punctuation(TokenKind::Plus, 1),
            '*' => self.punctuation(TokenKind::Star, 1),
            '/' => self.punctuation(TokenKind::Slash, 1),
            '=' if next == Some('=') => self.punctuation(TokenKind::EqEq, 2),
            '=' => self.punctuation(TokenKind::Eq, 1),
            '!' if next == Some('=') => self.punctuation(TokenKind::NotEq, 2),
            '<' if next == Some('=') => self.punctuation(TokenKind::Le, 2),
            '<' => self.punctuation(TokenKind::Lt, 1),
            '>' if next == Some('=') => self.punctuation(TokenKind::Ge, 2),
            '>' => self.punctuation(TokenKind::Gt, 1),
            '.' if next == Some('.') => {
                if self.peek_at(start + 2) == Some('<') {
                    self.punctuation(TokenKind::DotDotLt, 3);
                } else {
                    self.punctuation(TokenKind::DotDot, 2);
                }
            }
            '.' => self.punctuation(TokenKind::Dot, 1),
            _ => self.invalid_character(c),
        }
    }

    fn punctuation(&mut self, kind: TokenKind, length: usize) {
        let start = self.pos;
        self.pos += length;
        self.push(kind, start, self.pos);
    }

    /// U0001 for a character that cannot start a token here, one diagnostic per character.
    fn invalid_character(&mut self, c: char) {
        let start = self.pos;
        let end = start + c.len_utf8();
        let message = forbidden(self.source, start, c).unwrap_or_else(|| {
            if c.is_ascii() {
                let quoted = if c == '`' {
                    "`` ` ``".to_owned()
                } else {
                    format!("`{c}`")
                };
                match invalid_character_hint(c) {
                    Some(hint) => format!("invalid character {quoted}; {hint}"),
                    None => format!("invalid character {quoted}"),
                }
            } else {
                format!("non-ASCII character `{c}` outside a text literal or comment")
            }
        });
        self.error(Code::U0001, start, end, message);
        self.pos = end;
    }

    fn closer(&mut self, square: bool) {
        let start = self.pos;
        let symbol = if square { "]" } else { ")" };
        match self.brackets.last().copied() {
            Some(open) if open.square == square => {
                self.brackets.pop();
                let kind = if square {
                    TokenKind::RBracket
                } else {
                    TokenKind::RParen
                };
                self.punctuation(kind, 1);
            }
            Some(open) => {
                let opener = if open.square { "[" } else { "(" };
                self.error(
                    Code::U0101,
                    start,
                    start + 1,
                    format!("`{symbol}` does not close `{opener}`"),
                );
                self.pos += 1;
            }
            None => {
                self.error(
                    Code::U0101,
                    start,
                    start + 1,
                    format!("unmatched `{symbol}`"),
                );
                self.pos += 1;
            }
        }
    }

    /// A comment runs to the end of its line (`spec/01 §3`); forbidden characters inside it are
    /// still U0001 (ADR 0017).
    fn comment(&mut self) {
        let start = self.pos;
        let end = self
            .text
            .get(start..)
            .and_then(|rest| rest.find('\n'))
            .map_or(self.text.len(), |length| start + length);
        let text = self.text;
        for (index, c) in text.get(start..end).unwrap_or("").char_indices() {
            if let Some(message) = forbidden(self.source, start + index, c) {
                self.error(
                    Code::U0001,
                    start + index,
                    start + index + c.len_utf8(),
                    message,
                );
            }
        }
        let doc = self.slice(start, end).starts_with("---");
        let span = self.span(start, end);
        self.out.comments.push(Comment { span, doc });
        self.pos = end;
    }

    /// An integer literal, with U0007 and a mechanical edit when malformed (ADR 0017).
    /// A letter right after the digits starts a new token, as in `12abc`.
    fn integer(&mut self) {
        let start = self.pos;
        self.consume_while(|c| c.is_ascii_digit() || c == '_');
        let raw = self.slice(start, self.pos).to_owned();
        let canonical = canonical_integer(&raw);
        if is_malformed_integer(&raw) {
            let span = self.span(start, self.pos);
            let error = SyntaxError::new(
                Code::U0007,
                span,
                format!("malformed integer literal `{raw}`"),
            )
            .with_edit(format!("write `{canonical}`"), span, canonical.clone());
            self.out.errors.push(error);
        }
        let digits: String = canonical.chars().filter(|c| *c != '_').collect();
        self.push(TokenKind::Int(digits), start, self.pos);
    }

    fn lower_word(&mut self) {
        let start = self.pos;
        self.consume_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        let word = self.slice(start, self.pos).to_owned();
        let kind = if let Some(keyword) = Keyword::from_word(&word) {
            TokenKind::Keyword(keyword)
        } else if let Some(message) = reserved_for_later(&word) {
            self.reserved(&word, message, start);
            TokenKind::Reserved(self.interner.intern(&word))
        } else {
            TokenKind::LowerIdent(self.interner.intern(&word))
        };
        self.push(kind, start, self.pos);
    }

    fn upper_word(&mut self) {
        let start = self.pos;
        self.consume_while(|c| c.is_ascii_alphanumeric() || c == '_');
        let word = self.slice(start, self.pos).to_owned();
        let symbol = self.interner.intern(&word);
        self.push(TokenKind::UpperIdent(symbol), start, self.pos);
    }

    /// `@` followed by a lower-case name (`spec/01 §4`).
    fn capability_reference(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let name_start = self.pos;
        self.consume_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        let name = self.slice(name_start, self.pos).to_owned();
        if Keyword::from_word(&name).is_some() {
            // A keyword cannot name a capability.
            self.error(
                Code::U0101,
                name_start,
                self.pos,
                format!("keyword `{name}` cannot name a capability"),
            );
        } else if let Some(message) = reserved_for_later(&name) {
            self.reserved(&name, message, name_start);
        }
        let symbol = self.interner.intern(&name);
        self.push(TokenKind::CapRef(symbol), start, self.pos);
    }

    /// A text literal with escapes and interpolation (`spec/01 §6`, ADR 0017).
    fn text_literal(&mut self) {
        self.text_literal_with(true);
    }

    /// A text literal; `interpolate` is false for a literal nested in an interpolation, which
    /// is U0006 already and takes `{` and `}` as plain characters. That bounds the recursion
    /// between literals and interpolations.
    fn text_literal_with(&mut self, interpolate: bool) {
        let quote = self.pos;
        self.push(TokenKind::TextStart, quote, quote + 1);
        self.pos += 1;
        let mut chunk = String::new();
        let mut chunk_start = self.pos;
        loop {
            let here = self.pos;
            match self.peek() {
                None | Some('\n') => {
                    self.flush_chunk(&mut chunk, chunk_start, here);
                    self.error(
                        Code::U0004,
                        quote,
                        here,
                        "unterminated text literal; close it with `\"` on the same line",
                    );
                    self.push(TokenKind::TextEnd, here, here);
                    return;
                }
                Some('"') => {
                    self.flush_chunk(&mut chunk, chunk_start, here);
                    self.push(TokenKind::TextEnd, here, here + 1);
                    self.pos += 1;
                    return;
                }
                Some('\\') => self.escape(&mut chunk),
                Some('{') if interpolate => {
                    self.flush_chunk(&mut chunk, chunk_start, here);
                    self.push(TokenKind::InterpStart, here, here + 1);
                    self.pos += 1;
                    match self.interpolation(quote) {
                        Interpolation::Continues => chunk_start = self.pos,
                        Interpolation::EndsLiteral => return,
                    }
                }
                Some('}') if interpolate => {
                    let span = self.span(here, here + 1);
                    let error =
                        SyntaxError::new(Code::U0007, span, "unescaped `}` in text; write `\\}`")
                            .with_edit("write `\\}`", span, "\\}");
                    self.out.errors.push(error);
                    chunk.push('}');
                    self.pos += 1;
                }
                Some(c) => {
                    if let Some(message) = forbidden(self.source, here, c) {
                        self.error(Code::U0001, here, here + c.len_utf8(), message);
                    } else {
                        chunk.push(c);
                    }
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn flush_chunk(&mut self, chunk: &mut String, start: usize, end: usize) {
        if !chunk.is_empty() {
            self.push(TokenKind::TextChunk(std::mem::take(chunk)), start, end);
        }
    }

    /// An escape sequence after `\` (`spec/01 §6`): valid escapes append their character,
    /// invalid ones report U0005 at the backslash, spanning the sequence.
    fn escape(&mut self, chunk: &mut String) {
        let start = self.pos;
        self.pos += 1;
        let Some(c) = self.peek() else {
            return;
        };
        let simple = match c {
            '"' => Some('"'),
            '\\' => Some('\\'),
            'n' => Some('\n'),
            't' => Some('\t'),
            '{' => Some('{'),
            '}' => Some('}'),
            _ => None,
        };
        if let Some(decoded) = simple {
            chunk.push(decoded);
            self.pos += 1;
            return;
        }
        match c {
            '\n' => {}
            'u' => self.unicode_escape(start, chunk),
            _ => {
                self.pos += c.len_utf8();
                self.error(
                    Code::U0005,
                    start,
                    self.pos,
                    format!("invalid escape `\\{c}`; write `\\\\` for a backslash"),
                );
            }
        }
    }

    fn unicode_escape(&mut self, start: usize, chunk: &mut String) {
        self.pos += 1;
        if self.peek() != Some('{') {
            self.error(
                Code::U0005,
                start,
                self.pos,
                "`\\u` escape needs braces, as in `\\u{1F600}`",
            );
            return;
        }
        self.pos += 1;
        let digits_start = self.pos;
        self.consume_while(|c| c.is_ascii_hexdigit());
        let digits = self.slice(digits_start, self.pos).to_owned();
        if self.peek() != Some('}') {
            self.consume_while(|c| c != '}' && c != '"' && c != '\n');
            if self.peek() == Some('}') {
                self.pos += 1;
            }
            self.error(
                Code::U0005,
                start,
                self.pos,
                "`\\u{...}` escape needs one to six hexadecimal digits and a closing `}`",
            );
            return;
        }
        self.pos += 1;
        let message = if digits.is_empty() {
            Some("empty `\\u{}` escape".to_owned())
        } else if digits.len() > 6 {
            Some("`\\u{...}` escape has more than six digits".to_owned())
        } else {
            match u32::from_str_radix(&digits, 16)
                .ok()
                .and_then(char::from_u32)
            {
                Some(decoded) => {
                    chunk.push(decoded);
                    None
                }
                None => Some(format!("`\\u{{{digits}}}` is not a Unicode scalar value")),
            }
        };
        if let Some(message) = message {
            self.error(Code::U0005, start, self.pos, message);
        }
    }

    /// The expression tokens of an interpolation, after its `{` (`spec/01 §6`). Brackets inside
    /// it balance on their own. A quote that has no partner on the line closes the enclosing
    /// literal, whose interpolation then lacks its `}` (ADR 0017, decision 4).
    fn interpolation(&mut self, outer_quote: usize) -> Interpolation {
        let outer_brackets = std::mem::take(&mut self.brackets);
        let outcome = loop {
            self.skip_blanks();
            let here = self.pos;
            match self.peek() {
                None | Some('\n') => {
                    self.close_interpolation_brackets(here);
                    self.error(
                        Code::U0004,
                        outer_quote,
                        here,
                        "unterminated text literal; close it with `\"` on the same line",
                    );
                    self.push(TokenKind::InterpEnd, here, here);
                    self.push(TokenKind::TextEnd, here, here);
                    break Interpolation::EndsLiteral;
                }
                Some('}') => {
                    self.close_interpolation_brackets(here);
                    self.push(TokenKind::InterpEnd, here, here + 1);
                    self.pos += 1;
                    break Interpolation::Continues;
                }
                Some('"') => {
                    if let Some(close) = self.closing_quote_on_line(here + 1) {
                        self.error(
                            Code::U0006,
                            here,
                            close + 1,
                            "text literal inside interpolation; bind it with `let` and interpolate the name",
                        );
                        self.text_literal_with(false);
                    } else {
                        self.close_interpolation_brackets(here);
                        self.error(
                            Code::U0004,
                            outer_quote,
                            here + 1,
                            "unterminated text literal; its interpolation is missing `}`",
                        );
                        self.push(TokenKind::InterpEnd, here, here);
                        self.push(TokenKind::TextEnd, here, here + 1);
                        self.pos += 1;
                        break Interpolation::EndsLiteral;
                    }
                }
                Some('{') => self.invalid_character('{'),
                Some(_) => self.token(),
            }
        };
        self.brackets = outer_brackets;
        outcome
    }

    fn close_interpolation_brackets(&mut self, at: usize) {
        while let Some(open) = self.brackets.pop() {
            let (symbol, closer) = if open.square {
                ("[", TokenKind::RBracket)
            } else {
                ("(", TokenKind::RParen)
            };
            self.out.errors.push(SyntaxError::new(
                Code::U0101,
                open.span,
                format!("unclosed `{symbol}` before the end of the interpolation"),
            ));
            self.push(closer, at, at);
        }
    }

    /// The offset of the next unescaped quote on the current line, from `from`.
    fn closing_quote_on_line(&self, from: usize) -> Option<usize> {
        let mut escaped = false;
        for (index, c) in self.text.get(from..)?.char_indices() {
            match c {
                '\n' => return None,
                '\\' if !escaped => escaped = true,
                '"' if !escaped => return Some(from + index),
                _ => escaped = false,
            }
        }
        None
    }
}

fn spaces(count: usize) -> String {
    if count == 1 {
        "1 space".to_owned()
    } else {
        format!("{count} spaces")
    }
}

fn list(levels: &[usize]) -> String {
    let words: Vec<String> = levels.iter().map(ToString::to_string).collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

fn indent_description(level: usize) -> String {
    if level == 0 {
        "remove the indentation".to_owned()
    } else {
        format!("indent with {}", spaces(level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_integers() {
        for (raw, canonical) in [
            ("007", "7"),
            ("1_", "1"),
            ("_1", "1"),
            ("1__0", "1_0"),
            ("00", "0"),
            ("0_5", "5"),
            ("0__0", "0"),
            ("1_000", "1_000"),
            ("0", "0"),
        ] {
            assert_eq!(canonical_integer(raw), canonical, "{raw}");
        }
    }

    #[test]
    fn malformed_integers() {
        for raw in ["007", "00", "1_", "_1", "1__0", "0_5"] {
            assert!(is_malformed_integer(raw), "{raw}");
        }
        for raw in ["0", "7", "1_000", "10", "1_0_0"] {
            assert!(!is_malformed_integer(raw), "{raw}");
        }
    }

    #[test]
    fn level_lists_read_naturally() {
        assert_eq!(list(&[0, 2]), "0 or 2");
        assert_eq!(list(&[0, 2, 4, 6]), "0, 2, 4 or 6");
        assert_eq!(spaces(1), "1 space");
        assert_eq!(indent_description(0), "remove the indentation");
        assert_eq!(indent_description(2), "indent with 2 spaces");
    }
}
