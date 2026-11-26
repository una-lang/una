//! The lexer (`spec/01`): tokens, layout (`NEWLINE`, `INDENT`, `DEDENT`), comments as trivia,
//! and the lexical errors U0001 to U0007, U0103 and bracket balance (U0101).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;

use crate::error::{SyntaxError, SyntaxRepair};
use crate::intern::{Interner, Symbol};
use crate::keywords::{BINDING_WORDS_HELP, LET_CHANGED_IN, reserved_in};
use crate::source::{FileId, SourceFile, Span};
use crate::token::{Comment, Keyword, KeywordSet, Token, TokenKind};

/// The result of lexing one file.
#[derive(Clone, Debug, Default)]
pub struct Lexed {
    /// The tokens, ending with [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Every comment, in source order.
    pub comments: Vec<Comment>,
    /// The lexical errors, in the order they were found.
    pub errors: Vec<SyntaxError>,
    /// The keyword set the file was lexed with, which the parser follows.
    pub keywords: KeywordSet,
    /// The text of the file, for repairs that rewrite a range of it.
    pub text: String,
    /// The U0103 errors that get a rename edit, so that a package can choose each new name
    /// across its files ([`rename_across`]).
    pub renames: Vec<Rename>,
    /// Edition 0.3: the symbol of `from`, interned when the keyword `from` is followed by `:`,
    /// which the parser reads as the argument label of `copy_sign(from: y)` (`spec/29 §3`).
    pub from_label: Option<Symbol>,
    /// Edition 0.4: the symbols of `with` and `and`, interned when the keyword is followed by
    /// `:`, which the parser reads as the argument labels of the dense methods, as in
    /// `a.dot(with: b)` and `edit c.add_product(of: a, and: b)` (`spec/29 §17`, ADR 0120).
    pub with_label: Option<Symbol>,
    /// See [`Lexed::with_label`].
    pub and_label: Option<Symbol>,
    /// Edition 0.4: the symbol of `error`, interned when the keyword `error` is followed by
    /// `:` or follows `.`, which the parser reads as the field `error` of `Timed.Failed`
    /// (`spec/27 §5`, ADR 0117 rule 5).
    pub error_label: Option<Symbol>,
}

/// A U0103 error of [`Lexed::errors`] that gets a rename edit (issue #75, #105, #125).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    /// The error's index in [`Lexed::errors`].
    pub error: usize,
    /// The reserved word.
    pub word: String,
    /// Whether this occurrence names a kind or capability.
    pub upper: bool,
}

/// Lexes `source`, which is the file `file` of its database, with the keywords of edition 0.1.
pub fn lex(source: &SourceFile, file: FileId, interner: &mut Interner) -> Lexed {
    lex_in(source, file, interner, KeywordSet::V0_1)
}

/// Lexes `source` with the keywords and reserved words of `keywords` (`spec/17 §9`).
pub fn lex_in(
    source: &SourceFile,
    file: FileId,
    interner: &mut Interner,
    keywords: KeywordSet,
) -> Lexed {
    let mut out = lex_tokens(source, file, interner, keywords);
    // The names the file uses are collected only to choose a rename clear of them, so a file
    // with no rename, such as every standard module, skips the collection (#1894).
    if !out.renames.is_empty() {
        let used = names(&out, interner);
        let chosen = choose(&out.renames, &used);
        apply(&mut out, &chosen);
    }
    // The renames of names that start with `_` (ADR 0096 A).
    crate::rewrite::underscore_renames(&out.text, file, keywords, &mut out.errors);
    out
}

/// The tokens and lexical errors of `source` under `keywords`, with no repair chosen: what
/// [`lex_in`] starts from, and what [`parsing_renames`] lexes its trial text with, so that a
/// trial never starts another.
fn lex_tokens(
    source: &SourceFile,
    file: FileId,
    interner: &mut Interner,
    keywords: KeywordSet,
) -> Lexed {
    let mut lexer = Lexer {
        source,
        text: source.text(),
        file,
        interner,
        pos: 0,
        out: Lexed {
            keywords,
            text: source.text().to_owned(),
            ..Lexed::default()
        },
        indents: vec![0],
        brackets: Vec::new(),
        line_has_tokens: false,
        project: false,
    };
    lexer.run();
    lexer.out
}

/// Lexes a project file of edition 0.4, `ita.una`, `ita.lock` or `itaty.una`, by the
/// lexical rules of `spec/01` with the two differences of ADR 0122 rule 4: a version token,
/// matched before integers, and text without interpolation, where an unescaped `{` or `}` is
/// an error. No word is a keyword or a reserved word here: every lower-case word is a
/// [`TokenKind::LowerIdent`], and the grammar of the file gives it its meaning.
pub fn lex_project(source: &SourceFile, file: FileId, interner: &mut Interner) -> Lexed {
    let mut lexer = Lexer {
        source,
        text: source.text(),
        file,
        interner,
        pos: 0,
        out: Lexed {
            keywords: KeywordSet::V0_1,
            text: source.text().to_owned(),
            ..Lexed::default()
        },
        indents: vec![0],
        brackets: Vec::new(),
        line_has_tokens: false,
        project: true,
    };
    lexer.run();
    lexer.out
}

/// Chooses the rename edits of the U0103 errors of every file of a package together (#125):
/// one new name per word for the whole package, which no file of the package already has, so
/// that applying every edit of every file renames a declaration and each of its uses, in any
/// module, alike, and never makes a name refer to another declaration. [`lex_in`] chooses
/// them for one file; a package calls this with all its files once they are lexed.
pub fn rename_across(files: &mut [Lexed], interner: &Interner) {
    let mut used = BTreeSet::new();
    let mut renames = Vec::new();
    for lexed in files.iter() {
        used.extend(names(lexed, interner));
        renames.extend(lexed.renames.iter().cloned());
    }
    let chosen = choose(&renames, &used);
    for lexed in files.iter_mut() {
        apply(lexed, &chosen);
    }
}

/// Every name the tokens of `lexed` spell.
fn names(lexed: &Lexed, interner: &Interner) -> BTreeSet<String> {
    lexed
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::LowerIdent(name)
            | TokenKind::UpperIdent(name)
            | TokenKind::Reserved(name)
            | TokenKind::CapRef(name) => Some(interner.resolve(name).to_owned()),
            _ => None,
        })
        .collect()
}

/// One new name per word of `renames`, none of them in `used` (V12-002). It is capitalized
/// when any occurrence is a kind or capability name: `Service`, else `the_table`; a clash adds
/// a number, `the_table_2`.
fn choose(renames: &[Rename], used: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut chosen: BTreeMap<String, String> = BTreeMap::new();
    for rename in renames {
        if chosen.contains_key(&rename.word) {
            continue;
        }
        let word = &rename.word;
        let upper = renames.iter().any(|r| &r.word == word && r.upper);
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
    chosen
}

/// Sets the rename edit of each error of `lexed.renames` to the name `chosen` gives its word,
/// where the renamed file parses there (#836, [`parsing_renames`]). Elsewhere the word stands
/// where its construct's keyword would, as `edit` in `let edit total = 0` of edition 0.1, and
/// no new name keeps the program's meaning, so the error keeps no edit.
fn apply(lexed: &mut Lexed, chosen: &BTreeMap<String, String>) {
    let edits: Vec<(usize, Span, String)> = lexed
        .renames
        .iter()
        .filter_map(|rename| {
            let name = chosen.get(&rename.word)?;
            let error = lexed.errors.get(rename.error)?;
            Some((rename.error, error.span, name.clone()))
        })
        .collect();
    let parsing = parsing_renames(&lexed.text, lexed.keywords, edits);
    for rename in &lexed.renames {
        let Some(error) = lexed.errors.get_mut(rename.error) else {
            continue;
        };
        // The rename comes first; advice without an edit, such as edition 0.4's for `simd`,
        // stays after it. `apply` runs again across files, so an earlier rename is replaced,
        // and removed when the renames of the whole package no longer parse there.
        error.repairs.retain(|repair| repair.edit.is_none());
        let Some(name) = parsing.get(&rename.error) else {
            continue;
        };
        error.repairs.insert(
            0,
            SyntaxRepair {
                description: format!("rename it to `{name}`"),
                edit: Some((error.span, name.clone())),
            },
        );
    }
}

/// The renames of `edits`, each the index of its error, the reserved word's span in `text`
/// and its new name, whose result parses (#836): the text with every kept rename applied is
/// lexed and parsed, and a rename is dropped when a syntax error starts at its new name or
/// before the end of the next token that is not layout, which is where the parser meets a
/// name that stands where a keyword would (`let the_edit total = 0`, `g(the_take x)`, a
/// `the_together` line over a block). The parser reads a reserved word as it reads a name
/// (`Parser::lower_name`), so a rename that parses there gives the program the reading the
/// checks already gave it. Each trial drops at least one rename or ends the search, so a file
/// with `r` renames is parsed at most `r + 1` times, and each trial is linear in the length of
/// the text and its tokens, with `O(log r)` per syntax error; a file with no rename, as every
/// standard module, is never parsed here.
fn parsing_renames(
    text: &str,
    keywords: KeywordSet,
    mut edits: Vec<(usize, Span, String)>,
) -> BTreeMap<usize, String> {
    edits.sort_by_key(|(_, span, _)| span.start);
    loop {
        if edits.is_empty() {
            return BTreeMap::new();
        }
        let mut trial = String::with_capacity(text.len());
        let mut starts = Vec::with_capacity(edits.len());
        let mut at = 0;
        for (_, span, name) in &edits {
            trial.push_str(text.get(at..span.start).unwrap_or(""));
            starts.push(trial.len());
            trial.push_str(name);
            at = span.end;
        }
        trial.push_str(text.get(at..).unwrap_or(""));
        let mut sources = crate::source::SourceDb::new();
        let id = sources.add("rename.una", trial.as_bytes());
        let mut interner = Interner::new();
        let mut lexed = lex_tokens(sources.file(id), id, &mut interner, keywords);
        // Every parse error counts: none is explained away by a lexical error of the same
        // declaration, which the parser otherwise does not report (`Parser::report_error`).
        lexed.errors.clear();
        let parsed = crate::parser::parse_plain(&lexed, id, &interner);
        // Where each rename's window ends: the end of the first token after the new name that
        // is not a newline or an indentation change. `starts` is sorted, so one pass over the
        // tokens finds every end.
        let layout = |token: &Token| {
            matches!(
                token.kind,
                TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent
            )
        };
        let mut ends = Vec::with_capacity(starts.len());
        let mut next = 0;
        for start in &starts {
            while lexed
                .tokens
                .get(next)
                .is_some_and(|token| token.span.start <= *start)
            {
                next += 1;
            }
            let end = lexed
                .tokens
                .iter()
                .skip(next)
                .find(|token| !layout(token))
                .map_or(trial.len(), |token| token.span.end);
            ends.push(end);
        }
        // An error is blamed on the last rename that starts at or before it, when it lies in
        // that rename's window.
        let blamed: BTreeSet<usize> = parsed
            .errors
            .iter()
            .filter_map(|error| {
                let at = error.span.start;
                let last = starts
                    .partition_point(|start| *start <= at)
                    .checked_sub(1)?;
                ends.get(last).is_some_and(|end| at <= *end).then_some(last)
            })
            .collect();
        if blamed.is_empty() {
            return edits
                .into_iter()
                .map(|(error, _, name)| (error, name))
                .collect();
        }
        let mut index = 0;
        edits.retain(|_| {
            let keep = !blamed.contains(&index);
            index += 1;
            keep
        });
    }
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
/// function name followed by `(`, or by `[` for a generic function; `test` is the test
/// declaration of edition 0.3, under a keyword set that has it (ADR 0105), and a name before;
/// `example` is an identifier from edition 0.4 (ADR 0105, amendment of 2026-11-05).
pub(crate) fn starts_declaration(word: &str, next: Option<char>, keywords: KeywordSet) -> bool {
    (DECLARATION_KEYWORDS.contains(&word) && !(word == "example" && keywords >= KeywordSet::V0_4))
        || (word == "test" && keywords >= KeywordSet::V0_3)
        || matches!(next, Some('(' | '['))
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

/// Whether the word at `offset` of `text`, a lowercase name, is followed by `(`: a method call
/// such as `5.e2()`, which a float literal never is.
fn method_call(text: &str, offset: usize) -> bool {
    let rest = text.get(offset..).unwrap_or("");
    let name = rest
        .chars()
        .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
        .count();
    rest.get(name..).is_some_and(|after| after.starts_with('('))
}

/// Whether a float literal as written is malformed (ADR 0114 rule 2): an integer part or a
/// fraction that is a malformed digit run, an `E`, a `+`, or an exponent with a leading zero.
fn is_malformed_float(raw: &str) -> bool {
    raw != canonical_float(raw)
}

/// The accepted spelling of a float literal: `digits "." digits [ "e" [ "-" ] exponent ]`,
/// with the integer part and the fraction's runs of `_` repaired as for integers, `.0` added
/// when the point or the fraction is missing, a lowercase `e`, no `+`, and no leading zero in
/// the exponent.
fn canonical_float(raw: &str) -> String {
    let (mantissa, exponent) = match raw.find(['e', 'E']) {
        Some(at) => (raw.get(..at).unwrap_or(""), raw.get(at + 1..)),
        None => (raw, None),
    };
    let (int_part, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let int_part = canonical_integer(int_part);
    let mut fraction_out = String::new();
    for c in fraction.trim_matches('_').chars() {
        if !(c == '_' && fraction_out.ends_with('_')) {
            fraction_out.push(c);
        }
    }
    if fraction_out.is_empty() {
        fraction_out.push('0');
    }
    let mut out = format!("{int_part}.{fraction_out}");
    if let Some(exponent) = exponent {
        let (negative, digits) = match exponent.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, exponent.strip_prefix('+').unwrap_or(exponent)),
        };
        let digits: String = digits.chars().filter(|c| *c != '_').collect();
        let digits = digits.trim_start_matches('0');
        let digits = if digits.is_empty() { "0" } else { digits };
        out.push('e');
        if negative {
            out.push('-');
        }
        out.push_str(digits);
    }
    out
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
    /// Whether this is a project file of edition 0.4 ([`lex_project`]).
    project: bool,
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
    /// ([`lex_in`], [`rename_across`], issue #75, #105, #125); a word naming a construct Una
    /// writes differently says what to write instead. In the module path of a `module` or
    /// `import` line the repair is advice, since the file must move with the module (U1008); a
    /// name an `import` line lists, after its `(`, is renamed as its declaration is (#125).
    fn reserved(&mut self, word: &str, message: &str, start: usize) {
        let span = self.span(start, self.pos);
        let mut error = SyntaxError::new(Code::U0103, span, format!("`{word}` is {message}"));
        if message.starts_with("reserved for") {
            if self.in_module_path() {
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
                self.out.renames.push(Rename {
                    error: self.out.errors.len(),
                    word: word.to_owned(),
                    upper,
                });
            }
            // ADR 0139 rule 7: `simd` stays reserved in edition 0.4, and is a name from 0.5
            // (ADR 0120 rule 1); the advice follows the rename (#1743).
            if word == "simd" && self.out.keywords == KeywordSet::V0_4 {
                error.repairs.push(SyntaxRepair {
                    description: "`simd` is a name from edition 0.5 (ADR 0139): write `edition 0.5` in ita.una to use it".to_owned(),
                    edit: None,
                });
            }
        }
        // ADR 0137 rule 1: before edition 0.5, `var x` keeps its U0103 and gains the help that
        // names the edition of the binding words.
        if word == "var" && self.out.keywords < KeywordSet::V0_5 && self.binding_shape(start) {
            error.repairs.push(SyntaxRepair {
                description: BINDING_WORDS_HELP.to_owned(),
                edit: None,
            });
        }
        self.out.errors.push(error);
    }

    /// U0103 for `example` at column 1 of a package of edition 0.3, where the test declaration
    /// is written `test` (ADR 0105 rule 3). The first repair is the exact rewrite of the
    /// keyword, which `una fix` applies; the token stays the keyword `example`, so the parser
    /// reads the declaration as a test's and the body gets its own checks.
    fn example_in_0_3(&mut self, start: usize) {
        let span = self.span(start, self.pos);
        let mut error = SyntaxError::new(
            Code::U0103,
            span,
            "`example` is written `test` in edition 0.3",
        );
        error.repairs.push(SyntaxRepair {
            description: "write `test`".to_owned(),
            edit: Some((span, "test".to_owned())),
        });
        error.recovered = true;
        self.out.errors.push(error);
    }

    /// U7007 for `let` in a package of edition 0.5 (ADR 0137 rule 1, ADR 0121 rule 19): the
    /// binding words of edition 0.5 are `const` and `var`. The first repair is the exact
    /// rewrite of rule 11, which `una fix` applies: `let edit` becomes `var`, and `let`
    /// becomes `const`, keeping the name, the annotation and the value.
    fn let_in_0_5(&mut self, start: usize) {
        let span = self.span(start, self.pos);
        let rest = self.text.get(self.pos..).unwrap_or("");
        let after_spaces = rest.trim_start_matches(' ');
        let spaces = rest.len() - after_spaces.len();
        let edit = spaces > 0
            && after_spaces.starts_with("edit")
            && !after_spaces
                .as_bytes()
                .get(4)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        let (range, word) = if edit {
            (self.span(start, self.pos + spaces + 4), "var")
        } else {
            (span, "const")
        };
        let mut error = SyntaxError::new(
            Code::U7007,
            span,
            format!(
                "this form of edition 0.5 changed in release {LET_CHANGED_IN}: `let` is written `const`, or `var` for a binding that is reassigned"
            ),
        );
        error.repairs.push(SyntaxRepair {
            description: format!("write `{word}`"),
            edit: Some((range, word.to_owned())),
        });
        error.recovered = true;
        self.out.errors.push(error);
    }

    /// Whether the word that ends at the lexer's position starts its line, after the
    /// indentation, and is followed by a space and a lower-case name: the shape of a binding
    /// statement written `const x` or `var x` (ADR 0137 rule 1).
    fn binding_shape(&self, start: usize) -> bool {
        let before = self.text.get(..start).unwrap_or("");
        let line = before.rsplit('\n').next().unwrap_or(before);
        let rest = self.text.get(self.pos..).unwrap_or("");
        line.bytes().all(|c| c == b' ')
            && rest.starts_with(' ')
            && rest
                .trim_start_matches(' ')
                .starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
    }

    /// Whether the next token is in the module path of a `module` or `import` line: the line
    /// starts with one of them, and, for `import`, its `(` has not come yet.
    fn in_module_path(&self) -> bool {
        let mut head = None;
        let mut listed = false;
        for token in self.out.tokens.iter().rev() {
            match token.kind {
                TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent => break,
                TokenKind::LParen => {
                    listed = true;
                    head = Some(&token.kind);
                }
                ref kind => head = Some(kind),
            }
        }
        !listed
            && matches!(
                head,
                Some(TokenKind::Keyword(Keyword::Module | Keyword::Import))
            )
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
            && starts_declaration(
                word,
                rest.get(length..).and_then(|r| r.chars().next()),
                self.out.keywords,
            )
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
            '"' if self.project => self.text_literal_with(false),
            '"' => self.text_literal(),
            '0'..='9' if self.project && self.version_follows() => self.version(),
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
            '.' if next.is_some_and(|n| n.is_ascii_digit())
                && self.out.keywords >= KeywordSet::V0_3
                && !self
                    .text
                    .get(..start)
                    .and_then(|before| before.chars().last())
                    .is_some_and(|p| p.is_alphanumeric() || matches!(p, '_' | ')' | ']')) =>
            {
                self.leading_point();
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
        if c == ';' {
            self.semicolon_repair(start, end);
        }
        // ADR 0096, amendment of 2026-10-15: `%` gets a description, never an edit. Una's
        // `mod` is Euclidean and the `%` of C, TypeScript and Rust truncates, so a rewrite could
        // change what the writer meant for negative operands.
        if c == '%'
            && let Some(error) = self.out.errors.last_mut()
        {
            error.repairs.push(SyntaxRepair {
                description: "Una writes the remainder as `mod`, which is Euclidean: `-7 mod 3 == 2`, where `%` in C, TypeScript and Rust gives `-1`; the two agree when both operands are non-negative".to_owned(),
                edit: None,
            });
        }
        self.pos = end;
    }

    /// The exact rewrite of a `;` (ADR 0096 A): deleted at the end of its line, with the spaces
    /// after it, or a line break at the line's indentation between two statements.
    fn semicolon_repair(&mut self, start: usize, end: usize) {
        let line_rest = self
            .text
            .get(end..)
            .and_then(|rest| rest.split('\n').next())
            .unwrap_or("");
        let line_start = self
            .text
            .get(..start)
            .and_then(|before| before.rfind('\n'))
            .map_or(0, |at| at + 1);
        let indent: String = self
            .text
            .get(line_start..start)
            .unwrap_or("")
            .chars()
            .take_while(|c| *c == ' ')
            .collect();
        let (stop, replacement, description) = if line_rest.trim().is_empty() {
            (
                end + line_rest.len(),
                String::new(),
                "delete the `;`: a statement ends at the end of its line",
            )
        } else {
            let gap = line_rest.len() - line_rest.trim_start().len();
            (
                end + gap,
                format!("\n{indent}"),
                "put the next statement on a line of its own",
            )
        };
        let span = self.span(start, stop);
        if let Some(error) = self.out.errors.last_mut() {
            error.repairs.push(SyntaxRepair {
                description: description.to_owned(),
                edit: Some((span, replacement)),
            });
        }
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
        if self.out.keywords >= KeywordSet::V0_3 && self.float(start) {
            return;
        }
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

    /// Edition 0.3: `.5`, a float literal without its integer part, U0007 with an edit to
    /// `0.5` (ADR 0114 rule 2.2).
    fn leading_point(&mut self) {
        let start = self.pos;
        self.pos += 1;
        self.consume_while(|c| c.is_ascii_digit() || c == '_');
        let raw = self.slice(start, self.pos).to_owned();
        let canonical = canonical_float(&format!("0{raw}"));
        let span = self.span(start, self.pos);
        let error = SyntaxError::new(
            Code::U0007,
            span,
            format!("malformed float literal `{raw}`"),
        )
        .with_edit(format!("write `{canonical}`"), span, canonical.clone());
        self.out.errors.push(error);
        self.push(TokenKind::Float(canonical), start, self.pos);
    }

    /// Edition 0.3: a float literal whose integer part, from `start`, was just read
    /// (`spec/29`, ADR 0114 rule 2), with U0007 and an edit to the accepted form when it is
    /// malformed: `1.`, `1e5`, `1.e5`, `1.0E5`, `1.0e+5`, a leading zero or a misplaced `_`.
    /// Returns whether a float literal was read; otherwise the position is unchanged.
    fn float(&mut self, start: usize) -> bool {
        let after = self.pos;
        let text = self.text;
        let at = move |offset: usize| text.get(offset..).and_then(|rest| rest.chars().next());
        let point = at(after) == Some('.');
        let digit_after_point = point && at(after + 1).is_some_and(|c| c.is_ascii_digit());
        // An exponent right after `offset`: `e` or `E`, an optional sign, and a digit.
        let exponent_at = |offset: usize| match at(offset) {
            Some('e' | 'E') => match at(offset + 1) {
                Some('+' | '-') => at(offset + 2).is_some_and(|c| c.is_ascii_digit()),
                Some(c) => c.is_ascii_digit(),
                None => false,
            },
            _ => false,
        };
        let mut malformed = false;
        if digit_after_point {
            self.pos += 1;
            self.consume_while(|c| c.is_ascii_digit() || c == '_');
        } else if point && exponent_at(after + 1) && !method_call(text, after + 1) {
            // `1.e5`; `5.e2()` stays a call of a method named `e2`.
            self.pos += 1;
            malformed = true;
        } else if point
            && !at(after + 1).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            // `1.` followed by no digit and no method name.
            self.pos += 1;
            malformed = true;
        } else if !exponent_at(after) {
            return false;
        } else {
            // `1e5`: an exponent without a point.
            malformed = true;
        }
        if exponent_at(self.pos) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            self.consume_while(|c| c.is_ascii_digit() || c == '_');
        }
        let raw = self.slice(start, self.pos).to_owned();
        let canonical = canonical_float(&raw);
        if malformed || raw != canonical && is_malformed_float(&raw) {
            let span = self.span(start, self.pos);
            let error = SyntaxError::new(
                Code::U0007,
                span,
                format!("malformed float literal `{raw}`"),
            )
            .with_edit(format!("write `{canonical}`"), span, canonical.clone());
            self.out.errors.push(error);
            self.push(TokenKind::Float(canonical), start, self.pos);
        } else {
            self.push(TokenKind::Float(raw), start, self.pos);
        }
        true
    }

    /// Whether a version starts here: digits, then `.` and a digit (ADR 0122 rule 4).
    fn version_follows(&self) -> bool {
        let rest = self.text.get(self.pos..).unwrap_or("").as_bytes();
        let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        rest.get(digits) == Some(&b'.') && rest.get(digits + 1).is_some_and(u8::is_ascii_digit)
    }

    /// A version token of a project file (ADR 0122 rule 4): `num "." num [ "." num ]
    /// [ "-" pre { "." pre } ]`, where `num` has no leading zero and a numeric `pre` neither.
    /// A malformed version is U0007 for the whole token, which the project file reports as its
    /// own code.
    fn version(&mut self) {
        let start = self.pos;
        let mut parts = 0;
        let mut problem: Option<&str> = None;
        loop {
            let from = self.pos;
            self.consume_while(|c| c.is_ascii_digit());
            let digits = self.slice(from, self.pos);
            if digits.len() > 1 && digits.starts_with('0') {
                problem.get_or_insert("a part of a version has no leading zero");
            }
            parts += 1;
            let more = self.peek() == Some('.')
                && self
                    .peek_at(self.pos + 1)
                    .is_some_and(|c| c.is_ascii_digit());
            if !more {
                break;
            }
            self.pos += 1;
        }
        if parts > 3 {
            problem.get_or_insert("a version has at most three numeric parts");
        }
        let pre_follows = self.peek() == Some('-')
            && self
                .peek_at(self.pos + 1)
                .is_some_and(|c| c.is_ascii_alphanumeric());
        if pre_follows {
            self.pos += 1;
            loop {
                let from = self.pos;
                self.consume_while(|c| c.is_ascii_alphanumeric());
                let part = self.slice(from, self.pos);
                if part.is_empty() {
                    problem.get_or_insert("a pre-release part is not empty");
                } else if part.len() > 1
                    && part.starts_with('0')
                    && part.bytes().all(|b| b.is_ascii_digit())
                {
                    problem.get_or_insert("a numeric pre-release part has no leading zero");
                }
                if self.peek() != Some('.') {
                    break;
                }
                self.pos += 1;
            }
        }
        // A letter, `_`, `+` or `.` right after the version is part of no valid token here: a
        // build suffix (`+...`) is not a version of ADR 0122 rule 20.
        let tail_start = self.pos;
        self.consume_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '.' | '-'));
        if self.pos > tail_start {
            problem.get_or_insert(
                "a version is `MAJOR.MINOR.PATCH`, `MAJOR.MINOR`, with an optional `-pre.release`, and no build suffix",
            );
        }
        let written = self.slice(start, self.pos).to_owned();
        if let Some(problem) = problem {
            self.error(
                Code::U0007,
                start,
                self.pos,
                format!("malformed version `{written}`: {problem}"),
            );
        }
        self.push(TokenKind::Version(written), start, self.pos);
    }

    fn lower_word(&mut self) {
        let start = self.pos;
        self.consume_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        // A slice of the text, whose reference outlives this borrow of the lexer: no copy per
        // word (#1894).
        let text = self.text;
        let word = text.get(start..self.pos).unwrap_or("");
        if self.project {
            let symbol = self.interner.intern(word);
            self.push(TokenKind::LowerIdent(symbol), start, self.pos);
            return;
        }
        // Edition 0.2: after `.`, `with` names the map method (`spec/21 §2`, ADR 0069 M2).
        let after_dot = self
            .out
            .tokens
            .last()
            .is_some_and(|token| token.kind == TokenKind::Dot);
        // Edition 0.3: `from` right before `:` may be the argument label of
        // `copy_sign(from: y)` (`spec/29 §3`). It stays the keyword; the parser accepts it as a
        // label in a call's arguments only, so no declaration can take the name.
        if word == "from" && self.out.keywords >= KeywordSet::V0_3 && self.peek() == Some(':') {
            self.out.from_label = Some(self.interner.intern("from"));
        }
        // Edition 0.5: `with` and `and` right before `:` may be argument labels of the dense
        // methods (ADR 0120, moved to edition 0.5 by ADR 0139); they stay keywords, which the
        // parser accepts as labels in a call's arguments only.
        if self.out.keywords >= KeywordSet::V0_5 && self.peek() == Some(':') {
            match word {
                "with" => self.out.with_label = Some(self.interner.intern("with")),
                "and" => self.out.and_label = Some(self.interner.intern("and")),
                _ => {}
            }
        }
        // Edition 0.4: `error` right before `:` or after `.` may be the field of `Failed(error:
        // x)` (`spec/27 §5`). It stays the keyword; the parser accepts it as a field name, a
        // pattern's field and an argument label only, so no binding can take the name.
        if word == "error"
            && self.out.keywords >= KeywordSet::V0_4
            && (self.peek() == Some(':') || after_dot)
        {
            self.out.error_label = Some(self.interner.intern("error"));
        }
        // Edition 0.4: `diverges` alone on its line is the clause of a header (`spec/26 §5`);
        // anywhere else it stays reserved (U0103).
        let line_start = self
            .out
            .tokens
            .last()
            .is_none_or(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Indent));
        let clause = word == "diverges"
            && line_start
            && self.out.keywords >= KeywordSet::V0_4
            && self.rest_of_line_blank();
        // Edition 0.4: `within` right after `for ever` is the deadline of the loop (`spec/26
        // §7`), and right after an expression it is the deadline of that expression (`spec/27
        // §5`, ADR 0117 rule 5); anywhere else, as a name, it stays reserved (U0103).
        let deadline = word == "within"
            && self.out.keywords >= KeywordSet::V0_4
            && (self.after_for_ever() || self.after_expression());
        // Edition 0.3: the test declaration is written `test`, so `example` at column 1, where
        // a declaration starts, is U0103 with the exact rewrite (ADR 0105 rule 3); as a name it
        // stays the reserved keyword it is in editions 0.1 to 0.3 (rule 2). From edition 0.4 it
        // is an identifier, with no U0103 and no rewrite (ADR 0105, amendment of 2026-11-05).
        if word == "example"
            && self.out.keywords == KeywordSet::V0_3
            && (start == 0
                || self
                    .text
                    .get(..start)
                    .is_some_and(|before| before.ends_with('\n')))
            && self
                .text
                .get(self.pos..)
                .is_some_and(|rest| rest.trim_start_matches(' ').starts_with('"'))
        {
            self.example_in_0_3(start);
        }
        // Edition 0.5: `let` is reserved, U7007 with the exact rewrite to `const` or `var` (ADR
        // 0137 rules 1 and 11). The token stays the keyword `let`, so the parser reads the
        // binding and its body gets its own checks.
        if word == "let" && self.out.keywords >= KeywordSet::V0_5 {
            self.let_in_0_5(start);
        }
        let kind = if (word == "with" && after_dot && self.out.keywords >= KeywordSet::V0_2)
            || clause
            || deadline
        {
            TokenKind::LowerIdent(self.interner.intern(word))
        } else if let Some(keyword) = Keyword::from_word_in(word, self.out.keywords) {
            TokenKind::Keyword(keyword)
        } else if let Some(message) = reserved_in(word, self.out.keywords) {
            self.reserved(word, message, start);
            TokenKind::Reserved(self.interner.intern(word))
        } else {
            TokenKind::LowerIdent(self.interner.intern(word))
        };
        self.push(kind, start, self.pos);
    }

    /// Whether the last two tokens are `for` and `ever`, on the current line.
    fn after_for_ever(&self) -> bool {
        let tokens = &self.out.tokens;
        let n = tokens.len();
        if n < 2 {
            return false;
        }
        let (Some(first), Some(second)) = (tokens.get(n - 2), tokens.get(n - 1)) else {
            return false;
        };
        first.kind == TokenKind::Keyword(Keyword::For)
            && matches!(second.kind, TokenKind::LowerIdent(word) if self.interner.resolve(word) == "ever")
    }

    /// Whether the last token on the current line can end an expression: a name, a capability
    /// reference, a literal, or a closing bracket. A name never stands there, so `within` after
    /// one is the deadline of `spec/27 §5`.
    fn after_expression(&self) -> bool {
        self.out.tokens.last().is_some_and(|token| {
            matches!(
                token.kind,
                TokenKind::LowerIdent(_)
                    | TokenKind::UpperIdent(_)
                    | TokenKind::CapRef(_)
                    | TokenKind::Int(_)
                    | TokenKind::Float(_)
                    | TokenKind::TextEnd
                    | TokenKind::RParen
                    | TokenKind::RBracket
                    | TokenKind::Keyword(Keyword::True | Keyword::False | Keyword::Unit)
            )
        })
    }

    /// Whether the rest of the current line, from the cursor, holds only spaces or a comment.
    fn rest_of_line_blank(&self) -> bool {
        let rest = self.text.get(self.pos..).unwrap_or("");
        let line = rest
            .split('\n')
            .next()
            .unwrap_or("")
            .trim_start_matches([' ', '\t', '\r']);
        line.is_empty() || line.starts_with("--")
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
        if Keyword::from_word_in(&name, self.out.keywords).is_some() {
            // A keyword cannot name a capability.
            self.error(
                Code::U0101,
                name_start,
                self.pos,
                format!("keyword `{name}` cannot name a capability"),
            );
        } else if let Some(message) = reserved_in(&name, self.out.keywords) {
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
                // A `{` that no expression can follow, as in JSON written in text
                // (`"{\"a\": 1}"`), is a literal brace written without its escape: one
                // U0007 with the edit, and the literal goes on, so no cascade follows (#403).
                Some('{') if interpolate && self.brace_is_literal(here + 1) => {
                    let span = self.span(here, here + 1);
                    let error = SyntaxError::new(
                        Code::U0007,
                        span,
                        "unescaped `{` in text, where no interpolation can start; write `\\{`",
                    )
                    .with_edit("write `\\{`", span, "\\{");
                    self.out.errors.push(error);
                    chunk.push('{');
                    self.pos += 1;
                }
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
                Some(c @ ('{' | '}')) if self.project => {
                    self.error(
                        Code::U0007,
                        here,
                        here + 1,
                        format!(
                            "a text of a project file is not interpolated; write `\\{c}` for `{c}`"
                        ),
                    );
                    chunk.push(c);
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

    /// Whether the `{` before `from` is a literal brace: after any blanks, a quote, a
    /// backslash or a `}` follows it, none of which can start an expression.
    fn brace_is_literal(&self, from: usize) -> bool {
        let rest = self.text.get(from..).unwrap_or("");
        matches!(
            rest.trim_start_matches([' ', '\t']).chars().next(),
            Some('"' | '\\' | '}')
        )
    }

    fn flush_chunk(&mut self, chunk: &mut String, start: usize, end: usize) {
        // INVARIANT (#1175): a text chunk is never empty and so never has an empty span: each
        // character pushed to `chunk` advanced `pos` past `start`. With `InterpStart` (`{`) and
        // the `InterpEnd` of a closed interpolation (`}`), each one scalar wide, this is what
        // the formatter's linear cost bound in `una-fmt` `break_line` assumes. Only the
        // `InterpEnd` of an unterminated literal (U0004) is empty, and a file with errors is
        // never formatted.
        debug_assert!(chunk.is_empty() || end > start);
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
    fn the_standard_modules_lex_without_renames_or_errors() {
        // #1894: `lex_in` collects the names a file uses only when a reserved word needs a
        // rename. The standard modules, lexed on every check, must never need one, so that
        // their lexing stays on the path without the collection. Each is lexed alone under the
        // keyword set `una_check::types::Standard::load` lexes it with.
        let modules: [(&str, &str, KeywordSet); 12] = [
            (
                "prelude.una",
                include_str!("../../una-check/src/std/prelude.una"),
                KeywordSet::V0_1,
            ),
            (
                "prelude_0_2.una",
                include_str!("../../una-check/src/std/prelude_0_2.una"),
                KeywordSet::V0_2,
            ),
            (
                "prelude_0_3.una",
                include_str!("../../una-check/src/std/prelude_0_3.una"),
                KeywordSet::V0_3,
            ),
            (
                "prelude_0_4.una",
                include_str!("../../una-check/src/std/prelude_0_4.una"),
                KeywordSet::V0_5,
            ),
            (
                "host_0_4.una",
                include_str!("../../una-check/src/std/host_0_4.una"),
                KeywordSet::V0_4,
            ),
            (
                "testing.una",
                include_str!("../../una-check/src/std/testing.una"),
                KeywordSet::V0_1,
            ),
            (
                "testing_0_4.una",
                include_str!("../../una-check/src/std/testing_0_4.una"),
                KeywordSet::V0_4,
            ),
            (
                "time.una",
                include_str!("../../una-check/src/std/time.una"),
                KeywordSet::V0_4,
            ),
            (
                "drive.una",
                include_str!("../../una-check/src/std/drive.una"),
                KeywordSet::V0_4,
            ),
            (
                "collections.una",
                include_str!("../../una-check/src/std/collections.una"),
                KeywordSet::V0_2,
            ),
            (
                "map_0_4.una",
                include_str!("../../una-check/src/std/map_0_4.una"),
                KeywordSet::V0_2,
            ),
            (
                "limit.una",
                include_str!("../../una-check/src/std/limit.una"),
                KeywordSet::V0_4,
            ),
        ];
        for (name, text, keywords) in modules {
            let mut sources = crate::source::SourceDb::new();
            let id = sources.add(name, text.as_bytes());
            let mut interner = Interner::new();
            let lexed = lex_in(sources.file(id), id, &mut interner, keywords);
            assert!(lexed.renames.is_empty(), "{name}: {:?}", lexed.renames);
            assert!(lexed.errors.is_empty(), "{name}: {:?}", lexed.errors);
            assert!(!lexed.tokens.is_empty(), "{name}");
        }
    }

    /// The errors of `text` lexed under `keywords`, as (code, start, end, repairs), each
    /// repair as (description, edit range and replacement).
    #[allow(clippy::type_complexity)]
    fn lexed_errors(
        text: &str,
        keywords: KeywordSet,
    ) -> Vec<(
        Code,
        usize,
        usize,
        Vec<(String, Option<(usize, usize, String)>)>,
    )> {
        let mut sources = crate::source::SourceDb::new();
        let id = sources.add("m.una", text.as_bytes());
        let mut interner = Interner::new();
        lex_in(sources.file(id), id, &mut interner, keywords)
            .errors
            .into_iter()
            .map(|error| {
                let repairs = error
                    .repairs
                    .into_iter()
                    .map(|repair| {
                        (
                            repair.description,
                            repair.edit.map(|(span, text)| (span.start, span.end, text)),
                        )
                    })
                    .collect();
                (error.code, error.span.start, error.span.end, repairs)
            })
            .collect()
    }

    #[test]
    fn let_is_u7007_with_the_exact_rewrite_in_edition_0_5() {
        // ADR 0137 rules 1 and 11: `let edit x` becomes `var x`, `let x` becomes `const x`,
        // at the word, in blocks and at the top level.
        let text = "module m\n\nf() -> Int\n=\n  let edit a = 1\n  let b: Int = a\n  let  edit c = b\n  let edited = c\n  edited\n\nlet top = 1\n";
        let at = |needle: &str| text.find(needle).unwrap();
        let message = "this form of edition 0.5 changed in release v0.5.2: `let` is written `const`, or `var` for a binding that is reassigned";
        let errors = lexed_errors(text, KeywordSet::V0_5);
        let expected = [
            (at("let edit a"), at("let edit a") + 8, "var"),
            (at("let b"), at("let b") + 3, "const"),
            (at("let  edit c"), at("let  edit c") + 9, "var"),
            (at("let edited"), at("let edited") + 3, "const"),
            (at("let top"), at("let top") + 3, "const"),
        ];
        assert_eq!(errors.len(), expected.len(), "{errors:?}");
        for ((code, start, end, repairs), (from, to, word)) in errors.iter().zip(expected) {
            assert_eq!(*code, Code::U7007);
            assert_eq!((*start, *end), (from, from + 3), "the word `let`");
            assert_eq!(
                repairs,
                &vec![(format!("write `{word}`"), Some((from, to, word.to_owned())))]
            );
        }
        let mut sources = crate::source::SourceDb::new();
        let id = sources.add("m.una", text.as_bytes());
        let mut interner = Interner::new();
        let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_5);
        assert!(
            lexed
                .errors
                .iter()
                .all(|e| e.message == message && e.recovered)
        );
        // The token stays the keyword, so the parser reads the binding.
        assert!(
            lexed
                .tokens
                .iter()
                .any(|t| t.kind == TokenKind::Keyword(Keyword::Let))
        );
        // Editions 0.2 to 0.4 keep `let` and `let edit` (edition 0.1 reserves `edit`).
        for keywords in [KeywordSet::V0_2, KeywordSet::V0_3, KeywordSet::V0_4] {
            assert!(lexed_errors(text, keywords).is_empty(), "{keywords:?}");
        }
    }

    #[test]
    fn var_gains_the_help_of_edition_0_5_before_it() {
        // ADR 0137 rule 1: `var x` keeps U0103 in editions 0.1 to 0.4 and gains the help; a
        // `var` that does not start a binding's line gets no help; edition 0.5 has no error.
        let text = "module m\n\nf() -> Int\n=\n  var x = 1\n  g(var)\n";
        for keywords in [KeywordSet::V0_1, KeywordSet::V0_2, KeywordSet::V0_4] {
            let errors = lexed_errors(text, keywords);
            assert_eq!(errors.len(), 2, "{keywords:?}: {errors:?}");
            assert!(errors.iter().all(|(code, ..)| *code == Code::U0103));
            let helps: Vec<bool> = errors
                .iter()
                .map(|(_, _, _, repairs)| {
                    repairs
                        .iter()
                        .any(|(d, edit)| d == BINDING_WORDS_HELP && edit.is_none())
                })
                .collect();
            assert_eq!(helps, [true, false], "{keywords:?}");
        }
        assert!(lexed_errors(text, KeywordSet::V0_5).is_empty());
    }

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
