//! The registry's HTTP/1.1 endpoints (ADR 0122 rules 34, 36.2 and 43; `spec/30 §13`), on the
//! standard library alone.
//!
//! **Reading** (`GET`) serves files of the store as they are, at the paths of rule 34, and
//! nothing else: `root/<n>`, `checkpoint`, `checkpoint/<size>`, `tile/…`, `map/…`,
//! `blobs/sha256/<hex>`, and `witness/next` for witnesses. Nothing is computed per request,
//! so any web server or mirror can serve the same files.
//!
//! **Writing** (`POST`) takes a submission and answers the registry's signed promise:
//! - `/publish`: `ita-publish/1`, a line feed, the release statement's length in decimal and a
//!   line feed, the signed statement, then the archive;
//! - `/submit`: a signed yank, advisory or key event;
//! - `/register`: an owner's registration, only with the operator's bearer token, after the
//!   operator verified the account; `Ita-Review: accepted` releases a name held for review;
//! - `/cosign`: a checkpoint's size in decimal, a line feed, and a witness's cosignature line.
//!
//! **Limits, before allocation.** A request's head is at most 16 KiB and 64 header lines, read
//! within 30 seconds; a body needs `Content-Length`, at most the endpoint's bound (64 KiB for a
//! note, the archive's bound for a publication), and is read with no more than 60 seconds
//! without progress and at least 64 KiB per second on average past its first minute. Chunked
//! bodies, percent-encoding, queries, repeated `Content-Length` and folded headers are refused.
//! Each connection serves one request. At most `max_connections` are served at once, and at
//! most two publications are checked at once, since each holds its archive in memory.
//!
//! TLS is not here: the service is meant to sit behind a terminator that serves HTTPS
//! (rule 39: TLS keeps a private registry's credentials confidential; integrity rests on
//! digests, signatures and proofs).

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::registry::{Refusal, Registry};
use crate::store::{Store, check_path};
use crate::{MAX_ARCHIVE, MAX_ENTRY, decimal};

/// The largest request head.
pub const MAX_HEAD: usize = 16 << 10;
/// The most header lines.
pub const MAX_HEADERS: usize = 64;
/// The largest body of a note submission.
pub const MAX_NOTE_BODY: usize = 64 << 10;
/// The largest body of a publication: its framing, the statement, and the archive.
pub const MAX_PUBLISH_BODY: usize = 64 + MAX_ENTRY + MAX_ARCHIVE;
/// The largest body of a cosignature.
pub const MAX_COSIGN_BODY: usize = 2 << 10;
/// The largest file served.
const MAX_SERVED: usize = MAX_ARCHIVE;
/// How many publications are checked at once.
const MAX_PUBLISHING: usize = 2;

/// How the endpoints run.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The operator's bearer token for `/register`; registration is closed without one.
    pub register_token: Option<String>,
    /// How many connections are served at once.
    pub max_connections: usize,
    /// The time to read a request's head.
    pub head_timeout: Duration,
    /// The longest wait for progress on a body.
    pub idle_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig {
            register_token: None,
            max_connections: 256,
            head_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(60),
        }
    }
}

/// A request, read and checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// `GET` or `POST`.
    pub method: String,
    /// The path, without its leading `/`.
    pub path: String,
    /// The headers, names in lower case, in order.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// Its content type.
    pub content_type: &'static str,
    /// Whether the file never changes, so caches may keep it.
    pub immutable: bool,
    /// The body.
    pub body: Vec<u8>,
}

fn text(status: u16, message: impl Into<String>) -> Response {
    let mut body = message.into().into_bytes();
    body.push(b'\n');
    Response {
        status,
        content_type: "text/plain; charset=utf-8",
        immutable: false,
        body,
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

/// The registry behind its endpoints.
pub struct Service {
    store: Arc<dyn Store>,
    registry: Arc<Mutex<Registry>>,
    config: ServerConfig,
    connections: AtomicUsize,
    publishing: AtomicUsize,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service").finish_non_exhaustive()
    }
}

/// Releases a counted slot when dropped.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn take(counter: &AtomicUsize, max: usize) -> Option<Slot<'_>> {
    let previous = counter.fetch_add(1, Ordering::SeqCst);
    if previous >= max {
        counter.fetch_sub(1, Ordering::SeqCst);
        return None;
    }
    Some(Slot(counter))
}

/// Whether two byte strings are equal, in time that depends only on their lengths.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Which files a `GET` may read: the paths of rule 34.
fn servable(path: &str) -> Option<bool> {
    if check_path(path).is_err() {
        return None;
    }
    let segments: Vec<&str> = path.split('/').collect();
    match segments.as_slice() {
        ["checkpoint"] | ["witness", "next"] => Some(false),
        ["checkpoint", size] if decimal(size).is_some() => Some(false),
        ["root", n] if decimal(n).is_some_and(|n| n >= 1) => Some(true),
        ["blobs", "sha256", hex]
            if hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            Some(true)
        }
        ["tile", ..] | ["map", ..] => Some(true),
        _ => None,
    }
}

impl Service {
    /// The endpoints of `registry`, serving the files of `store`, which is the registry's.
    pub fn new(store: Arc<dyn Store>, registry: Registry, config: ServerConfig) -> Service {
        Service {
            store,
            registry: Arc::new(Mutex::new(registry)),
            config,
            connections: AtomicUsize::new(0),
            publishing: AtomicUsize::new(0),
        }
    }

    /// The registry, for an operator's tools and tests.
    pub fn registry(&self) -> &Mutex<Registry> {
        &self.registry
    }

    /// The bound of a request's body, by its method and path, `None` for a request that has
    /// no body.
    fn body_limit(method: &str, path: &str) -> Option<usize> {
        match (method, path) {
            ("POST", "publish") => Some(MAX_PUBLISH_BODY),
            ("POST", "submit" | "register") => Some(MAX_NOTE_BODY),
            ("POST", "cosign") => Some(MAX_COSIGN_BODY),
            _ => None,
        }
    }

    /// Answers a request that was read and checked.
    pub fn route(&self, request: Request) -> Response {
        match request.method.as_str() {
            "GET" => self.read(&request.path),
            "POST" => self.write(request),
            _ => text(405, "only GET and POST are served"),
        }
    }

    fn read(&self, path: &str) -> Response {
        let Some(immutable) = servable(path) else {
            return text(404, "not found");
        };
        match self.store.get(path, MAX_SERVED) {
            Ok(Some(body)) => Response {
                status: 200,
                content_type: if path.starts_with("tile/")
                    || path.starts_with("map/")
                    || path.starts_with("blobs/")
                {
                    "application/octet-stream"
                } else {
                    "text/plain; charset=utf-8"
                },
                immutable,
                body,
            },
            Ok(None) => text(404, "not found"),
            Err(_) => text(503, "the store cannot be read"),
        }
    }

    fn refused(refusal: &Refusal) -> Response {
        match refusal {
            Refusal::Invalid(m) => text(400, m.clone()),
            Refusal::Held(m) => text(409, m.clone()),
            Refusal::Unavailable(m) => text(503, m.clone()),
        }
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Registry>, Response> {
        self.registry
            .lock()
            .map_err(|_| text(503, "the registry stopped after an internal error"))
    }

    fn write(&self, request: Request) -> Response {
        match request.path.as_str() {
            "publish" => {
                let Some(_slot) = take(&self.publishing, MAX_PUBLISHING) else {
                    return text(503, "too many publications are being checked; try again");
                };
                let mut body = request.body;
                let (note, offset) = match publication(&body) {
                    Ok(found) => found,
                    Err(m) => return text(400, m),
                };
                let note = note.to_vec();
                body.drain(..offset);
                let prepared = match Registry::prepare(&note, body) {
                    Ok(prepared) => prepared,
                    Err(refusal) => return Service::refused(&refusal),
                };
                let mut registry = match self.locked() {
                    Ok(registry) => registry,
                    Err(response) => return response,
                };
                match registry.publish(prepared) {
                    Ok(receipt) => text(200, receipt.promise.trim_end().to_owned()),
                    Err(refusal) => Service::refused(&refusal),
                }
            }
            "submit" => {
                let mut registry = match self.locked() {
                    Ok(registry) => registry,
                    Err(response) => return response,
                };
                match registry.submit(&request.body) {
                    Ok(receipt) => text(200, receipt.promise.trim_end().to_owned()),
                    Err(refusal) => Service::refused(&refusal),
                }
            }
            "register" => {
                let Some(token) = &self.config.register_token else {
                    return text(403, "registration is closed");
                };
                let given = request
                    .header("authorization")
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .unwrap_or_default();
                if !same_secret(given.as_bytes(), token.as_bytes()) {
                    return text(401, "registration needs the operator's token");
                }
                let reviewed = request.header("ita-review") == Some("accepted");
                let mut registry = match self.locked() {
                    Ok(registry) => registry,
                    Err(response) => return response,
                };
                match registry.register(&request.body, reviewed) {
                    Ok(receipt) => text(200, receipt.promise.trim_end().to_owned()),
                    Err(refusal) => Service::refused(&refusal),
                }
            }
            "cosign" => {
                let Ok(body) = std::str::from_utf8(&request.body) else {
                    return text(400, "a cosignature is text");
                };
                let Some((size, line)) = body.split_once('\n') else {
                    return text(400, "a cosignature is `<size>`, a line feed, and its line");
                };
                let Some(size) = decimal(size) else {
                    return text(400, "the checkpoint's size is not a decimal");
                };
                let mut registry = match self.locked() {
                    Ok(registry) => registry,
                    Err(response) => return response,
                };
                match registry.cosign(size, line) {
                    Ok(()) => text(200, "cosigned"),
                    Err(refusal) => Service::refused(&refusal),
                }
            }
            _ => text(404, "not found"),
        }
    }

    /// Serves one connection: reads one request within the limits, answers it, and closes.
    pub fn handle(&self, mut stream: TcpStream) {
        let response = match take(&self.connections, self.config.max_connections) {
            None => text(503, "too many connections; try again"),
            Some(_slot) => match read_request(&mut stream, &self.config) {
                Ok(request) => self.route(request),
                Err(response) => response,
            },
        };
        let _ = write_response(&mut stream, &response);
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

/// Splits a publication's body into the statement and the offset of the archive.
fn publication(body: &[u8]) -> Result<(&[u8], usize), String> {
    let magic = b"ita-publish/1\n";
    let rest = body
        .strip_prefix(magic)
        .ok_or("a publication starts with `ita-publish/1`")?;
    let newline = rest
        .iter()
        .take(21)
        .position(|b| *b == b'\n')
        .ok_or("a publication gives its statement's length on its second line")?;
    let length = std::str::from_utf8(rest.get(..newline).unwrap_or_default())
        .ok()
        .and_then(decimal)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n <= MAX_ENTRY)
        .ok_or("the statement's length is not a decimal within the bound of an entry")?;
    let start = magic.len() + newline + 1;
    let end = start
        .checked_add(length)
        .filter(|end| *end <= body.len())
        .ok_or("the statement's length runs past the body")?;
    Ok((body.get(start..end).unwrap_or_default(), end))
}

/// Reads a request from `stream` within the limits of the module, or the response that
/// refuses it.
pub fn read_request(stream: &mut TcpStream, config: &ServerConfig) -> Result<Request, Response> {
    let started = Instant::now();
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(at) = find(&head, b"\r\n\r\n") {
            break at;
        }
        if head.len() > MAX_HEAD {
            return Err(text(431, "the request's head is too large"));
        }
        let left = config
            .head_timeout
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| text(408, "the request's head took too long"))?;
        let _ = stream.set_read_timeout(Some(left));
        match stream.read(&mut chunk) {
            Ok(0) => return Err(text(400, "the connection closed inside the request's head")),
            Ok(n) => head.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e) if is_timeout(&e) => return Err(text(408, "the request's head took too long")),
            Err(_) => return Err(text(400, "the request could not be read")),
        }
    };
    if end > MAX_HEAD {
        return Err(text(431, "the request's head is too large"));
    }
    let (method, path, headers) = parse_head(head.get(..end).unwrap_or_default())?;
    let already = head.get(end + 4..).unwrap_or_default().to_vec();
    let limit = Service::body_limit(&method, &path);
    let length = match headers
        .iter()
        .filter(|(n, _)| n == "content-length")
        .count()
    {
        0 => None,
        1 => Some(
            headers
                .iter()
                .find(|(n, _)| n == "content-length")
                .and_then(|(_, v)| decimal(v))
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| text(400, "`Content-Length` is not a decimal"))?,
        ),
        _ => return Err(text(400, "`Content-Length` is repeated")),
    };
    let body = match (limit, length) {
        (None, None | Some(0)) if already.is_empty() => Vec::new(),
        (None, _) => return Err(text(400, "this request has no body")),
        (Some(_), None) => return Err(text(411, "a body needs `Content-Length`")),
        (Some(limit), Some(length)) if length > limit => {
            return Err(text(
                413,
                format!("the body is larger than the {limit} bytes this endpoint takes"),
            ));
        }
        (Some(_), Some(length)) => read_body(stream, already, length, config)?,
    };
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn read_body(
    stream: &mut TcpStream,
    already: Vec<u8>,
    length: usize,
    config: &ServerConfig,
) -> Result<Vec<u8>, Response> {
    if already.len() > length {
        return Err(text(400, "the body is longer than `Content-Length`"));
    }
    let started = Instant::now();
    let deadline = config.idle_timeout + Duration::from_secs((length / (64 << 10)) as u64);
    let mut body = Vec::with_capacity(length);
    body.extend_from_slice(&already);
    let mut chunk = vec![0u8; 64 << 10];
    let _ = stream.set_read_timeout(Some(config.idle_timeout));
    while body.len() < length {
        if started.elapsed() > deadline {
            return Err(text(408, "the body took too long"));
        }
        let want = (length - body.len()).min(chunk.len());
        match stream.read(chunk.get_mut(..want).unwrap_or_default()) {
            Ok(0) => return Err(text(400, "the connection closed inside the body")),
            Ok(n) => body.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e) if is_timeout(&e) => return Err(text(408, "the body made no progress")),
            Err(_) => return Err(text(400, "the body could not be read")),
        }
    }
    Ok(body)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A request's head: its method, its target and its headers, in order.
pub type RequestHead = (String, String, Vec<(String, String)>);

/// Reads a request's head: its line and its headers, strictly.
pub fn parse_head(head: &[u8]) -> Result<RequestHead, Response> {
    let head =
        std::str::from_utf8(head).map_err(|_| text(400, "the request's head is not text"))?;
    let mut lines = head.split("\r\n");
    let line = lines.next().unwrap_or_default();
    let parts: Vec<&str> = line.split(' ').collect();
    let [method, target, version] = parts.as_slice() else {
        return Err(text(400, "the request line is `METHOD /path HTTP/1.1`"));
    };
    if !matches!(*version, "HTTP/1.1" | "HTTP/1.0") {
        return Err(text(400, "only HTTP/1.1 is served"));
    }
    if !matches!(*method, "GET" | "POST") {
        return Err(text(405, "only GET and POST are served"));
    }
    let path = target
        .strip_prefix('/')
        .ok_or_else(|| text(400, "the path starts with `/`"))?;
    if !path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
    {
        return Err(text(400, "the path has a character it may not have"));
    }
    let mut headers = Vec::new();
    for line in lines {
        if headers.len() >= MAX_HEADERS {
            return Err(text(431, "the request has too many headers"));
        }
        if line.starts_with([' ', '\t']) {
            return Err(text(400, "folded headers are refused"));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| text(400, "a header is `Name: value`"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            return Err(text(400, "a header's name is not a token"));
        }
        let value = value.trim_matches([' ', '\t']);
        if value.chars().any(|c| c.is_control()) {
            return Err(text(400, "a header's value has a control character"));
        }
        let name = name.to_ascii_lowercase();
        if name == "transfer-encoding" {
            return Err(text(501, "only bodies with `Content-Length` are taken"));
        }
        headers.push((name, value.to_owned()));
    }
    Ok(((*method).to_owned(), path.to_owned(), headers))
}

/// Writes a response and its headers.
pub fn write_response(stream: &mut impl Write, response: &Response) -> io::Result<()> {
    let cache = if response.immutable && response.status == 200 {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// Serves `listener` until it fails, one thread per connection.
pub fn serve(listener: &TcpListener, service: &Arc<Service>) -> io::Result<()> {
    for stream in listener.incoming() {
        let stream = stream?;
        let service = Arc::clone(service);
        std::thread::Builder::new()
            .name("itaty-connection".to_owned())
            .spawn(move || service.handle(stream))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn heads_are_read_strictly() {
        let (m, p, h) = parse_head(b"GET /tile/0/000 HTTP/1.1\r\nHost: x").unwrap();
        assert_eq!((m.as_str(), p.as_str(), h.len()), ("GET", "tile/0/000", 1));
        for bad in [
            &b"GET /a?b HTTP/1.1"[..],
            b"GET /a%2e HTTP/1.1",
            b"GET a HTTP/1.1",
            b"PUT /a HTTP/1.1",
            b"GET /a HTTP/2",
            b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked",
            b"GET /a HTTP/1.1\r\n folded",
            b"GET /a HTTP/1.1\r\nBad Name: x",
            b"GET /a  HTTP/1.1",
        ] {
            assert!(parse_head(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn only_the_files_of_rule_34_are_served() {
        for (path, immutable) in [
            ("checkpoint", Some(false)),
            ("checkpoint/12", Some(false)),
            ("witness/next", Some(false)),
            ("root/1", Some(true)),
            ("tile/0/000", Some(true)),
            ("map/3/ab", Some(true)),
        ] {
            assert_eq!(servable(path), immutable, "{path}");
        }
        for path in [
            "state/size",
            "root/0",
            "checkpoint/012",
            "blobs/sha256/AB",
            "tile/../state",
            "",
            "x",
        ] {
            assert_eq!(servable(path), None, "{path}");
        }
    }

    #[test]
    fn publications_frame_their_statement() {
        let body = b"ita-publish/1\n3\nabcARCHIVE";
        let (note, offset) = publication(body).unwrap();
        assert_eq!((note, &body[offset..]), (&b"abc"[..], &b"ARCHIVE"[..]));
        assert!(publication(b"ita-publish/1\n03\nabc").is_err());
        assert!(publication(b"ita-publish/1\n9\nabc").is_err());
        assert!(publication(b"ita-publish/2\n3\nabc").is_err());
    }

    #[test]
    fn secrets_compare_whole() {
        assert!(same_secret(b"token", b"token"));
        assert!(!same_secret(b"token", b"tokem"));
        assert!(!same_secret(b"token", b"toke"));
    }
}
