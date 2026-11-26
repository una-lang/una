//! Exact rewrites of foreign syntax (ADR 0096, part A).
//!
//! A shape that another language would accept stays a U0101: Una has one syntax (charter P4).
//! Its fix is the error's first repair, with an edit when the rewrite is exact (ADR 0096 rule 2):
//! it changes only the declaration it is reported in, the edited range then parses, the result
//! is the reading the table of rule 3 states, token for token, and it is deterministic.
//!
//! The shapes are recognized from the lines of the declaration, since a layout error is a matter
//! of lines and indentation. Each candidate edit is then checked by parsing the declaration with
//! the edit applied; a candidate that leaves a syntax error in or before the edited range is
//! dropped, and the repair keeps its description without an edit. One declaration is parsed
//! once more per broken declaration, so the cost stays linear in the source.

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;

use crate::error::{SyntaxError, SyntaxRepair};
use crate::intern::Interner;
use crate::lexer::{lex_in, starts_declaration};
use crate::parser::parse_plain;
use crate::source::{FileId, SourceDb, Span};
use crate::token::KeywordSet;

const HEADER: &str = "put `=` on a line of its own: a function whose body is a block has the long form, the header, `=`, then the indented body";
const IF_BLOCK: &str = "write the `if` with blocks: `if` and its condition, the indented branch, `else`, then the indented branch";
const GUARD: &str = "make the rest of the body the `else` branch of an `if` with blocks";
const WHEN: &str = "write the `when` as a block, each arm on a line of its own";
const ARM_BODY: &str = "start the body on the next line, as an indented block";
const FN_WORD: &str = "delete `fn`: a function declaration starts with its name";
const PRIV_WORD: &str = "delete `priv`: a declaration without `pub` is private";
const LAMBDA: &str =
    "a lambda's body is one line; move the block into a named function and pass it by name";
const LIST_PATTERN: &str = "Una has no list patterns: test `xs.len == 0`, or match `xs.at(index: 0)` with `is Some(first)` and `is None`";
const TUPLE: &str = "Una has no tuples: declare a record kind with named fields";

/// The description of a shape that has no exact rewrite (ADR 0096 rule 3): Una's form only.
fn advice_only(
    lines: &[Line<'_>],
    at: usize,
    message: &str,
    words: &[&str],
) -> Option<&'static str> {
    let content = lines.get(at)?.content();
    let previous = (0..at)
        .rev()
        .filter_map(|i| lines.get(i))
        .find(|l| !l.blank())
        .map_or("", |l| l.content());
    if (previous.trim_end().ends_with("->") && previous.contains("fn("))
        || (content.trim_end().ends_with("->") && content.contains("fn("))
    {
        return Some(LAMBDA);
    }
    if message.contains("expected a literal")
        && (content.starts_with("is [") || content.contains("::"))
    {
        return Some(LIST_PATTERN);
    }
    if message.contains("unexpected `(`, expected a type")
        || message.contains("unexpected `(`, expected a name")
        || words
            .iter()
            .any(|word| content.starts_with(&format!("{word} (")))
    {
        return Some(TUPLE);
    }
    None
}

/// The words that begin a binding statement under `keywords`: `let` up to edition 0.4, and
/// `const` and `var` in edition 0.5, where `let` is U7007 (ADR 0137 rules 1 and 2).
fn binding_words(keywords: KeywordSet) -> &'static [&'static str] {
    if keywords >= KeywordSet::V0_5 {
        &["let", "const", "var"]
    } else {
        &["let"]
    }
}

/// A line of the text: where it starts, its text without the line break, and its indentation.
#[derive(Clone, Copy)]
struct Line<'a> {
    start: usize,
    text: &'a str,
    indent: usize,
}

impl<'a> Line<'a> {
    fn content(&self) -> &'a str {
        self.text.get(self.indent..).unwrap_or("")
    }

    fn blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// The lines of `text`; `None` when an indentation holds a tab, which no rewrite handles.
fn lines_of(text: &str) -> Option<Vec<Line<'_>>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for text_line in text.split('\n') {
        let indent = text_line.len() - text_line.trim_start_matches(' ').len();
        if text_line
            .get(indent..)
            .is_some_and(|rest| rest.starts_with('\t'))
        {
            return None;
        }
        lines.push(Line {
            start,
            text: text_line,
            indent,
        });
        start += text_line.len() + 1;
    }
    Some(lines)
}

/// The index of the line of `lines` that holds the byte offset `at`: the last that starts at
/// or before it, if any. [`lines_of`] gives the lines in the order of the text, so their starts
/// increase strictly and a binary search finds it in O(log L) for `L` lines; a scan from the
/// end, once per error, made the repairs of a file with many errors quadratic (#1573).
fn line_at(lines: &[Line<'_>], at: usize) -> Option<usize> {
    lines
        .partition_point(|line| line.start <= at)
        .checked_sub(1)
}

/// Whether a line starts a declaration, as the parser's recovery reads it, or would with a
/// foreign word in front (`fn name(`, `priv name(`).
fn starts_decl(line: &Line<'_>, keywords: KeywordSet) -> bool {
    if line.indent != 0 || line.blank() {
        return false;
    }
    if foreign_word(line.text).is_some() {
        return true;
    }
    let word: String = line
        .text
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if word.is_empty() {
        return false;
    }
    let next = line
        .text
        .get(word.len()..)
        .and_then(|rest| rest.chars().next());
    starts_declaration(&word, next, keywords)
}

/// The indices of the lines of `lines` that start a declaration, in order: read once per file,
/// in O(L) for `L` lines.
fn decl_starts(lines: &[Line<'_>], keywords: KeywordSet) -> Vec<usize> {
    (0..lines.len())
        .filter(|&i| lines.get(i).is_some_and(|line| starts_decl(line, keywords)))
        .collect()
}

/// The declaration that holds line `at`, as `(first line, line past its end)`: the last start of
/// `starts` at or before `at`, to the next start or the end of the `len` lines. A binary search
/// finds it in O(log D) for `D` declarations; a scan of the lines back and forth, once per
/// error, made the repairs of one large declaration with many errors quadratic (#1635).
fn decl_around(starts: &[usize], at: usize, len: usize) -> Option<(usize, usize)> {
    let next = starts.partition_point(|&i| i <= at);
    let decl = *starts.get(next.checked_sub(1)?)?;
    Some((decl, starts.get(next).copied().unwrap_or(len)))
}

/// A position of `s` at bracket depth 0, outside text literals.
fn depth_zero(s: &str) -> Vec<bool> {
    let mut out = vec![false; s.len() + 1];
    let mut depth = 0i32;
    let mut in_text = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if in_text {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_text = false;
            }
            continue;
        }
        match c {
            '"' => in_text = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ if depth == 0 => {
                if let Some(slot) = out.get_mut(i) {
                    *slot = true;
                }
            }
            _ => {}
        }
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The positions of the whole word `word` in `s` at depth 0, outside text literals.
fn find_words(s: &str, word: &str) -> Vec<usize> {
    let zero = depth_zero(s);
    let mut found = Vec::new();
    for (i, _) in s.match_indices(word) {
        let before = s.get(..i).and_then(|b| b.chars().next_back());
        let after = s.get(i + word.len()..).and_then(|a| a.chars().next());
        let symbol = !word.chars().all(is_word_char);
        let bounded =
            symbol || (!before.is_some_and(is_word_char) && !after.is_some_and(is_word_char));
        if bounded && zero.get(i).copied().unwrap_or(false) {
            found.push(i);
        }
    }
    found
}

fn find_word(s: &str, word: &str) -> Option<usize> {
    find_words(s, word).first().copied()
}

/// A branch of a rewritten construct: a line of text, or lines of the source.
enum Part {
    Inline(String),
    Lines(usize, usize),
    /// An `else if` chain that continues the `if` (#935): the lines from its `else if` line
    /// to the end of the chain, kept at the indentation of the `if`, with a `then` that ends an
    /// `else if` line removed, since the chain's branches are blocks.
    Chain(usize, usize),
}

/// Appends `part` indented at `at`, keeping the relative indentation of source lines.
fn emit(out: &mut String, lines: &[Line<'_>], part: &Part, at: usize) {
    match part {
        Part::Inline(text) => {
            out.push_str(&" ".repeat(at));
            out.push_str(text.trim());
            out.push('\n');
        }
        Part::Lines(from, to) | Part::Chain(from, to) => {
            let chain = matches!(part, Part::Chain(..));
            let body = lines.get(*from..*to).unwrap_or(&[]);
            let least = body
                .iter()
                .filter(|line| !line.blank())
                .map(|line| line.indent)
                .min()
                .unwrap_or(at);
            for line in body {
                if line.blank() {
                    out.push('\n');
                } else {
                    out.push_str(&" ".repeat(line.indent - least + at));
                    let content = line.content();
                    // A chain line `else if c then` opens a block: `then` goes (#935).
                    let content = match content.trim_end().strip_suffix(" then") {
                        Some(head) if chain && line.indent == least && is_else_if(content) => head,
                        _ => content,
                    };
                    out.push_str(content);
                    out.push('\n');
                }
            }
        }
    }
}

/// Whether `content` opens an `else if` branch: the words `else` and `if`.
fn is_else_if(content: &str) -> bool {
    content
        .strip_prefix("else")
        .and_then(|rest| rest.strip_prefix(' '))
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix("if"))
        .is_some_and(|rest| !rest.starts_with(is_word_char))
}

/// The end of the `else if` chain whose `else if` line is `at` (#935): that line's block, then
/// each `else` line at `indent` with its block, blank lines between them included.
fn chain_end(lines: &[Line<'_>], at: usize, end: usize, indent: usize) -> usize {
    let mut stop = deeper(lines, at + 1, end, indent).max(at + 1);
    loop {
        let mut next = stop;
        while lines.get(next).is_some_and(|l| l.blank()) {
            next += 1;
        }
        let Some(line) = lines.get(next).filter(|_| next < end) else {
            return stop;
        };
        let after = line.content().get(4..).unwrap_or("x");
        if line.indent != indent
            || !line.content().starts_with("else")
            || after.starts_with(is_word_char)
        {
            return stop;
        }
        stop = deeper(lines, next + 1, end, indent).max(next + 1);
    }
}

/// The lines after `from` that are indented deeper than `indent`, without trailing blank lines:
/// the block that a line ending its own text opens.
fn deeper(lines: &[Line<'_>], from: usize, end: usize, indent: usize) -> usize {
    let mut last = from;
    let mut i = from;
    while i < end {
        let line = lines.get(i);
        match line {
            Some(line) if line.blank() => {}
            Some(line) if line.indent > indent => last = i + 1,
            _ => break,
        }
        i += 1;
    }
    last
}

/// The lines after `from` that belong to the block of a line at `indent`: every line indented
/// at least as deep, without trailing blank lines.
fn rest_of_block(lines: &[Line<'_>], from: usize, end: usize, indent: usize) -> usize {
    let mut last = from;
    let mut i = from;
    while i < end {
        match lines.get(i) {
            Some(line) if line.blank() => {}
            Some(line) if line.indent >= indent => last = i + 1,
            _ => break,
        }
        i += 1;
    }
    last
}

/// A candidate: the lines `from..to` of the declaration replaced by `text`.
struct Candidate {
    from: usize,
    to: usize,
    text: String,
    description: &'static str,
}

/// The `if` whose foreign layout covers line `at`: `if c then` at the end of a line, `then` or
/// `else` at the start of one, `if c then x else` at the end of a line, or `if c then x` with no
/// `else`. Rewritten as an `if` with blocks.
fn if_construct(lines: &[Line<'_>], at: usize, end: usize, words: &[&str]) -> Option<Candidate> {
    let mut k = at + 1;
    while k > 0 {
        k -= 1;
        let line = lines.get(k)?;
        if line.blank() || line.indent > lines.get(at)?.indent {
            continue;
        }
        if let Some(candidate) = if_at(lines, k, end, words)
            && candidate.from <= at
            && at < candidate.to
        {
            return Some(candidate);
        }
        if line.indent == 0 {
            return None;
        }
    }
    None
}

/// Whether a branch written on one line leaves its block: `fail` of a variant.
fn diverges(branch: &str) -> bool {
    let branch = branch.trim();
    branch.strip_prefix("fail ").is_some_and(|rest| {
        rest.trim_start()
            .starts_with(|c: char| c.is_ascii_uppercase())
    })
}

/// The length of what precedes `if` on a line that opens an `if`: nothing, or `let x = `
/// (`const x = ` or `var x = ` in edition 0.5).
fn if_prefix(content: &str, words: &[&str]) -> Option<usize> {
    if content.starts_with("if ") {
        return Some(0);
    }
    // A binding: `let` up to edition 0.4, `const` or `var` in edition 0.5 (ADR 0137).
    if words.iter().any(|word| {
        content
            .strip_prefix(word)
            .is_some_and(|rest| rest.starts_with(' '))
    }) {
        let eq = find_word(content, "=")?;
        let after = content.get(eq + 1..)?;
        let gap = after.len() - after.trim_start().len();
        if after.trim_start().starts_with("if ") {
            return Some(eq + 1 + gap);
        }
    }
    None
}

fn if_at(lines: &[Line<'_>], k: usize, end: usize, words: &[&str]) -> Option<Candidate> {
    let line = lines.get(k)?;
    let ind = line.indent;
    let content = line.content();
    let prefix = if_prefix(content, words)?;
    let after_if = content.get(prefix + 3..)?;
    let (condition, then_part, mut next, else_indent) = match find_word(after_if, "then") {
        Some(t) => {
            let condition = after_if.get(..t)?.trim().to_owned();
            let rest = after_if.get(t + 4..)?.trim();
            if rest.is_empty() {
                let stop = deeper(lines, k + 1, end, ind);
                if stop == k + 1 {
                    return None;
                }
                (condition, Part::Lines(k + 1, stop), stop, ind)
            } else if let Some(e) = find_word(rest, "else") {
                let then_text = rest.get(..e)?.trim().to_owned();
                let else_rest = rest.get(e + 4..)?.trim();
                let (else_part, stop) = if else_rest.is_empty() {
                    let block = deeper(lines, k + 1, end, ind);
                    if block > k + 1 {
                        (Part::Lines(k + 1, block), block)
                    } else {
                        // The rest of the block becomes the `else` branch only when the `then`
                        // branch leaves the block: otherwise the rest may be meant to run after
                        // the `if` either way, and the rewrite would change what runs (ADR
                        // 0096, amendment of 2026-10-13).
                        if prefix != 0 || !diverges(&then_text) {
                            return None;
                        }
                        let block = rest_of_block(lines, k + 1, end, ind);
                        if block == k + 1 {
                            return None;
                        }
                        (Part::Lines(k + 1, block), block)
                    }
                } else {
                    (Part::Inline(else_rest.to_owned()), k + 1)
                };
                let text = build(
                    line,
                    prefix,
                    &condition,
                    &Part::Inline(then_text),
                    &else_part,
                    lines,
                );
                return Some(Candidate {
                    from: k,
                    to: stop,
                    text,
                    description: IF_BLOCK,
                });
            } else {
                // `if c then x` with nothing after it on the line.
                let then_part = Part::Inline(rest.to_owned());
                if lines
                    .get(k + 1)
                    .is_some_and(|l| l.indent == ind && l.content().starts_with("else"))
                {
                    (condition, then_part, k + 1, ind)
                } else {
                    // A guard: only a `then` branch that fails leaves the rest of the body to
                    // the `else` branch with the same meaning (ADR 0096, amendment of
                    // 2026-10-13). A statement there, such as a print, may be meant to run
                    // before the rest either way.
                    if prefix != 0 || !diverges(rest) {
                        return None;
                    }
                    let stop = rest_of_block(lines, k + 1, end, ind);
                    if stop == k + 1 {
                        return None;
                    }
                    let text = build(
                        line,
                        prefix,
                        &condition,
                        &then_part,
                        &Part::Lines(k + 1, stop),
                        lines,
                    );
                    return Some(Candidate {
                        from: k,
                        to: stop,
                        text,
                        description: GUARD,
                    });
                }
            }
        }
        None => {
            // `if c` at the end of its line, then `then` at the start of the next one.
            let condition = after_if.trim().to_owned();
            let then_line = lines.get(k + 1)?;
            let after_then = then_line.content().get(4..).unwrap_or("x");
            if then_line.indent < ind
                || !then_line.content().starts_with("then")
                || after_then.starts_with(is_word_char)
            {
                return None;
            }
            let ti = then_line.indent;
            let rest = then_line.content().get(4..)?.trim();
            if rest.is_empty() {
                let stop = deeper(lines, k + 2, end, ti);
                if stop == k + 2 {
                    return None;
                }
                (condition, Part::Lines(k + 2, stop), stop, ti)
            } else {
                (condition, Part::Inline(rest.to_owned()), k + 2, ti)
            }
        }
    };
    // The `else` line.
    while lines.get(next).is_some_and(|l| l.blank()) {
        next += 1;
    }
    let else_line = lines.get(next)?;
    if next >= end || else_line.indent != else_indent || !else_line.content().starts_with("else") {
        return None;
    }
    let rest = else_line.content().get(4..)?;
    if rest.starts_with(is_word_char) {
        return None;
    }
    let rest = rest.trim();
    let (else_part, stop) = if rest.is_empty() {
        let stop = deeper(lines, next + 1, end, else_indent);
        if stop == next + 1 {
            return None;
        }
        (Part::Lines(next + 1, stop), stop)
    } else if is_else_if(else_line.content()) {
        // `else if d` continues the chain with blocks of its own (#935): it stays at the
        // indentation of the `if`, its lines as they are, rather than becoming a branch whose
        // block the edit would leave unindented.
        let stop = chain_end(lines, next, end, else_indent);
        (Part::Chain(next, stop), stop)
    } else {
        (Part::Inline(rest.to_owned()), next + 1)
    };
    let text = build(line, prefix, &condition, &then_part, &else_part, lines);
    Some(Candidate {
        from: k,
        to: stop,
        text,
        description: IF_BLOCK,
    })
}

/// `if c` / then-branch / `else` / else-branch, at the indentation of `line`.
fn build(
    line: &Line<'_>,
    prefix: usize,
    condition: &str,
    then: &Part,
    otherwise: &Part,
    lines: &[Line<'_>],
) -> String {
    let ind = line.indent;
    let mut out = String::new();
    out.push_str(&" ".repeat(ind));
    out.push_str(line.content().get(..prefix).unwrap_or(""));
    out.push_str("if ");
    out.push_str(condition);
    out.push('\n');
    emit(&mut out, lines, then, ind + 2);
    if matches!(otherwise, Part::Chain(..)) {
        // The chain brings its own `else if` lines, at the `if`'s indentation (#935).
        emit(&mut out, lines, otherwise, ind);
    } else {
        out.push_str(&" ".repeat(ind));
        out.push_str("else\n");
        emit(&mut out, lines, otherwise, ind + 2);
    }
    out
}

/// A `when` with its arms on its own line, after nothing, `let x = `, an arm's `->`, or a short
/// function header's `=`.
fn one_line_when(lines: &[Line<'_>], at: usize, end: usize) -> Option<Candidate> {
    let line = lines.get(at)?;
    let content = line.content();
    let w = find_words(content, "when").into_iter().find(|&w| {
        let before = content.get(..w).unwrap_or("").trim_end();
        before.is_empty() || before.ends_with('=') || before.ends_with("->")
    })?;
    let head = content.get(..w)?.trim_end();
    let tail = content.get(w + 4..)?;
    let mut keys: Vec<usize> = ["is", "in", "else"]
        .iter()
        .flat_map(|word| find_words(tail, word))
        .collect();
    keys.sort_unstable();
    let arrows = find_words(tail, "->");
    let mut arms = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let stop = keys.get(index + 1).copied().unwrap_or(tail.len());
        let after = tail.get(*key..).unwrap_or("");
        // An arm starts with a pattern: `else ->`, `is` and a variant or a literal, or `in`.
        let pattern = if let Some(rest) = after.strip_prefix("else") {
            rest.trim_start().starts_with("->")
        } else if let Some(rest) = after.strip_prefix("is") {
            rest.trim_start().starts_with(|c: char| {
                c.is_ascii_uppercase() || c.is_ascii_digit() || c == '"' || c == '-'
            }) || ["true", "false", "unit"]
                .iter()
                .any(|literal| rest.trim_start().starts_with(literal))
        } else {
            true
        };
        if pattern && arrows.iter().any(|a| key < a && *a < stop) {
            arms.push(*key);
        }
    }
    let first = *arms.first()?;
    let scrutinee = tail.get(..first)?.trim();
    if scrutinee.is_empty() {
        return None;
    }
    let ind = line.indent;
    let header = line.indent == 0 && head.ends_with('=') && head.contains("->");
    let (when_at, mut out) = if header {
        let signature = head.trim_end_matches('=').trim_end();
        (2, format!("{signature}\n=\n"))
    } else if head.ends_with("->") {
        (ind + 2, format!("{}{}\n", " ".repeat(ind), head))
    } else {
        (
            ind,
            format!(
                "{}{}{}",
                " ".repeat(ind),
                head,
                if head.is_empty() { "" } else { " " }
            ),
        )
    };
    if header || head.ends_with("->") {
        out.push_str(&" ".repeat(when_at));
    }
    out.push_str("when ");
    out.push_str(scrutinee);
    out.push('\n');
    for (index, start) in arms.iter().enumerate() {
        let stop = arms.get(index + 1).copied().unwrap_or(tail.len());
        out.push_str(&" ".repeat(when_at + 2));
        out.push_str(tail.get(*start..stop)?.trim());
        out.push('\n');
    }
    // A last arm that ends its line keeps the block that follows it.
    let mut stop = at + 1;
    if out.trim_end().ends_with("->") {
        let block = deeper(lines, at + 1, end, ind);
        if block == at + 1 {
            return None;
        }
        emit(&mut out, lines, &Part::Lines(at + 1, block), when_at + 4);
        stop = block;
    }
    Some(Candidate {
        from: at,
        to: stop,
        text: out,
        description: WHEN,
    })
}

/// `=` at the end of a function header, indented under its clauses, or followed by the body on
/// its own line after a long header. The body moves to column 2, its lines keeping their
/// relative indentation, since a file indents consistently.
fn header_equals(lines: &[Line<'_>], at: usize, decl: usize, end: usize) -> Option<Candidate> {
    let line = lines.get(at)?;
    let content = line.content();
    if line.indent == 0 && at == decl && content.trim_end().ends_with('=') && content.contains("->")
    {
        let signature = content.trim_end().trim_end_matches('=').trim_end();
        return Some(Candidate {
            from: at,
            to: at + 1,
            text: format!("{signature}\n=\n"),
            description: HEADER,
        });
    }
    if !content.starts_with('=') || content.starts_with("==") {
        return None;
    }
    let rest = content.get(1..)?;
    let body = rest.trim_start();
    if line.indent == 0 && body.is_empty() {
        return None;
    }
    // The column of the body's first line, and the lines that continue the body below it.
    let column = if body.is_empty() {
        None
    } else {
        Some(line.indent + 1 + (rest.len() - body.len()))
    };
    let block = deeper(lines, at + 1, end, line.indent);
    let least = lines
        .get(at + 1..block)
        .and_then(|b| b.iter().filter(|l| !l.blank()).map(|l| l.indent).min());
    // Everything below the body's first line moves with it.
    let base = match (column, least) {
        (Some(column), Some(least)) if least < column => return None,
        (Some(column), _) => column,
        (None, Some(least)) => least,
        (None, None) => return None,
    };
    let mut text = String::from("=\n");
    if column.is_some() {
        text.push_str("  ");
        text.push_str(body);
        text.push('\n');
    }
    if let Some(least) = least {
        emit(
            &mut text,
            lines,
            &Part::Lines(at + 1, block),
            2 + least - base,
        );
    }
    Some(Candidate {
        from: at,
        to: block.max(at + 1),
        text,
        description: HEADER,
    })
}

/// The foreign word in front of a declaration's name, `fn ` or `priv `, with the text after it.
fn foreign_word(text: &str) -> Option<(&'static str, &str)> {
    for (word, description) in [("fn ", FN_WORD), ("priv ", PRIV_WORD)] {
        if let Some(rest) = text.strip_prefix(word) {
            let name: String = rest.chars().take_while(|c| is_word_char(*c)).collect();
            if !name.is_empty()
                && rest
                    .get(name.len()..)
                    .is_some_and(|r| r.starts_with('(') || r.starts_with('['))
            {
                return Some((description, rest));
            }
        }
    }
    None
}

/// `fn name(` or `priv name(` at column 0.
fn leading_word(lines: &[Line<'_>], at: usize, decl: usize) -> Option<Candidate> {
    let line = lines.get(decl)?;
    if at != decl {
        return None;
    }
    let (description, rest) = foreign_word(line.text)?;
    Some(Candidate {
        from: decl,
        to: decl + 1,
        text: format!("{rest}\n"),
        description,
    })
}

/// An arm or branch whose body starts with a binding (`let`, or `const` or `var` in edition
/// 0.5) or `when` on its line, its block following.
fn arm_body(lines: &[Line<'_>], at: usize, end: usize, words: &[&str]) -> Option<Candidate> {
    let line = lines.get(at)?;
    let content = line.content();
    let arrow = find_words(content, "->").into_iter().find(|&a| {
        let body = content.get(a + 2..).unwrap_or("").trim_start();
        body.starts_with("when ")
            || words.iter().any(|word| {
                body.strip_prefix(word)
                    .is_some_and(|rest| rest.starts_with(' '))
            })
    })?;
    let head = content.get(..arrow + 2)?;
    if !(head.starts_with("is ") || head.starts_with("in ") || head.starts_with("else")) {
        return None;
    }
    let body = content.get(arrow + 2..)?.trim();
    let block = deeper(lines, at + 1, end, line.indent);
    let at_column = lines
        .get(at + 1..block)
        .and_then(|b| b.iter().filter(|l| !l.blank()).map(|l| l.indent).min())
        .unwrap_or(line.indent + 2);
    let mut text = format!(
        "{}{}\n{}{}\n",
        " ".repeat(line.indent),
        head,
        " ".repeat(at_column),
        body
    );
    if body.starts_with("when ") {
        // The arms of the `when` go one level deeper than the `when`.
        let mut moved = String::new();
        emit(
            &mut moved,
            lines,
            &Part::Lines(at + 1, block),
            at_column + 2,
        );
        text.push_str(&moved);
    } else {
        emit(&mut text, lines, &Part::Lines(at + 1, block), at_column);
    }
    Some(Candidate {
        from: at,
        to: block.max(at + 1),
        text,
        description: ARM_BODY,
    })
}

/// Whether `text` parses with lines `from..to` of the declaration `decl..decl_end` replaced by
/// `replacement`: no syntax error starts before the end of the replaced text.
fn parses(
    text: &str,
    lines: &[Line<'_>],
    (decl, decl_end): (usize, usize),
    candidate: &Candidate,
    keywords: KeywordSet,
) -> bool {
    let offset = |i: usize| lines.get(i).map_or(text.len(), |l| l.start);
    let (start, from, to, stop) = (
        offset(decl),
        offset(candidate.from),
        offset(candidate.to),
        offset(decl_end),
    );
    let mut snippet = String::from("module m\n");
    snippet.push_str(text.get(start..from).unwrap_or(""));
    snippet.push_str(&candidate.text);
    let edited_end = snippet.len();
    snippet.push_str(text.get(to.min(stop)..stop).unwrap_or(""));
    let mut sources = SourceDb::new();
    let id = sources.add("rewrite.una", snippet.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex_in(sources.file(id), id, &mut interner, keywords);
    let parsed = parse_plain(&lexed, id, &interner);
    lexed
        .errors
        .iter()
        .chain(&parsed.errors)
        .all(|error| error.span.start >= edited_end)
}

/// Adds the exact rewrite of each syntax error of `errors` that has no edit yet, when its
/// declaration holds a shape of ADR 0096's table and the rewrite parses; with the shape but no
/// parse, the description alone.
pub(crate) fn attach(text: &str, file: FileId, keywords: KeywordSet, errors: &mut [SyntaxError]) {
    let Some(lines) = lines_of(text) else {
        return;
    };
    let words = binding_words(keywords);
    // Read once, when the first error needs it.
    let mut starts: Option<Vec<usize>> = None;
    for error in errors.iter_mut() {
        if error.code != Code::U0101 || error.repairs.iter().any(|r| r.edit.is_some()) {
            continue;
        }
        let Some(at) = line_at(&lines, error.span.start) else {
            continue;
        };
        let starts = starts.get_or_insert_with(|| decl_starts(&lines, keywords));
        let Some((decl, decl_end)) = decl_around(starts, at, lines.len()) else {
            continue;
        };
        // An error of a module header or an import line has no rewrite here.
        if lines.get(decl).is_some_and(|l| {
            l.text.starts_with("module")
                || l.text.starts_with("trusted")
                || l.text.starts_with("import")
        }) {
            continue;
        }
        let candidates = [
            header_equals(&lines, at, decl, decl_end),
            leading_word(&lines, at, decl),
            one_line_when(&lines, at, decl_end),
            if_construct(&lines, at, decl_end, words),
            arm_body(&lines, at, decl_end, words),
        ];
        let Some(candidate) = candidates.into_iter().flatten().next() else {
            if error.repairs.is_empty()
                && let Some(description) = advice_only(&lines, at, &error.message, words)
            {
                error.repairs.push(SyntaxRepair {
                    description: description.to_owned(),
                    edit: None,
                });
            }
            continue;
        };
        let offset = |i: usize| lines.get(i).map_or(text.len(), |l| l.start);
        let repair = if parses(text, &lines, (decl, decl_end), &candidate, keywords) {
            let (from, to) = (offset(candidate.from), offset(candidate.to).min(text.len()));
            // A replaced range that runs to the end of a file without a final line break ends
            // with the text, and so does its replacement.
            let mut replacement = candidate.text.clone();
            if to == text.len() && !text.ends_with('\n') {
                replacement.pop();
            }
            SyntaxRepair {
                description: candidate.description.to_owned(),
                edit: Some((
                    Span {
                        file,
                        start: from,
                        end: to,
                    },
                    replacement,
                )),
            }
        } else {
            SyntaxRepair {
                description: candidate.description.to_owned(),
                edit: None,
            }
        };
        if repair.edit.is_some() || error.repairs.is_empty() {
            error.repairs.insert(0, repair);
        }
    }
}

/// A declaration of a file, as the parser's recovery delimits it: from a line that starts a
/// declaration (a module header and an import line included) to the next such line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The byte offset of its first line.
    pub start: usize,
    /// The byte offset just past its last line, the start of the next declaration.
    pub end: usize,
    /// Its first line, 0-based.
    pub first_line: usize,
    /// Just past its last line that is not blank, 0-based.
    pub last_line: usize,
    /// What it is.
    pub what: DeclarationKind,
    /// The names it declares, read from its tokens: its name, a kind's upper-case names, a
    /// capability's operations, an import's names.
    pub names: Vec<String>,
    /// Every word of its text, names, keywords and words of comments alike.
    pub words: BTreeSet<String>,
    /// For a module header or an import line, the module path it names.
    pub module: Option<String>,
}

impl Declaration {
    /// Whether this declaration is an import line of the module `path`.
    pub fn imports_from(&self, path: &str) -> bool {
        self.what == DeclarationKind::Import && self.module.as_deref() == Some(path)
    }
}

/// What a [`Declaration`] is, as far as its first line tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationKind {
    /// The module header.
    Header,
    /// An import line.
    Import,
    /// Any other declaration whose name could be read.
    Named,
    /// A declaration whose name cannot be read from its first line.
    Unnamed,
}

/// The declarations of `text` under the keywords of edition 0.1; empty when an indentation
/// holds a tab.
pub fn declarations(text: &str) -> Vec<Declaration> {
    declarations_in(text, KeywordSet::V0_1)
}

/// The declarations of `text` under the keyword set `keywords`, which decides whether a line
/// `test "..."` starts one (edition 0.3, ADR 0105); empty when an indentation holds a tab.
pub fn declarations_in(text: &str, keywords: KeywordSet) -> Vec<Declaration> {
    let Some(lines) = lines_of(text) else {
        return Vec::new();
    };
    let starts: Vec<usize> = (0..lines.len())
        .filter(|&i| lines.get(i).is_some_and(|line| starts_decl(line, keywords)))
        .collect();
    let mut out = Vec::new();
    for (index, &first) in starts.iter().enumerate() {
        let next = starts.get(index + 1).copied().unwrap_or(lines.len());
        let start = lines.get(first).map_or(text.len(), |l| l.start);
        let end = lines
            .get(next)
            .map_or(text.len(), |l| l.start)
            .min(text.len());
        let last_line = (first..next)
            .rev()
            .find(|&i| lines.get(i).is_some_and(|l| !l.blank()))
            .map_or(first + 1, |i| i + 1);
        let body = text.get(start..end).unwrap_or("");
        let (what, names) = declared(body, keywords);
        let module = matches!(what, DeclarationKind::Header | DeclarationKind::Import)
            .then(|| module_path(body))
            .flatten();
        out.push(Declaration {
            module,
            start,
            end,
            first_line: first,
            last_line,
            what,
            names,
            words: words(body).map(str::to_owned).collect(),
        });
    }
    out
}

/// The module path after `module`, `trusted module` or `import` on the first line of `body`.
fn module_path(body: &str) -> Option<String> {
    let first = body.split('\n').next().unwrap_or("");
    let rest = first
        .strip_prefix("trusted module ")
        .or_else(|| first.strip_prefix("module "))
        .or_else(|| first.strip_prefix("import "))?;
    let path: String = rest
        .trim_start()
        .chars()
        .take_while(|c| is_word_char(*c) || *c == '.')
        .collect();
    (!path.is_empty()).then_some(path)
}

/// What a declaration's text declares, read from its tokens (ADR 0096 rule 8), under the keyword
/// set `keywords`.
fn declared(body: &str, keywords: KeywordSet) -> (DeclarationKind, Vec<String>) {
    let first = body.split('\n').next().unwrap_or("");
    let first = foreign_word(first).map_or(first, |(_, rest)| rest);
    let first = first.strip_prefix("pub ").unwrap_or(first);
    let mut tokens = words(first);
    let Some(word) = tokens.next() else {
        return (DeclarationKind::Unnamed, Vec::new());
    };
    let upper = |text: &str| -> Vec<String> {
        words(text)
            .filter(|w| w.starts_with(|c: char| c.is_ascii_uppercase()))
            .map(str::to_owned)
            .collect()
    };
    match word {
        "module" | "trusted" => (DeclarationKind::Header, Vec::new()),
        "import" => {
            let inside = first
                .split_once('(')
                .map_or("", |(_, rest)| rest.split(')').next().unwrap_or(rest));
            (
                DeclarationKind::Import,
                words(inside).map(str::to_owned).collect(),
            )
        }
        "kind" => match tokens.next() {
            Some(name) => {
                let mut names = vec![name.to_owned()];
                names.extend(upper(body));
                (DeclarationKind::Named, names)
            }
            None => (DeclarationKind::Unnamed, Vec::new()),
        },
        "capability" => match tokens.next() {
            Some(name) => {
                let mut names = vec![name.to_owned()];
                for line in body.split('\n') {
                    if let Some(rest) = line.trim_start().strip_prefix("op ") {
                        names.extend(words(rest).next().map(str::to_owned));
                    }
                }
                (DeclarationKind::Named, names)
            }
            None => (DeclarationKind::Unnamed, Vec::new()),
        },
        "predicate" => match tokens.find(|w| *w != "reflect" && *w != "opaque") {
            Some(name) => (DeclarationKind::Named, vec![name.to_owned()]),
            None => (DeclarationKind::Unnamed, Vec::new()),
        },
        "handler" | "policy" | "requirement" => match tokens.next() {
            Some(name) => (DeclarationKind::Named, vec![name.to_owned()]),
            None => (DeclarationKind::Unnamed, Vec::new()),
        },
        // An example, or a test in edition 0.3 (ADR 0105), declares nothing another
        // declaration names; `test(` is a function named `test` in editions 0.1 and 0.2. From
        // edition 0.4 `example` is an identifier (ADR 0105, amendment of 2026-11-05), so
        // `example(` there is a function named `example`, read by the arm of a function name.
        "example" if keywords < KeywordSet::V0_4 => (DeclarationKind::Named, Vec::new()),
        "test"
            if !first
                .get(4..)
                .is_some_and(|rest| rest.starts_with('(') || rest.starts_with('[')) =>
        {
            (DeclarationKind::Named, Vec::new())
        }
        name if first
            .get(name.len()..)
            .is_some_and(|rest| rest.starts_with('(') || rest.starts_with('[')) =>
        {
            (DeclarationKind::Named, vec![name.to_owned()])
        }
        _ => (DeclarationKind::Unnamed, Vec::new()),
    }
}

/// The prelude and `std.testing`, whose names a rename never takes (ADR 0096 rule 3a), with
/// what edition 0.4 adds to `std.testing` (ADR 0116 rule 19) and the variants it adds to
/// `HttpError` (ADR 0116 rule 12, ADR 0117 rule 8).
const PRELUDES: [&str; 6] = [
    include_str!("../../una-check/src/std/prelude.una"),
    include_str!("../../una-check/src/std/prelude_0_2.una"),
    include_str!("../../una-check/src/std/testing.una"),
    include_str!("../../una-check/src/std/testing_0_4.una"),
    "Interrupted",
    "RateLimited",
];

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !is_word_char(c))
        .filter(|w| !w.is_empty())
}

/// The names a rename may not take (rule 3a): every word of the file, so any binding,
/// declaration, import or call it holds; every word of the prelude and `std.testing`; and the
/// keywords and reserved words of every edition. Every name visible at an occurrence is one of
/// these. The words are read once per file into a set, so each look-up is O(log w) for `w`
/// distinct words; reading the whole file again for each name made the repairs of a file with
/// many `_x` errors O(errors x file size) (#1573). The set holds the words of the text as it
/// is, never the names earlier repairs propose: each repair is computed alone, as before.
struct Taken<'a> {
    words: BTreeSet<&'a str>,
}

impl<'a> Taken<'a> {
    /// The words of `text` and of the preludes: O(n log w) for `n` words.
    fn of(text: &'a str) -> Taken<'a> {
        let mut words: BTreeSet<&'a str> = self::words(text).collect();
        for prelude in PRELUDES {
            words.extend(self::words(prelude));
        }
        Taken { words }
    }

    /// Whether a rename may not take `name`.
    fn contains(&self, name: &str) -> bool {
        self.words.contains(name)
            || crate::token::Keyword::from_word(name).is_some()
            || crate::keywords::reserved_for_later(name).is_some()
            || crate::keywords::reserved_in(name, KeywordSet::V0_2).is_some()
            || crate::keywords::is_contextual(name)
    }
}

/// `base`, or the first of `base2`, `base3`, ... that is not taken.
fn fresh(taken: &Taken<'_>, base: &str) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2..10_000)
        .map(|n| format!("{base}{n}"))
        .find(|name| !taken.contains(name))
        .unwrap_or_else(|| format!("{base}_renamed"))
}

/// Which bytes of `region` are inside a text literal and outside its interpolations.
fn in_plain_text(region: &str) -> Vec<bool> {
    let mut out = vec![false; region.len() + 1];
    let mut in_text = false;
    let mut interpolation = 0usize;
    let mut escaped = false;
    for (i, c) in region.char_indices() {
        if in_text && interpolation == 0 {
            if let Some(slot) = out.get_mut(i) {
                *slot = true;
            }
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_text = false;
            } else if c == '{' {
                interpolation = 1;
            }
        } else if in_text {
            match c {
                '{' => interpolation += 1,
                '}' => interpolation -= 1,
                _ => {}
            }
        } else if c == '"' {
            in_text = true;
        }
    }
    out
}

/// What the renames read of one declaration, once, when its first error needs it.
struct DeclText<'a> {
    /// The byte offset of its first line.
    start: usize,
    /// The byte offset just past its header line.
    header_end: usize,
    /// Its text.
    region: &'a str,
    /// Which bytes of `region` are inside a text literal and outside its interpolations.
    plain: Vec<bool>,
    /// The offsets in `region` of each whole word, in order.
    words: BTreeMap<&'a str, Vec<usize>>,
}

impl<'a> DeclText<'a> {
    /// The declaration of the lines `decl..decl_end`: O(m log w) for its `m` bytes and `w`
    /// distinct words.
    fn of(text: &'a str, lines: &[Line<'_>], (decl, decl_end): (usize, usize)) -> Option<Self> {
        let offset = |i: usize| lines.get(i).map_or(text.len(), |l| l.start);
        let header_end = lines.get(decl).map_or(0, |l| l.start + l.text.len());
        let (start, end) = (offset(decl), offset(decl_end).min(text.len()));
        let region = text.get(start..end)?;
        let mut words: BTreeMap<&'a str, Vec<usize>> = BTreeMap::new();
        let mut word_start: Option<usize> = None;
        for (i, c) in region.char_indices().chain([(region.len(), ' ')]) {
            match (is_word_char(c), word_start) {
                (true, None) => word_start = Some(i),
                (false, Some(from)) => {
                    if let Some(word) = region.get(from..i) {
                        words.entry(word).or_default().push(from);
                    }
                    word_start = None;
                }
                _ => {}
            }
        }
        Some(DeclText {
            start,
            header_end,
            region,
            plain: in_plain_text(region),
            words,
        })
    }

    /// The rename of `name` to a fresh name from `base` at every occurrence in this
    /// declaration, or `None` when an occurrence is not one a rename may take: O(k log L + s)
    /// for its `k` occurrences, `L` lines and the `s` bytes the edit spans.
    fn rename(
        &self,
        lines: &[Line<'_>],
        file: FileId,
        name: &str,
        new: impl FnOnce() -> String,
    ) -> Option<SyntaxRepair> {
        let (start, region) = (self.start, self.region);
        let occurrences = self.words.get(name)?;
        for &i in occurrences {
            let before = region.get(..i).and_then(|b| b.chars().next_back());
            let after_text = region.get(i + name.len()..).unwrap_or("");
            let absolute = start + i;
            let line_text = lines
                .get(line_at(lines, absolute).unwrap_or(0))
                .map_or("", |l| l.text);
            let labeled = after_text.trim_start().starts_with(':')
                && !after_text.trim_start().starts_with("::");
            let bound_by_let = region
                .get(..i)
                .is_some_and(|b| b.trim_end().ends_with("let"));
            if self.plain.get(i).copied().unwrap_or(false)
                || before == Some('.')
                || (absolute == start && !line_text.starts_with(' '))
                || (labeled && absolute >= self.header_end && !bound_by_let)
            {
                return None;
            }
        }
        let (&first, &last) = (occurrences.first()?, occurrences.last()?);
        let new = new();
        let mut replacement = String::new();
        let mut cursor = first;
        for &i in occurrences {
            replacement.push_str(region.get(cursor..i).unwrap_or(""));
            replacement.push_str(&new);
            cursor = i + name.len();
        }
        Some(SyntaxRepair {
            description: format!(
                "rename `{name}` to `{new}` in its declaration: a name starts with a letter"
            ),
            edit: Some((
                Span {
                    file,
                    start: start + first,
                    end: start + last + name.len(),
                },
                replacement,
            )),
        })
    }
}

/// The renames of names that start with `_` (U0001, ADR 0096 rule 3): `_x`, a local, a
/// parameter or a pattern's binding, becomes `x`, or the first fresh name of rule 3a, at every
/// occurrence in its declaration; `Variant(_)` loses its binding, and an arm `_ ->` becomes
/// `else ->`. Every occurrence of one name gets the same edit, which `una fix` applies once.
///
/// Each declaration is read once, into its words and their offsets, and each rename is
/// computed once per (declaration, name) and reused by every error of that name. With `n`
/// bytes, `e` errors and `r` bytes of repairs, the pass takes O((n + e) log n + r): reading the
/// declaration again for each error made one large declaration with many names quadratic
/// (#1635).
pub(crate) fn underscore_renames(
    text: &str,
    file: FileId,
    keywords: KeywordSet,
    errors: &mut [SyntaxError],
) {
    // The errors this repairs: U0001 on a `_`, with no repair yet. The lines of the text are
    // read only when there is one (#1894): every file is lexed through here.
    let wanted = |error: &SyntaxError| {
        error.code == Code::U0001 && error.message.contains("`_`") && error.repairs.is_empty()
    };
    if !errors.iter().any(wanted) {
        return;
    }
    let Some(lines) = lines_of(text) else {
        return;
    };
    // Each read once, when the first rename needs it.
    let mut taken: Option<Taken<'_>> = None;
    let mut starts: Option<Vec<usize>> = None;
    let mut decls: BTreeMap<usize, Option<DeclText<'_>>> = BTreeMap::new();
    let mut renames: BTreeMap<(usize, String), Option<SyntaxRepair>> = BTreeMap::new();
    for error in errors.iter_mut() {
        if !wanted(error) {
            continue;
        }
        let at_offset = error.span.start;
        let name: String = text
            .get(at_offset..)
            .map(|rest| rest.chars().take_while(|c| is_word_char(*c)).collect())
            .unwrap_or_default();
        let base = name.trim_start_matches('_');
        let Some(at) = line_at(&lines, at_offset) else {
            continue;
        };
        if base.is_empty() {
            if let Some(repair) = lone_underscore(text, file, at_offset) {
                error.repairs.push(repair);
            }
            continue;
        }
        if !base.starts_with(|c: char| c.is_ascii_lowercase()) {
            continue;
        }
        let starts = starts.get_or_insert_with(|| decl_starts(&lines, keywords));
        let Some(span) = decl_around(starts, at, lines.len()) else {
            continue;
        };
        let decl = decls
            .entry(span.0)
            .or_insert_with(|| DeclText::of(text, &lines, span));
        let Some(decl) = decl else {
            continue;
        };
        let repair = renames.entry((span.0, name.clone())).or_insert_with(|| {
            decl.rename(&lines, file, &name, || {
                fresh(taken.get_or_insert_with(|| Taken::of(text)), base)
            })
        });
        if let Some(repair) = repair {
            error.repairs.push(repair.clone());
        }
    }
}

/// `Variant(_)` becomes `Variant`, and an arm `_ ->` becomes `else ->`.
fn lone_underscore(text: &str, file: FileId, at: usize) -> Option<SyntaxRepair> {
    let before = text.get(..at)?;
    let after = text.get(at + 1..)?;
    if before.ends_with('(') && after.starts_with(')') {
        let variant = before.get(..before.len() - 1)?;
        let name: String = variant
            .chars()
            .rev()
            .take_while(|c| is_word_char(*c))
            .collect();
        if name.chars().last().is_some_and(|c| c.is_ascii_uppercase()) {
            return Some(SyntaxRepair {
                description: "leave the binding out: a pattern binds only the fields it names"
                    .to_owned(),
                edit: Some((
                    Span {
                        file,
                        start: at - 1,
                        end: at + 2,
                    },
                    String::new(),
                )),
            });
        }
    }
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    if before
        .get(line_start..)
        .is_some_and(|b| b.trim().is_empty())
        && after.trim_start().starts_with("->")
    {
        return Some(SyntaxRepair {
            description: "write the catch-all arm as `else`".to_owned(),
            edit: Some((
                Span {
                    file,
                    start: at,
                    end: at + 1,
                },
                "else".to_owned(),
            )),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(source: &str) -> Option<String> {
        let mut sources = SourceDb::new();
        let id = sources.add("t.una", source.as_bytes());
        let mut interner = Interner::new();
        let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_1);
        let parsed = crate::parser::parse(&lexed, id, &interner);
        let error = parsed.errors.first()?;
        let (span, replacement) = error.repairs.first()?.edit.clone()?;
        let mut out = source.to_owned();
        out.replace_range(span.start..span.end, &replacement);
        Some(out)
    }

    #[test]
    fn a_header_ending_in_equals_gets_the_long_form() {
        let source = "module m\n\nf(x: Int) -> Int =\n  x + 1\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some("module m\n\nf(x: Int) -> Int\n=\n  x + 1\n")
        );
    }

    #[test]
    fn an_indented_equals_moves_to_column_0() {
        let source = "module m\n\nf(x: Int) -> Int\n  requires x > 0\n  = x + 1\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some("module m\n\nf(x: Int) -> Int\n  requires x > 0\n=\n  x + 1\n")
        );
    }

    #[test]
    fn then_at_the_end_of_a_line_becomes_an_if_with_blocks() {
        let source = "module m\n\nf(x: Int) -> Int\n=\n  if x > 0 then\n    x\n  else\n    0\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some("module m\n\nf(x: Int) -> Int\n=\n  if x > 0\n    x\n  else\n    0\n")
        );
    }

    #[test]
    fn then_at_the_end_of_a_line_before_an_else_if_chain_keeps_the_chain() {
        // #935: the `else if` branch has a block of its own, which stays indented under it, and
        // the chain's last `else` stays at the `if`'s indentation.
        let source = "module m\n\nf(grams: Int) -> Text\n=\n  if grams <= 1_000 then\n    \"small\"\n  else if grams <= 5_000\n    let kilos = (grams + 999) / 1_000\n    \"medium {kilos}\"\n  else\n    \"large\"\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some(
                "module m\n\nf(grams: Int) -> Text\n=\n  if grams <= 1_000\n    \"small\"\n  else if grams <= 5_000\n    let kilos = (grams + 999) / 1_000\n    \"medium {kilos}\"\n  else\n    \"large\"\n"
            )
        );
    }

    #[test]
    fn then_at_the_end_of_an_else_if_line_goes_with_the_chain() {
        // #935: a chain whose `else if` line ends with `then` too is repaired in one edit, so
        // the edited range parses.
        let source = "module m\n\nf(x: Int) -> Int\n=\n  if x > 10 then\n    2\n  else if x > 0 then\n    1\n  else\n    0\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some(
                "module m\n\nf(x: Int) -> Int\n=\n  if x > 10\n    2\n  else if x > 0\n    1\n  else\n    0\n"
            )
        );
    }

    #[test]
    fn an_else_chain_on_the_next_lines_becomes_an_if_with_blocks() {
        let source =
            "module m\n\nf(x: Int) -> Int\n=\n  if x > 0 then fail Broke else\n  let y = x\n  y\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some(
                "module m\n\nf(x: Int) -> Int\n=\n  if x > 0\n    fail Broke\n  else\n    let y = x\n    y\n"
            )
        );
    }

    #[test]
    fn then_at_the_start_of_a_line_becomes_an_if_with_blocks() {
        let source = "module m\n\nf(x: Int) -> Int\n=\n  if x > 0\n    then 1\n    else 0\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some("module m\n\nf(x: Int) -> Int\n=\n  if x > 0\n    1\n  else\n    0\n")
        );
    }

    #[test]
    fn a_guard_without_else_takes_the_rest_of_the_body() {
        let source = "module m\n\nkind E = Broke\n\nf(x: Int) -> Int\n  fails E\n=\n  if x < 0 then fail Broke\n  x + 1\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some(
                "module m\n\nkind E = Broke\n\nf(x: Int) -> Int\n  fails E\n=\n  if x < 0\n    fail Broke\n  else\n    x + 1\n"
            )
        );
    }

    #[test]
    fn a_guard_that_does_not_fail_gets_no_edit() {
        // ADR 0096, amendment of 2026-10-13: the print may be meant to run before the rest
        // either way, so the rest does not become an `else` branch.
        let source = "module m\n\nf(n: Int) -> Unit\n  uses @console: Console[print]\n=\n  if n > 0 then @console.print(\"positive\")\n  @console.print(\"done\")\n";
        assert_eq!(rewrite(source), None);
    }

    #[test]
    fn an_else_chain_after_a_value_gets_no_edit() {
        let source = "module m\n\nf(x: Int) -> Int\n=\n  if x > 0 then 1 else\n  let y = x\n  y\n";
        assert_eq!(rewrite(source), None);
    }

    #[test]
    fn a_one_line_when_gets_one_arm_per_line() {
        let source = "module m\n\nf(x: Option[Int]) -> Int\n=\n  when x is Some(value) -> value is None -> 0\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some(
                "module m\n\nf(x: Option[Int]) -> Int\n=\n  when x\n    is Some(value) -> value\n    is None -> 0\n"
            )
        );
    }

    fn lexical(source: &str) -> Option<String> {
        let mut sources = SourceDb::new();
        let id = sources.add("t.una", source.as_bytes());
        let mut interner = Interner::new();
        let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_1);
        let error = lexed.errors.first()?;
        let (span, replacement) = error.repairs.first()?.edit.clone()?;
        let mut out = source.to_owned();
        out.replace_range(span.start..span.end, &replacement);
        Some(out)
    }

    #[test]
    fn an_underscore_name_is_renamed_in_its_declaration() {
        let source = "module m\n\nf(_seats: Int) -> Int = _seats + 1\n";
        assert_eq!(
            lexical(source).as_deref(),
            Some("module m\n\nf(seats: Int) -> Int = seats + 1\n")
        );
    }

    #[test]
    fn a_rename_takes_no_name_of_an_outer_local_or_a_function() {
        // `x` is a function of the module and a local of the scope: the rename takes `x2`.
        let source = "module m\n\nx(n: Int) -> Int = n\n\nf(a: Int) -> Int\n=\n  let x = a\n  let _x = x(n: x)\n  _x + x\n";
        assert_eq!(
            lexical(source).as_deref(),
            Some(
                "module m\n\nx(n: Int) -> Int = n\n\nf(a: Int) -> Int\n=\n  let x = a\n  let x2 = x(n: x)\n  x2 + x\n"
            )
        );
    }

    #[test]
    fn one_name_in_two_declarations_gets_a_rename_in_each() {
        // The renames are computed once per (declaration, name): each error of `_seats` gets the
        // edit of its own declaration, the same for both errors of one declaration.
        let source = "module m\n\nf(_seats: Int) -> Int = _seats + 1\n\ng(_seats: Int) -> Int = _seats + 2\n";
        let mut sources = SourceDb::new();
        let id = sources.add("t.una", source.as_bytes());
        let mut interner = Interner::new();
        let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_1);
        let edits: Vec<(usize, usize, String)> = lexed
            .errors
            .iter()
            .filter_map(|e| e.repairs.first()?.edit.clone())
            .map(|(span, text)| (span.start, span.end, text))
            .collect();
        let f = source.find("_seats").unwrap_or(0);
        let g = source.rfind("g(_seats").unwrap_or(0) + 2;
        let one = (
            f,
            f + "_seats: Int) -> Int = _seats".len(),
            "seats: Int) -> Int = seats".to_owned(),
        );
        let two = (
            g,
            g + "_seats: Int) -> Int = _seats".len(),
            "seats: Int) -> Int = seats".to_owned(),
        );
        assert_eq!(edits, vec![one.clone(), one, two.clone(), two]);
    }

    #[test]
    fn a_lone_underscore_binding_is_left_out() {
        let source = "module m\n\nf(o: Option[Int]) -> Bool\n=\n  when o\n    is Some(_) -> true\n    is None -> false\n";
        assert_eq!(
            lexical(source).as_deref(),
            Some(
                "module m\n\nf(o: Option[Int]) -> Bool\n=\n  when o\n    is Some -> true\n    is None -> false\n"
            )
        );
    }

    #[test]
    fn a_semicolon_at_the_end_of_a_line_is_deleted() {
        let source = "module m\n\nf(a: Int) -> Int\n=\n  let b = a;\n  b\n";
        assert_eq!(
            lexical(source).as_deref(),
            Some("module m\n\nf(a: Int) -> Int\n=\n  let b = a\n  b\n")
        );
    }

    #[test]
    fn fn_before_a_declaration_is_deleted() {
        let source = "module m\n\nfn f(x: Int) -> Int = x\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some("module m\n\nf(x: Int) -> Int = x\n")
        );
    }

    #[test]
    fn a_comparison_in_a_branch_is_not_an_arm() {
        let source = "module m\n\nf(x: Option[Int], y: Bool) -> Bool\n=\n  when x is Some(v) -> y is None -> false\n";
        assert_eq!(
            rewrite(source).as_deref(),
            Some(
                "module m\n\nf(x: Option[Int], y: Bool) -> Bool\n=\n  when x\n    is Some(v) -> y\n    is None -> false\n"
            )
        );
    }
}
