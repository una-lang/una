//! U7001: where a file differs from its canonical form (ADR 0020 rule 10).

use una_diag::{Code, Diagnostic, Edit, Repair};
use una_syntax::{FileId, SourceDb, Span};

/// Above this many cells, the line diff reports the changed middle as one region.
const DIFF_CELLS: usize = 4_000_000;

/// The U7001 diagnostics of the file `id`, whose canonical form is `canonical`: one at the
/// first `\r\n`, if any, and one per changed region of the line diff, each with a mechanical
/// edit.
pub fn differences(db: &SourceDb, id: FileId, canonical: &str) -> Vec<Diagnostic> {
    let source = db.file(id);
    let text = source.text();
    let mut diagnostics = Vec::new();
    if let Some(offset) = source.first_crlf() {
        let line = db.location(Span::at(id, offset)).start.line;
        let mut diagnostic = Diagnostic::error(
            Code::U7001,
            format!(
                "line {line} ends with a carriage return; canonical line breaks are line feeds"
            ),
            db.location(Span::at(id, offset)),
        );
        diagnostic.repairs.push(Repair {
            widens_authority: false,
            rank: 1,
            escape_cost: 0,
            description: "convert every line break to a line feed with `una fmt`".to_owned(),
            edit: None,
        });
        diagnostics.push(diagnostic);
    }
    let old: Vec<&str> = text.split_inclusive('\n').collect();
    let new: Vec<&str> = canonical.split_inclusive('\n').collect();
    let mut offsets = Vec::with_capacity(old.len() + 1);
    let mut offset = 0;
    for line in &old {
        offsets.push(offset);
        offset += line.len();
    }
    offsets.push(offset);
    let at = |line: usize| offsets.get(line).copied().unwrap_or(text.len());
    for hunk in hunks(&old, &new) {
        let removed = old.get(hunk.old.clone()).unwrap_or_default();
        let added = new.get(hunk.new.clone()).unwrap_or_default();
        let edit = Span::new(id, at(hunk.old.start), at(hunk.old.end));
        let primary = if hunk.old.is_empty() {
            Span::at(id, edit.start)
        } else {
            let last = removed
                .last()
                .map_or(0, |line| line.trim_end_matches('\n').len());
            Span::new(id, edit.start, at(hunk.old.end - 1) + last)
        };
        let mut diagnostic = Diagnostic::error(
            Code::U7001,
            message(hunk.old.start + 1, removed, added),
            db.location(primary),
        );
        diagnostic.repairs.push(Repair {
            widens_authority: false,
            rank: 1,
            escape_cost: 0,
            description: "rewrite in canonical form, as `una fmt` does".to_owned(),
            edit: Some(Edit {
                location: db.location(edit),
                replacement: added.concat(),
            }),
        });
        diagnostics.push(diagnostic);
    }
    diagnostics
}

/// The message of a changed region starting at 1-based `line`.
fn message(line: usize, removed: &[&str], added: &[&str]) -> String {
    let lines = |count: usize| {
        if count == 1 {
            format!("line {line}")
        } else {
            format!("lines {line} to {}", line + count - 1)
        }
    };
    let blank = |line: &&str| line.trim().is_empty();
    let same_after_trimming = removed.len() == added.len()
        && removed
            .iter()
            .zip(added)
            .all(|(old, new)| old.trim_end() == new.trim_end());
    if same_after_trimming && removed.iter().all(|old| old.ends_with('\n')) {
        format!("trailing whitespace on {}", lines(removed.len()))
    } else if same_after_trimming {
        "the file does not end with a line break".to_owned()
    } else if removed.is_empty() && added.iter().all(blank) {
        if added.len() == 1 {
            format!("a blank line is missing before line {line}")
        } else {
            format!("{} blank lines are missing before line {line}", added.len())
        }
    } else if added.is_empty() && removed.iter().all(blank) {
        if removed.len() == 1 {
            format!("line {line} is an extra blank line")
        } else {
            format!("{} are extra blank lines", lines(removed.len()))
        }
    } else if removed.is_empty() {
        format!("the canonical form adds lines before line {line}")
    } else if removed.len() == 1 {
        format!("line {line} is not in canonical form")
    } else {
        format!("{} are not in canonical form", lines(removed.len()))
    }
}

/// A changed region: lines `old` of the file become lines `new` of the canonical form.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hunk {
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
}

/// The changed regions between two lists of lines, from a longest common subsequence of the
/// part between their common prefix and suffix.
fn hunks(old: &[&str], new: &[&str]) -> Vec<Hunk> {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(old.len().min(new.len()) - prefix)
        .take_while(|(left, right)| left == right)
        .count();
    let a = old.get(prefix..old.len() - suffix).unwrap_or_default();
    let b = new.get(prefix..new.len() - suffix).unwrap_or_default();
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    if a.is_empty() || b.is_empty() || a.len().saturating_mul(b.len()) > DIFF_CELLS {
        return vec![Hunk {
            old: prefix..prefix + a.len(),
            new: prefix..prefix + b.len(),
        }];
    }
    // table[i * width + j] is the length of a longest common subsequence of a[i..] and b[j..].
    let width = b.len() + 1;
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            let value = if a.get(i) == b.get(j) {
                table
                    .get((i + 1) * width + j + 1)
                    .map_or(0, |next| next + 1)
            } else {
                let down = table.get((i + 1) * width + j).copied().unwrap_or(0);
                let right = table.get(i * width + j + 1).copied().unwrap_or(0);
                down.max(right)
            };
            if let Some(cell) = table.get_mut(i * width + j) {
                *cell = value;
            }
        }
    }
    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let cell = |i: usize, j: usize| table.get(i * width + j).copied().unwrap_or(0);
        if i < a.len() && j < b.len() && a.get(i) == b.get(j) {
            hunks.extend(current.take());
            i += 1;
            j += 1;
            continue;
        }
        let hunk = current.get_or_insert(Hunk {
            old: prefix + i..prefix + i,
            new: prefix + j..prefix + j,
        });
        if j >= b.len() || (i < a.len() && cell(i + 1, j) >= cell(i, j + 1)) {
            i += 1;
            hunk.old.end = prefix + i;
        } else {
            j += 1;
            hunk.new.end = prefix + j;
        }
    }
    hunks.extend(current);
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.split_inclusive('\n').collect()
    }

    #[test]
    fn identical_texts_have_no_hunks() {
        assert_eq!(hunks(&lines("a\nb\n"), &lines("a\nb\n")), vec![]);
    }

    #[test]
    fn separate_changes_are_separate_hunks() {
        let old = lines("a\nb \nc\nd\ne \n");
        let new = lines("a\nb\nc\nd\ne\n");
        assert_eq!(
            hunks(&old, &new),
            vec![
                Hunk {
                    old: 1..2,
                    new: 1..2
                },
                Hunk {
                    old: 4..5,
                    new: 4..5
                },
            ]
        );
    }

    #[test]
    fn insertions_and_deletions() {
        assert_eq!(
            hunks(&lines("a\nb\n"), &lines("a\n\nb\n")),
            vec![Hunk {
                old: 1..1,
                new: 1..2
            }]
        );
        // The second of two blank lines is the extra one.
        assert_eq!(
            hunks(&lines("a\n\n\nb\n"), &lines("a\n\nb\n")),
            vec![Hunk {
                old: 2..3,
                new: 2..2
            }]
        );
        assert_eq!(
            hunks(&lines("f\n=\n  1\ng\n"), &lines("f = 1\ng\n")),
            vec![Hunk {
                old: 0..3,
                new: 0..1
            }]
        );
    }

    #[test]
    fn messages_name_the_difference() {
        assert_eq!(
            message(3, &["x  \n"], &["x\n"]),
            "trailing whitespace on line 3"
        );
        assert_eq!(
            message(3, &["x \n", "y \n"], &["x\n", "y\n"]),
            "trailing whitespace on lines 3 to 4"
        );
        assert_eq!(
            message(7, &["x"], &["x\n"]),
            "the file does not end with a line break"
        );
        assert_eq!(
            message(5, &[], &["\n"]),
            "a blank line is missing before line 5"
        );
        assert_eq!(message(5, &["\n"], &[]), "line 5 is an extra blank line");
        assert_eq!(
            message(5, &["\n", "\n"], &[]),
            "lines 5 to 6 are extra blank lines"
        );
        assert_eq!(
            message(2, &["a+b\n"], &["a + b\n"]),
            "line 2 is not in canonical form"
        );
        assert_eq!(
            message(2, &["f\n", "=\n", "  1\n"], &["f = 1\n"]),
            "lines 2 to 4 are not in canonical form"
        );
    }
}
