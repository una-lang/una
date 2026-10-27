//! Where each byte of a TOML string's value is written in its literal (#727): a value with
//! escapes, or a multi-line string, is shorter than its literal, so an offset in the value is
//! not an offset in the file.

/// For a string value, where each of its byte boundaries is written in its literal, as offsets
/// from the literal's first byte (its opening quote).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offsets {
    /// For each boundary `i` of the value, `0..=len`: where the character that starts at `i`
    /// is written, or the closing delimiter at `len`.
    starts: Vec<usize>,
    /// For each boundary `i` of the value, `0..=len`: just past where the character that ends
    /// at `i` is written, or where the content starts at `0`.
    ends: Vec<usize>,
}

impl Offsets {
    /// The offsets of text written as itself, with no delimiters or escapes: a `uses` clause
    /// on its own line of `ita.una` (ADR 0122 rule 11), where byte `i` of the value is byte
    /// `i` of the text.
    pub fn identity(len: usize) -> Offsets {
        Offsets {
            starts: (0..=len).collect(),
            ends: (0..=len).collect(),
        }
    }

    /// Where the bytes `start..end` of the value are written in the literal. A boundary past
    /// the value is the closing delimiter.
    pub fn place(&self, start: usize, end: usize) -> (usize, usize) {
        let at =
            |table: &[usize], i: usize| table.get(i).or_else(|| table.last()).copied().unwrap_or(0);
        let start = at(&self.starts, start);
        (start, at(&self.ends, end).max(start))
    }

    /// The bytes of the value written at `start..end` of the literal, when both are
    /// boundaries that [`Offsets::place`] gives.
    pub fn value_range(&self, start: usize, end: usize) -> Option<std::ops::Range<usize>> {
        let from = self.starts.partition_point(|&at| at < start);
        let to = self.ends.partition_point(|&at| at < end);
        (self.starts.get(from) == Some(&start) && self.ends.get(to) == Some(&end) && from <= to)
            .then_some(from..to)
    }
}

/// The offsets of `value` in `literal`, the text of a TOML string as the file writes it,
/// delimiters included: basic, literal, or either multi-line form. `None` when `literal`
/// does not decode to `value`, so that a position is never guessed.
pub fn offsets(literal: &str, value: &str) -> Option<Offsets> {
    // A multi-line string's line ending is kept as written or read as a line feed, as the
    // TOML reader does; the decoding that gives `value` is the one used.
    [false, true].into_iter().find_map(|lf| {
        let (decoded, offsets) = decode(literal, lf)?;
        (decoded == value).then_some(offsets)
    })
}

/// The kind of a string literal.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Basic,
    Literal,
}

/// Decodes `literal`, recording where each character of the value is written. With `lf`, a
/// carriage return and line feed in a multi-line string read as one line feed.
fn decode(literal: &str, lf: bool) -> Option<(String, Offsets)> {
    let bytes = literal.as_bytes();
    let len = bytes.len();
    let (kind, multi) = if len >= 6 && literal.starts_with("\"\"\"") && literal.ends_with("\"\"\"")
    {
        (Kind::Basic, true)
    } else if len >= 6 && literal.starts_with("'''") && literal.ends_with("'''") {
        (Kind::Literal, true)
    } else if len >= 2 && literal.starts_with('"') && literal.ends_with('"') {
        (Kind::Basic, false)
    } else if len >= 2 && literal.starts_with('\'') && literal.ends_with('\'') {
        (Kind::Literal, false)
    } else {
        return None;
    };
    let delimiter = if multi { 3 } else { 1 };
    let end = len - delimiter;
    let mut at = delimiter;
    // A newline right after the opening delimiter of a multi-line string is trimmed.
    if multi {
        if literal[at..end].starts_with('\n') {
            at += 1;
        } else if literal[at..end].starts_with("\r\n") {
            at += 2;
        }
    }
    let mut value = String::new();
    let mut offsets = Offsets {
        starts: Vec::new(),
        ends: vec![at],
    };
    let mut push = |c: char, from: usize, to: usize, verbatim: bool| {
        let width = c.len_utf8();
        for k in 0..width {
            offsets.starts.push(if verbatim { from + k } else { from });
            if k > 0 {
                offsets.ends.push(if verbatim { from + k } else { from });
            }
        }
        offsets.ends.push(to);
        value.push(c);
    };
    while at < end {
        let rest = &literal[at..end];
        if multi && lf && rest.starts_with("\r\n") {
            push('\n', at, at + 2, false);
            at += 2;
            continue;
        }
        let c = rest.chars().next()?;
        if kind == Kind::Literal || c != '\\' {
            push(c, at, at + c.len_utf8(), true);
            at += c.len_utf8();
            continue;
        }
        let escape = rest[1..].chars().next()?;
        let simple = match escape {
            'b' => Some('\u{8}'),
            't' => Some('\t'),
            'n' => Some('\n'),
            'f' => Some('\u{c}'),
            'r' => Some('\r'),
            'e' => Some('\u{1b}'),
            '"' => Some('"'),
            '\\' => Some('\\'),
            _ => None,
        };
        if let Some(decoded) = simple {
            push(decoded, at, at + 2, false);
            at += 2;
            continue;
        }
        let digits = match escape {
            'x' => 2,
            'u' => 4,
            'U' => 8,
            _ => 0,
        };
        if digits > 0 {
            let hex = rest.get(2..2 + digits)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let decoded = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
            push(decoded, at, at + 2 + digits, false);
            at += 2 + digits;
            continue;
        }
        // A line-ending backslash of a multi-line string: it, the rest of its line's
        // whitespace, and every whitespace and newline after it are trimmed.
        if !multi {
            return None;
        }
        let blank = rest[1..].len() - rest[1..].trim_start_matches([' ', '\t']).len();
        let after = &rest[1 + blank..];
        if !(after.starts_with('\n') || after.starts_with("\r\n")) {
            return None;
        }
        let trimmed = after.len() - after.trim_start_matches([' ', '\t', '\n', '\r']).len();
        at += 1 + blank + trimmed;
    }
    offsets.starts.push(end);
    Some((value, offsets))
}

#[cfg(test)]
mod tests {
    use super::offsets;

    /// The text of the literal at the bytes `start..end` of its value.
    fn written<'l>(literal: &'l str, value: &str, start: usize, end: usize) -> &'l str {
        let offsets = offsets(literal, value).unwrap();
        let (from, to) = offsets.place(start, end);
        assert_eq!(offsets.value_range(from, to), Some(start..end));
        &literal[from..to]
    }

    #[test]
    fn a_plain_string_is_its_value_past_the_quote() {
        let literal = "\"@c: C[op where x > 0]\"";
        let value = "@c: C[op where x > 0]";
        assert_eq!(written(literal, value, 15, 20), "x > 0");
        assert_eq!(written("'a b'", "a b", 2, 3), "b");
    }

    #[test]
    fn a_position_after_an_escape_is_placed_past_it() {
        // `name == \"a\", set where valid(sku)`: the call is 2 bytes later in the literal.
        let literal = r#""name == \"a\", set where valid(sku)""#;
        let value = r#"name == "a", set where valid(sku)"#;
        let call = value.find("valid").unwrap();
        assert_eq!(written(literal, value, call, call + 10), "valid(sku)");
        assert_eq!(written(literal, value, 8, 11), r#"\"a\""#);
        for (escape, decoded) in [
            (r"\b", "\u{8}"),
            (r"\t", "\t"),
            (r"\n", "\n"),
            (r"\f", "\u{c}"),
            (r"\r", "\r"),
            (r"\e", "\u{1b}"),
            (r"\\", "\\"),
            (r"\xe9", "é"),
            (r"\u00e9", "é"),
            (r"\U0001F600", "😀"),
        ] {
            let literal = format!("\"a{escape}b\"");
            let value = format!("a{decoded}b");
            let b = value.len() - 1;
            assert_eq!(written(&literal, &value, b, b + 1), "b", "{escape}");
            assert_eq!(written(&literal, &value, 1, b), escape, "{escape}");
        }
    }

    #[test]
    fn multi_line_strings_trim_their_first_newline_and_line_ending_backslashes() {
        let literal = "\"\"\"\nx > 0 \\\n    and y\"\"\"";
        let value = "x > 0 and y";
        assert_eq!(written(literal, value, 0, 5), "x > 0");
        assert_eq!(written(literal, value, 10, 11), "y");
        let literal = "'''\r\nkeep \\ y'''";
        assert_eq!(written(literal, "keep \\ y", 7, 8), "y");
        // A line ending is kept as written, or read as a line feed.
        assert!(offsets("\"\"\"a\r\nb\"\"\"", "a\r\nb").is_some());
        let literal = "\"\"\"a\r\nb\"\"\"";
        assert_eq!(written(literal, "a\nb", 2, 3), "b");
        // Up to two quotes before the closing delimiter belong to the value.
        assert_eq!(written("\"\"\"a\"\"\"\"", "a\"", 1, 2), "\"");
    }

    #[test]
    fn a_literal_that_does_not_decode_to_the_value_has_no_offsets() {
        assert_eq!(offsets("\"a\"", "b"), None);
        assert_eq!(offsets("\"a\\qb\"", "aqb"), None);
        assert_eq!(offsets("\"a\\u00zz\"", "a"), None);
        assert_eq!(offsets("\"a\\\nb\"", "ab"), None);
        assert_eq!(offsets("a", "a"), None);
    }

    /// The value the TOML reader gives `literal`, when it reads it.
    fn read(literal: &str) -> Option<String> {
        let text = format!("k = {literal}\n");
        let table = toml::de::DeTable::parse(&text).ok()?;
        let (_, value) = table.get_ref().iter().next()?;
        assert_eq!(value.span(), 4..4 + literal.len(), "{literal:?}");
        match value.get_ref() {
            toml::de::DeValue::String(value) => Some(value.to_string()),
            _ => None,
        }
    }

    #[test]
    fn generated_literals_decode_as_the_toml_reader_reads_them() {
        // #727: a few hundred literals of the four forms, from a fixed seed. Each one the TOML
        // reader accepts decodes to its value, with offsets in bounds, in order, and that map
        // each character boundary back to itself.
        const BASIC: &[&str] = &[
            "a",
            "b",
            " ",
            "é",
            "😀",
            "x > 0",
            "'",
            r#"\""#,
            r"\\",
            r"\t",
            r"\n",
            r"\b",
            r"\f",
            r"\r",
            r"\e",
            r"\xe9",
            r"\u00e9",
            r"\U0001F600",
        ];
        const MULTI_BASIC: &[&str] = &["\n", "\r\n", "\\\n  ", "\\ \t\r\n\n  ", "\"", "\t"];
        const LITERAL: &[&str] = &["a", "b", " ", "é", "😀", "\\", "\\n", "\"", "x > 0"];
        const MULTI_LITERAL: &[&str] = &["\n", "\r\n", "'", "\t"];
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(state >> 33).unwrap_or(0) % bound
        };
        let mut read_count = 0;
        for case in 0..400 {
            let form = case % 4;
            let pieces: Vec<&str> = match form {
                0 => BASIC.to_vec(),
                1 => [BASIC, MULTI_BASIC].concat(),
                2 => LITERAL.to_vec(),
                _ => [LITERAL, MULTI_LITERAL].concat(),
            };
            let (open, close) = match form {
                0 => ("\"", "\""),
                1 => ("\"\"\"", "\"\"\""),
                2 => ("'", "'"),
                _ => ("'''", "'''"),
            };
            let mut literal = open.to_owned();
            if form % 2 == 1 && next(3) == 0 {
                literal.push('\n');
            }
            for _ in 0..next(12) {
                literal.push_str(pieces[next(pieces.len())]);
            }
            literal.push_str(close);
            let Some(value) = read(&literal) else {
                continue;
            };
            read_count += 1;
            let offsets = offsets(&literal, &value)
                .unwrap_or_else(|| panic!("{literal:?} reads as {value:?}"));
            let n = value.len();
            assert_eq!(offsets.starts.len(), n + 1, "{literal:?}");
            assert_eq!(offsets.ends.len(), n + 1, "{literal:?}");
            for table in [&offsets.starts, &offsets.ends] {
                assert!(table.windows(2).all(|w| w[0] <= w[1]), "{literal:?}");
                assert!(table.iter().all(|&at| at <= literal.len()), "{literal:?}");
            }
            let boundaries: Vec<usize> = (0..=n).filter(|&i| value.is_char_boundary(i)).collect();
            for (k, &i) in boundaries.iter().enumerate() {
                for &j in &boundaries[k..] {
                    let (from, to) = offsets.place(i, j);
                    assert!(from <= to && to <= literal.len(), "{literal:?} {i}..{j}");
                    if i < j {
                        assert_eq!(offsets.value_range(from, to), Some(i..j), "{literal:?}");
                    }
                }
            }
        }
        assert!(read_count > 300, "{read_count}");
    }
}
