//! JSON-RPC 2.0 messages over a byte stream, framed by a `Content-Length` header, as LSP
//! sends them (ADR 0063 rule 2).
//!
//! The peer on the input is not trusted with memory: a header line, the number of header
//! lines and the body each have a fixed upper bound, and input past one of them is refused
//! with an error of kind `InvalidData` before anything of its size is buffered or allocated.

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

/// Reads one message: `None` at the end of the input. A frame that is not valid JSON, or that
/// passes [`MAX_HEADER_LINE`], [`MAX_HEADER_LINES`] or [`MAX_CONTENT_LENGTH`], is an error of
/// kind `InvalidData`; an over-limit `Content-Length` is refused before any byte of the body
/// is read or reserved.
pub fn read_message(input: &mut dyn BufRead) -> io::Result<Option<Value>> {
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
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
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

    #[test]
    fn a_missing_or_malformed_content_length_is_refused() {
        let none = frame("Content-Type: x\r\n", "{}");
        assert!(refused(read_message(&mut Cursor::new(none))).contains("without"));
        let bad = frame("Content-Length: -2\r\n", "{}");
        assert!(refused(read_message(&mut Cursor::new(bad))).contains("not a number"));
    }
}
