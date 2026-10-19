//! The contextual keywords and the words reserved for later versions (`spec/01 §5`, ADR 0014).

use crate::token::KeywordSet;

/// Keywords only inside `requirement` and `policy` blocks, and identifiers elsewhere.
pub const CONTEXTUAL: [&str; 12] = [
    "active",
    "generated",
    "human",
    "owner",
    "proposed",
    "purpose",
    "regulation",
    "source",
    "status",
    "text",
    "to",
    "via",
];

/// Words reserved for later versions, or naming constructs Una expresses differently, with
/// the message of their U0103 diagnostic (`spec/01 §5`, ADR 0014).
pub const RESERVED_FOR_LATER: [(&str, &str); 31] = [
    ("edit", "reserved for v0.2 (memory model)"),
    ("take", "reserved for v0.2 (memory model)"),
    ("region", "reserved for v0.2 (memory model)"),
    ("shared", "reserved for v0.2 (memory model)"),
    ("iso", "reserved for v0.2 (memory model)"),
    ("together", "reserved for v0.2 (concurrency)"),
    ("parallel", "reserved for v0.2 (concurrency)"),
    ("actor", "reserved for v0.2 (concurrency)"),
    ("simd", "reserved for v0.2 (systems profile)"),
    ("profile", "reserved for v0.2 (systems profile)"),
    ("trait", "reserved for v0.2 (constraints on generics)"),
    ("impl", "reserved for v0.2 (constraints on generics)"),
    ("law", "reserved for v0.3 (laws)"),
    ("all", "reserved for v0.3 (laws)"),
    ("costs", "reserved for v0.3 or later (cost contracts)"),
    ("traced", "reserved for v0.3 (provenance tracing)"),
    ("service", "reserved for v0.4 (platform)"),
    ("table", "reserved for v0.4 (platform)"),
    ("query", "reserved for v0.4 (platform)"),
    ("observe", "reserved for v0.4 (platform)"),
    ("commit", "reserved for v0.4 (platform)"),
    ("match", "not a keyword in Una; use `when`"),
    (
        "return",
        "not a keyword in Una; the last expression is the result",
    ),
    (
        "var",
        "not a keyword in Una; values are immutable, use `let`",
    ),
    ("while", "not a keyword in Una; use recursion or `fold`"),
    ("loop", "not a keyword in Una; use recursion or `fold`"),
    ("type", "not a keyword in Una; use `kind`"),
    ("unsafe", "not a keyword in Una; use a `trusted` module"),
    ("macro", "not supported (charter §6)"),
    ("async", "not supported; see structured concurrency in v0.2"),
    ("await", "not supported; see structured concurrency in v0.2"),
];

/// The rows of the reserved-word table that edition 0.2 changes or adds (`spec/17 §9`, ADR
/// 0056; `spec/19 §9`, ADR 0065; `spec/20 §9`, ADR 0066); `edit`, `take` and `together` are
/// keywords there.
pub const RESERVED_IN_0_2: [(&str, &str); 13] = [
    ("shared", "reserved for v0.3 (aliasing)"),
    ("iso", "reserved for v0.3 (aliasing)"),
    ("weak", "reserved for v0.3 (aliasing)"),
    (
        "region",
        "not a keyword in Una; the backend chooses where values live",
    ),
    ("var", "not a keyword in Una; use `let edit`"),
    ("while", "not a keyword in Una; use `for` or recursion"),
    ("loop", "not a keyword in Una; use `for` or recursion"),
    (
        "parallel",
        "reserved for a later version (concurrency beyond `together`)",
    ),
    (
        "actor",
        "reserved for a later version (concurrency beyond `together`)",
    ),
    ("async", "not supported; use `together`"),
    ("await", "not supported; use `together`"),
    ("trait", "reserved for a later version (user traits)"),
    ("impl", "reserved for a later version (user traits)"),
];

/// The words that edition 0.4 reserves for the constructs of ADR 0116 (`spec/26 §1`), with
/// the message of their U0103 diagnostic, until each becomes a keyword.
pub const RESERVED_IN_0_4: [(&str, &str); 2] = [
    (
        "diverges",
        "reserved for edition 0.4 (long-running programs)",
    ),
    ("within", "reserved for edition 0.4 (long-running programs)"),
];

/// The U0103 message text of `word` under the keyword set `set`, if it is reserved. In edition
/// 0.4, `simd` is not reserved (ADR 0120 rule 1).
pub fn reserved_in(word: &str, set: KeywordSet) -> Option<&'static str> {
    if set >= KeywordSet::V0_4
        && let Some((_, message)) = RESERVED_IN_0_4
            .iter()
            .find(|(reserved, _)| *reserved == word)
    {
        return Some(message);
    }
    // ADR 0120 rule 1: `simd` stops being reserved in edition 0.4, where it is an identifier
    // outside the clause of rule 58.
    if set >= KeywordSet::V0_4 && word == "simd" {
        return None;
    }
    if set >= KeywordSet::V0_2 {
        if matches!(word, "edit" | "take" | "together") {
            return None;
        }
        if let Some((_, message)) = RESERVED_IN_0_2
            .iter()
            .find(|(reserved, _)| *reserved == word)
        {
            return Some(message);
        }
    }
    reserved_for_later(word)
}

/// The U0103 message text of `word`, if it is reserved for a later version.
pub fn reserved_for_later(word: &str) -> Option<&'static str> {
    RESERVED_FOR_LATER
        .iter()
        .find(|(reserved, _)| *reserved == word)
        .map(|(_, message)| *message)
}

/// Whether `word` is a contextual keyword.
pub fn is_contextual(word: &str) -> bool {
    CONTEXTUAL.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simd_is_reserved_before_edition_0_4_only() {
        // ADR 0120 rule 1: `simd` stays reserved in editions 0.1 to 0.3.
        for set in [KeywordSet::V0_1, KeywordSet::V0_2, KeywordSet::V0_3] {
            assert_eq!(
                reserved_in("simd", set),
                Some("reserved for v0.2 (systems profile)")
            );
        }
        assert_eq!(reserved_in("simd", KeywordSet::V0_4), None);
        assert!(reserved_in("profile", KeywordSet::V0_4).is_some());
    }
}
