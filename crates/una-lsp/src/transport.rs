//! JSON-RPC 2.0 messages over a byte stream, framed by a `Content-Length` header, as LSP
//! sends them (ADR 0063 rule 2).

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// Reads one message: `None` at the end of the input. A frame that is not valid JSON is an
/// error of kind `InvalidData`.
pub fn read_message(input: &mut dyn BufRead) -> io::Result<Option<Value>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().ok();
        }
    }
    let Some(length) = length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a message without a Content-Length header",
        ));
    };
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
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
