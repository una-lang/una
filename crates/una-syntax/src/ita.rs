//! The project files of edition 0.4: `ita.una`, `ita.lock` and `itaty.una` (ADR 0122 rules 3
//! to 5, 16 and 19).
//!
//! Each file is read by the lexer of `spec/01` in its project mode ([`crate::lex_project`]),
//! then grouped into a tree of lines by its indentation, then read by the closed grammar of its
//! kind. Nothing in these files is evaluated: there are no expressions, no interpolation, no
//! conditions and no includes. Every problem is an [`Error`] at the smallest range that
//! explains it, and the caller gives it the code of the file (U7008 for `ita.una` and
//! `itaty.una`, U7009 for `ita.lock`).
//!
//! Costs: lexing, grouping and reading are each one pass over the file, O(n) in its size.

use std::ops::Range;

use crate::intern::Interner;
use crate::lexer::lex_project;
use crate::source::{FileId, SourceFile};
use crate::token::TokenKind;

/// The largest `ita.una` or `itaty.una`, in bytes (ADR 0122 rule 3).
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// The largest `ita.lock`, in bytes (ADR 0122 rule 3).
pub const MAX_LOCK_BYTES: usize = 16 << 20;

/// A problem in a project file, at a byte range of its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Where it is.
    pub range: Range<usize>,
    /// One sentence, lowercase start, no trailing period.
    pub message: String,
}

impl Error {
    fn new(range: Range<usize>, message: impl Into<String>) -> Error {
        Error {
            range,
            message: message.into(),
        }
    }
}

/// A value and where it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At<T> {
    /// The value.
    pub value: T,
    /// Where it is written, in bytes of the file's text.
    pub range: Range<usize>,
}

impl<T> At<T> {
    fn new(value: T, range: Range<usize>) -> At<T> {
        At { value, range }
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens and lines
// ---------------------------------------------------------------------------------------------

/// A token of a project file, as the grammars read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A lower-case word.
    Word(String),
    /// An upper-case word.
    Upper(String),
    /// A version.
    Version(String),
    /// An integer, its decimal digits.
    Int(String),
    /// A text literal's value, escapes decoded.
    Text(String),
    /// A capability reference, `@name`, without the `@`.
    CapRef(String),
    /// A punctuation token, as written.
    Punct(&'static str),
}

impl Item {
    fn describe(&self) -> String {
        match self {
            Item::Word(word) | Item::Upper(word) => format!("`{word}`"),
            Item::Version(version) => format!("version `{version}`"),
            Item::Int(digits) => format!("`{digits}`"),
            Item::Text(_) => "a text".to_owned(),
            Item::CapRef(name) => format!("`@{name}`"),
            Item::Punct(symbol) => format!("`{symbol}`"),
        }
    }
}

/// A token and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    /// The token.
    pub item: Item,
    /// Where it is written.
    pub range: Range<usize>,
}

/// A logical line of a project file and the lines indented below it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Its tokens, at least one.
    pub tokens: Vec<Tok>,
    /// From its first token to its last.
    pub range: Range<usize>,
    /// The lines one level deeper, directly below it.
    pub children: Vec<Line>,
    /// The comments on their own lines just before it, for the formatter.
    pub comments_before: Vec<Range<usize>>,
    /// The comment at the end of the line, for the formatter.
    pub trailing_comment: Option<Range<usize>>,
}

impl Line {
    fn word(&self, index: usize) -> Option<&str> {
        match self.tokens.get(index).map(|token| &token.item) {
            Some(Item::Word(word)) => Some(word),
            _ => None,
        }
    }

    /// The range of the first token.
    fn head(&self) -> Range<usize> {
        self.tokens
            .first()
            .map_or(self.range.clone(), |token| token.range.clone())
    }
}

/// A file read into lines: the top-level lines, the comments after the last line, and the
/// lexical errors.
#[derive(Clone, Debug, Default)]
pub struct Lines {
    /// The top-level lines.
    pub lines: Vec<Line>,
    /// The comments after the last line.
    pub comments_after: Vec<Range<usize>>,
    /// Lexical and layout errors.
    pub errors: Vec<Error>,
    /// The text that every range indexes: the text given to [`lines`], as written. The lexer
    /// reads `\r\n` as `\n` (`spec/01 §1`), and each range it gives is mapped back to the
    /// bytes it names in this text (#2249).
    pub text: String,
}

/// The text of a project file, as its readers take it: written, or decoded already (#2252).
///
/// A file is decoded exactly once (`spec/01 §1`): `\r\n` read as `\n`, a lone `\r` kept for
/// the lexer to report. Decoding a decoded text again would read a `\r\n` that a written
/// `\r\r\n` decodes to as one more line end, and the lone `\r` would go unreported; so a
/// reader decodes a [`Text::Written`] text once, and reads a [`Text::Decoded`] file as it is.
#[derive(Clone, Copy, Debug)]
pub enum Text<'a> {
    /// The text as written, which the reader decodes; every range indexes this text.
    Written(&'a str),
    /// A file decoded once by [`SourceFile::new`]; every range indexes its decoded text, which
    /// is what a [`crate::SourceDb`] locates diagnostics against.
    Decoded(&'a SourceFile),
}

impl<'a> Text<'a> {
    /// The text that every range of its reading indexes: the written text, or the decoded
    /// text of the decoded file.
    pub fn as_str(self) -> &'a str {
        match self {
            Text::Written(text) => text,
            Text::Decoded(file) => file.text(),
        }
    }

    /// The number of bytes the file holds as written, which the size bounds measure
    /// (`spec/30 §1` rule 4): the length of a written text, and the raw length of a decoded
    /// file ([`SourceFile::raw_len`]), never the length after `\r\n` became `\n`.
    pub fn raw_len(self) -> usize {
        match self {
            Text::Written(text) => text.len(),
            Text::Decoded(file) => file.raw_len(),
        }
    }
}

impl<'a> From<&'a str> for Text<'a> {
    fn from(text: &'a str) -> Text<'a> {
        Text::Written(text)
    }
}

impl<'a> From<&'a String> for Text<'a> {
    fn from(text: &'a String) -> Text<'a> {
        Text::Written(text)
    }
}

impl<'a, 'b: 'a> From<&'a std::borrow::Cow<'b, str>> for Text<'a> {
    fn from(text: &'a std::borrow::Cow<'b, str>) -> Text<'a> {
        Text::Written(text)
    }
}

impl<'a> From<&'a SourceFile> for Text<'a> {
    fn from(file: &'a SourceFile) -> Text<'a> {
        Text::Decoded(file)
    }
}

/// Reads `text` into lines (ADR 0122 rule 4): the lexer's project mode, then one tree of lines
/// by indentation. A written text is decoded once, the lexer reads the decoded text, where each
/// `\r\n` is one `\n`, and its offsets are mapped back to the written text at the end, so
/// every range indexes the text as written (#2249). A decoded file is read as it is, never
/// decoded again, and every range indexes its decoded text (#2252). O(n), and O(r log c) for
/// the r ranges of a written text with c `\r\n` line ends.
pub fn lines<'a>(text: impl Into<Text<'a>>) -> Lines {
    match text.into() {
        Text::Written(given) => {
            let file = SourceFile::new("project", given.as_bytes());
            let mut out = read_lines(&file);
            out.text = given.to_owned();
            let crlf = Crlf::of(given);
            if !crlf.is_empty() {
                crlf.lines(&mut out);
            }
            out
        }
        Text::Decoded(file) => read_lines(file),
    }
}

/// Reads a decoded file into lines; every range indexes its decoded text. O(n).
fn read_lines(file: &SourceFile) -> Lines {
    let mut interner = Interner::new();
    let lexed = lex_project(file, FileId(0), &mut interner);
    let text = file.text();
    let mut out = Lines {
        text: text.to_owned(),
        errors: lexed
            .errors
            .iter()
            .map(|error| Error::new(error.span.start..error.span.end, error.message.clone()))
            .collect(),
        ..Lines::default()
    };
    // The comments in source order, each either trailing a line or standing on its own.
    let mut comments = lexed.comments.iter().map(|comment| comment.span).peekable();
    // A stack of open blocks: the lines read so far at each depth.
    let mut stack: Vec<Vec<Line>> = vec![Vec::new()];
    let mut current: Vec<Tok> = Vec::new();
    let mut pending_comments: Vec<Range<usize>> = Vec::new();
    let mut text_start: Option<usize> = None;
    let mut text_value = String::new();
    // Whether no line feed lies between two offsets: the same physical line. O(distance).
    let same_line =
        |from: usize, to: usize| text.get(from..to).is_some_and(|gap| !gap.contains('\n'));
    for token in &lexed.tokens {
        let range = token.span.start..token.span.end;
        let item = match &token.kind {
            TokenKind::TextStart => {
                text_start = Some(range.start);
                text_value.clear();
                continue;
            }
            TokenKind::TextChunk(chunk) => {
                text_value.push_str(chunk);
                continue;
            }
            TokenKind::TextEnd => {
                let start = text_start.take().unwrap_or(range.start);
                let item = Item::Text(std::mem::take(&mut text_value));
                current.push(Tok {
                    item,
                    range: start..range.end,
                });
                continue;
            }
            TokenKind::InterpStart | TokenKind::InterpEnd => continue,
            TokenKind::LowerIdent(symbol) | TokenKind::Reserved(symbol) => {
                Item::Word(interner.resolve(*symbol).to_owned())
            }
            TokenKind::Keyword(keyword) => Item::Word(keyword.as_str().to_owned()),
            TokenKind::UpperIdent(symbol) => Item::Upper(interner.resolve(*symbol).to_owned()),
            TokenKind::CapRef(symbol) => Item::CapRef(interner.resolve(*symbol).to_owned()),
            TokenKind::Int(digits) => Item::Int(digits.clone()),
            TokenKind::Version(version) => Item::Version(version.clone()),
            TokenKind::Newline => {
                if let Some(first) = current.first() {
                    let start = first.range.start;
                    let end = current.last().map_or(start, |last| last.range.end);
                    // Comments that start before this line are its leading comments; one on
                    // the same physical line as its last token is its trailing comment.
                    let mut before = std::mem::take(&mut pending_comments);
                    while let Some(comment) = comments.peek() {
                        if comment.start < start {
                            before.push(comment.start..comment.end);
                            comments.next();
                        } else {
                            break;
                        }
                    }
                    let mut trailing = None;
                    if let Some(comment) = comments.peek()
                        && same_line(end, comment.start)
                    {
                        trailing = Some(comment.start..comment.end);
                        comments.next();
                    }
                    let line = Line {
                        tokens: std::mem::take(&mut current),
                        range: start..end,
                        children: Vec::new(),
                        comments_before: before,
                        trailing_comment: trailing,
                    };
                    if let Some(block) = stack.last_mut() {
                        block.push(line);
                    }
                }
                continue;
            }
            TokenKind::Indent => {
                stack.push(Vec::new());
                continue;
            }
            TokenKind::Dedent => {
                close_block(&mut stack, &mut out.errors);
                continue;
            }
            TokenKind::Eof => continue,
            punctuation => {
                let symbol = punctuation.symbol();
                if symbol.is_empty() {
                    continue;
                }
                Item::Punct(symbol)
            }
        };
        current.push(Tok { item, range });
    }
    while stack.len() > 1 {
        close_block(&mut stack, &mut out.errors);
    }
    out.lines = stack.pop().unwrap_or_default();
    out.comments_after = pending_comments;
    out.comments_after
        .extend(comments.map(|comment| comment.start..comment.end));
    out
}

/// The `\r\n` line ends of a text, by the offset of their `\n` in the decoded text, where
/// each is one `\n`: what maps an offset of the decoded text back to the text (#2249).
///
/// A text given as `&str` is valid UTF-8, so decoding it (`spec/01 §1`) changes nothing but
/// its `\r\n` line ends, each one byte shorter. An offset `d` of the decoded text is the
/// offset `d + k` of the text, for the k line ends whose `\n` lies before `d`: an offset at
/// such a `\n`, the end of a line's last token or an empty range at the end of a line, lands
/// before its `\r`, and an offset after it lands after the `\n`. Each is a character
/// boundary of the text, since `\r` and `\n` are ASCII.
struct Crlf(Vec<usize>);

impl Crlf {
    /// O(n).
    fn of(text: &str) -> Crlf {
        Crlf(
            text.match_indices("\r\n")
                .enumerate()
                .map(|(k, (at, _))| at - k)
                .collect(),
        )
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// O(log c).
    fn offset(&self, decoded: usize) -> usize {
        decoded + self.0.partition_point(|&lf| lf < decoded)
    }

    fn range(&self, range: &mut Range<usize>) {
        *range = self.offset(range.start)..self.offset(range.end);
    }

    /// Maps every range of `out` back to the text. O(r log c).
    fn lines(&self, out: &mut Lines) {
        for error in &mut out.errors {
            self.range(&mut error.range);
        }
        for comment in &mut out.comments_after {
            self.range(comment);
        }
        let mut stack: Vec<&mut Line> = out.lines.iter_mut().collect();
        while let Some(line) = stack.pop() {
            self.range(&mut line.range);
            for token in &mut line.tokens {
                self.range(&mut token.range);
            }
            for comment in &mut line.comments_before {
                self.range(comment);
            }
            if let Some(comment) = &mut line.trailing_comment {
                self.range(comment);
            }
            stack.extend(line.children.iter_mut());
        }
    }
}

/// Closes the innermost block: its lines become the children of the last line of the block
/// around it. An indented block with no line to hang from is an error at its first line.
fn close_block(stack: &mut Vec<Vec<Line>>, errors: &mut Vec<Error>) {
    let Some(block) = stack.pop() else {
        return;
    };
    let Some(parent) = stack.last_mut() else {
        return;
    };
    match parent.last_mut() {
        Some(line) => line.children.extend(block),
        None => {
            if let Some(first) = block.first() {
                errors.push(Error::new(
                    first.range.clone(),
                    "this line is indented, but no line above it opens a block",
                ));
            }
            parent.extend(block);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reading lines
// ---------------------------------------------------------------------------------------------

/// A cursor over the tokens of one line.
struct Cursor<'l> {
    line: &'l Line,
    at: usize,
}

impl<'l> Cursor<'l> {
    fn new(line: &'l Line) -> Cursor<'l> {
        Cursor { line, at: 0 }
    }

    fn peek(&self) -> Option<&'l Tok> {
        self.line.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<&'l Tok> {
        let token = self.line.tokens.get(self.at);
        if token.is_some() {
            self.at += 1;
        }
        token
    }

    fn done(&self) -> bool {
        self.at >= self.line.tokens.len()
    }

    /// The range at which a missing token is reported: the next token, or the end of the line.
    fn here(&self) -> Range<usize> {
        self.peek().map_or_else(
            || self.line.range.end..self.line.range.end,
            |token| token.range.clone(),
        )
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if matches!(self.peek().map(|t| &t.item), Some(Item::Word(w)) if w == word) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn eat_punct(&mut self, symbol: &str) -> bool {
        if matches!(self.peek().map(|t| &t.item), Some(Item::Punct(p)) if *p == symbol) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, what: &str) -> Result<At<String>, Error> {
        match self.peek() {
            Some(Tok {
                item: Item::Word(word),
                range,
            }) => {
                self.at += 1;
                Ok(At::new(word.clone(), range.clone()))
            }
            other => Err(expected(what, other, self.here())),
        }
    }

    fn text(&mut self, what: &str) -> Result<At<String>, Error> {
        match self.peek() {
            Some(Tok {
                item: Item::Text(value),
                range,
            }) => {
                self.at += 1;
                Ok(At::new(value.clone(), range.clone()))
            }
            other => Err(expected(what, other, self.here())),
        }
    }

    fn version(&mut self, what: &str) -> Result<At<String>, Error> {
        match self.peek() {
            Some(Tok {
                item: Item::Version(value),
                range,
            }) => {
                self.at += 1;
                Ok(At::new(value.clone(), range.clone()))
            }
            other => Err(expected(what, other, self.here())),
        }
    }

    fn int(&mut self, what: &str) -> Result<At<u64>, Error> {
        match self.peek() {
            Some(Tok {
                item: Item::Int(digits),
                range,
            }) => {
                self.at += 1;
                digits
                    .parse::<u64>()
                    .map(|value| At::new(value, range.clone()))
                    .map_err(|_| Error::new(range.clone(), format!("{what} is too large")))
            }
            other => Err(expected(what, other, self.here())),
        }
    }

    /// An integer's digits as written, for the items of a `serve` grant, whose ranges and
    /// spelling `una_core::http::serve` checks.
    fn digits(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(Tok {
                item: Item::Int(digits),
                ..
            }) => {
                self.at += 1;
                Ok(digits.clone())
            }
            other => Err(expected("a number", other, self.here()).message),
        }
    }

    /// `at most <n>`, as its digits.
    fn at_most(&mut self) -> Result<String, String> {
        if !(self.eat_word("at") && self.eat_word("most")) {
            return Err("expected `at most` and a number".to_owned());
        }
        self.digits()
    }

    /// The unit of an item, which must be the one the item counts in.
    fn unit(&mut self, unit: &str, item: &str) -> Result<(), String> {
        match self.next() {
            Some(Tok {
                item: Item::Upper(written),
                ..
            }) if written == unit => Ok(()),
            _ => Err(format!(
                "`{item}` is a number of {unit}, written `{item} at most <n> {unit}`"
            )),
        }
    }

    /// The end of the line: anything left is an error.
    fn end(&self) -> Result<(), Error> {
        match self.peek() {
            None => Ok(()),
            Some(token) => Err(Error::new(
                token.range.start..self.line.range.end,
                format!(
                    "unexpected {} at the end of this line",
                    token.item.describe()
                ),
            )),
        }
    }

    /// A module path: lower-case words joined by `.`.
    fn module_path(&mut self, what: &str) -> Result<At<String>, Error> {
        let first = self.word(what)?;
        let mut path = first.value;
        let start = first.range.start;
        let mut end = first.range.end;
        while self.eat_punct(".") {
            let segment = self.word("a module path segment")?;
            path.push('.');
            path.push_str(&segment.value);
            end = segment.range.end;
        }
        Ok(At::new(path, start..end))
    }

    /// A package name: `name` or `owner/name` (ADR 0122 rule 5, `pkg_name`).
    fn pkg_name(&mut self) -> Result<At<PkgName>, Error> {
        let first = self.word("a package name")?;
        if self.eat_punct("/") {
            let name = self.word("the package name after `/`")?;
            let range = first.range.start..name.range.end;
            return Ok(At::new(
                PkgName {
                    owner: Some(first.value),
                    name: name.value,
                },
                range,
            ));
        }
        Ok(At::new(
            PkgName {
                owner: None,
                name: first.value,
            },
            first.range,
        ))
    }

    /// A capability type with operations: `{ lower "." } Upper "[" lower { "," lower } "]"`.
    fn cap_type(&mut self) -> Result<CapType, Error> {
        let start = self.here().start;
        let mut capability = String::new();
        loop {
            match self.peek() {
                Some(Tok {
                    item: Item::Word(word),
                    ..
                }) => {
                    self.at += 1;
                    capability.push_str(word);
                    if !self.eat_punct(".") {
                        return Err(Error::new(
                            self.here(),
                            "expected `.` after a module path segment of a capability",
                        ));
                    }
                    capability.push('.');
                }
                Some(Tok {
                    item: Item::Upper(word),
                    ..
                }) => {
                    self.at += 1;
                    capability.push_str(word);
                    break;
                }
                other => {
                    return Err(expected(
                        "a capability with its operations, such as `Http[get]`",
                        other,
                        self.here(),
                    ));
                }
            }
        }
        if !self.eat_punct("[") {
            return Err(Error::new(
                self.here(),
                format!(
                    "expected `[` and the operations of `{capability}`, such as `{capability}[get]`"
                ),
            ));
        }
        let mut ops: Vec<At<String>> = Vec::new();
        loop {
            let op = self.word("an operation")?;
            ops.push(op);
            if self.eat_punct(",") {
                continue;
            }
            break;
        }
        let close = self.here();
        if !self.eat_punct("]") {
            return Err(Error::new(close, "expected `,` or `]` after an operation"));
        }
        Ok(CapType {
            capability,
            ops,
            range: start..close.end,
        })
    }

    /// One or more capability types separated by `,`.
    fn cap_types(&mut self) -> Result<Vec<CapType>, Error> {
        let mut caps = vec![self.cap_type()?];
        while self.eat_punct(",") {
            caps.push(self.cap_type()?);
        }
        Ok(caps)
    }
}

fn expected(what: &str, found: Option<&Tok>, at: Range<usize>) -> Error {
    match found {
        Some(token) => Error::new(
            at,
            format!("expected {what}, found {}", token.item.describe()),
        ),
        None => Error::new(at, format!("expected {what} at the end of this line")),
    }
}

/// An error for a line that has children it may not have.
fn no_children(line: &Line, errors: &mut Vec<Error>) {
    if let Some(first) = line.children.first() {
        errors.push(Error::new(
            first.range.clone(),
            "unexpected indented line here; this line opens no block",
        ));
    }
}

// ---------------------------------------------------------------------------------------------
// `ita.una`
// ---------------------------------------------------------------------------------------------

/// A package name, `name` or `owner/name` (ADR 0122 rules 6 and 44).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PkgName {
    /// The owner, for a name `owner/name`.
    pub owner: Option<String>,
    /// The name, the last segment.
    pub name: String,
}

impl std::fmt::Display for PkgName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.owner {
            Some(owner) => write!(f, "{owner}/{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// A capability with operations, as a `may` line writes it: `Http[get]`, or
/// `ledger.store.Store[put, find]` for one a dependency declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapType {
    /// The capability, with its module path when it has one.
    pub capability: String,
    /// The operations, in the order written.
    pub ops: Vec<At<String>>,
    /// The whole `C[ops]`.
    pub range: Range<usize>,
}

/// Where a package comes from (ADR 0122 rule 7, `origin`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// `from <source>`: a registry, `itaty` or a source of the root's `sources`.
    Source(String),
    /// `from path "<dir>"`: a directory relative to the package.
    Path(String),
    /// `from workspace`: a member of the workspace (rule 19).
    Workspace,
}

/// A line of `needs` (ADR 0122 rule 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    /// The package's name.
    pub name: At<PkgName>,
    /// The minimum version, when written.
    pub version: Option<At<String>>,
    /// `as <local>`.
    pub local: Option<At<String>>,
    /// `from ...`.
    pub origin: Option<At<Origin>>,
    /// The `may` lines: the ceiling this package grants the dependency.
    pub may: Vec<CapType>,
    /// The `avoid` lines.
    pub avoid: Vec<At<String>>,
    /// The `accept name` line, where written.
    pub accept_name: Option<Range<usize>>,
    /// The need's own line.
    pub range: Range<usize>,
}

/// A source of `sources` (ADR 0122 rule 8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// Its name.
    pub name: At<String>,
    /// The registry's base URL.
    pub url: At<String>,
    /// The digest of its root metadata.
    pub root: Option<At<String>>,
    /// The owners it serves.
    pub owners: Vec<At<String>>,
}

/// A line of `replace` (ADR 0122 rule 9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replace {
    /// The package replaced.
    pub name: At<PkgName>,
    /// Its version.
    pub version: At<String>,
    /// Where it is taken from instead.
    pub origin: At<Origin>,
}

/// A trust line (ADR 0122 rule 10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustLine {
    /// The package's name.
    pub name: At<PkgName>,
    /// Its major line, `2` or `0.3`, when written.
    pub major: Option<At<String>>,
    /// Its source, when written.
    pub from: Option<At<String>>,
    /// The module paths trusted.
    pub modules: Vec<At<String>>,
    /// The ceiling of the trusted modules (rule 53.1).
    pub may: Vec<CapType>,
    /// The line.
    pub range: Range<usize>,
}

/// An entry of `boundaries` (ADR 0093, ADR 0122 rule 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryEntry {
    /// The module path.
    pub module: At<String>,
    /// Its ceiling; empty when no `may` line is written.
    pub may: Vec<CapType>,
}

/// An entry of `contracts` (ADR 0092 rule 8, ADR 0122 rule 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractEntry {
    /// The function's qualified name.
    pub function: At<String>,
    /// Its locked `uses` clauses, each the text of its line, as a header writes it.
    pub clauses: Vec<At<String>>,
}

/// The kind of a limit line (ADR 0117 rule 8, `spec/27 §9`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LimitKind {
    /// `at most <n> per second|minute|hour`, with the period's word.
    Rate(String),
    /// `at most <n> at once`.
    AtOnce,
    /// `waiting at most <n>`.
    Waiting,
}

/// A limit line under an `http` grant (ADR 0117 rule 8): its kind, its count as written, and
/// its line. The count's range is checked by the reader of the manifest (U7008).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limit {
    /// The kind.
    pub kind: LimitKind,
    /// The count.
    pub count: At<u64>,
    /// The line.
    pub range: Range<usize>,
}

/// A grant of `run grants` (ADR 0122 rule 12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The grant in the text form of `spec/13 §5`, such as `files:data/:read`.
    pub spec: String,
    /// The grant's line.
    pub range: Range<usize>,
    /// The limit lines below it that parse (ADR 0117 rule 8); each that does not is an error.
    /// A `serve` grant has none: its lines are its items.
    pub limits: Vec<Limit>,
    /// The item lines below a `serve` grant (`spec/31 §2` rule 3), each the range of its line,
    /// in the order of the `;` items of `spec`.
    pub items: Vec<Range<usize>>,
}

/// The `run` section (ADR 0122 rules 11 and 12).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    /// `entry`.
    pub entry: Option<At<String>>,
    /// `max_memory <MiB>`: the declared memory budget (ADR 0109 rule 4), checked by the
    /// project crate.
    pub max_memory: Option<At<u64>>,
    /// The grants.
    pub grants: Vec<Grant>,
}

/// The `drive` section (ADR 0116 rule 1, ADR 0122 rule 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    /// `module`.
    pub module: Option<At<String>>,
    /// `ticks`.
    pub ticks: Option<At<u64>>,
    /// `budget`.
    pub budget: Option<At<u64>>,
    /// The section's header line.
    pub range: Range<usize>,
}

/// The `serve` section (ADR 0123 rule 16, ADR 0122 rule 11): the module whose `handle` the host
/// calls for each request, and the optional per-request deadline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Serve {
    /// `module`.
    pub module: Option<At<String>>,
    /// `within`, in milliseconds.
    pub within: Option<At<u64>>,
    /// The section's header line.
    pub range: Range<usize>,
    /// The whole section, from its header to the end of its last line: a problem inside it
    /// means the section is not read (`spec/31 §7` rule 1).
    pub extent: Range<usize>,
}

/// The sections of `ita.una`, in their canonical order (ADR 0122 rule 5).
pub const SECTIONS: [&str; 12] = [
    "sources",
    "needs",
    "replace",
    "trust",
    "labels",
    "trusted",
    "escapes",
    "boundaries",
    "contracts",
    "run",
    "drive",
    "serve",
];

/// The head lines of `ita.una`, in their canonical order (ADR 0122 rule 5).
pub const HEAD_LINES: [&str; 7] = [
    "version", "edition", "profile", "about", "license", "home", "publish",
];

/// `ita.una`, as read (ADR 0122 rules 5 to 12). Every section is empty when absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// `ita <name>`.
    pub name: Option<At<PkgName>>,
    /// The `ita` line.
    pub head: Range<usize>,
    /// `version`.
    pub version: Option<At<String>>,
    /// `edition`.
    pub edition: Option<At<String>>,
    /// `profile`.
    pub profile: Option<At<String>>,
    /// `about`.
    pub about: Option<At<String>>,
    /// `license`.
    pub license: Option<At<String>>,
    /// `home`.
    pub home: Option<At<String>>,
    /// `publish to <source>`.
    pub publish_to: Option<At<String>>,
    /// The sections present, by name, at their header lines.
    pub sections: Vec<At<String>>,
    /// `sources`.
    pub sources: Vec<Source>,
    /// `needs`.
    pub needs: Vec<Need>,
    /// `replace`.
    pub replace: Vec<Replace>,
    /// `trust`.
    pub trust: Vec<TrustLine>,
    /// `labels`.
    pub labels: Vec<At<String>>,
    /// `trusted`.
    pub trusted: Vec<At<String>>,
    /// `escapes`: `checked per module <n>`.
    pub escapes: Option<At<u64>>,
    /// `boundaries`.
    pub boundaries: Vec<BoundaryEntry>,
    /// `contracts`.
    pub contracts: Vec<ContractEntry>,
    /// `run`.
    pub run: Option<Run>,
    /// `drive`.
    pub drive: Option<Drive>,
    /// `serve`.
    pub serve: Option<Serve>,
}

/// Reads `ita.una` by the closed grammar of ADR 0122 rule 5. Every line that has none of its
/// forms is an error at the line; reading goes on with the next line. O(n).
pub fn parse_manifest<'a>(text: impl Into<Text<'a>>) -> (Manifest, Vec<Error>) {
    let text = text.into();
    let mut manifest = Manifest::default();
    if text.raw_len() > MAX_MANIFEST_BYTES {
        return (
            manifest,
            vec![Error::new(
                0..0,
                format!(
                    "the file holds {} bytes, more than the {MAX_MANIFEST_BYTES} a manifest may hold",
                    text.raw_len()
                ),
            )],
        );
    }
    let read = lines(text);
    let written = read.text.as_str();
    let decoded = matches!(text, Text::Decoded(_));
    let mut errors = read.errors.clone();
    let mut lines = read.lines.iter();
    match lines.next() {
        Some(line) if line.word(0) == Some("ita") => head(line, &mut manifest, &mut errors),
        Some(line) => errors.push(Error::new(
            line.range.clone(),
            "ita.una starts with `ita <name>` and its head lines",
        )),
        None => errors.push(Error::new(
            0..0,
            "ita.una is empty; it starts with `ita <name>` and its head lines",
        )),
    }
    let mut last: Option<usize> = None;
    for line in lines {
        let Some(name) = line.word(0) else {
            errors.push(Error::new(
                line.range.clone(),
                format!(
                    "expected a section: {}",
                    SECTIONS.map(|s| format!("`{s}`")).join(", ")
                ),
            ));
            continue;
        };
        let Some(index) = SECTIONS.iter().position(|section| *section == name) else {
            let hint = match name {
                "foreign" => "; a package holds Una code only, and there is no `foreign` section",
                "ita" => "; a file has one `ita` head",
                _ => "",
            };
            errors.push(Error::new(
                line.head(),
                format!("unknown section `{name}`{hint}"),
            ));
            continue;
        };
        if line.tokens.len() > 1 {
            errors.push(Error::new(
                line.range.clone(),
                format!("the section `{name}` is a word alone on its line"),
            ));
            continue;
        }
        if manifest.sections.iter().any(|seen| seen.value == name) {
            errors.push(Error::new(
                line.head(),
                format!("the section `{name}` appears twice"),
            ));
            continue;
        }
        if last.is_some_and(|last| last > index) {
            errors.push(Error::new(
                line.head(),
                format!(
                    "the section `{name}` is out of order; sections appear in the order {}",
                    SECTIONS.map(|s| format!("`{s}`")).join(", ")
                ),
            ));
            continue;
        }
        last = Some(index);
        manifest
            .sections
            .push(At::new(name.to_owned(), line.head()));
        section(name, line, &mut manifest, &mut errors);
    }
    for contract in &mut manifest.contracts {
        for clause in &mut contract.clauses {
            // The clause as written, a `\r\n` within it read as `\n`: decoded here for a
            // written text, never twice for a decoded one (#2252).
            let value = written.get(clause.range.clone()).unwrap_or("");
            clause.value = if decoded {
                value.to_owned()
            } else {
                value.replace("\r\n", "\n")
            };
        }
    }
    (manifest, errors)
}

fn head(line: &Line, manifest: &mut Manifest, errors: &mut Vec<Error>) {
    manifest.head = line.range.clone();
    let mut cursor = Cursor::new(line);
    cursor.next();
    match cursor
        .pkg_name()
        .and_then(|name| cursor.end().map(|()| name))
    {
        Ok(name) => manifest.name = Some(name),
        Err(error) => errors.push(error),
    }
    let mut seen: Vec<&str> = Vec::new();
    for child in &line.children {
        no_children(child, errors);
        let Some(word) = child.word(0) else {
            errors.push(Error::new(
                child.range.clone(),
                format!(
                    "expected a head line: {}",
                    HEAD_LINES.map(|s| format!("`{s}`")).join(", ")
                ),
            ));
            continue;
        };
        let Some(&known) = HEAD_LINES.iter().find(|known| **known == word) else {
            errors.push(Error::new(
                child.head(),
                format!("unknown head line `{word}`"),
            ));
            continue;
        };
        if seen.contains(&known) {
            errors.push(Error::new(
                child.head(),
                format!("the head line `{known}` appears twice"),
            ));
            continue;
        }
        seen.push(known);
        let mut cursor = Cursor::new(child);
        cursor.next();
        let result = match known {
            "version" | "edition" => cursor.version(&format!("a version after `{known}`")),
            "profile" => cursor.word("`app` or `systems`"),
            "about" | "license" | "home" => cursor.text(&format!("a text after `{known}`")),
            _ => {
                if cursor.eat_word("to") {
                    cursor.word("a source name")
                } else {
                    Err(Error::new(cursor.here(), "expected `publish to <source>`"))
                }
            }
        }
        .and_then(|value| cursor.end().map(|()| value));
        match result {
            Ok(value) => {
                let slot = match known {
                    "version" => &mut manifest.version,
                    "edition" => &mut manifest.edition,
                    "profile" => &mut manifest.profile,
                    "about" => &mut manifest.about,
                    "license" => &mut manifest.license,
                    "home" => &mut manifest.home,
                    _ => &mut manifest.publish_to,
                };
                *slot = Some(value);
            }
            Err(error) => errors.push(error),
        }
    }
    for required in ["version", "edition"] {
        if !seen.contains(&required) {
            errors.push(Error::new(
                line.range.clone(),
                format!("the head has no `{required}` line"),
            ));
        }
    }
}

fn section(name: &str, line: &Line, manifest: &mut Manifest, errors: &mut Vec<Error>) {
    match name {
        "sources" => sources(line, &mut manifest.sources, errors),
        "needs" => {
            for child in &line.children {
                match need(child) {
                    Ok(need) => manifest.needs.push(need),
                    Err(error) => errors.push(error),
                }
            }
        }
        "replace" => replace(line, &mut manifest.replace, errors),
        "trust" => trust(line, &mut manifest.trust, errors),
        "labels" => {
            for child in &line.children {
                no_children(child, errors);
                match child.tokens.as_slice() {
                    [
                        Tok {
                            item: Item::Upper(label),
                            range,
                        },
                    ] => manifest.labels.push(At::new(label.clone(), range.clone())),
                    _ => errors.push(Error::new(
                        child.range.clone(),
                        "a line of `labels` is one category name, such as `PII`",
                    )),
                }
            }
        }
        "trusted" => module_lines(line, &mut manifest.trusted, errors),
        "escapes" => {
            for child in &line.children {
                no_children(child, errors);
                let mut cursor = Cursor::new(child);
                let result = if cursor.eat_word("checked")
                    && cursor.eat_word("per")
                    && cursor.eat_word("module")
                {
                    cursor
                        .int("the budget")
                        .and_then(|value| cursor.end().map(|()| value))
                } else {
                    Err(Error::new(
                        child.range.clone(),
                        "a line of `escapes` is `checked per module <n>`",
                    ))
                };
                match result {
                    Ok(value) if manifest.escapes.is_none() => manifest.escapes = Some(value),
                    Ok(_) => errors.push(Error::new(
                        child.range.clone(),
                        "`checked per module` appears twice",
                    )),
                    Err(error) => errors.push(error),
                }
            }
        }
        "boundaries" => {
            for child in &line.children {
                let mut cursor = Cursor::new(child);
                let module = match cursor
                    .module_path("a module path")
                    .and_then(|module| cursor.end().map(|()| module))
                {
                    Ok(module) => module,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                };
                let may = may_lines(child, errors);
                manifest.boundaries.push(BoundaryEntry { module, may });
            }
        }
        "contracts" => {
            for child in &line.children {
                let mut cursor = Cursor::new(child);
                let function = match cursor
                    .module_path("a function's qualified name")
                    .and_then(|function| cursor.end().map(|()| function))
                {
                    Ok(function) => function,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                };
                let mut clauses = Vec::new();
                for clause in &child.children {
                    no_children(clause, errors);
                    // The clause is parsed by the production a header uses, by the checker.
                    clauses.push(At::new(String::new(), clause.range.clone()));
                }
                if clauses.is_empty() {
                    errors.push(Error::new(
                        child.range.clone(),
                        format!(
                            "`{}` locks no `uses` clause; write each below it",
                            function.value
                        ),
                    ));
                }
                manifest.contracts.push(ContractEntry { function, clauses });
            }
        }
        "run" => manifest.run = Some(run(line, errors)),
        "drive" => manifest.drive = Some(drive(line, errors)),
        _ => manifest.serve = Some(serve(line, errors)),
    }
}

fn sources(line: &Line, out: &mut Vec<Source>, errors: &mut Vec<Error>) {
    for child in &line.children {
        let mut cursor = Cursor::new(child);
        let header = cursor.word("a source name").and_then(|name| {
            cursor
                .text("the source's base URL")
                .and_then(|url| cursor.end().map(|()| (name, url)))
        });
        let (name, url) = match header {
            Ok(header) => header,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let mut source = Source {
            name,
            url,
            root: None,
            owners: Vec::new(),
        };
        let mut owners_seen = false;
        for detail in &child.children {
            no_children(detail, errors);
            let mut cursor = Cursor::new(detail);
            if cursor.eat_word("root") {
                match cursor
                    .text("the digest of the source's root metadata")
                    .and_then(|root| cursor.end().map(|()| root))
                {
                    Ok(root) if source.root.is_none() => source.root = Some(root),
                    Ok(_) => errors.push(Error::new(detail.range.clone(), "`root` appears twice")),
                    Err(error) => errors.push(error),
                }
            } else if cursor.eat_word("owners") {
                if owners_seen {
                    errors.push(Error::new(detail.range.clone(), "`owners` appears twice"));
                    continue;
                }
                owners_seen = true;
                let result = (|| {
                    let mut owners = vec![cursor.word("an owner name")?];
                    while cursor.eat_punct(",") {
                        owners.push(cursor.word("an owner name")?);
                    }
                    cursor.end()?;
                    Ok(owners)
                })();
                match result {
                    Ok(owners) => source.owners = owners,
                    Err(error) => errors.push(error),
                }
            } else {
                errors.push(Error::new(
                    detail.range.clone(),
                    "a source's lines are `root \"sha256:...\"` and `owners <owner>, ...`",
                ));
            }
        }
        if source.root.is_none() {
            errors.push(Error::new(
                child.range.clone(),
                format!(
                    "the source `{}` has no `root` line pinning its root metadata",
                    source.name.value
                ),
            ));
        }
        out.push(source);
    }
}

/// `from ...` after the `from` word.
fn origin(cursor: &mut Cursor<'_>, from: Range<usize>) -> Result<At<Origin>, Error> {
    let word = cursor.word("a source, `path \"<dir>\"` or `workspace` after `from`")?;
    let start = from.start;
    match word.value.as_str() {
        "path" => {
            let dir = cursor.text("the directory of a path dependency")?;
            Ok(At::new(Origin::Path(dir.value), start..dir.range.end))
        }
        "workspace" => Ok(At::new(Origin::Workspace, start..word.range.end)),
        "git" => Err(Error::new(
            word.range,
            "there are no git dependencies; use a path dependency, a workspace member, or a source",
        )),
        _ => Ok(At::new(Origin::Source(word.value), start..word.range.end)),
    }
}

fn need(line: &Line) -> Result<Need, Error> {
    let mut cursor = Cursor::new(line);
    let name = cursor.pkg_name()?;
    let version = match cursor.peek().map(|t| &t.item) {
        Some(Item::Version(_)) => Some(cursor.version("a version")?),
        Some(Item::Int(_)) => {
            return Err(Error::new(
                cursor.here(),
                "a version names at least `MAJOR.MINOR`, such as `2.0`",
            ));
        }
        _ => None,
    };
    let local = if cursor.eat_word("as") {
        Some(cursor.word("a local name after `as`")?)
    } else {
        None
    };
    let origin = match cursor.peek() {
        Some(Tok {
            item: Item::Word(word),
            range,
        }) if word == "from" => {
            let range = range.clone();
            cursor.next();
            Some(origin(&mut cursor, range)?)
        }
        _ => None,
    };
    cursor.end()?;
    let mut need = Need {
        name,
        version,
        local,
        origin,
        may: Vec::new(),
        avoid: Vec::new(),
        accept_name: None,
        range: line.range.clone(),
    };
    for child in &line.children {
        if let Some(grandchild) = child.children.first() {
            return Err(Error::new(
                grandchild.range.clone(),
                "unexpected indented line here; this line opens no block",
            ));
        }
        let mut cursor = Cursor::new(child);
        if cursor.eat_word("may") {
            need.may.extend(cursor.cap_types()?);
            cursor.end()?;
        } else if cursor.eat_word("avoid") {
            need.avoid.push(cursor.version("the version to avoid")?);
            cursor.end()?;
        } else if cursor.eat_word("accept") {
            if !cursor.eat_word("name") {
                return Err(Error::new(cursor.here(), "expected `accept name`"));
            }
            cursor.end()?;
            if need.accept_name.is_some() {
                return Err(Error::new(
                    child.range.clone(),
                    "`accept name` appears twice",
                ));
            }
            need.accept_name = Some(child.range.clone());
        } else {
            return Err(Error::new(
                child.range.clone(),
                "a need's lines are `may C[ops]`, `avoid <version>` and `accept name`",
            ));
        }
    }
    Ok(need)
}

fn replace(line: &Line, out: &mut Vec<Replace>, errors: &mut Vec<Error>) {
    for child in &line.children {
        no_children(child, errors);
        let mut cursor = Cursor::new(child);
        let result = (|| {
            let name = cursor.pkg_name()?;
            let version = cursor.version("the version replaced")?;
            if !cursor.eat_word("with") {
                return Err(Error::new(cursor.here(), "expected `with` and an origin"));
            }
            let at = cursor.here();
            let origin = origin(&mut cursor, at)?;
            cursor.end()?;
            Ok(Replace {
                name,
                version,
                origin,
            })
        })();
        match result {
            Ok(entry) => out.push(entry),
            Err(error) => errors.push(error),
        }
    }
}

fn trust(line: &Line, out: &mut Vec<TrustLine>, errors: &mut Vec<Error>) {
    for child in &line.children {
        let mut cursor = Cursor::new(child);
        let result = (|| {
            let name = cursor.pkg_name()?;
            let major = match cursor.peek() {
                Some(Tok {
                    item: Item::Int(digits) | Item::Version(digits),
                    range,
                }) => {
                    let major = At::new(digits.clone(), range.clone());
                    cursor.next();
                    Some(major)
                }
                _ => None,
            };
            let from = if cursor.eat_word("from") {
                let source = cursor.word("a source name")?;
                if source.value == "git" || source.value == "path" || source.value == "workspace" {
                    return Err(Error::new(
                        source.range,
                        "a trust line names the registry source of a package",
                    ));
                }
                Some(source)
            } else {
                None
            };
            if !cursor.eat_word("modules") {
                return Err(Error::new(
                    cursor.here(),
                    "expected `modules` and the trusted module paths",
                ));
            }
            let mut modules = vec![cursor.module_path("a module path")?];
            while cursor.eat_punct(",") {
                modules.push(cursor.module_path("a module path")?);
            }
            cursor.end()?;
            Ok(TrustLine {
                name,
                major,
                from,
                modules,
                may: Vec::new(),
                range: child.range.clone(),
            })
        })();
        match result {
            Ok(mut entry) => {
                entry.may = may_lines(child, errors);
                out.push(entry);
            }
            Err(error) => errors.push(error),
        }
    }
}

/// The `may` lines below `line`: a ceiling.
fn may_lines(line: &Line, errors: &mut Vec<Error>) -> Vec<CapType> {
    let mut may = Vec::new();
    for child in &line.children {
        no_children(child, errors);
        let mut cursor = Cursor::new(child);
        if !cursor.eat_word("may") {
            errors.push(Error::new(
                child.range.clone(),
                "expected a `may C[ops]` line",
            ));
            continue;
        }
        match cursor
            .cap_types()
            .and_then(|caps| cursor.end().map(|()| caps))
        {
            Ok(caps) => may.extend(caps),
            Err(error) => errors.push(error),
        }
    }
    may
}

fn module_lines(line: &Line, out: &mut Vec<At<String>>, errors: &mut Vec<Error>) {
    for child in &line.children {
        no_children(child, errors);
        let mut cursor = Cursor::new(child);
        match cursor
            .module_path("a module path")
            .and_then(|module| cursor.end().map(|()| module))
        {
            Ok(module) => out.push(module),
            Err(error) => errors.push(error),
        }
    }
}

fn run(line: &Line, errors: &mut Vec<Error>) -> Run {
    let mut run = Run::default();
    let mut grants_seen = false;
    for child in &line.children {
        let mut cursor = Cursor::new(child);
        if cursor.eat_word("entry") {
            no_children(child, errors);
            match cursor
                .module_path("the entry module's path")
                .and_then(|entry| cursor.end().map(|()| entry))
            {
                Ok(_) if run.entry.is_some() => {
                    errors.push(Error::new(child.range.clone(), "`entry` appears twice"));
                }
                Ok(_) if run.max_memory.is_some() => {
                    errors.push(Error::new(
                        child.range.clone(),
                        "`entry` comes before `max_memory`",
                    ));
                }
                Ok(_) if grants_seen => {
                    errors.push(Error::new(
                        child.range.clone(),
                        "`entry` comes before `grants`",
                    ));
                }
                Ok(entry) => run.entry = Some(entry),
                Err(error) => errors.push(error),
            }
        } else if cursor.eat_word("max_memory") {
            // `max_memory <int>` (ADR 0109 rule 4): its range is the project crate's check.
            no_children(child, errors);
            match cursor
                .int("the value of `max_memory`")
                .and_then(|value| cursor.end().map(|()| value))
            {
                Ok(_) if run.max_memory.is_some() => {
                    errors.push(Error::new(
                        child.range.clone(),
                        "`max_memory` appears twice",
                    ));
                }
                Ok(_) if grants_seen => {
                    errors.push(Error::new(
                        child.range.clone(),
                        "`max_memory` comes before `grants`",
                    ));
                }
                Ok(value) => run.max_memory = Some(value),
                Err(error) => errors.push(error),
            }
        } else if cursor.eat_word("grants") {
            if let Err(error) = cursor.end() {
                errors.push(error);
                continue;
            }
            if grants_seen {
                errors.push(Error::new(child.range.clone(), "`grants` appears twice"));
                continue;
            }
            grants_seen = true;
            for grant_line in &child.children {
                match grant(grant_line, errors) {
                    Ok(grant) => run.grants.push(grant),
                    Err(error) => errors.push(error),
                }
            }
        } else {
            errors.push(Error::new(
                child.range.clone(),
                "the lines of `run` are `entry <module>`, `max_memory <MiB>` and `grants`",
            ));
        }
    }
    run
}

/// A limit line (ADR 0117 rule 8): `at most <n> per second|minute|hour`, `at most <n> at once`
/// or `waiting at most <n>`, with no lines below it.
fn limit(line: &Line, errors: &mut Vec<Error>) -> Result<Limit, Error> {
    no_children(line, errors);
    let mut cursor = Cursor::new(line);
    let form = "a limit: `at most <n> per second`, `minute` or `hour`, `at most <n> at once`, or `waiting at most <n>`";
    let waiting = cursor.eat_word("waiting");
    if !cursor.eat_word("at") {
        return Err(expected(form, cursor.peek(), cursor.here()));
    }
    if !cursor.eat_word("most") {
        return Err(expected("`most` after `at`", cursor.peek(), cursor.here()));
    }
    let count = cursor.int("the count of the limit")?;
    let kind = if waiting {
        LimitKind::Waiting
    } else if cursor.eat_word("per") {
        let period = cursor.word("`second`, `minute` or `hour`")?;
        if !matches!(period.value.as_str(), "second" | "minute" | "hour") {
            return Err(Error::new(
                period.range,
                "a rate's period is `second`, `minute` or `hour`",
            ));
        }
        LimitKind::Rate(period.value)
    } else if cursor.eat_word("at") {
        if !cursor.eat_word("once") {
            return Err(expected("`once` after `at`", cursor.peek(), cursor.here()));
        }
        LimitKind::AtOnce
    } else {
        return Err(expected(
            "`per second`, `per minute`, `per hour` or `at once`",
            cursor.peek(),
            cursor.here(),
        ));
    };
    cursor.end()?;
    Ok(Limit {
        kind,
        count,
        range: line.range.clone(),
    })
}

/// A grant line. Every problem of a `serve` grant's item lines, or of another grant's limit
/// lines, is reported, and the grant is then dropped, so no grant is ever read with fewer limits
/// than written.
fn grant(line: &Line, errors: &mut Vec<Error>) -> Result<Grant, Error> {
    let mut cursor = Cursor::new(line);
    if cursor.eat_word("serve") {
        // The last error is the result; the others are reported beside it.
        return serve_grant(line, &mut cursor).map_err(|mut found| {
            let last = found
                .pop()
                .unwrap_or_else(|| Error::new(line.range.clone(), "invalid serve grant"));
            errors.extend(found);
            last
        });
    }
    grant_line(line, &mut cursor, errors)
}

fn grant_line(
    line: &Line,
    cursor: &mut Cursor<'_>,
    errors: &mut Vec<Error>,
) -> Result<Grant, Error> {
    let kind =
        cursor.word("a grant: `console`, `clock`, `env`, `keys`, `files`, `http` or `serve`")?;
    let spec = match kind.value.as_str() {
        "console" | "clock" | "env" => kind.value.clone(),
        "keys" => {
            let file = cursor.text("the key file")?;
            format!("keys:{}", file.value)
        }
        "files" => {
            let path = cursor.text("the path the grant covers")?;
            let mut modes: Vec<String> = Vec::new();
            loop {
                let mode = cursor.word("`read` or `write`")?;
                if mode.value != "read" && mode.value != "write" {
                    return Err(Error::new(
                        mode.range,
                        "a file grant's modes are `read` and `write`",
                    ));
                }
                if modes.contains(&mode.value) {
                    return Err(Error::new(
                        mode.range,
                        format!("`{}` appears twice", mode.value),
                    ));
                }
                modes.push(mode.value);
                if !cursor.eat_punct(",") {
                    break;
                }
            }
            if modes == ["write", "read"] {
                modes.reverse();
            }
            format!("files:{}:{}", path.value, modes.join(","))
        }
        "http" => {
            let origin = cursor.text("the origin the grant covers")?;
            let mut methods: Vec<String> = Vec::new();
            if !cursor.done() {
                loop {
                    let method = cursor.word("an HTTP method: `get`, `post`, `put` or `delete`")?;
                    if !matches!(method.value.as_str(), "get" | "post" | "put" | "delete") {
                        return Err(Error::new(
                            method.range,
                            "the HTTP methods are `get`, `post`, `put` and `delete`",
                        ));
                    }
                    if methods.contains(&method.value) {
                        return Err(Error::new(
                            method.range,
                            format!("`{}` appears twice", method.value),
                        ));
                    }
                    methods.push(method.value);
                    if !cursor.eat_punct(",") {
                        break;
                    }
                }
            }
            if methods.is_empty() {
                format!("http:{}", origin.value)
            } else {
                format!("http:{}:{}", origin.value, methods.join(","))
            }
        }
        "foreign" => {
            return Err(Error::new(
                kind.range,
                "there is no foreign grant; a package holds Una code only",
            ));
        }
        other => {
            return Err(Error::new(
                kind.range,
                format!(
                    "unknown grant `{other}`; the grants are `console`, `clock`, `env`, `keys`, `files`, `http` and `serve`"
                ),
            ));
        }
    };
    cursor.end()?;
    // A grant whose limit lines do not all parse is dropped: read without one of its limits,
    // it would allow more than written.
    let mut limits = Vec::new();
    let mut failed: Option<Error> = None;
    for child in &line.children {
        match limit(child, errors) {
            Ok(limit) => limits.push(limit),
            Err(error) => {
                if let Some(earlier) = failed.replace(error) {
                    errors.push(earlier);
                }
            }
        }
    }
    if let Some(error) = failed {
        return Err(error);
    }
    Ok(Grant {
        spec,
        range: line.range.clone(),
        limits,
        items: Vec::new(),
    })
}

/// `serve "<origin>"` and its item lines (`spec/31 §2` rule 3), written as the text spec
/// `serve:<origin>;<item>;...`. Whether the origin and the values are valid is
/// `una_core::http::serve`'s to say; this reads the form of each line. An error of an item
/// line is reported at the line.
fn serve_grant(line: &Line, cursor: &mut Cursor<'_>) -> Result<Grant, Vec<Error>> {
    let origin = cursor
        .text("the address the grant listens on, as \"http://127.0.0.1:8080\"")
        .and_then(|origin| cursor.end().map(|()| origin))
        .map_err(|error| vec![error])?;
    // Each item is a line of its own, so the origin cannot carry one after a `;`.
    if origin.value.contains(';') {
        return Err(vec![Error::new(
            origin.range,
            "the address of a serve grant holds no `;`; write each item on a line below it",
        )]);
    }
    let mut spec = format!("serve:{}", origin.value);
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for child in &line.children {
        match serve_item(child) {
            Ok(item) => {
                spec.push(';');
                spec.push_str(&item);
                items.push(child.range.clone());
            }
            Err(message) => errors.push(Error::new(child.range.clone(), message)),
        }
        if !child.children.is_empty() {
            errors.push(Error::new(
                child.range.clone(),
                "an item of a serve grant has no lines below it",
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Grant {
        spec,
        range: line.range.clone(),
        limits: Vec::new(),
        items,
    })
}

/// One item line of a `serve` grant, as its text item: `certificate "F"` is `certificate=F`,
/// `at most 6 at once` is `6/once`, `body at most 64 KiB` is `body=64`, and so on.
fn serve_item(line: &Line) -> Result<String, String> {
    let mut cursor = Cursor::new(line);
    let message = |error: Error| error.message;
    let name = cursor
        .word("an item of a serve grant (spec/31 §2)")
        .map_err(message)?;
    let item = match name.value.as_str() {
        "tls12" => "tls12".to_owned(),
        "certificate" | "key" => {
            let path = cursor.text("the file's path").map_err(message)?;
            if path.value.contains(';') {
                return Err(format!(
                    "the path of `{}` holds `;`, which ends an item of a grant",
                    name.value
                ));
            }
            format!("{}={}", name.value, path.value)
        }
        "hosts" => {
            let mut names = Vec::new();
            loop {
                let host = cursor.text("a host name").map_err(message)?;
                if host.value.contains(',') || host.value.contains(';') {
                    return Err(format!("`{}` is not a host name", host.value));
                }
                names.push(host.value);
                if !cursor.eat_punct(",") {
                    break;
                }
            }
            format!("hosts={}", names.join(","))
        }
        "at" => {
            if !cursor.eat_word("most") {
                return Err("expected `at most`".to_owned());
            }
            let count = cursor.digits()?;
            if cursor.eat_word("at") && cursor.eat_word("once") {
                format!("{count}/once")
            } else if cursor.eat_word("per") {
                let period = cursor
                    .word("`second`, `minute` or `hour`")
                    .map_err(message)?;
                if !matches!(period.value.as_str(), "second" | "minute" | "hour") {
                    return Err("a rate is per `second`, `minute` or `hour`".to_owned());
                }
                format!("{count}/{}", period.value)
            } else {
                return Err("expected `at once`, or `per` and a period".to_owned());
            }
        }
        "peer" | "peer_buffers" | "peer_replies" if cursor.eat_word("none") => {
            format!("{}=none", name.value)
        }
        "waiting" | "connections" | "requests" | "headers" | "peer" => {
            format!("{}={}", name.value, cursor.at_most()?)
        }
        "handshakes" => {
            let count = cursor.at_most()?;
            if !(cursor.eat_word("per") && cursor.eat_word("second")) {
                return Err(
                    "`handshakes` is a number per second: `handshakes at most 512 per second`"
                        .to_owned(),
                );
            }
            format!("handshakes={count}")
        }
        "line" | "header" | "body" => {
            let count = cursor.at_most()?;
            cursor.unit("KiB", &name.value)?;
            format!("{}={count}", name.value)
        }
        "buffers" | "replies" | "delivered" | "peer_buffers" | "peer_replies" => {
            let count = cursor.at_most()?;
            cursor.unit("MiB", &name.value)?;
            format!("{}={count}", name.value)
        }
        "idle" | "read" | "write" | "answer" => {
            if !cursor.eat_word("within") {
                return Err(format!("expected `{} within <n> seconds`", name.value));
            }
            let count = cursor.digits()?;
            if !(cursor.eat_word("seconds") || cursor.eat_word("second")) {
                return Err(format!("`{}` is a number of seconds", name.value));
            }
            format!("{}={count}", name.value)
        }
        other => {
            return Err(format!(
                "`{other}` is not an item of a serve grant (spec/31 §2)"
            ));
        }
    };
    cursor.end().map_err(message)?;
    Ok(item)
}

fn drive(line: &Line, errors: &mut Vec<Error>) -> Drive {
    let mut drive = Drive {
        module: None,
        ticks: None,
        budget: None,
        range: line.range.clone(),
    };
    for child in &line.children {
        no_children(child, errors);
        let mut cursor = Cursor::new(child);
        let word = child.word(0).unwrap_or_default().to_owned();
        cursor.next();
        let result = match word.as_str() {
            "module" => cursor
                .module_path("the driven module's path")
                .and_then(|module| cursor.end().map(|()| module))
                .map(|module| {
                    if drive.module.is_some() {
                        Err(Error::new(child.range.clone(), "`module` appears twice"))
                    } else {
                        drive.module = Some(module);
                        Ok(())
                    }
                }),
            "ticks" | "budget" => cursor
                .int(&format!("the value of `{word}`"))
                .and_then(|value| cursor.end().map(|()| value))
                .map(|value| {
                    let slot = if word == "ticks" {
                        &mut drive.ticks
                    } else {
                        &mut drive.budget
                    };
                    if slot.is_some() {
                        Err(Error::new(
                            child.range.clone(),
                            format!("`{word}` appears twice"),
                        ))
                    } else {
                        *slot = Some(value);
                        Ok(())
                    }
                }),
            _ => Err(Error::new(
                child.range.clone(),
                "the lines of `drive` are `module`, `ticks` and `budget`",
            )),
        };
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) | Err(error) => errors.push(error),
        }
    }
    drive
}

fn serve(line: &Line, errors: &mut Vec<Error>) -> Serve {
    fn end(line: &Line) -> usize {
        line.children
            .iter()
            .map(end)
            .chain([line.range.end])
            .max()
            .unwrap_or(line.range.end)
    }
    let mut serve = Serve {
        module: None,
        within: None,
        range: line.range.clone(),
        extent: line.range.start..end(line),
    };
    for child in &line.children {
        no_children(child, errors);
        let mut cursor = Cursor::new(child);
        let word = child.word(0).unwrap_or_default().to_owned();
        cursor.next();
        let result = match word.as_str() {
            "module" => cursor
                .module_path("the served module's path")
                .and_then(|module| cursor.end().map(|()| module))
                .map(|module| {
                    if serve.module.is_some() {
                        Err(Error::new(child.range.clone(), "`module` appears twice"))
                    } else {
                        serve.module = Some(module);
                        Ok(())
                    }
                }),
            "within" => cursor
                .int("the value of `within`")
                .and_then(|value| cursor.end().map(|()| value))
                .map(|value| {
                    if serve.within.is_some() {
                        Err(Error::new(child.range.clone(), "`within` appears twice"))
                    } else {
                        serve.within = Some(value);
                        Ok(())
                    }
                }),
            _ => Err(Error::new(
                child.range.clone(),
                "the lines of `serve` are `module` and `within`",
            )),
        };
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) | Err(error) => errors.push(error),
        }
    }
    serve
}

// ---------------------------------------------------------------------------------------------
// `ita.lock`
// ---------------------------------------------------------------------------------------------

/// The `solver` section of `ita.lock`, with the six keys of `spec/12 §9` (ADR 0122 rule 18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockSolver {
    /// `encoding`.
    pub encoding: At<String>,
    /// `name`.
    pub name: At<String>,
    /// `version`.
    pub version: At<String>,
    /// `logic`.
    pub logic: At<String>,
    /// `steps`.
    pub steps: At<u64>,
    /// `seed`.
    pub seed: At<u64>,
}

/// A `registry` section of `ita.lock` (ADR 0122 rule 15).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockRegistry {
    /// The source's name.
    pub source: At<String>,
    /// The digest of its root metadata.
    pub root: At<String>,
    /// The resolution point: the checkpoint's size.
    pub checkpoint_size: At<u64>,
    /// The resolution point: the checkpoint's root hash.
    pub checkpoint_hash: At<String>,
    /// `last root <n> "<digest>"`: the last root version the client verified when it wrote the
    /// lock, which a vendored provenance's root chain must hold (ADR 0122, amendment of
    /// 2026-11-03, item 4).
    pub last_root: At<u64>,
    /// That version's digest.
    pub last_root_digest: At<String>,
    /// `accepted <owner> <index> …`: per owner, the recoveries, disputes and co-signer changes
    /// of its chain pending at the resolution point whose pending state the lock was written under
    /// (`spec/30 §14`).
    pub accepted: Vec<LockAccepted>,
    /// `effect <owner> <index> …`: per owner, the recoveries, disputes and co-signer changes of
    /// its chain in effect at the resolution point whose taking effect the lock was written under.
    pub effects: Vec<LockAccepted>,
}

/// An `accepted` line of a `registry` section: an owner and the log indices of the entries of
/// its chain whose key change is consented to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockAccepted {
    /// The owner.
    pub owner: At<String>,
    /// The log indices, ascending, each once.
    pub indices: Vec<At<u64>>,
    /// Where the line is.
    pub range: Range<usize>,
}

/// Where a locked package comes from (ADR 0122 rule 16, `lock_origin`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockOrigin {
    /// A registry source.
    Source(String),
    /// A path, relative to the root.
    Path(String),
    /// A workspace member, by its directory.
    Workspace(String),
}

/// A `granted by` line of a locked package (ADR 0122 rule 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockGrant {
    /// The dependent.
    pub by: At<PkgName>,
    /// The dependent's major and source, for a dependent other than the root.
    pub identity: Option<(At<String>, At<String>)>,
    /// The ceiling it wrote.
    pub may: Vec<CapType>,
}

/// A `needs` line of a locked package: source, name, version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockNeed {
    /// The source.
    pub source: At<String>,
    /// The package.
    pub name: At<PkgName>,
    /// The version selected.
    pub version: At<String>,
}

/// A `package` section of `ita.lock` (ADR 0122 rule 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockPackage {
    /// The package's name.
    pub name: At<PkgName>,
    /// Its version.
    pub version: At<String>,
    /// Its origin.
    pub from: Option<At<LockOrigin>>,
    /// Its content digest.
    pub digest: Option<At<String>>,
    /// The publisher key that signed its release statement.
    pub signed: Option<At<String>>,
    /// The log index of its release statement.
    pub logged: Option<At<u64>>,
    /// Whether `replace` supplied it.
    pub replaced: bool,
    /// Its own resolved needs.
    pub needs: Vec<LockNeed>,
    /// The ceilings its dependents wrote for it.
    pub granted: Vec<LockGrant>,
    /// The content digest under which its trust was consented.
    pub consented: Option<At<String>>,
    /// The trusted modules consented.
    pub trusted: Vec<At<String>>,
    /// The section's header line.
    pub range: Range<usize>,
}

/// `ita.lock`, as read (ADR 0122 rule 16).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lock {
    /// The format's version, `lock <n>`.
    pub format: Option<At<u64>>,
    /// The solver pin.
    pub solver: Option<LockSolver>,
    /// The registries.
    pub registries: Vec<LockRegistry>,
    /// The packages, in the order written.
    pub packages: Vec<LockPackage>,
}

/// The format version of `ita.lock` this release writes and reads (ADR 0122 rule 16).
pub const LOCK_FORMAT: u64 = 1;

/// Reads `ita.lock` by the grammar of ADR 0122 rule 16. O(n).
pub fn parse_lock<'a>(text: impl Into<Text<'a>>) -> (Lock, Vec<Error>) {
    let text = text.into();
    let mut lock = Lock::default();
    if text.raw_len() > MAX_LOCK_BYTES {
        return (
            lock,
            vec![Error::new(
                0..0,
                format!(
                    "the file holds {} bytes, more than the {MAX_LOCK_BYTES} a lock may hold",
                    text.raw_len()
                ),
            )],
        );
    }
    let read = lines(text);
    let mut errors = read.errors;
    let mut lines = read.lines.iter().peekable();
    match lines.next() {
        Some(line) if line.word(0) == Some("lock") => {
            no_children(line, &mut errors);
            let mut cursor = Cursor::new(line);
            cursor.next();
            match cursor
                .int("the lock's format version")
                .and_then(|format| cursor.end().map(|()| format))
            {
                Ok(format) if format.value == LOCK_FORMAT => lock.format = Some(format),
                Ok(format) => errors.push(Error::new(
                    format.range,
                    format!(
                        "lock format {} is not one this release reads; it reads format {LOCK_FORMAT}",
                        format.value
                    ),
                )),
                Err(error) => errors.push(error),
            }
        }
        _ => {
            errors.push(Error::new(0..0, "ita.lock starts with `lock 1`"));
            return (lock, errors);
        }
    }
    match lines.next() {
        Some(line) if line.word(0) == Some("solver") && line.tokens.len() == 1 => {
            lock.solver = solver(line, &mut errors);
        }
        Some(line) => {
            errors.push(Error::new(
                line.range.clone(),
                "the `solver` section follows `lock 1`",
            ));
            return (lock, errors);
        }
        None => {
            errors.push(Error::new(0..0, "ita.lock has no `solver` section"));
            return (lock, errors);
        }
    }
    let mut packages_started = false;
    for line in lines {
        match line.word(0) {
            Some("registry") if !packages_started => match registry(line, &mut errors) {
                Some(registry) => lock.registries.push(registry),
                None => continue,
            },
            Some("registry") => errors.push(Error::new(
                line.head(),
                "`registry` sections come before the `package` sections",
            )),
            Some("package") => {
                packages_started = true;
                if let Some(package) = package(line, &mut errors) {
                    lock.packages.push(package);
                }
            }
            _ => errors.push(Error::new(
                line.range.clone(),
                "expected a `registry` or `package` section",
            )),
        }
    }
    (lock, errors)
}

fn solver(line: &Line, errors: &mut Vec<Error>) -> Option<LockSolver> {
    const KEYS: [&str; 6] = ["encoding", "name", "version", "logic", "steps", "seed"];
    let mut texts: [Option<At<String>>; 4] = [None, None, None, None];
    let mut numbers: [Option<At<u64>>; 2] = [None, None];
    let mut order = 0;
    for child in &line.children {
        no_children(child, errors);
        let Some(key) = child.word(0) else {
            errors.push(Error::new(
                child.range.clone(),
                "expected a key of `solver`",
            ));
            continue;
        };
        let Some(index) = KEYS.iter().position(|k| *k == key) else {
            errors.push(Error::new(
                child.head(),
                format!("unknown key `{key}` in `solver`"),
            ));
            continue;
        };
        if index < order {
            errors.push(Error::new(
                child.head(),
                format!(
                    "`{key}` is out of order; the keys of `solver` are {}",
                    KEYS.map(|k| format!("`{k}`")).join(", ")
                ),
            ));
            continue;
        }
        let mut cursor = Cursor::new(child);
        cursor.next();
        if let Some(slot) = texts.get_mut(index) {
            if slot.is_some() {
                errors.push(Error::new(child.head(), format!("`{key}` appears twice")));
                continue;
            }
            match cursor
                .text(&format!("the text of `{key}`"))
                .and_then(|value| cursor.end().map(|()| value))
            {
                Ok(value) => *slot = Some(value),
                Err(error) => errors.push(error),
            }
        } else if let Some(slot) = numbers.get_mut(index - 4) {
            if slot.is_some() {
                errors.push(Error::new(child.head(), format!("`{key}` appears twice")));
                continue;
            }
            match cursor
                .int(&format!("the value of `{key}`"))
                .and_then(|value| cursor.end().map(|()| value))
            {
                Ok(value) => *slot = Some(value),
                Err(error) => errors.push(error),
            }
        }
        order = index;
    }
    let [encoding, name, version, logic] = texts;
    let [steps, seed] = numbers;
    let mut missing = Vec::new();
    for (key, present) in KEYS.iter().zip([
        encoding.is_some(),
        name.is_some(),
        version.is_some(),
        logic.is_some(),
        steps.is_some(),
        seed.is_some(),
    ]) {
        if !present {
            missing.push(format!("`{key}`"));
        }
    }
    if !missing.is_empty() {
        errors.push(Error::new(
            line.range.clone(),
            format!("`solver` has no {}", missing.join(", ")),
        ));
        return None;
    }
    Some(LockSolver {
        encoding: encoding?,
        name: name?,
        version: version?,
        logic: logic?,
        steps: steps?,
        seed: seed?,
    })
}

fn registry(line: &Line, errors: &mut Vec<Error>) -> Option<LockRegistry> {
    let mut cursor = Cursor::new(line);
    cursor.next();
    let source = match cursor
        .word("the registry's source name")
        .and_then(|source| cursor.end().map(|()| source))
    {
        Ok(source) => source,
        Err(error) => {
            errors.push(error);
            return None;
        }
    };
    let mut root = None;
    let mut checkpoint = None;
    let mut last = None;
    let mut accepted: Vec<LockAccepted> = Vec::new();
    let mut effects: Vec<LockAccepted> = Vec::new();
    for child in &line.children {
        no_children(child, errors);
        let mut cursor = Cursor::new(child);
        let consent = if last.is_none() {
            None
        } else if cursor.eat_word("accepted") && effects.is_empty() {
            Some(false)
        } else if cursor.eat_word("effect") {
            Some(true)
        } else {
            None
        };
        if let Some(effect) = consent {
            let size = checkpoint
                .as_ref()
                .map_or(0, |(size, _): &(At<u64>, At<String>)| size.value);
            let owner = match cursor.word("the owner whose key change is consented to") {
                Ok(owner) => owner,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            let mut indices: Vec<At<u64>> = Vec::new();
            let mut bad = false;
            while !cursor.done() {
                match cursor.int("a log index") {
                    Ok(index) if index.value >= size => {
                        errors.push(Error::new(
                            index.range.clone(),
                            format!(
                                "the {} index {} is not below the registry's checkpoint size {size}",
                                if effect { "effect" } else { "accepted" },
                                index.value
                            ),
                        ));
                        bad = true;
                    }
                    Ok(index) => indices.push(index),
                    Err(error) => {
                        errors.push(error);
                        bad = true;
                        break;
                    }
                }
            }
            if bad {
                continue;
            }
            let list = if effect { &mut effects } else { &mut accepted };
            let ascending = indices
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if a.value < b.value));
            let ordered = list
                .last()
                .is_none_or(|previous: &LockAccepted| previous.owner.value < owner.value);
            if indices.is_empty() || !ascending || !ordered {
                errors.push(Error::new(
                    child.range.clone(),
                    "an `accepted` or `effect` line names an owner after the previous one's, then its log indices in ascending order, each once; `accepted` lines come before `effect` lines",
                ));
                continue;
            }
            list.push(LockAccepted {
                owner,
                indices,
                range: child.range.clone(),
            });
        } else if cursor.eat_word("root") && root.is_none() && checkpoint.is_none() {
            match cursor
                .text("the digest of the root metadata")
                .and_then(|value| cursor.end().map(|()| value))
            {
                Ok(value) => root = Some(value),
                Err(error) => errors.push(error),
            }
        } else if cursor.eat_word("checkpoint") && root.is_some() && checkpoint.is_none() {
            match cursor.int("the checkpoint's size").and_then(|size| {
                cursor
                    .text("the checkpoint's root hash")
                    .and_then(|hash| cursor.end().map(|()| (size, hash)))
            }) {
                Ok(value) => checkpoint = Some(value),
                Err(error) => errors.push(error),
            }
        } else if checkpoint.is_some() && last.is_none() && cursor.eat_word("last") {
            let read = if cursor.eat_word("root") {
                cursor.int("the last root version").and_then(|version| {
                    cursor
                        .text("the digest of the last root version")
                        .and_then(|digest| cursor.end().map(|()| (version, digest)))
                })
            } else {
                Err(Error::new(
                    child.range.clone(),
                    "expected `last root <version> \"...\"`",
                ))
            };
            match read {
                Ok(value) => last = Some(value),
                Err(error) => errors.push(error),
            }
        } else {
            errors.push(Error::new(
                child.range.clone(),
                "a registry's lines are `root \"...\"`, `checkpoint <size> \"...\"` and `last root <version> \"...\"`, once each and in this order, then its `accepted` and `effect` lines",
            ));
        }
    }
    match (root, checkpoint, last) {
        (
            Some(root),
            Some((checkpoint_size, checkpoint_hash)),
            Some((last_root, last_root_digest)),
        ) => Some(LockRegistry {
            source,
            root,
            checkpoint_size,
            checkpoint_hash,
            last_root,
            last_root_digest,
            accepted,
            effects,
        }),
        _ => {
            errors.push(Error::new(
                line.range.clone(),
                "a registry section has a `root`, a `checkpoint` and a `last root` line; write the lock again with `una ita lock`",
            ));
            None
        }
    }
}

fn lock_origin(cursor: &mut Cursor<'_>) -> Result<At<LockOrigin>, Error> {
    let word = cursor.word("a source, `path \"...\"` or `workspace \"...\"`")?;
    match word.value.as_str() {
        "path" => {
            let dir = cursor.text("the package's directory")?;
            Ok(At::new(
                LockOrigin::Path(dir.value),
                word.range.start..dir.range.end,
            ))
        }
        "workspace" => {
            let dir = cursor.text("the member's directory")?;
            Ok(At::new(
                LockOrigin::Workspace(dir.value),
                word.range.start..dir.range.end,
            ))
        }
        "git" => Err(Error::new(
            word.range,
            "there are no git origins; a package comes from a source, a path or a workspace",
        )),
        _ => Ok(At::new(LockOrigin::Source(word.value), word.range)),
    }
}

fn package(line: &Line, errors: &mut Vec<Error>) -> Option<LockPackage> {
    let mut cursor = Cursor::new(line);
    cursor.next();
    let header = cursor.pkg_name().and_then(|name| {
        cursor
            .version("the package's version")
            .and_then(|version| cursor.end().map(|()| (name, version)))
    });
    let (name, version) = match header {
        Ok(header) => header,
        Err(error) => {
            errors.push(error);
            return None;
        }
    };
    let mut package = LockPackage {
        name,
        version,
        from: None,
        digest: None,
        signed: None,
        logged: None,
        replaced: false,
        needs: Vec::new(),
        granted: Vec::new(),
        consented: None,
        trusted: Vec::new(),
        range: line.range.clone(),
    };
    // The order of rule 16: from, digest, [signed, logged], [replaced], needs*, granted*,
    // [consented], trusted*.
    const ORDER: [&str; 9] = [
        "from",
        "digest",
        "signed",
        "logged",
        "replaced",
        "needs",
        "granted",
        "consented",
        "trusted",
    ];
    let mut at = 0;
    for child in &line.children {
        no_children(child, errors);
        let Some(key) = child.word(0) else {
            errors.push(Error::new(
                child.range.clone(),
                "expected a line of `package`",
            ));
            continue;
        };
        let Some(index) = ORDER.iter().position(|k| *k == key) else {
            errors.push(Error::new(
                child.head(),
                format!("unknown line `{key}` in `package`"),
            ));
            continue;
        };
        let repeatable = matches!(key, "needs" | "granted" | "trusted");
        if index < at || (index == at && !repeatable && at > 0 && child_seen(&package, key)) {
            errors.push(Error::new(
                child.head(),
                format!(
                    "`{key}` is out of order or repeated; the lines of `package` are {}",
                    ORDER.map(|k| format!("`{k}`")).join(", ")
                ),
            ));
            continue;
        }
        at = index;
        let mut cursor = Cursor::new(child);
        cursor.next();
        let result: Result<(), Error> = (|| {
            match key {
                "from" => package.from = Some(lock_origin(&mut cursor)?),
                "digest" => package.digest = Some(cursor.text("the content digest")?),
                "signed" => package.signed = Some(cursor.text("the publisher key")?),
                "logged" => package.logged = Some(cursor.int("the log index")?),
                "replaced" => package.replaced = true,
                "needs" => {
                    let source = cursor.word("the need's source")?;
                    let name = cursor.pkg_name()?;
                    let version = cursor.version("the version selected")?;
                    package.needs.push(LockNeed {
                        source,
                        name,
                        version,
                    });
                }
                "granted" => {
                    if !cursor.eat_word("by") {
                        return Err(Error::new(cursor.here(), "expected `granted by`"));
                    }
                    let by = cursor.pkg_name()?;
                    let identity = match cursor.peek().map(|t| &t.item) {
                        Some(Item::Int(_) | Item::Version(_)) => {
                            let token = cursor.next().cloned();
                            let major = match token {
                                Some(Tok {
                                    item: Item::Int(v) | Item::Version(v),
                                    range,
                                }) => At::new(v, range),
                                _ => return Err(Error::new(cursor.here(), "expected a major")),
                            };
                            if !cursor.eat_word("from") {
                                return Err(Error::new(cursor.here(), "expected `from <source>`"));
                            }
                            let source = cursor.word("the dependent's source")?;
                            Some((major, source))
                        }
                        _ => None,
                    };
                    if !cursor.eat_word("may") {
                        return Err(Error::new(cursor.here(), "expected `may` and the ceiling"));
                    }
                    let may = cursor.cap_types()?;
                    package.granted.push(LockGrant { by, identity, may });
                }
                "consented" => package.consented = Some(cursor.text("the digest consented")?),
                _ => {
                    if !cursor.eat_word("modules") {
                        return Err(Error::new(cursor.here(), "expected `trusted modules`"));
                    }
                    package.trusted.push(cursor.module_path("a module path")?);
                    while cursor.eat_punct(",") {
                        package.trusted.push(cursor.module_path("a module path")?);
                    }
                }
            }
            cursor.end()
        })();
        if let Err(error) = result {
            errors.push(error);
        }
    }
    // A line that was written but did not read has its own error already.
    let written = |key: &str| line.children.iter().any(|child| child.word(0) == Some(key));
    if (package.from.is_none() && !written("from"))
        || (package.digest.is_none() && !written("digest"))
    {
        errors.push(Error::new(
            line.range.clone(),
            "a locked package has a `from` and a `digest` line",
        ));
    }
    if package.signed.is_some() != package.logged.is_some() {
        errors.push(Error::new(
            line.range.clone(),
            "a locked package has both `signed` and `logged`, or neither",
        ));
    }
    Some(package)
}

fn child_seen(package: &LockPackage, key: &str) -> bool {
    match key {
        "from" => package.from.is_some(),
        "digest" => package.digest.is_some(),
        "signed" => package.signed.is_some(),
        "logged" => package.logged.is_some(),
        "replaced" => package.replaced,
        "consented" => package.consented.is_some(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// `itaty.una`
// ---------------------------------------------------------------------------------------------

/// `itaty.una`, as read (ADR 0122 rule 19).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    /// The member directories.
    pub members: Vec<At<String>>,
    /// `sources`.
    pub sources: Vec<Source>,
    /// `replace`.
    pub replace: Vec<Replace>,
    /// `trust`.
    pub trust: Vec<TrustLine>,
}

/// Reads `itaty.una` by the grammar of ADR 0122 rule 19. O(n).
pub fn parse_workspace<'a>(text: impl Into<Text<'a>>) -> (Workspace, Vec<Error>) {
    let text = text.into();
    let mut workspace = Workspace::default();
    if text.raw_len() > MAX_MANIFEST_BYTES {
        return (
            workspace,
            vec![Error::new(
                0..0,
                format!(
                    "the file holds {} bytes, more than the {MAX_MANIFEST_BYTES} it may hold",
                    text.raw_len()
                ),
            )],
        );
    }
    let read = lines(text);
    let mut errors = read.errors;
    let mut lines = read.lines.iter();
    match lines.next() {
        Some(line) if line.word(0) == Some("itaty") && line.tokens.len() == 1 => {
            let mut members_seen = false;
            for child in &line.children {
                if child.word(0) == Some("members") && child.tokens.len() == 1 && !members_seen {
                    members_seen = true;
                    for member in &child.children {
                        no_children(member, &mut errors);
                        let mut cursor = Cursor::new(member);
                        match cursor
                            .text("a member's directory")
                            .and_then(|dir| cursor.end().map(|()| dir))
                        {
                            Ok(dir) => workspace.members.push(dir),
                            Err(error) => errors.push(error),
                        }
                    }
                } else {
                    errors.push(Error::new(
                        child.range.clone(),
                        "the one line of `itaty` is `members`, with a directory per line below it",
                    ));
                }
            }
            if !members_seen {
                errors.push(Error::new(line.range.clone(), "`itaty` has no `members`"));
            }
        }
        _ => {
            errors.push(Error::new(
                0..0,
                "itaty.una starts with `itaty` and its `members`",
            ));
            return (workspace, errors);
        }
    }
    const ORDER: [&str; 3] = ["sources", "replace", "trust"];
    let mut at: Option<usize> = None;
    for line in lines {
        let Some(index) = line
            .word(0)
            .and_then(|word| ORDER.iter().position(|known| *known == word))
            .filter(|_| line.tokens.len() == 1)
        else {
            errors.push(Error::new(
                line.range.clone(),
                "after `itaty`, a workspace has the sections `sources`, `replace` and `trust`",
            ));
            continue;
        };
        if at.is_some_and(|at| at >= index) {
            errors.push(Error::new(
                line.head(),
                "the sections of itaty.una appear once each, in the order `sources`, `replace`, `trust`",
            ));
            continue;
        }
        at = Some(index);
        match index {
            0 => sources(line, &mut workspace.sources, &mut errors),
            1 => replace(line, &mut workspace.replace, &mut errors),
            _ => trust(line, &mut workspace.trust, &mut errors),
        }
    }
    (workspace, errors)
}

// ---------------------------------------------------------------------------------------------
// The canonical form of `ita.una` and `itaty.una`
// ---------------------------------------------------------------------------------------------

/// The canonical form of `ita.una` or `itaty.una` (ADR 0122 rule 5): two spaces per level,
/// one blank line between top-level blocks and none inside them, the head lines in their
/// canonical order, one space between words, none before `,`, `.`, `]` or after `[`, `.`, `/`,
/// and comments kept with the line they precede or end. `None` when the file has a lexical
/// or layout error, which the formatter does not repair. O(n log n) for the head's sort, O(n)
/// otherwise.
pub fn format<'a>(text: impl Into<Text<'a>>) -> Option<String> {
    // The canonical form ends its lines with `\n` alone; reading the decoded text keeps every
    // token and comment free of a `\r` of a `\r\n` line end. A written text is decoded once
    // here, a decoded one is read as it is (#2252).
    let written;
    let file = match text.into() {
        Text::Written(text) => {
            written = SourceFile::new("project", text.as_bytes());
            &written
        }
        Text::Decoded(file) => file,
    };
    let read = lines(file);
    if !read.errors.is_empty() {
        return None;
    }
    let text = read.text.as_str();
    let mut out = String::new();
    for (index, line) in read.lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let mut children: Vec<&Line> = line.children.iter().collect();
        if index == 0 && line.word(0) == Some("ita") {
            children.sort_by_key(|child| {
                child
                    .word(0)
                    .and_then(|word| HEAD_LINES.iter().position(|known| *known == word))
                    .unwrap_or(HEAD_LINES.len())
            });
        }
        write_line(text, line, &children, 0, &mut out);
    }
    for comment in &read.comments_after {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(text.get(comment.clone()).unwrap_or("").trim_end());
        out.push('\n');
    }
    Some(out)
}

fn write_line(text: &str, line: &Line, children: &[&Line], depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    for comment in &line.comments_before {
        out.push_str(&indent);
        out.push_str(text.get(comment.clone()).unwrap_or("").trim_end());
        out.push('\n');
    }
    out.push_str(&indent);
    out.push_str(&rendered(text, &line.tokens));
    if let Some(comment) = &line.trailing_comment {
        out.push_str("  ");
        out.push_str(text.get(comment.clone()).unwrap_or("").trim_end());
    }
    out.push('\n');
    for child in children {
        let grandchildren: Vec<&Line> = child.children.iter().collect();
        write_line(text, child, &grandchildren, depth + 1, out);
    }
}

/// A line's tokens with canonical spacing, when they read back as the same tokens; otherwise,
/// as where dropping a space would join `1.` and `2` into a version, one space between every
/// two tokens, which always reads back the same. O(length of the line).
fn rendered(text: &str, tokens: &[Tok]) -> String {
    let canonical = canonical_tokens(text, tokens);
    let read = lines(&canonical);
    let same = read.errors.is_empty()
        && matches!(read.lines.as_slice(), [line] if line.children.is_empty()
            && line.tokens.len() == tokens.len()
            && line.tokens.iter().zip(tokens).all(|(a, b)| a.item == b.item));
    if same {
        return canonical;
    }
    tokens
        .iter()
        .map(|token| text.get(token.range.clone()).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The tokens of a line with canonical spacing; a token is written as the file writes it.
fn canonical_tokens(text: &str, tokens: &[Tok]) -> String {
    let mut out = String::new();
    let mut previous: Option<&Item> = None;
    for token in tokens {
        let written = text.get(token.range.clone()).unwrap_or("");
        let space = !matches!(
            (previous, &token.item),
            (None, _)
                | (_, Item::Punct("," | "." | "]" | ":" | "/" | ")"))
                | (Some(Item::Punct("[" | "." | "/" | "(")), _)
                | (Some(Item::Upper(_)), Item::Punct("["))
                | (Some(Item::Word(_)), Item::Punct("("))
        );
        if space {
            out.push(' ');
        }
        out.push_str(written);
        previous = Some(&token.item);
    }
    out
}
