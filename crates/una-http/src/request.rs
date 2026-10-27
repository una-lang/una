//! The server's reader of one request, fed bytes as they arrive, with no I/O (`spec/31 §3`
//! and §4, ADR 0123 rules 8, 10 and 11).
//!
//! The reader is built on the strict framing of [`crate::strict`]: it refuses every form two
//! implementations might read differently, and every refusal closes the connection. It reads
//! one request and stops at its end, so the bytes of a pipelined next request are left for the
//! next reader (rule 12: one request at a time).
//!
//! Reading is O(b) for a request of b bytes, each byte examined a bounded number of times, and
//! every limit is checked before the bytes it bounds are kept.

use crate::strict::{
    self, BodyFraming, ChunkedBody, ConnectionOptions, HeadBudget, LineReader, Method, OriginForm,
    Refusal, Version,
};

/// The limits of a request, from the grant's items of ADR 0123 rule 7, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestLimits {
    /// The request line, its CRLF included (`line`).
    pub line: usize,
    /// Header lines, trailers included (`headers`).
    pub headers: usize,
    /// Bytes of the header section, trailers included (`header`).
    pub header: usize,
    /// Bytes of the body (`body`).
    pub body: u64,
}

impl Default for RequestLimits {
    /// The defaults of ADR 0123 rule 7: 8 KiB, 100 lines, 16 KiB and 1024 KiB.
    fn default() -> RequestLimits {
        RequestLimits {
            line: 8 * 1024,
            headers: 100,
            header: 16 * 1024,
            body: 1024 * 1024,
        }
    }
}

/// The scheme of the grant the request came through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// `http`.
    Http,
    /// `https`.
    Https,
}

/// A `Host` value, read by ADR 0123 rule 8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// The name, in ASCII lower case, without its port, and without one trailing `.` of a
    /// registered name; an IPv6 address keeps its brackets.
    pub name: String,
    /// The port, when the value has one.
    pub port: Option<u16>,
    /// Whether the name is an IP literal, IPv4 or IPv6.
    pub ip: bool,
}

/// A request, as the host read it: the plain owned data the program's `Request` is built from
/// (ADR 0123 rules 1 and 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The method.
    pub method: Method,
    /// The version.
    pub version: Version,
    /// The target in origin-form; an absolute-form target is rewritten to it (`spec/31 §4`).
    pub target: String,
    /// The path, split on `/` and decoded segment by segment.
    pub segments: Vec<String>,
    /// The query, decoded, in order.
    pub query: Vec<(String, String)>,
    /// The request's own headers, names in ASCII lower case, in the order received, without
    /// the headers that belong to the connection.
    pub headers: Vec<(String, String)>,
    /// The whole body, UTF-8.
    pub body: String,
    /// The `Host` value, read.
    pub host: Host,
    /// The connection closes after the reply: the request said `Connection: close`, or it is
    /// `HTTP/1.0` (rule 12).
    pub close: bool,
}

/// Where the reader is.
#[derive(Clone, Debug)]
enum Stage {
    /// Reading the request line.
    Line,
    /// Reading header lines, up to the empty line.
    Headers,
    /// Reading a body of this many more bytes.
    Length(u64),
    /// Reading a chunked body.
    Chunked(Box<ChunkedBody>),
    /// The request is read.
    Done,
}

/// The reader of one request.
#[derive(Clone, Debug)]
pub struct RequestReader {
    limits: RequestLimits,
    scheme: Scheme,
    stage: Stage,
    lines: LineReader,
    budget: HeadBudget,
    method: Method,
    version: Version,
    target: String,
    /// The header lines as received: the name as sent, and the trimmed value.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// What the head decided, once it is read.
    head: Option<Head>,
}

/// What the end of the head decided.
#[derive(Clone, Debug)]
struct Head {
    framing: BodyFraming,
    connection: ConnectionOptions,
    expect_continue: bool,
    host: Host,
    origin: String,
    segments: Vec<String>,
    query: Vec<(String, String)>,
}

/// How far a feed got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// The bytes of the feed that were read. Once the request is complete, the rest belong to
    /// the next request.
    pub used: usize,
    /// Whether the request is complete.
    pub done: bool,
}

impl RequestReader {
    /// A reader of one request through a grant of `scheme`, with `limits`.
    pub fn new(limits: RequestLimits, scheme: Scheme) -> RequestReader {
        RequestReader {
            limits,
            scheme,
            stage: Stage::Line,
            lines: LineReader::new(),
            budget: HeadBudget::new(limits.headers, limits.header),
            method: Method::Get,
            version: Version::Http11,
            target: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
            head: None,
        }
    }

    /// Whether any byte of the request has been read, which starts the `read` timer (rule 12).
    pub fn started(&self) -> bool {
        !matches!(self.stage, Stage::Line) || self.lines.pending() > 0
    }

    /// The body's framing, once the head is read and admitted.
    pub fn framing(&self) -> Option<BodyFraming> {
        self.head.as_ref().map(|head| head.framing)
    }

    /// Whether the host owes `100 Continue` once it admits the body: an `HTTP/1.1` request
    /// with `Expect: 100-continue` and a body (rule 10).
    pub fn expects_continue(&self) -> bool {
        self.head
            .as_ref()
            .is_some_and(|head| head.expect_continue && !matches!(head.framing, BodyFraming::Empty))
    }

    /// The bytes of body read so far.
    pub fn body_length(&self) -> usize {
        self.body.len()
    }

    /// Reads from `bytes`, which follow the bytes fed before.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Progress, Refusal> {
        let mut used = 0usize;
        while used < bytes.len() && !matches!(self.stage, Stage::Done) {
            let rest = bytes.get(used..).unwrap_or_default();
            used += match self.stage {
                Stage::Done => 0,
                Stage::Line => self.line_bytes(rest)?,
                Stage::Headers => self.header_bytes(rest)?,
                Stage::Length(left) => {
                    let take = usize::try_from(left).unwrap_or(usize::MAX).min(rest.len());
                    self.body
                        .extend_from_slice(rest.get(..take).unwrap_or_default());
                    let took = u64::try_from(take).unwrap_or(u64::MAX);
                    self.stage = if took == left {
                        Stage::Done
                    } else {
                        Stage::Length(left - took)
                    };
                    take
                }
                Stage::Chunked(ref mut chunked) => {
                    let read = chunked.feed(rest, &mut self.body)?;
                    if chunked.done() {
                        self.stage = Stage::Done;
                    }
                    read
                }
            };
        }
        Ok(Progress {
            used,
            done: matches!(self.stage, Stage::Done),
        })
    }

    /// The request, once complete: its body checked as UTF-8, its target decoded.
    pub fn finish(self) -> Result<Request, Refusal> {
        let (Stage::Done, Some(head)) = (self.stage, self.head) else {
            return Err(Refusal::BadRequest);
        };
        let body = String::from_utf8(self.body).map_err(|_| Refusal::BadRequest)?;
        let headers = self
            .headers
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value))
            .filter(|(name, _)| !strict::CONNECTION_HEADERS.contains(&name.as_str()))
            .collect();
        Ok(Request {
            method: self.method,
            version: self.version,
            close: head.connection.close || self.version == Version::Http10,
            target: head.origin,
            segments: head.segments,
            query: head.query,
            headers,
            body,
            host: head.host,
        })
    }

    /// Bytes of the request line.
    fn line_bytes(&mut self, bytes: &[u8]) -> Result<usize, Refusal> {
        let (used, ended) = self
            .lines
            .feed(bytes, self.limits.line, Refusal::UriTooLong)?;
        if ended {
            let line = self.lines.take();
            let read = strict::request_line(&line)?;
            // An origin-form target is checked at once; an absolute-form one needs `Host`.
            if read.target.starts_with('/') {
                strict::origin_form(read.target)?;
            } else if absolute_form(read.target).is_none() {
                return Err(Refusal::BadRequest);
            }
            self.method = read.method;
            self.version = read.version;
            self.target = read.target.to_owned();
            self.stage = Stage::Headers;
        }
        Ok(used)
    }

    /// Bytes of the header section.
    fn header_bytes(&mut self, bytes: &[u8]) -> Result<usize, Refusal> {
        let (used, ended) =
            self.lines
                .feed(bytes, self.budget.bytes, Refusal::HeaderFieldsTooLarge)?;
        if ended {
            let line = self.lines.take();
            self.budget.charge(line.len(), !line.is_empty())?;
            if line.is_empty() {
                self.end_of_head()?;
            } else {
                let (name, value) = strict::header_line(&line)?;
                if confusable_name(name) {
                    return Err(Refusal::BadRequest);
                }
                self.headers.push((name.to_owned(), value.to_owned()));
            }
        }
        Ok(used)
    }

    /// The checks of the whole head, in this order: framing, `Connection`, `Host`, the
    /// target and its decoding, `Content-Encoding`, `Expect`. All of them come before any
    /// byte of the body is read.
    fn end_of_head(&mut self) -> Result<(), Refusal> {
        let values = |wanted: &str| -> Vec<&str> {
            self.headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case(wanted))
                .map(|(_, value)| value.as_str())
                .collect()
        };
        let framing = strict::body_framing(
            self.version,
            &values("content-length"),
            &values("transfer-encoding"),
            self.limits.body,
        )?;
        let mut connection = ConnectionOptions::default();
        for value in values("connection") {
            strict::connection(value, &mut connection)?;
        }
        let host = match values("host").as_slice() {
            [value] => host(value)?,
            _ => return Err(Refusal::BadRequest),
        };
        let origin = if self.target.starts_with('/') {
            self.target.clone()
        } else {
            let hosts = values("host");
            let sent = hosts.first().copied().unwrap_or_default();
            rewrite_absolute(&self.target, self.scheme, sent)?
        };
        let OriginForm { path, query } = strict::origin_form(&origin)?;
        let segments = strict::segments(path)?;
        let query = match query {
            Some(query) => strict::query(query)?,
            None => Vec::new(),
        };
        for value in values("content-encoding") {
            if !value.eq_ignore_ascii_case("identity") {
                return Err(Refusal::UnsupportedMediaType);
            }
        }
        let mut expect_continue = false;
        if self.version == Version::Http11 {
            for value in values("expect") {
                if !value.eq_ignore_ascii_case("100-continue") {
                    return Err(Refusal::ExpectationFailed);
                }
                expect_continue = true;
            }
        }
        self.stage = match framing {
            BodyFraming::Empty => Stage::Done,
            BodyFraming::Length(length) => Stage::Length(length),
            BodyFraming::Chunked => {
                Stage::Chunked(Box::new(ChunkedBody::new(self.limits.body, self.budget)))
            }
        };
        self.head = Some(Head {
            framing,
            connection,
            expect_continue,
            host,
            origin,
            segments,
            query,
        });
        Ok(())
    }
}

/// Whether a header name differs from a name that frames the request or belongs to the
/// connection only by `_` for `-`, such as `Transfer_Encoding`: a proxy that maps one to the
/// other would read a framing this reader does not, so it is refused (`spec/31 §3` rule 3).
fn confusable_name(name: &str) -> bool {
    name.contains('_')
        && strict::CONNECTION_HEADERS
            .iter()
            .chain(&["host", "content-encoding", "expect"])
            .any(|known| {
                known.len() == name.len()
                    && known
                        .bytes()
                        .zip(name.bytes())
                        .all(|(k, n)| k == n.to_ascii_lowercase() || (k == b'-' && n == b'_'))
            })
}

/// The scheme and the rest of an absolute-form target, when it is one: `http://` or
/// `https://`, in any letter case.
fn absolute_form(target: &str) -> Option<(Scheme, &str)> {
    let colon = target.find("://")?;
    let (scheme, rest) = target.split_at(colon);
    let rest = rest.get(3..)?;
    if scheme.eq_ignore_ascii_case("http") {
        Some((Scheme::Http, rest))
    } else if scheme.eq_ignore_ascii_case("https") {
        Some((Scheme::Https, rest))
    } else {
        None
    }
}

/// The origin-form of an absolute-form target (`spec/31 §4` rule 1): accepted only when its
/// scheme is the grant's and its authority equals `Host`, in ASCII letter case; then its path
/// and query, with `/` for an empty path.
fn rewrite_absolute(target: &str, grant: Scheme, sent_host: &str) -> Result<String, Refusal> {
    let (scheme, rest) = absolute_form(target).ok_or(Refusal::BadRequest)?;
    if scheme != grant {
        return Err(Refusal::BadRequest);
    }
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.is_empty() || !authority.eq_ignore_ascii_case(sent_host) {
        return Err(Refusal::BadRequest);
    }
    Ok(if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    })
}

/// Reads a `Host` value (ADR 0123 rule 8): a `uri-host` and an optional `:` with a port of 1
/// to 5 digits, at most 65535. The host is a registered name of letters, digits, `-`, `_` and
/// `.`, an IPv4 address in dotted-quad form, or an IPv6 address in brackets. Anything else is
/// `400`: an empty value or name, userinfo, a path, an empty port, a zone, and a registered
/// name whose last label is numeric but which is not an IPv4 address, such as `127.1` or
/// `0x7f.0.0.1`, which resolvers read as addresses in different ways.
pub fn host(value: &str) -> Result<Host, Refusal> {
    let (name, port) = if let Some(rest) = value.strip_prefix('[') {
        let close = rest.find(']').ok_or(Refusal::BadRequest)?;
        let (inner, after) = rest.split_at(close);
        let after = after.get(1..).unwrap_or_default();
        inner
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| Refusal::BadRequest)?;
        let port = match after {
            "" => None,
            _ => Some(port(after.strip_prefix(':').ok_or(Refusal::BadRequest)?)?),
        };
        return Ok(Host {
            name: format!("[{}]", inner.to_ascii_lowercase()),
            port,
            ip: true,
        });
    } else {
        match value.split_once(':') {
            Some((name, digits)) => (name, Some(port(digits)?)),
            None => (value, None),
        }
    };
    if name.parse::<std::net::Ipv4Addr>().is_ok() {
        return Ok(Host {
            name: name.to_owned(),
            port,
            ip: true,
        });
    }
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return Err(Refusal::BadRequest);
    }
    let last = name.rsplit('.').next().unwrap_or_default();
    let numeric = !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit());
    let hex = last.len() > 2 && (last.starts_with("0x") || last.starts_with("0X"));
    if numeric || hex || name.split('.').any(str::is_empty) {
        return Err(Refusal::BadRequest);
    }
    Ok(Host {
        name: name.to_ascii_lowercase(),
        port,
        ip: false,
    })
}

/// A port of 1 to 5 digits, at most 65535.
fn port(digits: &str) -> Result<u16, Refusal> {
    if digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Refusal::BadRequest);
    }
    digits.parse().map_err(|_| Refusal::BadRequest)
}

#[cfg(test)]
mod tests;
