//! An HTTP/1.1 response parser fed bytes as they arrive (`spec/24 §3`, §4): it holds no socket,
//! so the same bytes give the same response however they are split.
//!
//! Every step is linear in the bytes it is fed: the parser scans each byte once, and keeps the
//! body in one growing buffer that the limits of `spec/24 §4` bound.

/// The limits of a response (`spec/24 §4` rule 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most headers of one response.
    pub max_headers: usize,
    /// The most bytes of the status line and headers together.
    pub max_header_bytes: usize,
    /// The most bytes of the body.
    pub max_body: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_headers: 100,
            max_header_bytes: 64 * 1024,
            max_body: 16 * 1024 * 1024,
        }
    }
}

/// Why the bytes are not a response the client may return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not valid HTTP/1.1 as `spec/24 §3` reads it, or more headers than the limits allow.
    Protocol,
    /// A body longer than the limit.
    TooLarge,
}

/// A parsed response, with its body as bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The headers, as received, in order.
    pub headers: Vec<(String, String)>,
    /// The body, after its transfer coding.
    pub body: Vec<u8>,
}

/// How the body ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    /// After this many more bytes.
    Length(usize),
    /// By chunks.
    Chunked,
    /// When the connection closes.
    Close,
}

/// Where the parser is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Reading the status line and the headers, up to the empty line.
    Head,
    /// Reading the body, framed so.
    Body(Framing),
    /// Reading a chunk-size line.
    ChunkSize,
    /// Reading the data of a chunk, this many bytes more.
    ChunkData(usize),
    /// Reading the line end after a chunk's data.
    ChunkEnd,
    /// Reading trailers, after the last chunk, up to the empty line.
    Trailers,
    /// The response is complete.
    Done,
}

/// The parser of one response.
#[derive(Debug)]
pub struct Parser {
    limits: Limits,
    state: State,
    /// The bytes of the line being read: the head, a chunk-size line, or the trailers.
    line: Vec<u8>,
    /// The bytes of head and trailers read so far, against `max_header_bytes`.
    head_bytes: usize,
    status: u16,
    /// The 1xx responses read so far, at most [`MAX_INTERIM`].
    interim: usize,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// The most 1xx responses before the final one; a server that sends more is stalling.
const MAX_INTERIM: usize = 10;

impl Parser {
    /// A parser with `limits`.
    pub fn new(limits: Limits) -> Parser {
        Parser {
            limits,
            state: State::Head,
            line: Vec::new(),
            head_bytes: 0,
            status: 0,
            interim: 0,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Whether the response is complete: the client may stop reading.
    pub fn done(&self) -> bool {
        self.state == State::Done
    }

    /// Reads `bytes`, the next ones from the connection. Bytes after a complete response are
    /// ignored, since the client sends `Connection: close`.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut rest = bytes;
        while !rest.is_empty() {
            rest = match self.state {
                State::Done => return Ok(()),
                State::Head | State::Trailers => self.head_byte(rest)?,
                State::ChunkSize | State::ChunkEnd => self.chunk_line_byte(rest)?,
                State::ChunkData(left) => {
                    let take = left.min(rest.len());
                    let (data, after) = rest.split_at(take);
                    self.push_body(data)?;
                    self.state = if left == take {
                        State::ChunkEnd
                    } else {
                        State::ChunkData(left - take)
                    };
                    after
                }
                State::Body(Framing::Length(left)) => {
                    let take = left.min(rest.len());
                    let (data, after) = rest.split_at(take);
                    self.push_body(data)?;
                    self.state = if left == take {
                        State::Done
                    } else {
                        State::Body(Framing::Length(left - take))
                    };
                    after
                }
                State::Body(Framing::Close) => {
                    self.push_body(rest)?;
                    &[]
                }
                State::Body(Framing::Chunked) => {
                    self.state = State::ChunkSize;
                    rest
                }
            };
        }
        Ok(())
    }

    /// The response, when the connection has closed: complete, or delimited by the close.
    pub fn finish(mut self) -> Result<Response, Error> {
        if self.state == State::Body(Framing::Close) {
            self.state = State::Done;
        }
        if self.state != State::Done {
            return Err(Error::Protocol);
        }
        Ok(Response {
            status: self.status,
            headers: self.headers,
            body: self.body,
        })
    }

    fn push_body(&mut self, data: &[u8]) -> Result<(), Error> {
        if self.body.len() + data.len() > self.limits.max_body {
            return Err(Error::TooLarge);
        }
        self.body.extend_from_slice(data);
        Ok(())
    }

    /// One byte of the head or the trailers, counted against their limit; at each line end,
    /// the line.
    fn head_byte<'b>(&mut self, bytes: &'b [u8]) -> Result<&'b [u8], Error> {
        let Some((&byte, rest)) = bytes.split_first() else {
            return Ok(bytes);
        };
        self.head_bytes += 1;
        if self.head_bytes > self.limits.max_header_bytes {
            return Err(Error::Protocol);
        }
        if byte != b'\n' {
            self.line.push(byte);
            return Ok(rest);
        }
        // A line ends with CRLF; a bare LF is not HTTP/1.1.
        if self.line.pop() != Some(b'\r') {
            return Err(Error::Protocol);
        }
        let line = std::mem::take(&mut self.line);
        if self.state == State::Trailers {
            // Trailers are read and dropped; their names and values still have to be valid.
            if line.is_empty() {
                self.state = State::Done;
            } else {
                header(&line)?;
            }
            return Ok(rest);
        }
        if self.status == 0 {
            self.status = status_line(&line)?;
        } else if line.is_empty() {
            self.end_of_head()?;
        } else {
            if self.headers.len() == self.limits.max_headers {
                return Err(Error::Protocol);
            }
            self.headers.push(header(&line)?);
        }
        Ok(rest)
    }

    /// After the empty line of a head: the next response for an informational one, else the
    /// body's framing.
    fn end_of_head(&mut self) -> Result<(), Error> {
        if (100..200).contains(&self.status) {
            // A 1xx response precedes the real one, whose head counts anew.
            self.interim += 1;
            if self.status == 101 || self.interim > MAX_INTERIM {
                return Err(Error::Protocol);
            }
            self.status = 0;
            self.headers.clear();
            self.head_bytes = 0;
            return Ok(());
        }
        let mut length: Option<usize> = None;
        let mut chunked = false;
        for (name, value) in &self.headers {
            if name.eq_ignore_ascii_case("content-length") {
                let parsed = content_length(value)?;
                if length.is_some_and(|known| known != parsed) {
                    return Err(Error::Protocol);
                }
                length = Some(parsed);
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                // Only `chunked`, once: the client asks for no other coding.
                if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                    return Err(Error::Protocol);
                }
                chunked = true;
            }
        }
        // A 204 or 304 has no body, whatever its headers say (RFC 9112 §6.3 rule 1).
        if self.status == 204 || self.status == 304 {
            self.state = State::Done;
            return Ok(());
        }
        self.state = match (chunked, length) {
            (true, Some(_)) => return Err(Error::Protocol),
            (true, None) => State::Body(Framing::Chunked),
            (false, Some(0)) => State::Done,
            (false, Some(n)) => {
                if n > self.limits.max_body {
                    return Err(Error::TooLarge);
                }
                State::Body(Framing::Length(n))
            }
            (false, None) => State::Body(Framing::Close),
        };
        Ok(())
    }

    /// One byte of a chunk-size line, or of the line end after a chunk's data.
    fn chunk_line_byte<'b>(&mut self, bytes: &'b [u8]) -> Result<&'b [u8], Error> {
        let Some((&byte, rest)) = bytes.split_first() else {
            return Ok(bytes);
        };
        // A chunk-size line is short; a long one is an attack, not a size.
        if self.line.len() > 1024 {
            return Err(Error::Protocol);
        }
        if byte != b'\n' {
            self.line.push(byte);
            return Ok(rest);
        }
        if self.line.pop() != Some(b'\r') {
            return Err(Error::Protocol);
        }
        let line = std::mem::take(&mut self.line);
        if self.state == State::ChunkEnd {
            if !line.is_empty() {
                return Err(Error::Protocol);
            }
            self.state = State::ChunkSize;
            return Ok(rest);
        }
        let size = chunk_size(&line)?;
        if self.body.len().saturating_add(size) > self.limits.max_body {
            return Err(Error::TooLarge);
        }
        self.state = if size == 0 {
            self.head_bytes = 0;
            State::Trailers
        } else {
            State::ChunkData(size)
        };
        Ok(rest)
    }
}

/// The status code of a status line: `HTTP/1.1` or `HTTP/1.0`, a space, three digits, and a
/// space before an optional reason.
fn status_line(line: &[u8]) -> Result<u16, Error> {
    let rest = line
        .strip_prefix(b"HTTP/1.1 ")
        .or_else(|| line.strip_prefix(b"HTTP/1.0 "))
        .ok_or(Error::Protocol)?;
    let (code, reason) = rest.split_at(rest.len().min(3));
    if code.len() != 3 || !code.iter().all(u8::is_ascii_digit) {
        return Err(Error::Protocol);
    }
    if !reason.is_empty() && reason.first() != Some(&b' ') {
        return Err(Error::Protocol);
    }
    if reason.iter().any(|&b| b < 0x20 && b != b'\t' || b == 0x7f) {
        return Err(Error::Protocol);
    }
    let code = std::str::from_utf8(code).map_err(|_| Error::Protocol)?;
    let status: u16 = code.parse().map_err(|_| Error::Protocol)?;
    if status < 100 {
        return Err(Error::Protocol);
    }
    Ok(status)
}

/// A header line: a token, a colon, and a value with no control characters but tabs. Folded
/// lines, obsolete in HTTP/1.1, are not headers.
fn header(line: &[u8]) -> Result<(String, String), Error> {
    let colon = line
        .iter()
        .position(|&b| b == b':')
        .ok_or(Error::Protocol)?;
    let (name, value) = line.split_at(colon);
    let value = value.get(1..).unwrap_or_default();
    let token = |b: &u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(b);
    if name.is_empty() || !name.iter().all(token) {
        return Err(Error::Protocol);
    }
    if value.iter().any(|&b| b < 0x20 && b != b'\t' || b == 0x7f) {
        return Err(Error::Protocol);
    }
    let name = String::from_utf8(name.to_vec()).map_err(|_| Error::Protocol)?;
    let value = String::from_utf8_lossy(value)
        .trim_matches([' ', '\t'])
        .to_owned();
    Ok((name, value))
}

/// The value of a `Content-Length`: decimal digits only, within the address space.
fn content_length(value: &str) -> Result<usize, Error> {
    let value = value.trim();
    if value.is_empty() || value.len() > 19 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Protocol);
    }
    value.parse().map_err(|_| Error::Protocol)
}

/// The size of a chunk-size line: hex digits, then an optional extension after `;`, which is
/// ignored.
fn chunk_size(line: &[u8]) -> Result<usize, Error> {
    // Hex digits alone, with no whitespace around them (RFC 9112 §7.1).
    let digits = line.split(|&b| b == b';').next().unwrap_or_default();
    if digits.is_empty() || digits.len() > 15 || !digits.iter().all(u8::is_ascii_hexdigit) {
        return Err(Error::Protocol);
    }
    let digits = std::str::from_utf8(digits).map_err(|_| Error::Protocol)?;
    usize::from_str_radix(digits, 16).map_err(|_| Error::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The response of `bytes` fed in pieces at `splits`, then closed.
    fn parse_split(bytes: &[u8], splits: &[usize], limits: Limits) -> Result<Response, Error> {
        let mut parser = Parser::new(limits);
        let mut from = 0;
        for &at in splits.iter().chain(std::iter::once(&bytes.len())) {
            let at = at.clamp(from, bytes.len());
            parser.feed(bytes.get(from..at).unwrap_or_default())?;
            from = at;
        }
        parser.finish()
    }

    fn parse(bytes: &[u8]) -> Result<Response, Error> {
        parse_split(bytes, &[], Limits::default())
    }

    #[test]
    fn framings_give_the_body() {
        let length =
            parse(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-A: b \r\n\r\nhello").unwrap();
        assert_eq!(length.status, 200);
        assert_eq!(
            length.headers,
            vec![
                ("Content-Length".to_owned(), "5".to_owned()),
                ("X-A".to_owned(), "b".to_owned())
            ]
        );
        assert_eq!(length.body, b"hello");
        let chunked = parse(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;ext=1\r\nhello\r\n6\r\n world\r\n0\r\nX-T: 1\r\n\r\n",
        )
        .unwrap();
        assert_eq!(chunked.body, b"hello world");
        let closed = parse(b"HTTP/1.0 404 Not Found\r\n\r\ngone").unwrap();
        assert_eq!((closed.status, closed.body.as_slice()), (404, &b"gone"[..]));
        let empty = parse(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
        assert!(empty.body.is_empty());
        let informational = parse(
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(informational.status, 201);
    }

    #[test]
    fn malformed_responses_are_protocol_errors() {
        for bad in [
            &b""[..],
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel",
            b"HTTP/2 200 OK\r\n\r\n",
            b"HTTP/1.1 20 OK\r\n\r\n",
            b"HTTP/1.1 200OK\r\n\r\n",
            b"HTTP/1.1 200 OK\nContent-Length: 0\n\n",
            b"HTTP/1.1 200 OK\r\nBad Name: x\r\n\r\n",
            b"HTTP/1.1 200 OK\r\n folded: x\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nX: a\x01b\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nab",
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n-1\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabcX\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n",
            b"HTTP/1.1 101 Switching Protocols\r\n\r\n",
        ] {
            assert_eq!(
                parse(bad),
                Err(Error::Protocol),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn the_details_of_rfc_9112_hold() {
        // A 204 or 304 has no body, whatever its headers say.
        for head in [
            "HTTP/1.1 204 No Content\r\nTransfer-Encoding: chunked\r\n\r\n",
            "HTTP/1.1 304 Not Modified\r\nTransfer-Encoding: chunked\r\n\r\n",
            "HTTP/1.1 304 Not Modified\r\nContent-Length: 5\r\n\r\n",
            "HTTP/1.1 204 No Content\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n",
        ] {
            let mut parser = Parser::new(Limits::default());
            parser.feed(head.as_bytes()).unwrap();
            let response = parser.finish().unwrap();
            assert!(response.body.is_empty(), "{head}");
        }
        // Ten 1xx responses before the final one, and no more.
        let interim = "HTTP/1.1 103 Early Hints\r\nLink: </a>\r\n\r\n";
        let last = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let ten = format!("{}{last}", interim.repeat(10));
        assert_eq!(parse(ten.as_bytes()).unwrap().status, 200);
        let eleven = format!("{}{last}", interim.repeat(11));
        assert_eq!(parse(eleven.as_bytes()), Err(Error::Protocol));
        // A chunk size is hex digits alone: no whitespace around them.
        for line in ["5 ", " 5", "5\t", "5 ;ext", "\t5;ext"] {
            let bad = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{line}\r\nhello\r\n0\r\n\r\n"
            );
            assert_eq!(parse(bad.as_bytes()), Err(Error::Protocol), "{line:?}");
        }
    }

    #[test]
    fn limits_are_enforced_as_bytes_arrive() {
        let limits = Limits {
            max_headers: 2,
            max_header_bytes: 64,
            max_body: 4,
        };
        let many = b"HTTP/1.1 200 OK\r\nA: 1\r\nB: 2\r\nC: 3\r\n\r\n";
        assert_eq!(parse_split(many, &[], limits), Err(Error::Protocol));
        let long = format!("HTTP/1.1 200 OK\r\nA: {}\r\n\r\n", "x".repeat(100));
        assert_eq!(
            parse_split(long.as_bytes(), &[], limits),
            Err(Error::Protocol)
        );
        // A declared length over the limit fails before any byte of the body.
        let declared = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n";
        assert_eq!(parse_split(declared, &[], limits), Err(Error::TooLarge));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n3\r\n";
        assert_eq!(parse_split(chunked, &[], limits), Err(Error::TooLarge));
        let closed = b"HTTP/1.1 200 OK\r\n\r\nabcde";
        assert_eq!(parse_split(closed, &[], limits), Err(Error::TooLarge));
    }

    /// A small deterministic generator, so the property tests need no crate.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % (n as u64).max(1)).unwrap_or(0)
        }
    }

    /// A random well-formed response, and its body.
    fn response(rng: &mut Rng) -> (Vec<u8>, Vec<u8>) {
        let body: Vec<u8> = (0..rng.below(40))
            .map(|_| b"abc \r\n{}0"[rng.below(9)])
            .collect();
        // 204 and 304 have no body, so the generator never picks them.
        let status = [200, 201, 202, 400, 404, 500][rng.below(6)];
        let mut out = format!("HTTP/1.1 {status} R\r\n").into_bytes();
        for i in 0..rng.below(4) {
            out.extend_from_slice(format!("X-H{i}: v{}\r\n", rng.below(100)).as_bytes());
        }
        match rng.below(3) {
            0 => {
                out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
                out.extend_from_slice(&body);
            }
            1 => {
                out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
                let mut rest = body.as_slice();
                while !rest.is_empty() {
                    let (chunk, after) = rest.split_at(rng.below(rest.len()) + 1);
                    out.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                    out.extend_from_slice(chunk);
                    out.extend_from_slice(b"\r\n");
                    rest = after;
                }
                out.extend_from_slice(b"0\r\n");
                if rng.below(2) == 0 {
                    out.extend_from_slice(b"X-Trailer: t\r\n");
                }
                out.extend_from_slice(b"\r\n");
            }
            _ => {
                out.extend_from_slice(b"\r\n");
                out.extend_from_slice(&body);
            }
        }
        (out, body)
    }

    #[test]
    fn every_split_parses_as_the_whole() {
        // A response split at every byte boundary, and at random sets of them, parses as
        // in one piece, valid or not.
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..300 {
            let (bytes, body) = response(&mut rng);
            let whole = parse(&bytes);
            assert_eq!(whole.as_ref().map(|r| r.body.clone()), Ok(body));
            for at in 0..=bytes.len() {
                assert_eq!(parse_split(&bytes, &[at], Limits::default()), whole);
            }
            let mut splits: Vec<usize> = (0..rng.below(8))
                .map(|_| rng.below(bytes.len() + 1))
                .collect();
            splits.sort_unstable();
            assert_eq!(parse_split(&bytes, &splits, Limits::default()), whole);
        }
    }

    #[test]
    fn mutated_responses_never_panic_and_split_alike() {
        // Random mutations of valid responses (bytes replaced, inserted or removed, and
        // truncations) give a response or an error, the same whatever the splits.
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..2000 {
            let (mut bytes, _) = response(&mut rng);
            for _ in 0..rng.below(4) + 1 {
                let at = rng.below(bytes.len());
                match rng.below(4) {
                    0 => bytes[at] = b"0aZ \r\n:;-\x00\xff"[rng.below(11)],
                    1 => bytes.insert(at, b"0\r\n:"[rng.below(4)]),
                    2 => {
                        bytes.remove(at);
                    }
                    _ => bytes.truncate(at),
                }
                if bytes.is_empty() {
                    break;
                }
            }
            let whole = parse(&bytes);
            for _ in 0..4 {
                let at = rng.below(bytes.len() + 1);
                assert_eq!(parse_split(&bytes, &[at], Limits::default()), whole);
            }
        }
    }
}
