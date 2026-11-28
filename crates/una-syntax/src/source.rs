//! Source files, spans and positions (`spec/01 §1`, `spec/13 §1`, ADR 0015).

use std::collections::BTreeMap;

use una_diag::{Location, Position};

/// Identifies a file of a [`SourceDb`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub(crate) usize);

/// A range of a file's decoded text, in byte offsets; `end` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    /// The file the range belongs to.
    pub file: FileId,
    /// Offset of the first byte.
    pub start: usize,
    /// Offset just past the last byte.
    pub end: usize,
}

impl Span {
    /// A span of `file` from `start` to `end`.
    pub fn new(file: FileId, start: usize, end: usize) -> Span {
        Span { file, start, end }
    }

    /// The empty span at `offset`.
    pub fn at(file: FileId, offset: usize) -> Span {
        Span::new(file, offset, offset)
    }

    /// The smallest span that covers `self` and `other`, which must be in the same file.
    pub fn to(self, other: Span) -> Span {
        Span::new(
            self.file,
            self.start.min(other.start),
            self.end.max(other.end),
        )
    }

    /// The empty span at the start of `self`.
    pub fn start_point(self) -> Span {
        Span::at(self.file, self.start)
    }

    /// The empty span at the end of `self`.
    pub fn end_point(self) -> Span {
        Span::at(self.file, self.end)
    }
}

/// A decoded source file (`spec/01 §1`, ADR 0015 D2).
///
/// The text is the file's bytes decoded as UTF-8, with each maximal ill-formed subsequence
/// replaced by U+FFFD and `\r\n` normalized to `\n`. A byte order mark and lone carriage
/// returns stay in the text, so the lexer can report them where they are.
#[derive(Clone, Debug)]
pub struct SourceFile {
    path: String,
    text: String,
    line_starts: Vec<usize>,
    invalid: Vec<usize>,
    first_crlf: Option<usize>,
    raw_len: usize,
    unread: Option<Unread>,
}

/// What is known of a file left unread past its bound (`spec/30 §1` rule 4): the number of
/// bytes it holds, from its size when it was opened, or only that it holds more than the bound,
/// when the read stopped there (a file that grew while read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unread {
    /// It holds this many bytes, more than its bound.
    Holds(usize),
    /// It holds more than this many bytes, its bound; its full length is not known.
    MoreThan(usize),
}

impl SourceFile {
    /// Decodes `bytes` as the file at `path`, relative to the package root.
    pub fn new(path: impl Into<String>, bytes: &[u8]) -> SourceFile {
        let mut text = String::with_capacity(bytes.len());
        let mut invalid = Vec::new();
        let mut first_crlf = None;
        for chunk in bytes.utf8_chunks() {
            if first_crlf.is_none() {
                first_crlf = chunk.valid().find("\r\n").map(|index| text.len() + index);
            }
            text.push_str(&chunk.valid().replace("\r\n", "\n"));
            if !chunk.invalid().is_empty() {
                invalid.push(text.len());
                text.push(char::REPLACEMENT_CHARACTER);
            }
        }
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(offset, _)| offset + 1))
            .collect();
        SourceFile {
            path: path.into(),
            text,
            line_starts,
            invalid,
            first_crlf,
            raw_len: bytes.len(),
            unread: None,
        }
    }

    /// A file left unread past its bound (`spec/30 §1` rule 4): its text is empty, and only
    /// what is known of its length is kept. Every reader of a project file refuses an unread
    /// file, whatever that length, with the message of its bound.
    pub fn unread(path: impl Into<String>, unread: Unread) -> SourceFile {
        let raw_len = match unread {
            Unread::Holds(len) => len,
            Unread::MoreThan(bound) => bound.saturating_add(1),
        };
        SourceFile {
            path: path.into(),
            text: String::new(),
            line_starts: vec![0],
            invalid: Vec::new(),
            first_crlf: None,
            raw_len,
            unread: Some(unread),
        }
    }

    /// What is known of the length of a file left unread past its bound, or `None` for a file
    /// that was read.
    pub fn unread_len(&self) -> Option<Unread> {
        self.unread
    }

    /// The path of the file, relative to the package root, with `/` as separator.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The decoded text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The number of bytes the file holds as written, before decoding: each `\r\n` counts
    /// two bytes, and an ill-formed sequence counts its own bytes. The size bounds of project
    /// files are measured on this length (`spec/30 §1` rule 4). For an unread file it is the
    /// length that put it past its bound, at least one byte past it.
    pub fn raw_len(&self) -> usize {
        self.raw_len
    }

    /// The offset of each line's first character, in order; the first line starts at 0.
    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    /// The offset, in the decoded text, of the line feed of the first `\r\n` of the file, if
    /// any: the canonical form uses `\n` alone (ADR 0020).
    pub fn first_crlf(&self) -> Option<usize> {
        self.first_crlf
    }

    /// Whether the character at `offset` is a U+FFFD that replaced invalid UTF-8.
    pub fn is_invalid_sequence(&self, offset: usize) -> bool {
        self.invalid.binary_search(&offset).is_ok()
    }

    /// The position of `offset`: a 1-based line and a 1-based column counted in Unicode
    /// scalar values of the decoded text (`spec/13 §1`). An offset past the end of the text is
    /// clamped to the end, which is one column past the last character.
    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next.saturating_sub(1),
        };
        let line_start = self.line_starts.get(line).copied().unwrap_or(0);
        let col = self
            .text
            .get(line_start..offset)
            .map_or(offset - line_start, |prefix| prefix.chars().count());
        Position {
            line: to_u32(line + 1),
            col: to_u32(col + 1),
        }
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The source files of one check, in load order.
#[derive(Clone, Debug, Default)]
pub struct SourceDb {
    files: Vec<SourceFile>,
}

impl SourceDb {
    /// An empty database.
    pub fn new() -> SourceDb {
        SourceDb::default()
    }

    /// Adds the file at `path` with contents `bytes`, and returns its id.
    pub fn add(&mut self, path: impl Into<String>, bytes: &[u8]) -> FileId {
        self.insert(SourceFile::new(path, bytes))
    }

    /// Adds a file already decoded, such as [`SourceFile::unread`], and returns its id.
    pub fn insert(&mut self, file: SourceFile) -> FileId {
        self.files.push(file);
        FileId(self.files.len() - 1)
    }

    /// The file with id `file`.
    #[allow(clippy::indexing_slicing)]
    pub fn file(&self, file: FileId) -> &SourceFile {
        // INVARIANT: a FileId is created only by `add` of the database that stores the file,
        // and files are never removed.
        &self.files[file.0]
    }

    /// Every file, in load order.
    pub fn files(&self) -> impl Iterator<Item = (FileId, &SourceFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(index, file)| (FileId(index), file))
    }

    /// The location of `span`, for diagnostics.
    pub fn location(&self, span: Span) -> Location {
        let file = self.file(span.file);
        Location {
            file: file.path().to_owned(),
            start: file.position(span.start),
            end: file.position(span.end),
        }
    }

    /// The decoded text of every file, keyed by path, for human rendering.
    pub fn texts(&self) -> BTreeMap<String, String> {
        self.files
            .iter()
            .map(|file| (file.path().to_owned(), file.text().to_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, col: u32) -> Position {
        Position { line, col }
    }

    #[test]
    fn each_maximal_ill_formed_subsequence_becomes_one_replacement_character() {
        // `\xE2\x82` is a truncated three-byte sequence: one subsequence. `\xFF` and `\xFE`
        // cannot start a sequence: one each.
        let file = SourceFile::new("a.una", b"a\xE2\x82b\xFF\xFEc");
        assert_eq!(file.text(), "a\u{FFFD}b\u{FFFD}\u{FFFD}c");
        assert!(file.is_invalid_sequence(1));
        assert!(!file.is_invalid_sequence(0));
        assert!(!file.is_invalid_sequence(4));
        assert!(file.is_invalid_sequence(5));
        assert!(file.is_invalid_sequence(8));
    }

    #[test]
    fn a_real_replacement_character_is_not_an_invalid_sequence() {
        let file = SourceFile::new("a.una", "-- \u{FFFD}".as_bytes());
        assert!(!file.is_invalid_sequence(3));
    }

    #[test]
    fn the_first_crlf_is_recorded_in_decoded_offsets() {
        let file = SourceFile::new("t.una", b"a\nbc\r\nd\r\n");
        assert_eq!(file.first_crlf(), Some(4));
        assert_eq!(file.text().get(4..5), Some("\n"));
        assert_eq!(SourceFile::new("t.una", b"a\rb\n").first_crlf(), None);
        assert_eq!(file.line_starts(), [0, 2, 5, 7]);
    }

    #[test]
    fn crlf_becomes_lf_and_lone_carriage_returns_stay() {
        let file = SourceFile::new("a.una", b"a\r\nb\rc\r\n");
        assert_eq!(file.text(), "a\nb\rc\n");
    }

    #[test]
    fn positions_count_scalar_values_after_decoding() {
        // ADR 0015 D2: an invalid subsequence, a byte order mark and a tab are one column each.
        let file = SourceFile::new("a.una", b"\xEF\xBB\xBFab\n\tx\xE2\x82y\n");
        assert_eq!(file.position(0), at(1, 1));
        assert_eq!(file.position(3), at(1, 2));
        assert_eq!(file.position(6), at(2, 1));
        assert_eq!(file.position(7), at(2, 2));
        assert_eq!(file.position(8), at(2, 3));
        assert_eq!(file.position(11), at(2, 4));
    }

    #[test]
    fn crlf_lines_have_no_extra_column() {
        let file = SourceFile::new("a.una", b"ab\r\ncd");
        assert_eq!(file.position(2), at(1, 3));
        assert_eq!(file.position(3), at(2, 1));
    }

    #[test]
    fn end_of_file_is_one_past_the_last_character() {
        let with_newline = SourceFile::new("a.una", b"ab\n");
        assert_eq!(with_newline.position(3), at(2, 1));
        let without = SourceFile::new("a.una", b"ab");
        assert_eq!(without.position(2), at(1, 3));
        assert_eq!(without.position(99), at(1, 3));
        let empty = SourceFile::new("a.una", b"");
        assert_eq!(empty.position(0), at(1, 1));
    }

    #[test]
    fn locations_name_the_file_and_both_ends() {
        let mut db = SourceDb::new();
        let first = db.add("src/a.una", b"module a\n");
        let second = db.add("src/b.una", b"x\nmodule b\n");
        assert_eq!(
            db.location(Span::new(second, 2, 8)),
            Location {
                file: "src/b.una".to_owned(),
                start: at(2, 1),
                end: at(2, 7),
            }
        );
        assert_eq!(db.file(first).path(), "src/a.una");
        assert_eq!(db.files().count(), 2);
        assert_eq!(
            db.texts().keys().collect::<Vec<_>>(),
            ["src/a.una", "src/b.una"]
        );
    }

    #[test]
    fn spans_join_and_shrink_to_points() {
        let file = FileId(0);
        let span = Span::new(file, 2, 5).to(Span::new(file, 4, 9));
        assert_eq!(span, Span::new(file, 2, 9));
        assert_eq!(span.start_point(), Span::at(file, 2));
        assert_eq!(span.end_point(), Span::at(file, 9));
    }
}
