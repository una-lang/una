//! The canonical text of a file: lines, indentation, spacing, blank lines and comments
//! (ADR 0020, rules 1 to 3, 7, 8 and 9).

use una_syntax::ast::File;
use una_syntax::{Comment, Interner, Lexed, SourceFile, Token, TokenKind};

use crate::lines::{Line, SourceLines, logical_lines};
use crate::rewrite::{Constructors, Rewrites, collect};
use crate::spacing::Spacer;

/// The canonical form of a file that lexed and parsed without errors (ADR 0020), on its own:
/// constructors of other modules keep their order.
///
/// Formatting the result again gives the same text.
pub fn format(source: &SourceFile, lexed: &Lexed, file: &File, interner: &Interner) -> String {
    format_with(source, lexed, file, interner, &Constructors::default())
}

/// The canonical form of a file of a package whose modules declare `constructors`, so that the
/// arguments of an imported constructor follow the declaration of its fields too (ADR 0020
/// rule 6).
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

/// A comma-separated piece of a logical line, with the delimiter group at its end and any
/// closing delimiters inside it.
struct WrapSegment {
    text: String,
    depth: usize,
    separator_group: Option<usize>,
    /// `(start, end, group id, indentation of the opener)` byte offsets in `text`.
    closers: Vec<(usize, usize, usize, usize)>,
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
    /// source line, continuation lines one level deeper (ADR 0020 rules 2, 3 and 9).
    fn code_elements(&self, output: &Output) -> Vec<(Key, Element)> {
        let mut sequence = Vec::new();
        for line in self
            .lines
            .get(output.first_line..=output.last_line)
            .unwrap_or_default()
        {
            self.emit(line.first, line.last, &mut sequence);
        }
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
            let breaks =
                !output.inside.is_empty() && physical.last().is_some_and(|(last, _)| *last != line);
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
        let wrapped = match (
            output.inside.is_empty(),
            output.trailing,
            physical.as_slice(),
        ) {
            (true, None, [(_, text)]) => self
                .wrap(output, &sequence)
                .unwrap_or_else(|| vec![(text.clone(), output.depth)]),
            _ => Vec::new(),
        };
        let physical = if wrapped.is_empty() {
            physical
                .into_iter()
                .enumerate()
                .map(|(rank, (_, text))| (text, output.depth + usize::from(rank > 0)))
                .collect()
        } else {
            wrapped
        };
        physical
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

    /// Wraps a comment-free logical line at commas inside parenthesized and bracketed lists.
    /// Segments are measured once, so packing a line is linear in its source length.
    fn wrap(&self, output: &Output, sequence: &[usize]) -> Option<Vec<(String, usize)>> {
        const WIDTH: usize = 100;

        let mut segments = Vec::new();
        let mut current = String::new();
        let mut closers = Vec::new();
        let mut spacer: Option<Spacer> = None;
        let mut previous: Option<&TokenKind> = None;
        let mut delimiters = Vec::new();
        let mut interpolation_depth = 0usize;
        let mut segment_depth = output.depth;
        let mut next_group = 0usize;

        for &index in sequence {
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
            if space {
                current.push(' ');
            }
            let token_start = current.len();
            current.push_str(self.token_text(index));
            let token_end = current.len();

            match token.kind {
                TokenKind::InterpStart => interpolation_depth += 1,
                TokenKind::InterpEnd => interpolation_depth = interpolation_depth.saturating_sub(1),
                TokenKind::LParen | TokenKind::LBracket => {
                    delimiters.push((next_group, output.depth + delimiters.len()));
                    next_group += 1;
                }
                TokenKind::RParen | TokenKind::RBracket => {
                    if let Some((group, opener_depth)) = delimiters.pop() {
                        closers.push((token_start, token_end, group, opener_depth));
                    }
                }
                TokenKind::Comma if !delimiters.is_empty() && interpolation_depth == 0 => {
                    let separator_group = delimiters.last().map(|(group, _)| *group);
                    segments.push(WrapSegment {
                        text: std::mem::take(&mut current),
                        depth: segment_depth,
                        separator_group,
                        closers: std::mem::take(&mut closers),
                    });
                    segment_depth = output.depth + delimiters.len();
                }
                _ => {}
            }
            previous = Some(&token.kind);
        }
        if !current.is_empty() || segments.is_empty() {
            segments.push(WrapSegment {
                text: current,
                depth: segment_depth,
                separator_group: None,
                closers,
            });
        }
        if segments.len() < 2 {
            return None;
        }

        // Greedily pack the latest comma-separated pieces that fit. Every piece is measured
        // once, so this pass is linear in the rendered source length.
        let mut line_ranges = Vec::new();
        let mut range_start = 0;
        let first = segments.first()?;
        let mut line_width = output.depth * 2 + first.text.chars().count();
        let mut previous_separator_group = first.separator_group;
        let mut wrapped_groups = std::collections::HashSet::new();
        for (index, segment) in segments.iter().enumerate().skip(1) {
            let segment_width = segment.text.chars().count();
            if line_width + segment_width > WIDTH {
                line_ranges.push((range_start, index));
                if let Some(group) = previous_separator_group {
                    wrapped_groups.insert(group);
                }
                range_start = index;
                line_width = segment.depth * 2 + segment_width.saturating_sub(1);
            } else {
                line_width += segment_width;
            }
            previous_separator_group = segment.separator_group;
        }
        line_ranges.push((range_start, segments.len()));
        if wrapped_groups.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        for (start, end) in line_ranges {
            let Some(first) = segments.get(start) else {
                continue;
            };
            let mut depth = first.depth;
            let mut line = String::new();
            for (index, segment) in segments.iter().enumerate().take(end).skip(start) {
                let trim_space = index == start && segment.text.starts_with(' ');
                let offset_base = usize::from(trim_space);
                let content = segment.text.get(offset_base..).unwrap_or("");
                let mut copied = 0;
                for &(close_start, close_end, group, opener_depth) in &segment.closers {
                    if !wrapped_groups.contains(&group) || close_start < offset_base {
                        continue;
                    }
                    let close_start = close_start - offset_base;
                    let close_end = close_end - offset_base;
                    let Some(before) = content.get(copied..close_start) else {
                        continue;
                    };
                    line.push_str(before);
                    if !line.is_empty() {
                        lines.push((std::mem::take(&mut line), depth));
                    }
                    let Some(closer) = content.get(close_start..close_end) else {
                        continue;
                    };
                    line.push_str(closer);
                    depth = opener_depth;
                    copied = close_end;
                }
                if let Some(rest) = content.get(copied..) {
                    line.push_str(rest);
                }
            }
            if !line.is_empty() {
                lines.push((line, depth));
            }
        }
        Some(lines)
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
