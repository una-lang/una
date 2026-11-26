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
    if diagnostic.code == Code::U7007 {
        return;
    }
    let rewrite = |text: &mut String| {
        if text.contains("`let") || text.contains("let`") {
            *text = words_of_0_5(text);
        }
    };
    rewrite(&mut diagnostic.message);
    for related in &mut diagnostic.related {
        rewrite(&mut related.message);
    }
    for repair in &mut diagnostic.repairs {
        rewrite(&mut repair.description);
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
}
