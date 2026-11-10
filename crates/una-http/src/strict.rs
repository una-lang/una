//! The strict framing of the server (`spec/31 §3`, ADR 0123 rules 10 and 11): the pieces of a
//! request that two HTTP implementations might read differently are refused here, never
//! repaired.
//!
//! The client's functions in [`crate::parse`] are lenient where these are strict, and they do
//! not change: the client sends `Connection: close` and reads one response per connection, so
//! its leniency cannot desynchronize it (`spec/24 §3` rule 2). The server reads many requests
//! from one connection, so every ambiguous form is a refusal that closes it.
//!
//! Every function here is linear in the bytes it is given and examines each byte a bounded
//! number of times. Every limit is checked before the bytes it bounds are kept.

/// Why the host refuses a request, and the status it answers with. Every refusal closes the
/// connection (`spec/31 §3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// `400 Bad Request`: a form two implementations might read differently, or malformed.
    BadRequest,
    /// `413 Content Too Large`: a body over the grant's `body` limit.
    ContentTooLarge,
    /// `414 URI Too Long`: a request line over the grant's `line` limit.
    UriTooLong,
    /// `415 Unsupported Media Type`: a `Content-Encoding` other than `identity`.
    UnsupportedMediaType,
    /// `417 Expectation Failed`: an `Expect` other than `100-continue`.
    ExpectationFailed,
    /// `431 Request Header Fields Too Large`: over the grant's `headers` or `header` limit.
    HeaderFieldsTooLarge,
    /// `501 Not Implemented`: a method outside `Method`, or a transfer coding.
    NotImplemented,
    /// `505 HTTP Version Not Supported`: a version other than `HTTP/1.1` and `HTTP/1.0`.
    VersionNotSupported,
}

impl Refusal {
    /// The status code the host answers with.
    pub fn status(self) -> u16 {
        match self {
            Refusal::BadRequest => 400,
            Refusal::ContentTooLarge => 413,
            Refusal::UriTooLong => 414,
            Refusal::UnsupportedMediaType => 415,
            Refusal::ExpectationFailed => 417,
            Refusal::HeaderFieldsTooLarge => 431,
            Refusal::NotImplemented => 501,
            Refusal::VersionNotSupported => 505,
        }
    }
}

/// The two versions a server reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Version {
    /// `HTTP/1.0`.
    Http10,
    /// `HTTP/1.1`.
    Http11,
}

/// The methods of `Method` (ADR 0123 rule 1). The host answers any other with `501`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`.
    Get,
    /// `HEAD`.
    Head,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
    /// `OPTIONS`.
    Options,
}

/// Whether `byte` is a character of a token (RFC 9110 §5.6.2).
pub fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Whether `bytes` is a token: one character or more, each a `tchar`.
pub fn is_token(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|&b| is_tchar(b))
}

/// Whether `byte` is a control character a field value may not hold: below 0x20 except tab,
/// or 0x7F.
fn is_forbidden_control(byte: u8) -> bool {
    (byte < 0x20 && byte != b'\t') || byte == 0x7f
}

/// Reads one line ended by CRLF, fed in pieces as they arrive (`spec/31 §3` rule 2).
///
/// A bare LF, or a CR not followed at once by LF, is [`Refusal::BadRequest`], wherever the
/// pieces split. The line's bytes are kept only after the limit has admitted them.
#[derive(Clone, Debug, Default)]
pub struct LineReader {
    line: Vec<u8>,
    /// The last byte fed was a CR, which the next byte must follow with LF.
    cr: bool,
}

impl LineReader {
    /// A reader at the start of a line.
    pub fn new() -> LineReader {
        LineReader::default()
    }

    /// Reads from `bytes` up to the end of the current line. The line may hold at most `room`
    /// bytes, its CRLF included; a longer one is `over`, refused before its bytes are kept.
    ///
    /// Returns how many bytes of `bytes` were read, and whether the line ended; the line is
    /// then [`LineReader::take`]n. Bytes after the line's CRLF are not read.
    pub fn feed(
        &mut self,
        bytes: &[u8],
        room: usize,
        over: Refusal,
    ) -> Result<(usize, bool), Refusal> {
        if self.cr {
            return match bytes.first() {
                None => Ok((0, false)),
                Some(b'\n') => {
                    self.cr = false;
                    Ok((1, true))
                }
                Some(_) => Err(Refusal::BadRequest),
            };
        }
        let end = bytes.iter().position(|&b| b == b'\r' || b == b'\n');
        let content = bytes.get(..end.unwrap_or(bytes.len())).unwrap_or_default();
        // The line with its CRLF still to come must fit, so a line that cannot fit is refused
        // as soon as it is known, before its bytes are kept.
        if self
            .line
            .len()
            .saturating_add(content.len())
            .saturating_add(2)
            > room
        {
            return Err(over);
        }
        self.line.extend_from_slice(content);
        let Some(end) = end else {
            return Ok((bytes.len(), false));
        };
        match (bytes.get(end), bytes.get(end + 1)) {
            (Some(b'\r'), Some(b'\n')) => Ok((end + 2, true)),
            (Some(b'\r'), None) => {
                self.cr = true;
                Ok((end + 1, false))
            }
            // A bare LF, or a CR followed by anything but LF.
            _ => Err(Refusal::BadRequest),
        }
    }

    /// The line just read, without its CRLF; the reader is at the start of the next line.
    pub fn take(&mut self) -> Vec<u8> {
        self.cr = false;
        std::mem::take(&mut self.line)
    }

    /// The line just read, without its CRLF, left in place: [`LineReader::clear`] then starts
    /// the next line in the same buffer. A reader of many short lines, a chunked body's, so
    /// allocates once rather than once a line, which every serve thread would otherwise do
    /// through the one counting allocator of the process (#1780).
    pub fn line(&self) -> &[u8] {
        &self.line
    }

    /// Starts the next line, keeping the buffer, whose capacity stays below twice the largest
    /// `room` fed so far (a growing buffer may overshoot it). O(1).
    pub fn clear(&mut self) {
        self.cr = false;
        self.line.clear();
    }

    /// The bytes of the current line kept so far.
    pub fn pending(&self) -> usize {
        self.line.len() + usize::from(self.cr)
    }
}

/// What remains of the limits of a header section, which its trailers share (`spec/31 §3`
/// rule 4): lines, and bytes counted with their CRLF, the empty line that ends the section
/// included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadBudget {
    /// Header lines that may still come.
    pub lines: usize,
    /// Bytes that may still come.
    pub bytes: usize,
}

impl HeadBudget {
    /// A budget of `lines` lines and `bytes` bytes.
    pub fn new(lines: usize, bytes: usize) -> HeadBudget {
        HeadBudget { lines, bytes }
    }

    /// Charges one line of `len` bytes without its CRLF; `field` when it is a header line, not
    /// the empty line. Over either limit, [`Refusal::HeaderFieldsTooLarge`].
    pub fn charge(&mut self, len: usize, field: bool) -> Result<(), Refusal> {
        let bytes = len.saturating_add(2);
        if bytes > self.bytes || (field && self.lines == 0) {
            return Err(Refusal::HeaderFieldsTooLarge);
        }
        self.bytes -= bytes;
        if field {
            self.lines -= 1;
        }
        Ok(())
    }
}

/// A header line, without its CRLF: a token, then at once `:`, then a value trimmed of spaces
/// and tabs at both ends (`spec/31 §3` rule 3).
///
/// Refused with `400`: a line that starts with a space or tab (obsolete folding), a name that is
/// not a token, whitespace before the colon, a control character other than tab in the value,
/// and a value that is not UTF-8, which is never repaired.
pub fn header_line(line: &[u8]) -> Result<(&str, &str), Refusal> {
    let colon = line
        .iter()
        .position(|&b| b == b':')
        .ok_or(Refusal::BadRequest)?;
    let (name, value) = line.split_at(colon);
    // A name is a token, so a folded line (leading whitespace) and whitespace before the colon
    // both fail here.
    if !is_token(name) {
        return Err(Refusal::BadRequest);
    }
    let value = value.get(1..).unwrap_or_default();
    if value.iter().any(|&b| is_forbidden_control(b)) {
        return Err(Refusal::BadRequest);
    }
    let start = value
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t')
        .map_or(start, |at| at + 1);
    let value = value.get(start..end).unwrap_or_default();
    let name = std::str::from_utf8(name).map_err(|_| Refusal::BadRequest)?;
    let value = std::str::from_utf8(value).map_err(|_| Refusal::BadRequest)?;
    Ok((name, value))
}

/// The value of a `Content-Length`: ASCII digits only, 1 to 19 of them, so it fits in 64 bits
/// (`spec/31 §3` rule 4). No sign, no whitespace, no list.
pub fn content_length(value: &str) -> Result<u64, Refusal> {
    if value.is_empty() || value.len() > 19 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Refusal::BadRequest);
    }
    value.parse().map_err(|_| Refusal::BadRequest)
}

/// The value of a `Transfer-Encoding`: only `chunked`, alone, in any letter case.
///
/// Refused with `400` when malformed: empty, an empty element, an element that is not a bare
/// token (parameters included), or `chunked` named twice. Refused with `501` when it is well
/// formed but names a coding the host does not implement, such as `gzip, chunked`.
pub fn transfer_encoding(value: &str) -> Result<(), Refusal> {
    if value.eq_ignore_ascii_case("chunked") {
        return Ok(());
    }
    let mut chunked = 0usize;
    for element in value.split(',') {
        let element = element.trim_matches([' ', '\t']);
        if !is_token(element.as_bytes()) {
            return Err(Refusal::BadRequest);
        }
        if element.eq_ignore_ascii_case("chunked") {
            chunked += 1;
        }
    }
    if chunked > 1 {
        return Err(Refusal::BadRequest);
    }
    Err(Refusal::NotImplemented)
}

/// The headers that belong to the connection, not to the request (ADR 0123 rule 2), in
/// lower case. The host removes them before the program sees the request, and they are the
/// only names besides `close` and `keep-alive` a `Connection` header may list.
pub const CONNECTION_HEADERS: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
];

/// What a `Connection` header asks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectionOptions {
    /// It lists `close`.
    pub close: bool,
    /// It lists `keep-alive`.
    pub keep_alive: bool,
}

/// Reads the value of a `Connection` header into `options` (`spec/31 §3` rule 3): a list of
/// tokens, compared in any letter case, each `close` or one of [`CONNECTION_HEADERS`].
/// Anything else is `400`, so a peer can never make the host remove a header that a front
/// proxy added, or `host`. An empty element is `400` too.
pub fn connection(value: &str, options: &mut ConnectionOptions) -> Result<(), Refusal> {
    for element in value.split(',') {
        let element = element.trim_matches([' ', '\t']);
        if !is_token(element.as_bytes()) {
            return Err(Refusal::BadRequest);
        }
        if element.eq_ignore_ascii_case("close") {
            options.close = true;
        } else if element.eq_ignore_ascii_case("keep-alive") {
            options.keep_alive = true;
        } else if !CONNECTION_HEADERS
            .iter()
            .any(|name| element.eq_ignore_ascii_case(name))
        {
            return Err(Refusal::BadRequest);
        }
    }
    Ok(())
}

/// How a request's body is framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyFraming {
    /// Neither header: an empty body.
    Empty,
    /// `Content-Length` bytes, at least one.
    Length(u64),
    /// `Transfer-Encoding: chunked`.
    Chunked,
}

/// The framing of a body from the values of its `Content-Length` and `Transfer-Encoding`
/// headers, every one received, in order (`spec/31 §3` rule 4, the defense against smuggling).
///
/// In this order: both headers, `400`; `Transfer-Encoding` in an `HTTP/1.0` request, `400`; two
/// `Transfer-Encoding` headers, `400`; a value of [`transfer_encoding`] it refuses; two
/// `Content-Length` headers, even equal, `400`; a value of [`content_length`] it refuses; a
/// length above `max_body`, `413`, before any byte of the body is read.
pub fn body_framing(
    version: Version,
    content_lengths: &[&str],
    transfer_encodings: &[&str],
    max_body: u64,
) -> Result<BodyFraming, Refusal> {
    match (content_lengths, transfer_encodings) {
        ([], []) => Ok(BodyFraming::Empty),
        ([_, ..], [_, ..]) => Err(Refusal::BadRequest),
        ([], [value]) => {
            if version == Version::Http10 {
                return Err(Refusal::BadRequest);
            }
            transfer_encoding(value)?;
            Ok(BodyFraming::Chunked)
        }
        ([], [_, _, ..]) => Err(Refusal::BadRequest),
        ([value], []) => {
            let length = content_length(value)?;
            if length > max_body {
                return Err(Refusal::ContentTooLarge);
            }
            Ok(if length == 0 {
                BodyFraming::Empty
            } else {
                BodyFraming::Length(length)
            })
        }
        ([_, _, ..], []) => Err(Refusal::BadRequest),
    }
}

/// The size of a chunk-size line, without its CRLF: 1 to 16 hex digits alone, with no
/// whitespace, sign or extension (`spec/31 §3` rule 4). Sixteen hex digits always fit in 64
/// bits.
pub fn chunk_size(line: &[u8]) -> Result<u64, Refusal> {
    if line.is_empty() || line.len() > MAX_CHUNK_DIGITS || !line.iter().all(u8::is_ascii_hexdigit) {
        return Err(Refusal::BadRequest);
    }
    let digits = std::str::from_utf8(line).map_err(|_| Refusal::BadRequest)?;
    u64::from_str_radix(digits, 16).map_err(|_| Refusal::BadRequest)
}

/// The most hex digits of a chunk size.
pub const MAX_CHUNK_DIGITS: usize = 16;

/// Where a chunked body is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chunk {
    /// Reading a chunk-size line.
    Size,
    /// Reading the data of a chunk, this many bytes more.
    Data(u64),
    /// Reading the CRLF after a chunk's data.
    DataEnd,
    /// Reading trailers, up to the empty line.
    Trailers,
    /// The body is complete.
    Done,
}

/// A chunked body, fed bytes as they arrive (`spec/31 §3` rule 4).
///
/// Strict where the client's reader is lenient: a chunk size is [`chunk_size`]; the line after
/// a chunk's data is empty; trailers are header lines of [`header_line`], read and dropped,
/// whose lines and bytes count against what remains of the header section's limits, not a
/// fresh allowance. A body that grows past its limit is `413` at the chunk-size line that
/// would pass it, before any byte of that chunk is kept.
///
/// **Costs.** O(b) for b bytes fed, each examined a bounded number of times. Its size lines,
/// data ends and trailers share one line buffer, kept between lines, so feeding allocates only
/// when that buffer or the body itself grows: O(log b) times, however many chunks the body
/// has, rather than once a chunk (#1780).
#[derive(Clone, Debug)]
pub struct ChunkedBody {
    state: Chunk,
    lines: LineReader,
    max_body: u64,
    /// The bytes of body read so far.
    length: u64,
    trailers: HeadBudget,
}

impl ChunkedBody {
    /// A chunked body of at most `max_body` bytes, whose trailers may use `trailers`: what
    /// remains of the header section's limits.
    pub fn new(max_body: u64, trailers: HeadBudget) -> ChunkedBody {
        ChunkedBody {
            state: Chunk::Size,
            lines: LineReader::new(),
            max_body,
            length: 0,
            trailers,
        }
    }

    /// Whether the body is complete, its trailers read.
    pub fn done(&self) -> bool {
        self.state == Chunk::Done
    }

    /// The bytes of body read so far.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Reads from `bytes`, appending the body's data to `body`. Returns how many bytes were
    /// read: all of them, or, once the body is complete, those up to its end, so the bytes of
    /// a next request are left for it.
    pub fn feed(&mut self, bytes: &[u8], body: &mut Vec<u8>) -> Result<usize, Refusal> {
        let mut read = 0usize;
        while read < bytes.len() {
            let rest = bytes.get(read..).unwrap_or_default();
            match self.state {
                Chunk::Done => break,
                Chunk::Data(left) => {
                    let take = usize::try_from(left).unwrap_or(usize::MAX).min(rest.len());
                    body.extend_from_slice(rest.get(..take).unwrap_or_default());
                    let took = u64::try_from(take).unwrap_or(u64::MAX);
                    self.length = self.length.saturating_add(took);
                    self.state = if took == left {
                        Chunk::DataEnd
                    } else {
                        Chunk::Data(left - took)
                    };
                    read += take;
                }
                Chunk::Size => {
                    let (used, ended) =
                        self.lines
                            .feed(rest, MAX_CHUNK_DIGITS + 2, Refusal::BadRequest)?;
                    read += used;
                    if ended {
                        let size = chunk_size(self.lines.line())?;
                        self.lines.clear();
                        if self.length.saturating_add(size) > self.max_body {
                            return Err(Refusal::ContentTooLarge);
                        }
                        self.state = if size == 0 {
                            Chunk::Trailers
                        } else {
                            Chunk::Data(size)
                        };
                    }
                }
                Chunk::DataEnd => {
                    let (used, ended) = self.lines.feed(rest, 2, Refusal::BadRequest)?;
                    read += used;
                    if ended {
                        // The room of 2 admits only an empty line.
                        self.lines.clear();
                        self.state = Chunk::Size;
                    }
                }
                Chunk::Trailers => {
                    let (used, ended) = self.lines.feed(
                        rest,
                        self.trailers.bytes,
                        Refusal::HeaderFieldsTooLarge,
                    )?;
                    read += used;
                    if ended {
                        let line = self.lines.line();
                        self.trailers.charge(line.len(), !line.is_empty())?;
                        if line.is_empty() {
                            self.state = Chunk::Done;
                        } else {
                            // Read and dropped, but valid like any header line.
                            header_line(line)?;
                        }
                        self.lines.clear();
                    }
                }
            }
        }
        Ok(read)
    }
}

/// The parsed request line (`spec/31 §3` rule 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestLine<'a> {
    /// The method.
    pub method: Method,
    /// The request target, as received; [`target`] reads it.
    pub target: &'a str,
    /// The version.
    pub version: Version,
}

/// A request line without its CRLF: `method SP target SP version`, single spaces, nothing
/// before the method (`spec/31 §3` rule 1).
///
/// Refused, in this order: an empty line (no empty line may come before a request), a line
/// that is not three parts split by single spaces, a method that is not a token, or a target
/// with a character outside the printable ASCII, `400`; a version other than `HTTP/1.1` and
/// `HTTP/1.0`, `505`; a token that is not a method of `Method` in upper case, `501`, `CONNECT`
/// and `TRACE` included. The target's own form is [`target`]'s to check.
pub fn request_line(line: &[u8]) -> Result<RequestLine<'_>, Refusal> {
    let mut parts = line.split(|&b| b == b' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Refusal::BadRequest);
    };
    if !is_token(method) || target.is_empty() || !target.iter().all(|&b| b > 0x20 && b < 0x7f) {
        return Err(Refusal::BadRequest);
    }
    let version = match version {
        b"HTTP/1.1" => Version::Http11,
        b"HTTP/1.0" => Version::Http10,
        _ => return Err(Refusal::VersionNotSupported),
    };
    let method = match method {
        b"GET" => Method::Get,
        b"HEAD" => Method::Head,
        b"POST" => Method::Post,
        b"PUT" => Method::Put,
        b"PATCH" => Method::Patch,
        b"DELETE" => Method::Delete,
        b"OPTIONS" => Method::Options,
        _ => return Err(Refusal::NotImplemented),
    };
    let target = std::str::from_utf8(target).map_err(|_| Refusal::BadRequest)?;
    Ok(RequestLine {
        method,
        target,
        version,
    })
}

/// Whether `byte` may stand unencoded in a path segment: an RFC 3986 `pchar` other than a
/// percent-encoding (`unreserved`, `sub-delims`, `:` and `@`).
fn is_pchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&byte)
}

/// Checks that `part` holds only `allowed` bytes and percent-encodings of two hex digits.
fn check_encoded(part: &[u8], allowed: impl Fn(u8) -> bool) -> Result<(), Refusal> {
    let mut bytes = part.iter();
    while let Some(&byte) = bytes.next() {
        if byte == b'%' {
            match (bytes.next(), bytes.next()) {
                (Some(high), Some(low)) if high.is_ascii_hexdigit() && low.is_ascii_hexdigit() => {}
                _ => return Err(Refusal::BadRequest),
            }
        } else if !allowed(byte) {
            return Err(Refusal::BadRequest);
        }
    }
    Ok(())
}

/// A target in origin-form, checked: its path and its query, still encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginForm<'a> {
    /// The path, starting with `/`.
    pub path: &'a str,
    /// The query, after `?`, when there is one.
    pub query: Option<&'a str>,
}

/// A target in origin-form (`spec/31 §4` rules 1 and 2): `/`, then path characters, then optionally
/// `?` and query characters; only the characters RFC 3986 allows there, and each `%` followed
/// by two hex digits. Anything else is `400`: a `#`, a space, `|`, `{`, `"`, `\`, a byte above
/// 0x7E. The asterisk-form and the authority-form are `400`; the absolute-form is the
/// request reader's, which rewrites an admissible one to origin-form first.
pub fn origin_form(target: &str) -> Result<OriginForm<'_>, Refusal> {
    if !target.starts_with('/') {
        return Err(Refusal::BadRequest);
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    };
    check_encoded(path.as_bytes(), |b| b == b'/' || is_pchar(b))?;
    if let Some(query) = query {
        check_encoded(query.as_bytes(), |b| b == b'/' || b == b'?' || is_pchar(b))?;
    }
    Ok(OriginForm { path, query })
}

/// The value of a hex digit; the caller has checked it is one.
fn hex(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        b'A'..=b'F' => digit - b'A' + 10,
        _ => 0,
    }
}

/// Percent-decodes `part`, whose encodings [`check_encoded`] has checked; with `plus`, a `+`
/// decodes to a space first, as in a query.
fn percent_decode(part: &[u8], plus: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(part.len());
    let mut bytes = part.iter();
    while let Some(&byte) = bytes.next() {
        match byte {
            b'%' => {
                let high = bytes.next().copied().map_or(0, hex);
                let low = bytes.next().copied().map_or(0, hex);
                out.push(high << 4 | low);
            }
            b'+' if plus => out.push(b' '),
            _ => out.push(byte),
        }
    }
    out
}

/// The segments of a checked path (`spec/31 §4` rule 3): the path after its first `/`, split on
/// `/`, each segment percent-decoded alone, so an encoded `/` never makes a new segment.
///
/// Refused with `400`: a segment that decodes to `.` or `..`, or that holds `/`, `\`, NUL or
/// another control character, or that is not UTF-8 after decoding. `/` gives `[""]`, and
/// `/a//b` gives `["a", "", "b"]`.
pub fn segments(path: &str) -> Result<Vec<String>, Refusal> {
    let rest = path.strip_prefix('/').ok_or(Refusal::BadRequest)?;
    rest.split('/')
        .map(|segment| {
            let decoded = percent_decode(segment.as_bytes(), false);
            if decoded == b"." || decoded == b".." {
                return Err(Refusal::BadRequest);
            }
            if decoded
                .iter()
                .any(|&b| b == b'/' || b == b'\\' || b < 0x20 || b == 0x7f)
            {
                return Err(Refusal::BadRequest);
            }
            String::from_utf8(decoded).map_err(|_| Refusal::BadRequest)
        })
        .collect()
}

/// The parameters of a checked query (`spec/31 §4` rule 4): split on `&`, empty parts skipped;
/// each part `name=value` split at its first `=`, or `name` with the value `""`; `+` decodes
/// to a space, then percent-decoding. A name or value that is not UTF-8 after decoding is
/// `400`.
pub fn query(query: &str) -> Result<Vec<(String, String)>, Refusal> {
    let decode = |part: &str| {
        String::from_utf8(percent_decode(part.as_bytes(), true)).map_err(|_| Refusal::BadRequest)
    };
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            Ok((decode(name)?, decode(value)?))
        })
        .collect()
}

#[cfg(test)]
mod tests;
