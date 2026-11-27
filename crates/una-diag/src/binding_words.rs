//! The binding words of a package's edition in messages (ADR 0137 rule 10): `let` and
//! `let edit` up to edition `0.4`, `const` and `var` in edition `0.5`.
//!
//! The toolchain writes its messages, helps and repair descriptions with the words of edition
//! `0.4`; for a package of edition `0.5`, [`in_edition_0_5`] rewrites each binding form a text
//! names between backticks, and the few phrases that ADR 0137 rule 10 words differently. A
//! text is read once: O(its length).

use crate::{Code, Diagnostic};

/// The phrases of ADR 0137 rule 10 that are not a one-to-one rename, in edition `0.4` words
/// and in edition `0.5` words. They are replaced before the backticked forms.
const PHRASES: [(&str, &str); 5] = [
    (
        "a block cannot end with `let`",
        "a block cannot end with a `const` or `var`",
    ),
    (
        "holds only `let` statements, one per branch",
        "holds only `const` or `var` statements, one per branch",
    ),
    ("not a `let`", "not a `const` or `var` statement"),
    (
        "needs at least two `let` branches",
        "needs at least two branches",
    ),
    (
        "expression of a statement or `let`",
        "expression of a statement, or of a `const` or `var`",
    ),
];

/// `text` with the binding words of edition `0.5`: each backticked form that starts with
/// `let edit` starts with `var`, and each that is `let` or starts with `let ` starts with
/// `const`. Text outside backticks, and a backticked word that only starts with the letters
/// `let`, as `` `letter` ``, are unchanged.
pub fn words_of_0_5(text: &str) -> String {
    let mut phrased = text.to_owned();
    for (old, new) in PHRASES {
        if phrased.contains(old) {
            phrased = phrased.replace(old, new);
        }
    }
    let mut out = String::with_capacity(phrased.len() + 8);
    let mut rest = phrased.as_str();
    let mut inside = false;
    while let Some(tick) = rest.find('`') {
        let (before, after) = rest.split_at(tick);
        out.push_str(before);
        out.push('`');
        let after = &after[1..];
        inside = !inside;
        // A token of the source that a message quotes, as "unexpected keyword `let`", is
        // the program's own word, which U7007 reports: it keeps it.
        let quoted = before.ends_with("keyword ");
        rest = if inside && !quoted {
            if let Some(tail) = after.strip_prefix("let edit") {
                if tail.starts_with([' ', '`']) {
                    out.push_str("var");
                    tail
                } else {
                    after
                }
            } else if let Some(tail) = after.strip_prefix("let") {
                if tail.starts_with([' ', '`']) {
                    out.push_str("const");
                    tail
                } else {
                    after
                }
            } else {
                after
            }
        } else {
            after
        };
    }
    out.push_str(rest);
    out
}

/// Rewrites the message, the related notes and the repair descriptions of `diagnostic` with
/// the binding words of edition `0.5` ([`words_of_0_5`]). U7007, whose message names `let` as
/// the form edition `0.5` changed, keeps its words.
pub fn in_edition_0_5(diagnostic: &mut Diagnostic) {
    in_edition_0_5_where(diagnostic, |_| true);
}

/// [`in_edition_0_5`] for a program whose packages have different editions (ADR 0122 rule
/// 49): each text takes the words of the file it is about, as `edition_0_5` tells for a file's
/// path. The message and the repair descriptions are about the main location, and each
/// related note about its own, so a note in a dependency of edition `0.4` keeps its words
/// under a root of edition `0.5`, and a note in the root takes the words of `0.5` in a
/// dependency's diagnostic.
pub fn in_edition_0_5_where(diagnostic: &mut Diagnostic, edition_0_5: impl Fn(&str) -> bool) {
    if diagnostic.code == Code::U7007 {
        return;
    }
    let rewrite = |text: &mut String| {
        if text.contains("`let") || text.contains("let`") {
            *text = words_of_0_5(text);
        }
    };
    if edition_0_5(&diagnostic.primary.file) {
        rewrite(&mut diagnostic.message);
        for repair in &mut diagnostic.repairs {
            rewrite(&mut repair.description);
        }
    }
    for related in &mut diagnostic.related {
        if edition_0_5(&related.location.file) {
            rewrite(&mut related.message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_messages_of_rule_10_read_in_the_words_of_edition_0_5() {
        // ADR 0137 rule 10, each message in its edition 0.4 words and its edition 0.5 words.
        for (old, new) in [
            (
                "`x` is not editable; declare it `let edit x`, or the parameter `edit x`",
                "`x` is not editable; declare it `var x`, or the parameter `edit x`",
            ),
            (
                "`x` is the variable of a `for` loop, which is read-only; bind a copy in the body with `let edit`, as `let edit next_x = x`",
                "`x` is the variable of a `for` loop, which is read-only; bind a copy in the body with `var`, as `var next_x = x`",
            ),
            (
                "bind its result with `let` first",
                "bind its result with `const` first",
            ),
            (
                "the body of `together` holds only `let` statements, one per branch; call a function from a branch's `let` for more steps",
                "the body of `together` holds only `const` or `var` statements, one per branch; call a function from a branch's `const` for more steps",
            ),
            ("not a `let`", "not a `const` or `var` statement"),
            (
                "a block cannot end with `let`; its last line is its value",
                "a block cannot end with a `const` or `var`; its last line is its value",
            ),
            (
                "`together` needs at least two `let` branches; with one, write a plain `let`",
                "`together` needs at least two branches; with one, write a plain `const`",
            ),
            (
                "the whole expression of a statement or `let`",
                "the whole expression of a statement, or of a `const` or `var`",
            ),
            (
                "annotate it, as `let d: Int of Meter = n.measured()`",
                "annotate it, as `const d: Int of Meter = n.measured()`",
            ),
        ] {
            assert_eq!(words_of_0_5(old), new, "{old}");
        }
    }

    #[test]
    fn other_words_and_text_outside_backticks_are_unchanged() {
        for text in [
            "`letter` is not in scope",
            "let the value be",
            "`edit` and `take`",
            "",
            "an unclosed `let",
            "unexpected keyword `let`, expected a declaration",
        ] {
            assert_eq!(words_of_0_5(text), text, "{text}");
        }
        assert_eq!(words_of_0_5("`let`, then `let`"), "`const`, then `const`");
        assert_eq!(words_of_0_5("`let (a, b) = e`"), "`const (a, b) = e`");
    }

    #[test]
    fn each_text_takes_the_words_of_its_own_file() {
        // ADR 0122 rule 49 with ADR 0137 rule 10 (#2260): the message is about the main
        // location's file, each related note about its own.
        let at = |file: &str| crate::Location {
            file: file.to_owned(),
            start: crate::Position { line: 1, col: 1 },
            end: crate::Position { line: 1, col: 2 },
        };
        let make = |primary: &str, related: &str| {
            let mut diagnostic =
                Diagnostic::error(Code::U2001, "bind its result with `let` first", at(primary));
            diagnostic.related.push(crate::Related {
                location: at(related),
                message: "not a `let`".to_owned(),
            });
            diagnostic
        };
        let edition_0_5 = |file: &str| file.starts_with("src/");
        let mut root = make("src/a.una", "deps/util/src/b.una");
        in_edition_0_5_where(&mut root, edition_0_5);
        assert_eq!(root.message, "bind its result with `const` first");
        assert_eq!(root.related[0].message, "not a `let`");
        let mut dependency = make("deps/util/src/b.una", "src/a.una");
        in_edition_0_5_where(&mut dependency, edition_0_5);
        assert_eq!(dependency.message, "bind its result with `let` first");
        assert_eq!(
            dependency.related[0].message,
            "not a `const` or `var` statement"
        );
    }
}
