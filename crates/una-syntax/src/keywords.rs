//! The contextual keywords and the words reserved for later versions (`spec/01 §5`, ADR 0014).

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
