//! Names of packages, owners and sources (ADR 0122 rules 6, 7, 44 and 45): their forms, the
//! reserved names, and skeletons.

use una_syntax::Keyword;
use una_syntax::keywords::{RESERVED_FOR_LATER, RESERVED_IN_0_2, RESERVED_IN_0_4};

/// The names reserved besides the keywords (ADR 0122 rule 45), host capabilities included.
const RESERVED: [&str; 16] = [
    "std",
    "prelude",
    "una",
    "ita",
    "itaty",
    "itaoca",
    "itauna",
    "path",
    "git",
    "workspace",
    "console",
    "clock",
    "env",
    "files",
    "keys",
    "http",
];

/// Whether `word` is a word of a package name: a `lower_ident` with no `__` and no trailing
/// `_` (U1005's rule), of `min` to `max` bytes.
pub fn is_name_word(word: &str, min: usize, max: usize) -> bool {
    let mut chars = word.chars();
    (min..=max).contains(&word.len())
        && chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !word.contains("__")
        && !word.ends_with('_')
}

/// The skeleton of a name (ADR 0122 rule 45): `_` removed; `0` to `o`; `1` and `i` to `l`;
/// `2` to `z`; `3` to `e`; `4` to `a`; `5` to `s`; `7` to `t`; `8` to `b`; `9` to `g`; then
/// `rn` to `m` and `vv` to `w`; then each run of one letter collapsed to one. O(length).
pub fn skeleton(name: &str) -> String {
    let mapped: String = name
        .chars()
        .filter(|c| *c != '_')
        .map(|c| match c.to_ascii_lowercase() {
            '0' => 'o',
            '1' | 'i' => 'l',
            '2' => 'z',
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            '8' => 'b',
            '9' => 'g',
            other => other,
        })
        .collect();
    let joined = mapped.replace("rn", "m").replace("vv", "w");
    let mut out = String::with_capacity(joined.len());
    for c in joined.chars() {
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
}

/// Every reserved name of ADR 0122 rule 45: the keywords and reserved words of every
/// edition, and the names of [`RESERVED`].
pub fn reserved_names() -> impl Iterator<Item = &'static str> {
    Keyword::ALL
        .iter()
        .chain(Keyword::EDITION_0_2.iter())
        .chain(Keyword::EDITION_0_3.iter())
        .chain(Keyword::EDITION_0_5.iter())
        .map(|keyword| keyword.as_str())
        .chain(RESERVED_FOR_LATER.iter().map(|(word, _)| *word))
        .chain(RESERVED_IN_0_2.iter().map(|(word, _)| *word))
        .chain(RESERVED_IN_0_4.iter().map(|(word, _)| *word))
        .chain(RESERVED)
}

/// The reserved name that `name` is, or whose skeleton it has, if any. O(r · length) for the
/// r reserved names, a few hundred, with no allocation per name but the skeletons.
pub fn reserved_like(name: &str) -> Option<&'static str> {
    let own = skeleton(name);
    reserved_names().find(|reserved| *reserved == name || skeleton(reserved) == own)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeletons() {
        assert_eq!(skeleton("std_"), "std");
        assert_eq!(skeleton("s7d"), "std");
        assert_eq!(skeleton("p_r_e_l_u_d_e"), "prelude");
        assert_eq!(skeleton("modern"), "modem");
        assert_eq!(skeleton("vvork"), "work");
        assert_eq!(skeleton("jsoon"), "json");
        assert_eq!(skeleton("1nit"), "lnlt");
    }

    #[test]
    fn reserved() {
        assert_eq!(reserved_like("std"), Some("std"));
        assert_eq!(reserved_like("s7d"), Some("std"));
        assert_eq!(reserved_like("std_"), Some("std"));
        assert_eq!(reserved_like("module"), Some("module"));
        assert_eq!(reserved_like("within"), Some("within"));
        assert_eq!(reserved_like("http"), Some("http"));
        assert_eq!(reserved_like("json"), None);
        assert_eq!(reserved_like("ledger"), None);
    }

    #[test]
    fn name_words() {
        assert!(is_name_word("ledger", 1, 64));
        assert!(is_name_word("text_utf8", 1, 64));
        assert!(!is_name_word("a__b", 1, 64));
        assert!(!is_name_word("a_", 1, 64));
        assert!(!is_name_word("Ledger", 1, 64));
        assert!(!is_name_word("events-example", 1, 64));
        assert!(!is_name_word("a", 2, 39));
    }
}
