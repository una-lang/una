//! Human rendering of diagnostics (`spec/13 §1`), drawn with ariadne.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;

use ariadne::{Config, Label, Report, ReportKind};

use crate::diagnostic::{Diagnostic, Location, Position, Ranked, Severity, in_output_order};

/// A span in ariadne's terms: a file and a range of character offsets.
type Span = (String, Range<usize>);

/// Renders diagnostics for people, in output order (`spec/13 §1`).
///
/// `sources` maps each file, as written in locations, to its text. A diagnostic whose files or
/// positions are not all available in `sources` is rendered as plain text, without excerpts.
pub fn render_human(
    diagnostics: &[Diagnostic],
    sources: &BTreeMap<String, String>,
    color: bool,
) -> String {
    let mut out = String::new();
    for diagnostic in in_output_order(diagnostics) {
        let rendered = render_with_excerpts(diagnostic, sources, color)
            .unwrap_or_else(|| render_plain(diagnostic));
        out.push_str(&rendered);
    }
    out
}

/// Renders a diagnostic with source excerpts, or returns `None` if a file or position is not
/// available. Every span is checked before ariadne sees it, because ariadne prints to stderr
/// when a source is missing and library crates never print.
fn render_with_excerpts(
    diagnostic: &Diagnostic,
    sources: &BTreeMap<String, String>,
    color: bool,
) -> Option<String> {
    let primary = span(&diagnostic.primary, sources)?;
    // ariadne underlines only labels that have a message, so the primary label repeats it.
    let mut labeled = vec![(primary.clone(), diagnostic.message.as_str())];
    for related in &diagnostic.related {
        labeled.push((span(&related.location, sources)?, related.message.as_str()));
    }
    // ariadne starts a new excerpt whenever a label is above the previous one in the same file,
    // so labels go in source order to keep one excerpt per file.
    labeled.sort_by(|(a, _), (b, _)| (&a.0, a.1.start, a.1.end).cmp(&(&b.0, b.1.start, b.1.end)));
    let mut files = BTreeMap::new();
    for ((file, _), _) in &labeled {
        files.insert(file.clone(), sources.get(file)?.clone());
    }
    let labels = labeled
        .into_iter()
        .map(|(label_span, message)| Label::new(label_span).with_message(message));
    let mut report = Report::build(report_kind(diagnostic.severity), primary)
        .with_config(Config::default().with_color(color))
        .with_code(diagnostic.code)
        .with_message(&diagnostic.message)
        .with_labels(labels);
    for help in helps(diagnostic) {
        report = report.with_help(help);
    }
    for note in notes(diagnostic) {
        report = report.with_note(note);
    }
    let mut bytes = Vec::new();
    report
        .finish()
        .write(ariadne::sources(files), &mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Renders a diagnostic without source excerpts, with the same header as ariadne.
fn render_plain(diagnostic: &Diagnostic) -> String {
    let mut out = format!(
        "[{}] {}: {}\n",
        diagnostic.code,
        kind_name(diagnostic.severity),
        diagnostic.message
    );
    // Writing to a String cannot fail, so the results of `writeln!` below are ignored.
    let _ = writeln!(out, "  at {}", describe(&diagnostic.primary));
    for related in &diagnostic.related {
        let _ = writeln!(
            out,
            "  related {}: {}",
            describe(&related.location),
            related.message
        );
    }
    for help in helps(diagnostic) {
        let _ = writeln!(out, "  help: {help}");
    }
    for note in notes(diagnostic) {
        let _ = writeln!(out, "  note: {note}");
    }
    out
}

/// One help line per repair, in output order.
fn helps(diagnostic: &Diagnostic) -> Vec<String> {
    diagnostic
        .repairs_in_order()
        .into_iter()
        .map(|Ranked { rank, repair }| {
            // ADR 0095 rule 5 and ADR 0097 rule 3: an escape or a widening says its kind and
            // what it costs.
            let mut help = match (repair.escape, repair.widens) {
                (Some(escape), _) => format!(
                    "repair {rank} (escape: {}; {}): {}",
                    escape.kind(),
                    escape.cost(),
                    repair.description
                ),
                (None, Some(widening)) => format!(
                    "repair {rank} (widens {}: {}): {}",
                    widening.kind(),
                    widening.cost(),
                    repair.description
                ),
                (None, None) => format!("repair {rank}: {}", repair.description),
            };
            if let Some(edit) = &repair.edit {
                let _ = write!(
                    help,
                    "; edit {}..{}:{} to {:?}",
                    describe(&edit.location),
                    edit.location.end.line,
                    edit.location.end.col,
                    edit.replacement
                );
            }
            help
        })
        .collect()
}

/// Notes for the obligation, the counterexample and the explain command.
fn notes(diagnostic: &Diagnostic) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(obligation) = &diagnostic.obligation {
        notes.push(format!(
            "obligation {} ({}) is {}: {}",
            obligation.id,
            obligation.kind.as_str(),
            obligation.status.as_str(),
            obligation.goal
        ));
    }
    if let Some(counterexample) = &diagnostic.counterexample {
        let bindings: Vec<String> = counterexample
            .bindings
            .iter()
            .map(|binding| match &binding.value {
                serde_json::Value::String(text) => format!("{} = {text}", binding.name),
                value => format!("{} = {value}", binding.name),
            })
            .collect();
        // #855: the solver's message names internal symbols and positions in the generated
        // query, so the human note is fixed; the message stays in the `--json` document.
        match &counterexample.unavailable {
            Some(_) => notes.push(
                "counterexample: not available (the solver gave no values for the model)"
                    .to_owned(),
            ),
            None => notes.push(format!("counterexample: {}", bindings.join(", "))),
        }
    }
    if let Some(limit) = &diagnostic.limit {
        let seen = limit.seen.map_or_else(
            || "a value that does not fit in 64 bits".to_owned(),
            |seen| seen.to_string(),
        );
        notes.push(format!(
            "limit {} is {}, and {seen} was seen",
            limit.name, limit.bound
        ));
    }
    notes.push(format!(
        "run `{}` for an explanation",
        diagnostic.explain_command()
    ));
    notes
}

fn report_kind(severity: Severity) -> ReportKind<'static> {
    match severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
        Severity::Note => ReportKind::Advice,
    }
}

/// The header word ariadne prints for each severity.
fn kind_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
        Severity::Note => "Advice",
    }
}

/// A location as `file:line:col`, naming its start.
fn describe(location: &Location) -> String {
    format!(
        "{}:{}:{}",
        location.file, location.start.line, location.start.col
    )
}

/// The ariadne span of a location, if its file is in `sources` and both ends exist in it.
fn span(location: &Location, sources: &BTreeMap<String, String>) -> Option<Span> {
    let text = sources.get(&location.file)?;
    let start = char_offset(text, location.start)?;
    let end = char_offset(text, location.end)?;
    (start <= end).then(|| (location.file.clone(), start..end))
}

/// The character offset of a position in `text`, if the position exists: an existing line, and a
/// column at most one past the last character of that line.
fn char_offset(text: &str, position: Position) -> Option<usize> {
    let line = usize::try_from(position.line).ok()?.checked_sub(1)?;
    let col = usize::try_from(position.col).ok()?.checked_sub(1)?;
    let mut offset = 0;
    for (index, content) in text.split('\n').enumerate() {
        let width = content.chars().count();
        if index == line {
            return (col <= width).then_some(offset + col);
        }
        offset += width + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, col: u32) -> Position {
        Position { line, col }
    }

    #[test]
    fn offsets_count_unicode_scalar_values() {
        let text = "ab\n\u{e9}t\u{e9}\n";
        assert_eq!(char_offset(text, at(1, 1)), Some(0));
        assert_eq!(char_offset(text, at(1, 3)), Some(2));
        assert_eq!(char_offset(text, at(2, 2)), Some(4));
        assert_eq!(char_offset(text, at(2, 4)), Some(6));
        assert_eq!(char_offset(text, at(3, 1)), Some(7));
    }

    #[test]
    fn positions_outside_the_text_have_no_offset() {
        let text = "ab\ncd";
        for position in [
            at(0, 1),
            at(1, 0),
            at(1, 4),
            at(3, 1),
            at(u32::MAX, u32::MAX),
        ] {
            assert_eq!(char_offset(text, position), None, "{position:?}");
        }
    }
}
