//! Text from outside the toolchain, quoted in a message so that it can neither disguise nor
//! flood a diagnostic (#2156): a registry's documents, an editor's bytes.

/// Whether `c` is invisible or changes how text around it reads: a control character, a format
/// character (Unicode category Cf: the direction marks and isolates, zero-width characters,
/// U+FEFF, the tag characters and the rest), or a line or paragraph separator.
pub fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

/// `text` as a message may quote it: every [`hidden`] character written as `\u{…}`, and at
/// most `max` characters of `text`, followed by `...` when more were cut. O(n) in the
/// characters kept.
pub fn quoted(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (count, c) in text.chars().enumerate() {
        if count == max {
            out.push_str("...");
            break;
        }
        if hidden(c) {
            out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters_are_escaped_and_long_text_is_cut() {
        assert_eq!(
            quoted("a\u{1b}[2J\r\0\u{7}\u{202e}b", 64),
            "a\\u{1b}[2J\\u{d}\\u{0}\\u{7}\\u{202e}b"
        );
        assert_eq!(quoted("abcdef", 3), "abc...");
        assert_eq!(quoted("abc", 3), "abc");
        assert_eq!(quoted("\u{e9}t\u{e9}", 64), "\u{e9}t\u{e9}");
    }
}
