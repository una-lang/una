//! The canonical text of a file: lines, indentation, spacing, blank lines and comments
//! (ADR 0020, rules 1 to 3, 7, 8 and 9), and, in edition 0.4, the soft wrapping of long lines
//! (ADR 0125 and its amendment of 2026-10-25).

use una_syntax::ast::File;
use una_syntax::{Comment, Interner, KeywordSet, Lexed, SourceFile, Token, TokenKind};

use crate::lines::{Line, SourceLines, logical_lines};
use crate::rewrite::{Constructors, Rewrites, collect};
use crate::spacing::Spacer;

/// The canonical form of a file that lexed and parsed without errors (ADR 0020), on its own:
/// constructors of other modules keep their order.
///
/// The file's edition is the keyword set it was lexed with (`lexed.keywords`): only edition 0.4
/// wraps long lines (ADR 0125, amendment of 2026-10-25).
///
/// Formatting the result again gives the same text.
pub fn format(source: &SourceFile, lexed: &Lexed, file: &File, interner: &Interner) -> String {
    format_with(source, lexed, file, interner, &Constructors::default())
}

/// The canonical form of a file of a package whose modules declare `constructors`, so that the
/// arguments of an imported constructor follow the declaration of its fields too (ADR 0020
/// rule 6).
///
/// The file's edition is the keyword set it was lexed with (`lexed.keywords`): only edition 0.4
/// wraps long lines (ADR 0125, amendment of 2026-10-25).
///
/// Formatting the result again, with the same constructors, gives the same text.
pub fn format_with(
    source: &SourceFile,
    lexed: &Lexed,
    file: &File,
    interner: &Interner,
    constructors: &Constructors,
) -> String {
    let lines = logical_lines(&lexed.tokens);
    let rewrites = collect(
        file,
        &lexed.tokens,
        &lexed.comments,
        &lines,
        interner,
        constructors,
    );
    let layout = Layout {
        text: source.text(),
        source: SourceLines::new(source.text(), source.line_starts()),
        tokens: &lexed.tokens,
        code_tokens: lexed
            .tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| !is_layout(&token.kind))
            .map(|(index, _)| index)
            .collect(),
        comments: &lexed.comments,
        lines: &lines,
        rewrites: &rewrites,
        soft_wrap: lexed.keywords >= KeywordSet::V0_4,
    };
    layout.render(file, interner)
}

/// One line of code in the output: a logical line, or several merged (ADR 0020 rule 4).
#[derive(Debug)]
struct Output {
    first_line: usize,
    last_line: usize,
    depth: usize,
    /// Where its first token starts and its last token ends.
    start: usize,
    end: usize,
    /// Whether it is the `=` line of a long-form function.
    equals: bool,
    /// The comments inside it, which keep its line breaks (ADR 0020 rule 2).
    inside: Vec<usize>,
    /// The comment at its end.
    trailing: Option<usize>,
}

/// A top-level item: the module header, an import, a declaration, or the comments after the
/// last declaration.
#[derive(Debug, PartialEq, Eq)]
enum ItemKind {
    Header,
    Import(String),
    Decl,
    End,
}

/// The order of an element inside its item: comments on their own lines by offset, the lines
/// of an output by its start and their rank.
type Key = (usize, usize, usize);

#[derive(Debug)]
struct Item {
    kind: ItemKind,
    start: usize,
    elements: Vec<(Key, Element)>,
}

/// A printed line.
#[derive(Debug)]
struct Element {
    depth: usize,
    /// Blank source lines right before it.
    blank: usize,
    equals: bool,
    text: String,
}

/// The soft target of a printed line, in Unicode scalar values, its indentation included
/// (ADR 0125 rule 1).
const WIDTH: usize = 100;

/// A token of a logical line that may wrap, as it is printed.
struct Piece<'t> {
    text: &'t str,
    /// Whether a space separates it from the token before it on the same line.
    space: bool,
    /// Its width in Unicode scalar values.
    width: usize,
    role: Role,
}

/// What a token is to wrapping. A group is a pair of `(` and `)`, or `[` and `]`, outside text
/// interpolation, numbered from 0 in the order of its opener; its level is the number of groups
/// of the logical line that hold it, itself included, so an outermost group has level 1.
#[derive(Clone, Copy)]
enum Role {
    Plain,
    /// The closer of `group`.
    Close {
        group: usize,
        level: usize,
    },
    /// A comma between the arguments or elements of `group`: a separator.
    Separator {
        group: usize,
        level: usize,
    },
}

struct Layout<'a> {
    text: &'a str,
    source: SourceLines<'a>,
    tokens: &'a [Token],
    /// The indices of the tokens that are not layout, in order.
    code_tokens: Vec<usize>,
    comments: &'a [Comment],
    lines: &'a [Line],
    rewrites: &'a Rewrites,
    /// Whether long lines wrap at their separators, the canonical form of edition 0.4 only
    /// (ADR 0125, amendment of 2026-10-25). Editions 0.1 to 0.3 keep each logical line without
    /// comments on one line, whatever its width.
    soft_wrap: bool,
}

fn is_layout(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
    )
}

impl Layout<'_> {
    fn token_text(&self, index: usize) -> &str {
        self.tokens
            .get(index)
            .and_then(|token| self.text.get(token.span.start..token.span.end))
            .unwrap_or("")
    }

    fn comment_text(&self, index: usize) -> &str {
        self.comments
            .get(index)
            .and_then(|comment| self.text.get(comment.span.start..comment.span.end))
            .unwrap_or("")
            .trim_end()
    }

    fn comment_start(&self, index: usize) -> usize {
        self.comments
            .get(index)
            .map_or(0, |comment| comment.span.start)
    }

    fn start_of(&self, token: usize) -> usize {
        self.tokens.get(token).map_or(0, |token| token.span.start)
    }

    fn end_of(&self, token: usize) -> usize {
        self.tokens.get(token).map_or(0, |token| token.span.end)
    }

    /// The lines of code, with the merges of the rewrites applied.
    fn outputs(&self) -> Vec<Output> {
        let mut outputs = Vec::new();
        let mut index = 0;
        while let Some(first) = self.lines.get(index) {
            let last_line = self.rewrites.merges.get(&index).copied().unwrap_or(index);
            let last = self.lines.get(last_line).unwrap_or(first);
            outputs.push(Output {
                first_line: index,
                last_line,
                depth: first.depth,
                start: self.start_of(first.first),
                end: self.end_of(last.last),
                equals: last_line == index
                    && first.first == first.last
                    && matches!(
                        self.tokens.get(first.first).map(|token| &token.kind),
                        Some(TokenKind::Eq)
                    ),
                inside: Vec::new(),
                trailing: None,
            });
            index = last_line + 1;
        }
        outputs
    }

    /// The last token that is not layout and ends at or before `offset`.
    fn token_before(&self, offset: usize) -> Option<usize> {
        let position = self
            .code_tokens
            .partition_point(|&index| self.end_of(index) <= offset);
        self.code_tokens.get(position.checked_sub(1)?).copied()
    }

    fn render(&self, file: &File, interner: &Interner) -> String {
        let mut outputs = self.outputs();
        // Comments on their own lines, between outputs: (comment, output before, output after).
        let mut own_lines = Vec::new();
        for (index, comment) in self.comments.iter().enumerate() {
            let offset = comment.span.start;
            let after = outputs.partition_point(|output| output.start < offset);
            let previous = after.checked_sub(1);
            if let Some(output) = previous.and_then(|previous| outputs.get_mut(previous)) {
                if offset < output.end {
                    output.inside.push(index);
                    continue;
                }
                let same_line = self.token_before(offset).is_some_and(|token| {
                    self.source.line(self.start_of(token)) == self.source.line(offset)
                });
                if same_line {
                    output.trailing = Some(index);
                    continue;
                }
            }
            own_lines.push((index, previous, (after < outputs.len()).then_some(after)));
        }
        let mut items = self.items(file, interner);
        let item_of = |items: &[Item], offset: usize| {
            items
                .partition_point(|item| item.start <= offset)
                .saturating_sub(1)
        };
        for output in &outputs {
            let item = item_of(&items, output.start);
            let elements = self.code_elements(output);
            if let Some(item) = items.get_mut(item) {
                item.elements.extend(elements);
            }
        }
        let mut end = Item {
            kind: ItemKind::End,
            start: usize::MAX,
            elements: Vec::new(),
        };
        for (comment, previous, next) in own_lines {
            let offset = self.comment_start(comment);
            let depth_of = |output: Option<usize>| {
                output
                    .and_then(|output| outputs.get(output))
                    .map_or(0, |output| output.depth)
            };
            let (low, high) = {
                let (before, after) = (depth_of(previous), depth_of(next));
                (before.min(after), before.max(after))
            };
            // The level nearest to the comment's column, ties deeper.
            let depth = self.source.column(offset).div_ceil(2).clamp(low, high);
            let element = Element {
                depth,
                blank: self.source.blank_before(self.source.line(offset)),
                equals: false,
                text: self.comment_text(comment).to_owned(),
            };
            let start_of = |output: Option<usize>| {
                output
                    .and_then(|output| outputs.get(output))
                    .map(|output| output.start)
            };
            let target = match (start_of(previous), start_of(next)) {
                (None, _) => Some(0),
                (Some(before), None) => (depth > 0).then(|| item_of(&items, before)),
                (Some(before), Some(after)) => {
                    let (before, after) = (item_of(&items, before), item_of(&items, after));
                    Some(if before == after || depth > 0 {
                        before
                    } else {
                        after
                    })
                }
            };
            match target.and_then(|item| items.get_mut(item)) {
                Some(item) => item.elements.push(((offset, 0, 0), element)),
                None => end.elements.push(((offset, 0, 0), element)),
            }
        }
        for item in &mut items {
            item.elements.sort_by_key(|(key, _)| *key);
        }
        // Imports sorted by module path, stably.
        let imports: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item.kind, ItemKind::Import(_)))
            .map(|(index, _)| index)
            .collect();
        if let (Some(&first), Some(&last)) = (imports.first(), imports.last())
            && let Some(slice) = items.get_mut(first..=last)
        {
            slice.sort_by(|left, right| match (&left.kind, &right.kind) {
                (ItemKind::Import(left), ItemKind::Import(right)) => left.cmp(right),
                _ => std::cmp::Ordering::Equal,
            });
        }
        if !end.elements.is_empty() {
            end.elements.sort_by_key(|(key, _)| *key);
            items.push(end);
        }
        Self::print(&items)
    }

    /// The top-level items of `file`, in source order.
    fn items(&self, file: &File, interner: &Interner) -> Vec<Item> {
        let mut items = Vec::new();
        if let Some(header) = &file.module {
            items.push(Item {
                kind: ItemKind::Header,
                start: header.span.start,
                elements: Vec::new(),
            });
        }
        for import in &file.imports {
            let path: Vec<&str> = import
                .module
                .segments
                .iter()
                .map(|segment| interner.resolve(segment.name))
                .collect();
            items.push(Item {
                kind: ItemKind::Import(path.join(".")),
                start: import.span.start,
                elements: Vec::new(),
            });
        }
        for decl in &file.decls {
            items.push(Item {
                kind: ItemKind::Decl,
                start: decl.span.start,
                elements: Vec::new(),
            });
        }
        items
    }

    /// The printed lines of an output: one line, or, when comments sit inside it, one line per
    /// source line, continuation lines one level deeper (ADR 0020 rules 2, 3 and 9). In edition
    /// 0.4 a comment at its end keeps its source line breaks too, and a line without comments
    /// wraps at its separators (ADR 0125 rules 1 to 4, amendment of 2026-10-25).
    fn code_elements(&self, output: &Output) -> Vec<(Key, Element)> {
        let mut sequence = Vec::new();
        for line in self
            .lines
            .get(output.first_line..=output.last_line)
            .unwrap_or_default()
        {
            self.emit(line.first, line.last, &mut sequence);
        }
        // A comment inside the line keeps its source line breaks (ADR 0020 rule 2). In edition
        // 0.4 so does a comment at its end, so that a comment after a wrapped call never joins
        // the call back into one line (ADR 0125 rule 4).
        let keeps_breaks =
            !output.inside.is_empty() || (self.soft_wrap && output.trailing.is_some());
        // Each physical line: its source line and its text.
        let mut physical: Vec<(usize, String)> = Vec::new();
        let mut spacer: Option<Spacer> = None;
        let mut previous: Option<&TokenKind> = None;
        for &index in &sequence {
            let Some(token) = self.tokens.get(index) else {
                continue;
            };
            let line = self.source.line(token.span.start);
            let space = match (previous, spacer.as_mut()) {
                (Some(previous), Some(spacer)) => spacer.space(previous, &token.kind),
                _ => {
                    spacer = Some(Spacer::new(&token.kind));
                    false
                }
            };
            let breaks = keeps_breaks && physical.last().is_some_and(|(last, _)| *last != line);
            match physical.last_mut() {
                Some((_, text)) if !breaks => {
                    if space {
                        text.push(' ');
                    }
                    text.push_str(self.token_text(index));
                }
                _ => physical.push((line, self.token_text(index).to_owned())),
            }
            previous = Some(&token.kind);
        }
        for &comment in &output.inside {
            let line = self.source.line(self.comment_start(comment));
            match physical.iter_mut().find(|(source, _)| *source == line) {
                Some((_, text)) => {
                    text.push(' ');
                    text.push_str(self.comment_text(comment));
                }
                None => physical.push((line, self.comment_text(comment).to_owned())),
            }
        }
        physical.sort_by_key(|(line, _)| *line);
        if let (Some(comment), Some((_, text))) = (output.trailing, physical.last_mut()) {
            text.push(' ');
            text.push_str(self.comment_text(comment));
        }
        let blank = self.source.blank_before(self.source.line(output.start));
        // Edition 0.4: a line without comments wraps at its separators (ADR 0125). Without a
        // comment, `physical` is one line.
        let wrapped = if self.soft_wrap && !keeps_breaks {
            self.wrap(output.depth, &sequence)
        } else {
            None
        };
        let printed: Vec<(String, usize)> = match wrapped {
            Some(lines) => lines,
            None => physical
                .into_iter()
                .enumerate()
                .map(|(rank, (_, text))| (text, output.depth + usize::from(rank > 0)))
                .collect(),
        };
        printed
            .into_iter()
            .enumerate()
            .map(|(rank, (text, depth))| {
                let element = Element {
                    depth,
                    blank: if rank == 0 { blank } else { 0 },
                    equals: output.equals,
                    text,
                };
                ((output.start, 1, rank), element)
            })
            .collect()
    }

    /// The printed lines of the logical line `sequence`, which holds no comment, at `depth`,
    /// wrapped at its separators (ADR 0125, amendment of 2026-10-25), or `None` when it fits
    /// within [`WIDTH`] or has no separator. Each token is read once, so building the pieces is
    /// linear in the line's length; [`break_line`] is too.
    fn wrap(&self, depth: usize, sequence: &[usize]) -> Option<Vec<(String, usize)>> {
        let mut pieces = Vec::with_capacity(sequence.len());
        let mut spacer: Option<Spacer> = None;
        let mut previous: Option<&TokenKind> = None;
        // The delimiter groups open at this token, outermost first, by id.
        let mut open: Vec<usize> = Vec::new();
        let mut groups = 0usize;
        let mut interpolation = 0usize;
        for &index in sequence {
            // INVARIANT: `emit` gives indices of `self.tokens` only; a missing token would print
            // nothing in `code_elements` either.
            let Some(token) = self.tokens.get(index) else {
                continue;
            };
            let space = match (previous, spacer.as_mut()) {
                (Some(previous), Some(spacer)) => spacer.space(previous, &token.kind),
                _ => {
                    spacer = Some(Spacer::new(&token.kind));
                    false
                }
            };
            previous = Some(&token.kind);
            let role = match token.kind {
                TokenKind::InterpStart => {
                    interpolation += 1;
                    Role::Plain
                }
                TokenKind::InterpEnd => {
                    interpolation = interpolation.saturating_sub(1);
                    Role::Plain
                }
                // A text literal holds no line break, so nothing inside an interpolation wraps,
                // and its brackets balance on their own.
                _ if interpolation > 0 => Role::Plain,
                TokenKind::LParen | TokenKind::LBracket => {
                    open.push(groups);
                    groups += 1;
                    Role::Plain
                }
                TokenKind::RParen | TokenKind::RBracket => match open.pop() {
                    Some(group) => Role::Close {
                        group,
                        level: open.len() + 1,
                    },
                    None => Role::Plain,
                },
                TokenKind::Comma => match open.last() {
                    Some(&group) => Role::Separator {
                        group,
                        level: open.len(),
                    },
                    None => Role::Plain,
                },
                _ => Role::Plain,
            };
            let text = self.token_text(index);
            pieces.push(Piece {
                text,
                space,
                width: text.chars().count(),
                role,
            });
        }
        break_line(&pieces, groups, depth, &mut 0)
    }

    /// Appends the tokens `first..=last` in printing order: dropped tokens left out and
    /// regions in canonical order.
    fn emit(&self, first: usize, last: usize, sequence: &mut Vec<usize>) {
        self.emit_part(first, last, None, sequence);
    }

    /// Like [`Self::emit`], inside the region that starts at `expanding`, whose first part
    /// starts at the same token and must not expand it again.
    fn emit_part(
        &self,
        first: usize,
        last: usize,
        expanding: Option<usize>,
        sequence: &mut Vec<usize>,
    ) {
        let mut index = first;
        while index <= last {
            if self.rewrites.dropped.contains(&index) {
                index += 1;
                continue;
            }
            if let Some(region) = self
                .rewrites
                .regions
                .get(&index)
                .filter(|_| expanding != Some(index))
            {
                for (position, &(low, high)) in region.parts.iter().enumerate() {
                    if let Some(&comma) = position
                        .checked_sub(1)
                        .and_then(|separator| region.separators.get(separator))
                    {
                        sequence.push(comma);
                    }
                    self.emit_part(low, high, Some(index), sequence);
                }
                index = region.last + 1;
                continue;
            }
            sequence.push(index);
            index += 1;
        }
    }

    /// Prints the items with the blank lines of ADR 0020 rule 8.
    fn print(items: &[Item]) -> String {
        let mut out = String::new();
        let mut previous: Option<&ItemKind> = None;
        for item in items {
            let mut before: Option<&Element> = None;
            for (_, element) in &item.elements {
                let blank = match (before, previous) {
                    (None, None) => 0,
                    (None, Some(ItemKind::Import(_)))
                        if matches!(item.kind, ItemKind::Import(_)) =>
                    {
                        0
                    }
                    (None, Some(_)) if item.kind == ItemKind::End => element.blank.min(1),
                    (None, Some(_)) => 1,
                    (Some(before), _) => {
                        if before.equals || element.equals || element.depth > before.depth {
                            0
                        } else {
                            element.blank.min(1)
                        }
                    }
                };
                for _ in 0..blank {
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(element.depth));
                out.push_str(&element.text);
                out.push('\n');
                before = Some(element);
            }
            if !item.elements.is_empty() {
                previous = Some(&item.kind);
            }
        }
        out
    }
}

/// Breaks the logical line `pieces`, at indentation level `depth`, whose groups are numbered
/// `0..groups`, into printed lines with their indentation levels (ADR 0125, amendment of
/// 2026-10-25), or gives `None` when it fits within [`WIDTH`] or has no separator. `work`
/// counts the pieces examined.
///
/// A printed line starts at the start of the logical line, after a chosen separator, or at the
/// closer of a wrapped group, and runs to the closer of the next wrapped group or to the end of
/// the logical line: its measured text is all of that, closers and the tail after the last
/// separator included, plus two scalars per indentation level. When that is wider than
/// [`WIDTH`], the line ends instead at a separator: the outermost one (the lowest level) whose
/// line up to and including it fits, the latest such one on a tie, and, when none fits, the
/// first one. Its group is then wrapped. A line after a separator of level `l` is indented
/// `depth + l`; the closer of a wrapped group of level `l` starts a line indented
/// `depth + l - 1`, that of its opener's line when no outer group wraps.
///
/// Every token of a file without errors is at least one scalar wide, so the scan of a line
/// stops at most [`WIDTH`] + 1 pieces after the start of the line or at the separator it
/// chooses, and the next line reads again only the pieces past that separator: at most
/// `n + (WIDTH + 2) * lines` pieces for `n` pieces, linear in the length of the line, with
/// [`WIDTH`] a constant.
fn break_line(
    pieces: &[Piece<'_>],
    groups: usize,
    depth: usize,
    work: &mut usize,
) -> Option<Vec<(String, usize)>> {
    // `before[i]`: the width of the pieces before piece `i`, each with its space.
    let mut before = Vec::with_capacity(pieces.len() + 1);
    let mut total = 0usize;
    before.push(total);
    for piece in pieces {
        total += piece.width + usize::from(piece.space);
        before.push(total);
    }
    // The width of the pieces `first..=last` printed on one line, without the space before
    // `first`.
    let span = |first: usize, last: usize| -> usize {
        let start = before.get(first).copied().unwrap_or(total);
        let end = before.get(last + 1).copied().unwrap_or(total);
        let space = pieces.get(first).is_some_and(|piece| piece.space);
        end.saturating_sub(start).saturating_sub(usize::from(space))
    };
    let last = pieces.len().checked_sub(1)?;
    let separated = pieces
        .iter()
        .any(|piece| matches!(piece.role, Role::Separator { .. }));
    if !separated || 2 * depth + span(0, last) <= WIDTH {
        return None;
    }
    // Whether each group is wrapped: one of its separators ended a line.
    let mut wrapped = vec![false; groups];
    let mut lines = Vec::new();
    let mut start = 0;
    let mut indent = depth;
    while let Some(first) = pieces.get(start) {
        if let Role::Close { level, .. } = first.role
            && closes_wrapped(first.role, &wrapped)
        {
            indent = depth + level - 1;
        }
        // Where the line ends unless it breaks: the closer of the next wrapped group, or the
        // end of the logical line.
        let mut end = pieces.len();
        // Separators as (piece, group, level): the outermost latest one that fits, and the
        // first one.
        let mut best: Option<(usize, usize, usize)> = None;
        let mut first_separator: Option<(usize, usize, usize)> = None;
        let mut over = false;
        let mut index = start;
        while let Some(piece) = pieces.get(index) {
            *work += 1;
            if index > start && closes_wrapped(piece.role, &wrapped) {
                end = index;
                break;
            }
            // Widths only grow along the line: once past the target, no later separator fits.
            if 2 * indent + span(start, index) > WIDTH {
                over = true;
                if first_separator.is_some() {
                    break;
                }
            }
            if let Role::Separator { group, level } = piece.role {
                let candidate = (index, group, level);
                if first_separator.is_none() {
                    first_separator = Some(candidate);
                }
                if over {
                    break;
                }
                if best.is_none_or(|(_, _, outer)| level <= outer) {
                    best = Some(candidate);
                }
            }
            index += 1;
        }
        let chosen = if over { best.or(first_separator) } else { None };
        match chosen {
            Some((separator, group, level)) => {
                if let Some(flag) = wrapped.get_mut(group) {
                    *flag = true;
                }
                lines.push((render(pieces, start, separator + 1), indent));
                start = separator + 1;
                indent = depth + level;
            }
            None => {
                lines.push((render(pieces, start, end), indent));
                start = end;
            }
        }
    }
    Some(lines)
}

/// Whether `role` is the closer of a wrapped group.
fn closes_wrapped(role: Role, wrapped: &[bool]) -> bool {
    matches!(role, Role::Close { group, .. } if wrapped.get(group).copied().unwrap_or(false))
}

/// The pieces `first..end` printed on one line.
fn render(pieces: &[Piece<'_>], first: usize, end: usize) -> String {
    let mut line = String::new();
    for (index, piece) in pieces.iter().enumerate().take(end).skip(first) {
        if index > first && piece.space {
            line.push(' ');
        }
        line.push_str(piece.text);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::{Piece, Role, WIDTH, break_line};

    /// `f(x0, x1, ...)` with `n` arguments: its pieces and its number of groups.
    fn long_list(n: usize) -> (Vec<Piece<'static>>, usize) {
        let mut pieces = vec![
            Piece {
                text: "f",
                space: false,
                width: 1,
                role: Role::Plain,
            },
            Piece {
                text: "(",
                space: false,
                width: 1,
                role: Role::Plain,
            },
        ];
        for index in 0..n {
            pieces.push(Piece {
                text: "item",
                space: index > 0,
                width: 4,
                role: Role::Plain,
            });
            if index + 1 < n {
                pieces.push(Piece {
                    text: ",",
                    space: false,
                    width: 1,
                    role: Role::Separator { group: 0, level: 1 },
                });
            }
        }
        pieces.push(Piece {
            text: ")",
            space: false,
            width: 1,
            role: Role::Close { group: 0, level: 1 },
        });
        (pieces, 1)
    }

    /// `f(a, f(a, ... f(a, b) ...))` nested `n` deep: its pieces and its number of groups.
    fn deep_nesting(n: usize) -> (Vec<Piece<'static>>, usize) {
        let mut pieces = Vec::new();
        for level in 1..=n {
            pieces.push(Piece {
                text: "f",
                space: level > 1,
                width: 1,
                role: Role::Plain,
            });
            pieces.push(Piece {
                text: "(",
                space: false,
                width: 1,
                role: Role::Plain,
            });
            pieces.push(Piece {
                text: "a",
                space: false,
                width: 1,
                role: Role::Plain,
            });
            pieces.push(Piece {
                text: ",",
                space: false,
                width: 1,
                role: Role::Separator {
                    group: level - 1,
                    level,
                },
            });
        }
        pieces.push(Piece {
            text: "b",
            space: true,
            width: 1,
            role: Role::Plain,
        });
        for level in (1..=n).rev() {
            pieces.push(Piece {
                text: ")",
                space: false,
                width: 1,
                role: Role::Close {
                    group: level - 1,
                    level,
                },
            });
        }
        (pieces, n)
    }

    /// The pieces examined to break `pieces`, and the printed lines.
    fn counted(pieces: &[Piece<'_>], groups: usize) -> (usize, Vec<(String, usize)>) {
        let mut work = 0;
        let lines = break_line(pieces, groups, 0, &mut work).unwrap();
        (work, lines)
    }

    #[test]
    fn breaking_a_line_examines_a_linear_number_of_pieces() {
        // ADR 0125 rule 6, amendment of 2026-10-25: counted work, not wall-clock time. Each
        // shape at 16 times the size takes at most 16 times the work, and never more than the
        // bound `n + (WIDTH + 2) * lines` that the scan guarantees.
        for shape in [long_list, deep_nesting] {
            let mut previous: Option<(usize, usize)> = None;
            for size in [256, 4_096, 65_536] {
                let (pieces, groups) = shape(size);
                let (work, lines) = counted(&pieces, groups);
                assert!(
                    work <= pieces.len() + (WIDTH + 2) * lines.len(),
                    "{work} pieces examined for {} pieces in {} lines",
                    pieces.len(),
                    lines.len()
                );
                let printed: usize = lines.iter().map(|(text, _)| text.len()).sum();
                assert!(printed <= pieces.len() * 5, "{printed}");
                if let Some((small, small_pieces)) = previous {
                    assert!(
                        work * small_pieces <= small * pieces.len() * 2,
                        "work grew from {small} to {work}"
                    );
                }
                previous = Some((work, pieces.len()));
            }
        }
    }

    #[test]
    fn a_long_list_fills_its_lines_and_closes_at_its_opener() {
        let (pieces, groups) = long_list(100);
        let (_, lines) = counted(&pieces, groups);
        assert!(
            lines
                .iter()
                .all(|(text, depth)| 2 * depth + text.chars().count() <= WIDTH)
        );
        assert_eq!(lines.first().map(|(_, depth)| *depth), Some(0));
        assert_eq!(lines.last(), Some(&(")".to_owned(), 0)));
        assert!(
            lines
                .iter()
                .skip(1)
                .rev()
                .skip(1)
                .all(|(_, depth)| *depth == 1)
        );
    }

    #[test]
    fn deep_nesting_breaks_the_outermost_group_first() {
        // Each line ends at its outermost separator that fits, so the next line holds the next
        // level, one level deeper, until the rest fits; then every wrapped group closes on a
        // line of its own, at its opener's level.
        let (pieces, groups) = deep_nesting(30);
        let (_, lines) = counted(&pieces, groups);
        let innermost = format!("{}b{}", "f(a, ".repeat(9), ")".repeat(9));
        let expected: Vec<(String, usize)> = (0..21)
            .map(|level| ("f(a,".to_owned(), level))
            .chain(std::iter::once((innermost, 21)))
            .chain((0..21).rev().map(|level| (")".to_owned(), level)))
            .collect();
        assert_eq!(lines, expected);
    }
}
