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

use std::collections::BTreeSet;

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
fn advice_only(lines: &[Line<'_>], at: usize, message: &str) -> Option<&'static str> {
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
        || content.starts_with("let (")
    {
        return Some(TUPLE);
    }
    None
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
}

/// Appends `part` indented at `at`, keeping the relative indentation of source lines.
fn emit(out: &mut String, lines: &[Line<'_>], part: &Part, at: usize) {
    match part {
        Part::Inline(text) => {
            out.push_str(&" ".repeat(at));
            out.push_str(text.trim());
            out.push('\n');
        }
        Part::Lines(from, to) => {
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
                    out.push_str(line.content());
                    out.push('\n');
                }
            }
        }
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
fn if_construct(lines: &[Line<'_>], at: usize, end: usize) -> Option<Candidate> {
    let mut k = at + 1;
    while k > 0 {
        k -= 1;
        let line = lines.get(k)?;
        if line.blank() || line.indent > lines.get(at)?.indent {
            continue;
        }
        if let Some(candidate) = if_at(lines, k, end)
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

/// The length of what precedes `if` on a line that opens an `if`: nothing, or `let x = `.
fn if_prefix(content: &str) -> Option<usize> {
    if content.starts_with("if ") {
        return Some(0);
    }
    if content.starts_with("let ") {
        let eq = find_word(content, "=")?;
        let after = content.get(eq + 1..)?;
        let gap = after.len() - after.trim_start().len();
        if after.trim_start().starts_with("if ") {
            return Some(eq + 1 + gap);
        }
    }
    None
}

fn if_at(lines: &[Line<'_>], k: usize, end: usize) -> Option<Candidate> {
    let line = lines.get(k)?;
    let ind = line.indent;
    let content = line.content();
    let prefix = if_prefix(content)?;
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
    out.push_str(&" ".repeat(ind));
    out.push_str("else\n");
    emit(&mut out, lines, otherwise, ind + 2);
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

/// An arm or branch whose body starts with `let` or `when` on its line, its block following.
fn arm_body(lines: &[Line<'_>], at: usize, end: usize) -> Option<Candidate> {
    let line = lines.get(at)?;
    let content = line.content();
    let arrow = find_words(content, "->").into_iter().find(|&a| {
        let body = content.get(a + 2..).unwrap_or("").trim_start();
        body.starts_with("let ") || body.starts_with("when ")
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
    for error in errors.iter_mut() {
        if error.code != Code::U0101 || error.repairs.iter().any(|r| r.edit.is_some()) {
            continue;
        }
        let Some(at) = lines
            .iter()
            .rposition(|line| line.start <= error.span.start)
        else {
            continue;
        };
        let Some(decl) = (0..=at)
            .rev()
            .find(|&i| lines.get(i).is_some_and(|line| starts_decl(line, keywords)))
        else {
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
        let decl_end = (at + 1..lines.len())
            .find(|&i| lines.get(i).is_some_and(|line| starts_decl(line, keywords)))
            .unwrap_or(lines.len());
        let candidates = [
            header_equals(&lines, at, decl, decl_end),
            leading_word(&lines, at, decl),
            one_line_when(&lines, at, decl_end),
            if_construct(&lines, at, decl_end),
            arm_body(&lines, at, decl_end),
        ];
        let Some(candidate) = candidates.into_iter().flatten().next() else {
            if error.repairs.is_empty()
                && let Some(description) = advice_only(&lines, at, &error.message)
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
        let (what, names) = declared(body);
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

/// What a declaration's text declares, read from its tokens (ADR 0096 rule 8).
fn declared(body: &str) -> (DeclarationKind, Vec<String>) {
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
        // declaration names; `test(` is a function named `test` in editions 0.1 and 0.2.
        "example" => (DeclarationKind::Named, Vec::new()),
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
/// what edition 0.4 adds to `std.testing` (ADR 0116 rule 19) and the variant it adds to
/// `HttpError` (ADR 0116 rule 12).
const PRELUDES: [&str; 5] = [
    include_str!("../../una-check/src/std/prelude.una"),
    include_str!("../../una-check/src/std/prelude_0_2.una"),
    include_str!("../../una-check/src/std/testing.una"),
    include_str!("../../una-check/src/std/testing_0_4.una"),
    "Interrupted",
];

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !is_word_char(c))
        .filter(|w| !w.is_empty())
}

/// Whether a rename may not take `name`: a word of the file, so any binding, declaration,
/// import or call it holds; a word of the prelude or `std.testing`; or a keyword or reserved
/// word of any edition. Every name visible at an occurrence is one of these (rule 3a).
fn taken(text: &str, name: &str) -> bool {
    words(text).any(|w| w == name)
        || PRELUDES
            .iter()
            .any(|prelude| words(prelude).any(|w| w == name))
        || crate::token::Keyword::from_word(name).is_some()
        || crate::keywords::reserved_for_later(name).is_some()
        || crate::keywords::reserved_in(name, KeywordSet::V0_2).is_some()
        || crate::keywords::is_contextual(name)
}

/// `base`, or the first of `base2`, `base3`, ... that is not [`taken`].
fn fresh(text: &str, base: &str) -> String {
    if !taken(text, base) {
        return base.to_owned();
    }
    (2..10_000)
        .map(|n| format!("{base}{n}"))
        .find(|name| !taken(text, name))
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

/// The renames of names that start with `_` (U0001, ADR 0096 rule 3): `_x`, a local, a
/// parameter or a pattern's binding, becomes `x`, or the first fresh name of rule 3a, at every
/// occurrence in its declaration; `Variant(_)` loses its binding, and an arm `_ ->` becomes
/// `else ->`. Every occurrence of one name gets the same edit, which `una fix` applies once.
pub(crate) fn underscore_renames(
    text: &str,
    file: FileId,
    keywords: KeywordSet,
    errors: &mut [SyntaxError],
) {
    let Some(lines) = lines_of(text) else {
        return;
    };
    let offset = |i: usize| lines.get(i).map_or(text.len(), |l| l.start);
    for error in errors.iter_mut() {
        if error.code != Code::U0001 || !error.message.contains("`_`") || !error.repairs.is_empty()
        {
            continue;
        }
        let at_offset = error.span.start;
        let name: String = text
            .get(at_offset..)
            .map(|rest| rest.chars().take_while(|c| is_word_char(*c)).collect())
            .unwrap_or_default();
        let base = name.trim_start_matches('_');
        let Some(at) = lines.iter().rposition(|l| l.start <= at_offset) else {
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
        let Some(decl) = (0..=at)
            .rev()
            .find(|&i| lines.get(i).is_some_and(|line| starts_decl(line, keywords)))
        else {
            continue;
        };
        let decl_end = (at + 1..lines.len())
            .find(|&i| lines.get(i).is_some_and(|line| starts_decl(line, keywords)))
            .unwrap_or(lines.len());
        let header_end = lines.get(decl).map_or(0, |l| l.start + l.text.len());
        let (start, end) = (offset(decl), offset(decl_end).min(text.len()));
        let Some(region) = text.get(start..end) else {
            continue;
        };
        let plain = in_plain_text(region);
        let mut occurrences = Vec::new();
        let mut exact = true;
        for (i, _) in region.match_indices(name.as_str()) {
            let before = region.get(..i).and_then(|b| b.chars().next_back());
            let after_text = region.get(i + name.len()..).unwrap_or("");
            if before.is_some_and(is_word_char) || after_text.starts_with(is_word_char) {
                continue;
            }
            let absolute = start + i;
            let line_text = lines
                .get(lines.iter().rposition(|l| l.start <= absolute).unwrap_or(0))
                .map_or("", |l| l.text);
            let labeled = after_text.trim_start().starts_with(':')
                && !after_text.trim_start().starts_with("::");
            let bound_by_let = region
                .get(..i)
                .is_some_and(|b| b.trim_end().ends_with("let"));
            if plain.get(i).copied().unwrap_or(false)
                || before == Some('.')
                || (absolute == start && !line_text.starts_with(' '))
                || (labeled && absolute >= header_end && !bound_by_let)
            {
                exact = false;
                break;
            }
            occurrences.push(i);
        }
        let (Some(&first), Some(&last)) = (occurrences.first(), occurrences.last()) else {
            continue;
        };
        if !exact {
            continue;
        }
        let new = fresh(text, base);
        let mut replacement = String::new();
        let mut cursor = first;
        for &i in &occurrences {
            replacement.push_str(region.get(cursor..i).unwrap_or(""));
            replacement.push_str(&new);
            cursor = i + name.len();
        }
        error.repairs.push(SyntaxRepair {
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
        });
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
