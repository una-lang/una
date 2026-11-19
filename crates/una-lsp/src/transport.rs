//! JSON-RPC 2.0 messages over a byte stream, framed by a `Content-Length` header, as LSP
//! sends them (ADR 0063 rule 2).
//!
//! The peer on the input is not trusted with memory: a header line, the number of header
//! lines and the body each have a fixed upper bound, and input past one of them is refused
//! with an error of kind `InvalidData` before anything of its size is buffered or allocated.
//! The JSON text of a body has two more bounds, [`MAX_DEPTH`] and [`MAX_VALUES`], checked by
//! one pass over its bytes before the parser makes any value (#2100), so the memory of a
//! parsed frame is bounded too: see [`MAX_VALUES`] for the arithmetic.

use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

/// The longest header line, in bytes, its line ending (`\r\n` or `\n`) not counted. LSP
/// defines two headers, `Content-Length` and `Content-Type`, whose lines are under 100
/// bytes; 4 KiB leaves room for any header a client may add.
pub const MAX_HEADER_LINE: usize = 4 * 1024;

/// The most header lines in one frame, the empty line that ends them not counted. With
/// [`MAX_HEADER_LINE`] it bounds the header of a frame to 64 KiB.
pub const MAX_HEADER_LINES: usize = 16;

/// The largest body, in bytes. A `didOpen` or a full `didChange` carries a whole document,
/// and JSON escaping can make its text up to six times longer (`\u001f` for a control
/// byte): 64 MiB holds a document of several MiB however it is escaped, while a peer can no
/// longer make the server allocate an unbounded amount.
pub const MAX_CONTENT_LENGTH: usize = 64 * 1024 * 1024;

/// The deepest nesting of arrays and objects in a body. It is the bound of the parser itself:
/// `serde_json` refuses a text nested deeper than 127 containers (its recursion limit of 128
/// counts the value inside the innermost one). The transport checks the same bound first, so
/// the refusal names it.
pub const MAX_DEPTH: usize = 127;

/// The bytes of heap that one parsed value is counted as, an upper bound on 64-bit targets.
/// A `serde_json::Value` is 32 bytes, and the costliest shape is an object with one member,
/// as in `[{"":0},{"":0},...]`: its map allocates a whole B-tree leaf, with room for 11 keys
/// of 24 bytes and 11 values of 32 bytes, 632 bytes, for that one member. With the object's
/// own slot in the array around it, 32 bytes that the growth of the array holds up to three
/// times over while it moves (96 bytes), that is 728 bytes for 2 values, 364 a value. An
/// object whose members are such objects costs slightly more: 373 bytes a value was the
/// largest measured, and 512 leaves room for the rounding of the allocator.
const VALUE_COST: usize = 512;

/// The most values in a body: the body's own value, every element of every array and every
/// member of every object (a member is one value, its key included). At [`VALUE_COST`] bytes
/// each, 131,072 values hold at most 64 MiB, the size of the largest body. A client's
/// `initialize`, the most structured message an editor sends, has a few thousand values.
///
/// So one frame holds at most five times [`MAX_CONTENT_LENGTH`] while it is parsed: the body
/// (64 MiB), the text of its strings (at most the body's size once parsed, and up to three
/// times that, 192 MiB, while a string with escapes grows in the parser's buffer and is then
/// copied) and its values (64 MiB). Once parsed, the body and the buffer are freed, and the
/// frame holds at most twice the limit. Without this bound, a 64 MiB body of one-member
/// objects made about 100 bytes of values for each byte read.
pub const MAX_VALUES: usize = MAX_CONTENT_LENGTH / VALUE_COST;

/// Why [`read_frame`] returned no message.
#[derive(Debug)]
pub(crate) enum Refused {
    /// The frame is past [`MAX_DEPTH`] or [`MAX_VALUES`]. Its body was read whole, so the
    /// next frame starts where it should and the input can still be read.
    Frame(String),
    /// The header or the body could not be read, or the body is not JSON: where the next
    /// frame starts is not known, and the input is read no further.
    Stream(io::Error),
}

/// How much of a body is reserved before its bytes arrive: a frame that announces a large
/// body but sends less makes the server hold only what it sent.
const INITIAL_BODY_CAPACITY: usize = 64 * 1024;

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Reads one header line without its line ending: `None` at the end of the input before any
/// byte. A line longer than [`MAX_HEADER_LINE`] is refused once it passes the bound, so no
/// more than `MAX_HEADER_LINE + 1` bytes of it are ever held.
fn read_header_line(input: &mut dyn BufRead) -> io::Result<Option<String>> {
    let too_long = || invalid(format!("a header line longer than {MAX_HEADER_LINE} bytes"));
    let mut line = Vec::new();
    let mut read_any = false;
    loop {
        let available = match input.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            if !read_any {
                return Ok(None);
            }
            break;
        }
        read_any = true;
        // Room for the bound and a `\r` that may precede the `\n`.
        let room = MAX_HEADER_LINE + 1 - line.len();
        let newline = available
            .iter()
            .take(room + 1)
            .position(|&byte| byte == b'\n');
        match newline {
            Some(at) => {
                line.extend_from_slice(available.get(..at).unwrap_or_default());
                input.consume(at + 1);
                break;
            }
            None => {
                // Past the bound, only a `\r` held last and a `\n` next keep the line.
                if room == 0 {
                    return Err(too_long());
                }
                let take = available.len().min(room);
                line.extend_from_slice(available.get(..take).unwrap_or_default());
                input.consume(take);
                if line.len() > MAX_HEADER_LINE && line.last() != Some(&b'\r') {
                    return Err(too_long());
                }
            }
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > MAX_HEADER_LINE {
        return Err(too_long());
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| invalid("a header line that is not UTF-8".to_owned()))
}

/// The length a `Content-Length` value states, or why it is refused.
fn content_length(value: &str) -> io::Result<usize> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(format!(
            "a Content-Length that is not a number: `{value}`"
        )));
    }
    match value.parse::<usize>() {
        Ok(length) if length <= MAX_CONTENT_LENGTH => Ok(length),
        _ => Err(invalid(format!(
            "a Content-Length of {value} bytes, above the limit of {MAX_CONTENT_LENGTH}"
        ))),
    }
}

/// Checks the JSON text of a body against `max_depth` and `max_values` in one pass over its
/// bytes, O(n) time and O(1) space, without making any value. Returns the number of values:
/// 1 for the body's own value, 1 for each element and 1 for each member.
///
/// The count is exact for a text that is JSON. For one that is not, it is an upper bound on
/// the values the parser makes before it stops: the parser makes an element or a member only
/// after the `[` or `{` that opens its container or after a `,`, and both readers agree on
/// where a string starts and ends up to the parser's first error.
fn check_shape(body: &[u8], max_depth: usize, max_values: usize) -> Result<usize, String> {
    let too_many = || format!("a message with more than {max_values} values (MAX_VALUES)");
    let mut depth: usize = 0;
    let mut values: usize = 1;
    // Whether the last token opened a container: its first element is not counted yet.
    let mut opened = false;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in body {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            b']' | b'}' => {
                opened = false;
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if opened {
            opened = false;
            values += 1;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max_depth {
                    return Err(format!(
                        "a message nested deeper than {max_depth} arrays and objects (MAX_DEPTH)"
                    ));
                }
                opened = true;
            }
            b',' => values += 1,
            _ => {}
        }
        if values > max_values {
            return Err(too_many());
        }
    }
    Ok(values)
}

/// Reads one message: `None` at the end of the input. A frame that is not valid JSON, or that
/// passes [`MAX_HEADER_LINE`], [`MAX_HEADER_LINES`], [`MAX_CONTENT_LENGTH`], [`MAX_DEPTH`] or
/// [`MAX_VALUES`], is an error of kind `InvalidData`; an over-limit `Content-Length` is
/// refused before any byte of the body is read or reserved, and a body past the depth or the
/// value bound before the parser makes any value.
pub fn read_message(input: &mut dyn BufRead) -> io::Result<Option<Value>> {
    read_frame(input).map_err(|refused| match refused {
        Refused::Frame(message) => invalid(message),
        Refused::Stream(error) => error,
    })
}

/// [`read_message`], telling a refusal that leaves the input readable from one that does not.
pub(crate) fn read_frame(input: &mut dyn BufRead) -> Result<Option<Value>, Refused> {
    let Some(body) = read_body(input).map_err(Refused::Stream)? else {
        return Ok(None);
    };
    check_shape(&body, MAX_DEPTH, MAX_VALUES).map_err(Refused::Frame)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| Refused::Stream(invalid(format!("a body that is not JSON: {error}"))))
}

/// Reads the header and the body of one frame: `None` at the end of the input.
fn read_body(input: &mut dyn BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length: Option<usize> = None;
    let mut lines = 0;
    loop {
        let Some(line) = read_header_line(input)? else {
            return Ok(None);
        };
        if line.is_empty() {
            break;
        }
        lines += 1;
        if lines > MAX_HEADER_LINES {
            return Err(invalid(format!(
                "a frame with more than {MAX_HEADER_LINES} header lines"
            )));
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(content_length(value)?);
        }
    }
    let Some(length) = length else {
        return Err(invalid(
            "a message without a Content-Length header".to_owned(),
        ));
    };
    let mut body = Vec::with_capacity(length.min(INITIAL_BODY_CAPACITY));
    let limit = u64::try_from(length).unwrap_or(u64::MAX);
    input.take(limit).read_to_end(&mut body)?;
    if body.len() < length {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the input ended inside a message's body",
        ));
    }
    Ok(Some(body))
}

/// Writes one message with its `Content-Length` header.
pub fn write_message(output: &mut dyn Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn frame(headers: &str, body: &str) -> Vec<u8> {
        format!("{headers}\r\n{body}").into_bytes()
    }

    fn refused(result: io::Result<Option<Value>>) -> String {
        match result {
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
                error.to_string()
            }
            Ok(value) => panic!("accepted: {value:?}"),
        }
    }

    #[test]
    fn a_frame_round_trips() {
        let mut bytes = Vec::new();
        let message = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "x" });
        write_message(&mut bytes, &message).ok();
        let mut input = Cursor::new(bytes);
        assert_eq!(read_message(&mut input).ok().flatten(), Some(message));
        assert!(matches!(read_message(&mut input), Ok(None)));
    }

    #[test]
    fn a_header_line_at_the_limit_is_accepted_and_one_byte_more_is_refused() {
        let pad = |len: usize| format!("X-Pad: {}", "a".repeat(len - "X-Pad: ".len()));
        let at = frame(
            &format!("{}\r\nContent-Length: 2\r\n", pad(MAX_HEADER_LINE)),
            "{}",
        );
        assert!(read_message(&mut Cursor::new(at)).ok().flatten().is_some());
        let over = frame(
            &format!("{}\r\nContent-Length: 2\r\n", pad(MAX_HEADER_LINE + 1)),
            "{}",
        );
        assert!(refused(read_message(&mut Cursor::new(over))).contains("header line"));
        // A bare `\n` ending counts the same way.
        let at = format!("{}\nContent-Length: 2\n\n{{}}", pad(MAX_HEADER_LINE));
        assert!(read_message(&mut Cursor::new(at)).ok().flatten().is_some());
        // An at-limit line whose `\r\n` arrives a byte at a time is still accepted.
        let at = frame(
            &format!("{}\r\nContent-Length: 2\r\n", pad(MAX_HEADER_LINE)),
            "{}",
        );
        let mut input = BufReader::with_capacity(1, Cursor::new(at));
        assert!(read_message(&mut input).ok().flatten().is_some());
        let over = frame(
            &format!("{}\r\nContent-Length: 2\r\n", pad(MAX_HEADER_LINE + 1)),
            "{}",
        );
        let mut input = BufReader::with_capacity(1, Cursor::new(over));
        assert!(refused(read_message(&mut input)).contains("header line"));
    }

    #[test]
    fn a_line_that_never_ends_is_refused_without_buffering_past_the_limit() {
        // An endless line, served a byte at a time: the reader stops at the bound.
        let mut endless = BufReader::with_capacity(1, io::repeat(b'a'));
        assert!(refused(read_message(&mut endless)).contains("header line"));
        let mut endless = BufReader::with_capacity(1 << 20, io::repeat(b'a'));
        assert!(refused(read_message(&mut endless)).contains("header line"));
    }

    #[test]
    fn header_lines_at_the_limit_are_accepted_and_one_more_is_refused() {
        let headers = |count: usize| {
            let mut text = "Content-Length: 2\r\n".to_owned();
            for _ in 1..count {
                text.push_str("X-Extra: 1\r\n");
            }
            text
        };
        let at = frame(&headers(MAX_HEADER_LINES), "{}");
        assert!(read_message(&mut Cursor::new(at)).ok().flatten().is_some());
        let over = frame(&headers(MAX_HEADER_LINES + 1), "{}");
        assert!(refused(read_message(&mut Cursor::new(over))).contains("header lines"));
    }

    #[test]
    fn a_content_length_at_the_limit_is_accepted_and_one_more_is_refused() {
        let filler = MAX_CONTENT_LENGTH - "\"\"".len();
        let body = format!("\"{}\"", "a".repeat(filler));
        let at = frame(&format!("Content-Length: {MAX_CONTENT_LENGTH}\r\n"), &body);
        let read = read_message(&mut Cursor::new(at)).ok().flatten();
        assert_eq!(
            read.as_ref().and_then(Value::as_str).map(str::len),
            Some(filler)
        );
        let over = frame(
            &format!("Content-Length: {}\r\n", MAX_CONTENT_LENGTH + 1),
            "{}",
        );
        assert!(refused(read_message(&mut Cursor::new(over))).contains("above the limit"));
    }

    #[test]
    fn an_over_limit_content_length_is_refused_before_the_body_is_read() {
        for value in [
            (MAX_CONTENT_LENGTH + 1).to_string(),
            usize::MAX.to_string(),
            "99999999999999999999999999999999".to_owned(),
        ] {
            let header = format!("Content-Length: {value}\r\n\r\n");
            let mut bytes = header.clone().into_bytes();
            bytes.extend_from_slice(b"{\"body\": \"never read\"}");
            let mut input = Cursor::new(bytes);
            assert!(refused(read_message(&mut input)).contains("above the limit"));
            // The refusal comes at the `Content-Length` line itself: neither the rest of the
            // header nor any byte of the body was read, and no body was reserved.
            let line = format!("Content-Length: {value}\r\n");
            assert_eq!(input.position(), line.len() as u64);
        }
        // A peer that announces the largest body and then sends nothing: refused at the end
        // of the input, holding only the initial reservation.
        let mut input = Cursor::new(format!("Content-Length: {MAX_CONTENT_LENGTH}\r\n\r\n"));
        let error = read_message(&mut input).err();
        assert_eq!(
            error.map(|error| error.kind()),
            Some(io::ErrorKind::UnexpectedEof)
        );
    }

    /// A frame whose body is `body`, as a client writes it.
    fn framed(body: &str) -> Vec<u8> {
        frame(&format!("Content-Length: {}\r\n", body.len()), body)
    }

    /// The values of a parsed text, as [`check_shape`] counts them.
    fn values_of(value: &Value) -> usize {
        match value {
            Value::Array(elements) => 1 + elements.iter().map(values_of).sum::<usize>(),
            Value::Object(members) => 1 + members.values().map(values_of).sum::<usize>(),
            _ => 1,
        }
    }

    #[test]
    fn the_shape_count_is_the_number_of_values_of_the_parsed_text() {
        for text in [
            "0",
            "\"a\"",
            "[]",
            "{}",
            " [ ] ",
            "[[]]",
            "[{}]",
            "[0]",
            "[0,1,2]",
            "[[0],[1,[2,3]],[]]",
            r#"{"a":0}"#,
            r#"{"a":0,"b":[1,2],"c":{"d":null,"e":{}}}"#,
            // Brackets, commas and quotes inside strings are text, not structure.
            r#"["[,{","a\"b,[","\\",",",{"k,[\"":"]}"}]"#,
            "[ 1 , true , null , \"x\" , -2.5e3 ]",
            "{\r\n\t\"a\" : [ ] ,\n\"b\" : { } }",
        ] {
            let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
            assert!(text == "null" || !parsed.is_null(), "not JSON: {text}");
            assert_eq!(
                check_shape(text.as_bytes(), MAX_DEPTH, MAX_VALUES).ok(),
                Some(values_of(&parsed)),
                "{text}"
            );
        }
        // A small bound, at it and one past it, for elements and for members.
        assert_eq!(check_shape(b"[0,1]", 1, 3), Ok(3));
        assert!(check_shape(b"[0,1,2]", 1, 3).is_err());
        assert_eq!(check_shape(br#"{"a":0,"b":1}"#, 1, 3), Ok(3));
        assert!(check_shape(br#"{"a":0,"b":1,"c":2}"#, 1, 3).is_err());
        assert_eq!(check_shape(b"[[]]", 2, 2), Ok(2));
        assert!(check_shape(b"[[[]]]", 2, 9).is_err());
        // Text that is not JSON is counted too, never under what the parser would make.
        assert!(check_shape(b"[0,0,0,0", 1, 3).is_err());
        assert!(check_shape(b"[[[[", 3, 9).is_err());
    }

    #[test]
    fn the_bounds_follow_from_the_size_of_a_value() {
        // The arithmetic of `VALUE_COST` and `MAX_VALUES` is for a value of 32 bytes; a
        // feature of `serde_json` that makes it larger must change them.
        assert!(std::mem::size_of::<Value>() <= 32);
        assert_eq!(MAX_VALUES, 131_072);
        assert_eq!(MAX_VALUES * VALUE_COST, MAX_CONTENT_LENGTH);
    }

    #[test]
    fn values_at_the_limit_are_accepted_and_one_more_is_refused() {
        // An array is one value and each element one more.
        let array = |values: usize| format!("[{}0]", "0,".repeat(values - 2));
        let read = read_message(&mut Cursor::new(framed(&array(MAX_VALUES))));
        let read = read.ok().flatten();
        assert_eq!(read.as_ref().map(values_of), Some(MAX_VALUES));
        let over = read_message(&mut Cursor::new(framed(&array(MAX_VALUES + 1))));
        assert!(refused(over).contains("more than 131072 values (MAX_VALUES)"));
        // An object is one value and each member one more; one-member objects are the
        // shape whose values cost the most memory.
        let objects = |values: usize| {
            let count = (values - 1) / 2;
            format!("[{}{{\"\":0}}]", "{\"\":0},".repeat(count - 1))
        };
        let at = objects(MAX_VALUES - 1);
        let read = read_message(&mut Cursor::new(framed(&at))).ok().flatten();
        assert_eq!(read.as_ref().map(values_of), Some(MAX_VALUES - 1));
        let over = read_message(&mut Cursor::new(framed(&objects(MAX_VALUES + 1))));
        assert!(refused(over).contains("MAX_VALUES"));
        let members = |values: usize| {
            let members: Vec<String> = (1..values).map(|n| format!("\"{n}\":0")).collect();
            format!("{{{}}}", members.join(","))
        };
        let read = read_message(&mut Cursor::new(framed(&members(MAX_VALUES))));
        assert_eq!(
            read.ok().flatten().as_ref().map(values_of),
            Some(MAX_VALUES)
        );
        let over = read_message(&mut Cursor::new(framed(&members(MAX_VALUES + 1))));
        assert!(refused(over).contains("MAX_VALUES"));
    }

    #[test]
    fn a_body_of_the_largest_size_past_the_value_limit_is_refused_before_it_is_parsed() {
        // 64 MiB of one-member objects: parsed, more than 6 GiB of values. The refusal is the
        // transport's own, from its pass over the bytes, so the parser never saw the body.
        let unit = "{\"\":0},";
        let count = (MAX_CONTENT_LENGTH - "[0]".len()) / unit.len();
        let body = format!("[{}0]", unit.repeat(count));
        assert!(body.len() <= MAX_CONTENT_LENGTH && body.len() + unit.len() > MAX_CONTENT_LENGTH);
        let mut input = Cursor::new(framed(&body));
        match read_frame(&mut input) {
            Err(Refused::Frame(message)) => assert!(message.contains("MAX_VALUES"), "{message}"),
            other => panic!("not refused as a frame: {:?}", other.map(|_| ())),
        }
        // The whole body was read: the input stands at the next frame.
        assert_eq!(input.position(), input.get_ref().len() as u64);
    }

    #[test]
    fn nesting_at_the_limit_is_accepted_and_one_level_more_is_refused() {
        let arrays = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let objects = |depth: usize| format!("{}0{}", "{\"\":".repeat(depth), "}".repeat(depth));
        for nested in [arrays, objects] {
            // The bound is the parser's own: it accepts this depth and no deeper one.
            let at = nested(MAX_DEPTH);
            assert!(serde_json::from_str::<Value>(&at).is_ok());
            assert!(serde_json::from_str::<Value>(&nested(MAX_DEPTH + 1)).is_err());
            let read = read_message(&mut Cursor::new(framed(&at)));
            assert!(read.ok().flatten().is_some());
            let over = read_message(&mut Cursor::new(framed(&nested(MAX_DEPTH + 1))));
            assert!(
                refused(over).contains("nested deeper than 127 arrays and objects (MAX_DEPTH)")
            );
        }
    }

    #[test]
    fn a_refused_shape_leaves_the_next_frame_readable_and_other_refusals_do_not() {
        let next = r#"{"jsonrpc":"2.0","method":"exit"}"#;
        for body in [
            format!("[{}0]", "0,".repeat(MAX_VALUES)),
            "[".repeat(MAX_DEPTH + 1),
        ] {
            let mut bytes = framed(&body);
            bytes.extend_from_slice(&framed(next));
            let mut input = Cursor::new(bytes);
            assert!(matches!(read_frame(&mut input), Err(Refused::Frame(_))));
            let read = read_frame(&mut input).ok().flatten();
            assert_eq!(read, serde_json::from_str::<Value>(next).ok());
        }
        for bytes in [
            framed("{not json}"),
            frame("Content-Length: 5\r\n", "{}"),
            frame(
                &format!("Content-Length: {}\r\n", MAX_CONTENT_LENGTH + 1),
                "{}",
            ),
            frame(&format!("X: {}\r\n", "a".repeat(MAX_HEADER_LINE)), "{}"),
            frame("Content-Type: x\r\n", "{}"),
        ] {
            assert!(matches!(
                read_frame(&mut Cursor::new(bytes)),
                Err(Refused::Stream(_))
            ));
        }
        let not_json = read_message(&mut Cursor::new(framed("{not json}")));
        assert!(refused(not_json).contains("a body that is not JSON"));
    }

    #[test]
    fn a_missing_or_malformed_content_length_is_refused() {
        let none = frame("Content-Type: x\r\n", "{}");
        assert!(refused(read_message(&mut Cursor::new(none))).contains("without"));
        let bad = frame("Content-Length: -2\r\n", "{}");
        assert!(refused(read_message(&mut Cursor::new(bad))).contains("not a number"));
    }
}
