//! JSON (`spec/23`, ADR 0073): one parser and one writer for both engines, over a tree of their
//! own that each engine converts to and from its values, so the engines agree by construction.
//!
//! Nothing here recurses over a value's depth: parsing keeps its open arrays and objects on a
//! stack of its own, writing walks the tree with an explicit stack, and dropping a tree drains
//! it iteratively (ADR 0060). Parsing is O(n + Σ k log k) for an input of n scalar values,
//! with k keys per object; writing is O(n) for an output of n scalar values.
//!
//! Both read the text in place and check the run's memory budget as they go, every
//! [`CHECK_EVERY`] values and every [`CHECK_EVERY`] scalar values of a string, through a hook
//! each engine supplies (`spec/11 §7`), so a large input ends with U6003 at the call
//! rather than with an allocation the process cannot make.

use std::collections::HashSet;
use std::convert::Infallible;

/// How often [`parse_within`] and [`write_within`] call their budget hook: every this many
/// values, and every this many scalar values inside one string.
pub const CHECK_EVERY: usize = 4096;

/// The deepest nesting of arrays and objects that [`parse`] accepts (`spec/23 §3` rule 4).
pub const MAX_DEPTH: usize = 512;

/// The most digits of an integer that [`parse`] accepts (`spec/23 §3`), shared with
/// `parse_int` and `to_int`.
pub use crate::MAX_INT_DIGITS;

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

/// Why [`parse_within`] stopped without a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    /// The text is not one JSON value.
    Invalid(Error),
    /// The budget hook said the run is over its memory budget (`spec/11 §7`).
    OverBudget,
}

impl From<Error> for Halt {
    fn from(error: Error) -> Halt {
        Halt::Invalid(error)
    }
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

/// The text being read, in place: `byte` is the offset of the next character, and `at` its
/// position in scalar values, which errors report (`spec/23 §3` rule 5).
struct Parser<'t, 'w, E> {
    text: &'t str,
    byte: usize,
    at: usize,
    ticks: usize,
    within: &'w mut dyn FnMut() -> Result<(), E>,
}

/// Why the parser stopped: the text is invalid, or the hook stopped it with its `E`.
enum Stop<E> {
    Invalid(Error),
    Hook(E),
}

impl<E> From<Error> for Stop<E> {
    fn from(error: Error) -> Stop<E> {
        Stop::Invalid(error)
    }
}

impl<E> Parser<'_, '_, E> {
    fn peek(&self) -> Option<char> {
        self.text
            .get(self.byte..)
            .and_then(|rest| rest.chars().next())
    }

    /// Moves past the next character, which the caller has seen.
    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.byte += c.len_utf8();
            self.at += 1;
        }
    }

    /// Counts one unit of work, and every [`CHECK_EVERY`] asks the budget hook.
    fn tick(&mut self) -> Result<(), Stop<E>> {
        self.ticks += 1;
        if self.ticks >= CHECK_EVERY {
            self.ticks = 0;
            (self.within)().map_err(Stop::Hook)?;
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.bump();
        }
    }

    fn expect(&mut self, c: char) -> Result<(), Error> {
        if self.peek() == Some(c) {
            self.bump();
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

    /// A number, at its first character. A number is ASCII, so its offsets in bytes and in
    /// scalar values advance together.
    fn number(&mut self) -> Result<Tree, Error> {
        let start = self.at;
        let bytes = self.text.as_bytes();
        let Some(end) = number_end(bytes, self.byte) else {
            let stop = number_error(bytes, self.byte);
            return Err(Error::Syntax(start + (stop - self.byte)));
        };
        let text = self.text.get(self.byte..end).ok_or(Error::Syntax(start))?;
        let text = text.to_owned();
        self.at += end - self.byte;
        self.byte = end;
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

    /// A string, at its opening quote.
    fn string(&mut self) -> Result<String, Stop<E>> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            self.tick()?;
            let Some(c) = self.peek() else {
                return Err(Error::Syntax(self.at).into());
            };
            match c {
                '"' => {
                    self.bump();
                    return Ok(out);
                }
                '\\' => {
                    self.bump();
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
                            self.bump();
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
                        _ => return Err(Error::Syntax(self.at).into()),
                    }
                    self.bump();
                }
                c if (c as u32) < 0x20 => return Err(Error::Syntax(self.at).into()),
                c => {
                    out.push(c);
                    self.bump();
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
            self.bump();
        }
        Ok(value)
    }
}

/// The end of the JSON number that starts at `start`, or `None` when none does.
fn number_end(chars: &[u8], start: usize) -> Option<usize> {
    let digit = |i: usize| chars.get(i).is_some_and(u8::is_ascii_digit);
    let mut i = start;
    if chars.get(i) == Some(&b'-') {
        i += 1;
    }
    match chars.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            while digit(i) {
                i += 1;
            }
        }
        _ => return None,
    }
    if chars.get(i) == Some(&b'.') {
        i += 1;
        if !digit(i) {
            return None;
        }
        while digit(i) {
            i += 1;
        }
    }
    if matches!(chars.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(chars.get(i), Some(b'+' | b'-')) {
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
fn number_error(chars: &[u8], start: usize) -> usize {
    let digit = |i: usize| chars.get(i).is_some_and(u8::is_ascii_digit);
    let mut i = start;
    if chars.get(i) == Some(&b'-') {
        i += 1;
    }
    match chars.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            while digit(i) {
                i += 1;
            }
        }
        _ => return i,
    }
    if chars.get(i) == Some(&b'.') {
        i += 1;
        if !digit(i) {
            return i;
        }
        while digit(i) {
            i += 1;
        }
    }
    if matches!(chars.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(chars.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        return i;
    }
    i
}

/// Whether `text` is exactly one JSON number (RFC 8259 §6) with a fraction or an exponent, as
/// `is_json_number` says: the numbers that `Decimal` holds, never those of `Number`.
pub fn is_json_number(text: &str) -> bool {
    number_end(text.as_bytes(), 0) == Some(text.len()) && text.contains(['.', 'e', 'E'])
}

/// The one JSON value of `text` (`spec/23 §3`), or the first error in reading order.
pub fn parse(text: &str) -> Result<Tree, Error> {
    // A hook that cannot stop: `Infallible` proves the parser ends with a value or an error.
    match parse_hooked(text, &mut || Ok::<(), Infallible>(())) {
        Ok(tree) => Ok(tree),
        Err(Stop::Invalid(error)) => Err(error),
        Err(Stop::Hook(never)) => match never {},
    }
}

/// [`parse`], asking `within` every [`CHECK_EVERY`] values and string characters whether the
/// run is still within its memory budget, and stopping with [`Halt::OverBudget`] when it is
/// not (`spec/11 §7`).
pub fn parse_within(text: &str, within: &mut dyn FnMut() -> bool) -> Result<Tree, Halt> {
    let mut hook = || if within() { Ok(()) } else { Err(()) };
    match parse_hooked(text, &mut hook) {
        Ok(tree) => Ok(tree),
        Err(Stop::Invalid(error)) => Err(Halt::Invalid(error)),
        Err(Stop::Hook(())) => Err(Halt::OverBudget),
    }
}

/// The parser, with a hook that may stop it with its own `E`.
fn parse_hooked<E>(text: &str, within: &mut dyn FnMut() -> Result<(), E>) -> Result<Tree, Stop<E>> {
    within().map_err(Stop::Hook)?;
    let mut parser = Parser {
        text,
        byte: 0,
        at: 0,
        ticks: 0,
        within,
    };
    let mut frames: Vec<Frame> = Vec::new();
    parser.skip_whitespace();
    let value = loop {
        parser.tick()?;
        // A value starts here: a scalar, or an array or object that opens a frame.
        let start = parser.at;
        let mut value = match parser.peek() {
            Some('{' | '[') => {
                if frames.len() == MAX_DEPTH {
                    return Err(Error::TooDeep(start).into());
                }
                let object = parser.peek() == Some('{');
                parser.bump();
                parser.skip_whitespace();
                let closer = if object { '}' } else { ']' };
                if parser.peek() == Some(closer) {
                    parser.bump();
                    Some(if object {
                        Tree::Object(Vec::new())
                    } else {
                        Tree::Array(Vec::new())
                    })
                } else if object {
                    let key_at = parser.at;
                    if parser.peek() != Some('"') {
                        return Err(Error::Syntax(key_at).into());
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
            _ => return Err(Error::Syntax(start).into()),
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
                            parser.bump();
                            parser.skip_whitespace();
                        }
                        Some(']') => {
                            parser.bump();
                            if let Some(Frame::Array(items)) = frames.pop() {
                                value = Some(Tree::Array(items));
                            }
                        }
                        _ => return Err(Error::Syntax(parser.at).into()),
                    }
                }
                Frame::Object { entries, keys, key } => {
                    if let Some(key) = key.take() {
                        entries.push((key, done));
                    }
                    match parser.peek() {
                        Some(',') => {
                            parser.bump();
                            parser.skip_whitespace();
                            let key_at = parser.at;
                            if parser.peek() != Some('"') {
                                return Err(Error::Syntax(key_at).into());
                            }
                            let next = parser.string()?;
                            if !keys.insert(next.clone()) {
                                return Err(Error::DuplicateKey(key_at).into());
                            }
                            *key = Some(next);
                            parser.skip_whitespace();
                            parser.expect(':')?;
                            parser.skip_whitespace();
                        }
                        Some('}') => {
                            parser.bump();
                            if let Some(Frame::Object { mut entries, .. }) = frames.pop() {
                                // The standard library's stable sort, which finds runs: keys
                                // already in order, as `to_json` writes them, sort in one pass,
                                // O(k), and each engine then builds the map in bulk (#886).
                                entries.sort_by(|a, b| a.0.cmp(&b.0));
                                value = Some(Tree::Object(entries));
                            }
                        }
                        _ => return Err(Error::Syntax(parser.at).into()),
                    }
                }
            }
        }
        if let Some(done) = value {
            break done;
        }
    };
    parser.skip_whitespace();
    if parser.byte != text.len() {
        return Err(Error::Syntax(parser.at).into());
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
    match write_hooked(tree, &mut || Ok::<(), Infallible>(())) {
        Ok(text) => text,
        Err(never) => match never {},
    }
}

/// [`write`], asking `within` every [`CHECK_EVERY`] values whether the run is still within its
/// memory budget, and stopping with `None` when it is not (`spec/11 §7`).
pub fn write_within(tree: &Tree, within: &mut dyn FnMut() -> bool) -> Option<String> {
    let mut hook = || if within() { Ok(()) } else { Err(()) };
    write_hooked(tree, &mut hook).ok()
}

/// The writer, with a hook that may stop it with its own `E`.
fn write_hooked<E>(tree: &Tree, within: &mut dyn FnMut() -> Result<(), E>) -> Result<String, E> {
    enum Step<'t> {
        Value(&'t Tree),
        Text(&'static str),
        Key(&'t str),
    }
    within()?;
    let mut out = String::new();
    let mut steps = vec![Step::Value(tree)];
    let mut ticks = 0;
    // The length of the output at the last check: every [`crate::INT_TEXT_RESERVED_FROM`]
    // bytes written since is a check too, so long numbers and strings cannot pass the hard
    // ceiling between two checks of values (#1223).
    let mut checked = 0usize;
    while let Some(step) = steps.pop() {
        ticks += 1;
        if ticks >= CHECK_EVERY || out.len() - checked >= crate::INT_TEXT_RESERVED_FROM {
            ticks = 0;
            checked = out.len();
            within()?;
        }
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
    Ok(out)
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
        // Surrogates, at the first character that cannot continue a valid escape.
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
        // Integers are `Number`, never `Decimal`.
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
