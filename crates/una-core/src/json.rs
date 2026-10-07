//! JSON (`spec/23`, ADR 0073): one parser and one writer for both engines, over a tree of their
//! own that each engine converts to and from its values, so the engines agree by construction.
//!
//! Nothing here recurses over a value's depth: parsing keeps its open arrays and objects on a
//! stack of its own, writing walks the tree with an explicit stack, and dropping a tree drains
//! it iteratively (ADR 0060). Parsing is O(n + Σ k log k) for an input of n scalar values,
//! with k keys per object; writing is O(n) for an output of n scalar values.

use std::collections::HashSet;

/// The deepest nesting of arrays and objects that [`parse`] accepts (`spec/23 §3` rule 4).
pub const MAX_DEPTH: usize = 512;

/// The most digits of an integer that [`parse`] accepts (`spec/23 §3`, N-27): converting
/// d decimal digits to an exact integer takes O(d²), so a longer literal is a failure, as in
/// CPython.
pub const MAX_INT_DIGITS: usize = 4300;

/// A JSON value, as `Json` holds it (`spec/23 §1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tree {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number with neither a fraction nor an exponent: its decimal digits, with a leading
    /// `-` when negative, and no leading zero; zero is `0`.
    Int(String),
    /// Any other number, as written: it matches [`is_json_number`].
    Decimal(String),
    /// A string, unescaped.
    Str(String),
    /// An array.
    Array(Vec<Tree>),
    /// An object, its entries ordered by key, with no two equal keys.
    Object(Vec<(String, Tree)>),
}

impl Drop for Tree {
    // Dropping a deep tree recursively would overflow the stack: drain the children into a
    // stack of trees whose children are already taken.
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_children(self, &mut pending);
        while let Some(mut tree) = pending.pop() {
            take_children(&mut tree, &mut pending);
        }
    }
}

fn take_children(tree: &mut Tree, into: &mut Vec<Tree>) {
    match tree {
        Tree::Array(items) => into.append(items),
        Tree::Object(entries) => into.extend(entries.drain(..).map(|(_, value)| value)),
        _ => {}
    }
}

/// Why a text is not one JSON value (`spec/23 §3` rule 5): each with the position, in scalar
/// values, of the first character in reading order that makes it so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No JSON text can continue at this position, or the text ends early there.
    Syntax(usize),
    /// This bracket opens a level deeper than [`MAX_DEPTH`].
    TooDeep(usize),
    /// This key equals an earlier key of the same object.
    DuplicateKey(usize),
    /// The integer that starts here has more than [`MAX_INT_DIGITS`] digits.
    NumberTooLong(usize),
}

/// An array or an object being read.
enum Frame {
    Array(Vec<Tree>),
    Object {
        entries: Vec<(String, Tree)>,
        keys: HashSet<String>,
        /// The key whose value is being read.
        key: Option<String>,
    },
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), Error> {
        if self.peek() == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(Error::Syntax(self.at))
        }
    }

    fn literal(&mut self, word: &str, value: Tree) -> Result<Tree, Error> {
        for c in word.chars() {
            self.expect(c)?;
        }
        Ok(value)
    }

    /// A number, at its first character.
    fn number(&mut self) -> Result<Tree, Error> {
        let start = self.at;
        let end =
            number_end(&self.chars, start).ok_or(Error::Syntax(self.at_number_error(start)))?;
        let text: String = self
            .chars
            .get(start..end)
            .ok_or(Error::Syntax(start))?
            .iter()
            .collect();
        self.at = end;
        let integer = !text.contains(['.', 'e', 'E']);
        if integer && text.trim_start_matches('-').len() > MAX_INT_DIGITS {
            return Err(Error::NumberTooLong(start));
        }
        Ok(if integer {
            Tree::Int(if text == "-0" { "0".to_owned() } else { text })
        } else {
            Tree::Decimal(text)
        })
    }

    /// The position where the number at `start` stops being valid.
    fn at_number_error(&self, start: usize) -> usize {
        number_error(&self.chars, start)
    }

    /// A string, at its opening quote.
    fn string(&mut self) -> Result<String, Error> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(Error::Syntax(self.at));
            };
            match c {
                '"' => {
                    self.at += 1;
                    return Ok(out);
                }
                '\\' => {
                    self.at += 1;
                    let escape = self.peek().ok_or(Error::Syntax(self.at))?;
                    match escape {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            self.at += 1;
                            let unit = self.unit_escape(false)?;
                            let scalar = if (0xD800..=0xDBFF).contains(&unit) {
                                // A high surrogate needs a low one right after it: the error is
                                // at the first character that cannot continue one (`spec/23 §3`).
                                self.expect('\\')?;
                                self.expect('u')?;
                                let low = self.unit_escape(true)?;
                                0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)
                            } else {
                                unit
                            };
                            out.push(char::from_u32(scalar).ok_or(Error::Syntax(self.at))?);
                            continue;
                        }
                        _ => return Err(Error::Syntax(self.at)),
                    }
                    self.at += 1;
                }
                c if (c as u32) < 0x20 => return Err(Error::Syntax(self.at)),
                c => {
                    out.push(c);
                    self.at += 1;
                }
            }
        }
    }

    /// The four hex digits of a `\\u` escape, after the `u`: a low surrogate when `low`, else
    /// anything but one. The error is at the first digit that cannot continue such an escape:
    /// the `D` of a low surrogate decides nothing yet, and its second digit, `C` to `F`, makes it
    /// one.
    fn unit_escape(&mut self, low: bool) -> Result<u32, Error> {
        let mut value = 0;
        for index in 0..4 {
            let at = self.at;
            let digit = self
                .peek()
                .and_then(|c| c.to_digit(16))
                .ok_or(Error::Syntax(at))?;
            if index == 0 && low && digit != 0xD {
                return Err(Error::Syntax(at));
            }
            if index == 1 && value == 0xD && low != (digit >= 0xC) {
                return Err(Error::Syntax(at));
            }
            value = value * 16 + digit;
            self.at += 1;
        }
        Ok(value)
    }
}

/// The end of the JSON number that starts at `start`, or `None` when none does.
fn number_end(chars: &[char], start: usize) -> Option<usize> {
    let digit = |i: usize| chars.get(i).is_some_and(char::is_ascii_digit);
    let mut i = start;
    if chars.get(i) == Some(&'-') {
        i += 1;
    }
    match chars.get(i) {
        Some('0') => i += 1,
        Some('1'..='9') => {
            while digit(i) {
                i += 1;
            }
        }
        _ => return None,
    }
    if chars.get(i) == Some(&'.') {
        i += 1;
        if !digit(i) {
            return None;
        }
        while digit(i) {
            i += 1;
        }
    }
    if matches!(chars.get(i), Some('e' | 'E')) {
        i += 1;
        if matches!(chars.get(i), Some('+' | '-')) {
            i += 1;
        }
        if !digit(i) {
            return None;
        }
        while digit(i) {
            i += 1;
        }
    }
    Some(i)
}

/// The position where the number that starts at `start` stops being valid: the first character
/// that no number can continue with.
fn number_error(chars: &[char], start: usize) -> usize {
    let digit = |i: usize| chars.get(i).is_some_and(char::is_ascii_digit);
    let mut i = start;
    if chars.get(i) == Some(&'-') {
        i += 1;
    }
    match chars.get(i) {
        Some('0') => i += 1,
        Some('1'..='9') => {
            while digit(i) {
                i += 1;
            }
        }
        _ => return i,
    }
    if chars.get(i) == Some(&'.') {
        i += 1;
        if !digit(i) {
            return i;
        }
        while digit(i) {
            i += 1;
        }
    }
    if matches!(chars.get(i), Some('e' | 'E')) {
        i += 1;
        if matches!(chars.get(i), Some('+' | '-')) {
            i += 1;
        }
        return i;
    }
    i
}

/// Whether `text` is exactly one JSON number (RFC 8259 §6) with a fraction or an exponent, as
/// `is_json_number` says: the numbers that `Decimal` holds, never those of `Number` (N-27).
pub fn is_json_number(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    number_end(&chars, 0) == Some(chars.len()) && text.contains(['.', 'e', 'E'])
}

/// The one JSON value of `text` (`spec/23 §3`), or the first error in reading order.
pub fn parse(text: &str) -> Result<Tree, Error> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
    };
    let mut frames: Vec<Frame> = Vec::new();
    parser.skip_whitespace();
    let value = loop {
        // A value starts here: a scalar, or an array or object that opens a frame.
        let start = parser.at;
        let mut value = match parser.peek() {
            Some('{' | '[') => {
                if frames.len() == MAX_DEPTH {
                    return Err(Error::TooDeep(start));
                }
                let object = parser.peek() == Some('{');
                parser.at += 1;
                parser.skip_whitespace();
                let closer = if object { '}' } else { ']' };
                if parser.peek() == Some(closer) {
                    parser.at += 1;
                    Some(if object {
                        Tree::Object(Vec::new())
                    } else {
                        Tree::Array(Vec::new())
                    })
                } else if object {
                    let key_at = parser.at;
                    if parser.peek() != Some('"') {
                        return Err(Error::Syntax(key_at));
                    }
                    let key = parser.string()?;
                    parser.skip_whitespace();
                    parser.expect(':')?;
                    parser.skip_whitespace();
                    let mut keys = HashSet::new();
                    keys.insert(key.clone());
                    frames.push(Frame::Object {
                        entries: Vec::new(),
                        keys,
                        key: Some(key),
                    });
                    None
                } else {
                    frames.push(Frame::Array(Vec::new()));
                    None
                }
            }
            Some('"') => Some(Tree::Str(parser.string()?)),
            Some('t') => Some(parser.literal("true", Tree::Bool(true))?),
            Some('f') => Some(parser.literal("false", Tree::Bool(false))?),
            Some('n') => Some(parser.literal("null", Tree::Null)?),
            Some('-' | '0'..='9') => Some(parser.number()?),
            _ => return Err(Error::Syntax(start)),
        };
        // A complete value goes into its frame; closing brackets complete the frames.
        while let Some(done) = value.take() {
            parser.skip_whitespace();
            let Some(frame) = frames.last_mut() else {
                value = Some(done);
                break;
            };
            match frame {
                Frame::Array(items) => {
                    items.push(done);
                    match parser.peek() {
                        Some(',') => {
                            parser.at += 1;
                            parser.skip_whitespace();
                        }
                        Some(']') => {
                            parser.at += 1;
                            if let Some(Frame::Array(items)) = frames.pop() {
                                value = Some(Tree::Array(items));
                            }
                        }
                        _ => return Err(Error::Syntax(parser.at)),
                    }
                }
                Frame::Object { entries, keys, key } => {
                    if let Some(key) = key.take() {
                        entries.push((key, done));
                    }
                    match parser.peek() {
                        Some(',') => {
                            parser.at += 1;
                            parser.skip_whitespace();
                            let key_at = parser.at;
                            if parser.peek() != Some('"') {
                                return Err(Error::Syntax(key_at));
                            }
                            let next = parser.string()?;
                            if !keys.insert(next.clone()) {
                                return Err(Error::DuplicateKey(key_at));
                            }
                            *key = Some(next);
                            parser.skip_whitespace();
                            parser.expect(':')?;
                            parser.skip_whitespace();
                        }
                        Some('}') => {
                            parser.at += 1;
                            if let Some(Frame::Object { mut entries, .. }) = frames.pop() {
                                // Keys are unique, so an unstable sort orders them fully; a
                                // run of keys already in order sorts in one pass.
                                entries.sort_by(|a, b| a.0.cmp(&b.0));
                                value = Some(Tree::Object(entries));
                            }
                        }
                        _ => return Err(Error::Syntax(parser.at)),
                    }
                }
            }
        }
        if let Some(done) = value {
            break done;
        }
    };
    parser.skip_whitespace();
    if parser.at != parser.chars.len() {
        return Err(Error::Syntax(parser.at));
    }
    Ok(value)
}

/// `text` as a JSON string literal (`spec/23 §4` rule 2).
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    push_quoted(&mut out, text);
    out
}

fn push_quoted(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20
                || c == '\u{7f}'
                || c == '\u{2028}'
                || c == '\u{2029}'
                || ('\u{202a}'..='\u{202e}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c) =>
            {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The canonical JSON text of `tree` (`spec/23 §4` rule 1).
pub fn write(tree: &Tree) -> String {
    enum Step<'t> {
        Value(&'t Tree),
        Text(&'static str),
        Key(&'t str),
    }
    let mut out = String::new();
    let mut steps = vec![Step::Value(tree)];
    while let Some(step) = steps.pop() {
        match step {
            Step::Text(text) => out.push_str(text),
            Step::Key(key) => {
                push_quoted(&mut out, key);
                out.push(':');
            }
            Step::Value(value) => match value {
                Tree::Null => out.push_str("null"),
                Tree::Bool(true) => out.push_str("true"),
                Tree::Bool(false) => out.push_str("false"),
                Tree::Int(digits) | Tree::Decimal(digits) => out.push_str(digits),
                Tree::Str(text) => push_quoted(&mut out, text),
                Tree::Array(items) => {
                    out.push('[');
                    steps.push(Step::Text("]"));
                    for (index, item) in items.iter().enumerate().rev() {
                        steps.push(Step::Value(item));
                        if index > 0 {
                            steps.push(Step::Text(","));
                        }
                    }
                }
                Tree::Object(entries) => {
                    out.push('{');
                    steps.push(Step::Text("}"));
                    for (index, (key, item)) in entries.iter().enumerate().rev() {
                        steps.push(Step::Value(item));
                        steps.push(Step::Key(key));
                        if index > 0 {
                            steps.push(Step::Text(","));
                        }
                    }
                }
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(digits: &str) -> Tree {
        Tree::Int(digits.to_owned())
    }

    #[test]
    fn values_parse_to_their_trees() {
        assert_eq!(parse(" null "), Ok(Tree::Null));
        assert_eq!(parse("true"), Ok(Tree::Bool(true)));
        assert_eq!(parse("-0"), Ok(int("0")));
        assert_eq!(
            parse("123456789012345678901234567890"),
            Ok(int("123456789012345678901234567890"))
        );
        assert_eq!(parse("1.50"), Ok(Tree::Decimal("1.50".to_owned())));
        assert_eq!(parse("-1E+3"), Ok(Tree::Decimal("-1E+3".to_owned())));
        assert_eq!(
            parse(r#""a\"\\\/\b\f\n\r\té😀""#),
            Ok(Tree::Str("a\"\\/\u{8}\u{c}\n\r\té😀".to_owned()))
        );
        assert_eq!(
            parse(r#"{"b": [1, {}], "a": []}"#),
            Ok(Tree::Object(vec![
                ("a".to_owned(), Tree::Array(Vec::new())),
                (
                    "b".to_owned(),
                    Tree::Array(vec![int("1"), Tree::Object(Vec::new())])
                ),
            ]))
        );
    }

    #[test]
    fn errors_name_the_first_position_in_reading_order() {
        assert_eq!(parse(""), Err(Error::Syntax(0)));
        assert_eq!(parse("[1,]"), Err(Error::Syntax(3)));
        assert_eq!(parse("01"), Err(Error::Syntax(1)));
        assert_eq!(parse("+1"), Err(Error::Syntax(0)));
        assert_eq!(parse("1."), Err(Error::Syntax(2)));
        assert_eq!(parse("1e"), Err(Error::Syntax(2)));
        assert_eq!(parse("NaN"), Err(Error::Syntax(0)));
        assert_eq!(parse("{'a': 1}"), Err(Error::Syntax(1)));
        assert_eq!(parse("[1] 2"), Err(Error::Syntax(4)));
        assert_eq!(parse("\u{feff}1"), Err(Error::Syntax(0)));
        assert_eq!(parse("\"a\u{1}\""), Err(Error::Syntax(2)));
        assert_eq!(parse(r#""\x""#), Err(Error::Syntax(2)));
        // Lone surrogates, at their escape.
        // Surrogates, at the first character that cannot continue a valid escape (N-27 F5).
        assert_eq!(parse(r#""ab\udc00""#), Err(Error::Syntax(6)));
        assert_eq!(parse(r#""\ud800x""#), Err(Error::Syntax(7)));
        assert_eq!(parse(r#""\ud800\u0041""#), Err(Error::Syntax(9)));
        assert_eq!(parse(r#""\uD800"#), Err(Error::Syntax(7)));
        assert_eq!(parse(r#""\uD800\"#), Err(Error::Syntax(8)));
        assert_eq!(parse(r#""\uDC00"#), Err(Error::Syntax(4)));
        assert_eq!(parse(r#""\uD800\uD800""#), Err(Error::Syntax(10)));
        assert_eq!(parse(r#"["\uD800""#), Err(Error::Syntax(8)));
        // Positions count scalar values, not bytes.
        assert_eq!(parse("[\"é\", ?]"), Err(Error::Syntax(6)));
        // A duplicate key before a later syntax error is reported first.
        assert_eq!(parse(r#"{"a":1,"a":2,?}"#), Err(Error::DuplicateKey(7)));
        assert_eq!(
            parse(r#"{"a":{"a":1},"b":2,"a":3}"#),
            Err(Error::DuplicateKey(19))
        );
    }

    #[test]
    fn nesting_stops_at_the_limit() {
        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse(&deep(MAX_DEPTH)).is_ok());
        assert_eq!(parse(&deep(MAX_DEPTH + 1)), Err(Error::TooDeep(MAX_DEPTH)));
        // A million brackets fail at the limit without recursing.
        assert_eq!(
            parse(&"[".repeat(1_000_000)),
            Err(Error::TooDeep(MAX_DEPTH))
        );
    }

    #[test]
    fn writing_is_canonical_and_round_trips() {
        let text = r#" { "z" : [ 1 , 2.50 , "x\u0001\u2028\u202e" ] , "a" : { } , "m" : null } "#;
        let tree = parse(text).unwrap();
        let written = write(&tree);
        assert_eq!(
            written,
            r#"{"a":{},"m":null,"z":[1,2.50,"x\u0001\u2028\u202e"]}"#
        );
        assert_eq!(parse(&written).unwrap(), tree);
        assert_eq!(write(&parse(&written).unwrap()), written);
        assert_eq!(
            quote("tab\there \"q\" \\ \u{7f} é"),
            "\"tab\\there \\\"q\\\" \\\\ \\u007f é\""
        );
    }

    #[test]
    fn deep_trees_write_and_drop_without_recursing() {
        let mut tree = Tree::Null;
        for _ in 0..1_000_000 {
            tree = Tree::Array(vec![tree]);
        }
        let written = write(&tree);
        assert_eq!(written.len(), 2_000_004);
        drop(tree);
    }

    #[test]
    fn numbers_follow_the_grammar() {
        for good in ["-1.5", "1e9", "1E-9", "0.0e+0", "5.0"] {
            assert!(is_json_number(good), "{good}");
        }
        // Integers are `Number`, never `Decimal` (N-27 F2).
        for bad in [
            "0", "-0", "12", "", "-", "01", "+1", "1.", ".5", "1e", "1e+", "0x1", " 1", "1 ",
        ] {
            assert!(!is_json_number(bad), "{bad}");
        }
    }

    #[test]
    fn integers_past_the_digit_limit_fail_where_they_start() {
        let at_limit = "9".repeat(MAX_INT_DIGITS);
        assert!(matches!(parse(&at_limit), Ok(Tree::Int(_))));
        let over = format!("[1, -{}]", "9".repeat(MAX_INT_DIGITS + 1));
        assert_eq!(parse(&over), Err(Error::NumberTooLong(4)));
        // A decimal keeps its text, so it has no such limit.
        let long = format!("{}.5", "9".repeat(100_000));
        assert!(matches!(parse(&long), Ok(Tree::Decimal(_))));
    }
}
