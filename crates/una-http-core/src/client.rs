//! Sending a request (`spec/24 §3`, §4): HTTP/1.1, directly to the origin, over TCP or TLS,
//! within one deadline for the whole request, its redirects included.
//!
//! TLS is the client that the process installs ([`install_tls`]): `una-http` holds it, over
//! `rustls`, so that this crate, and the runtime library that executables without `Http` link,
//! carry no TLS code (ADR 0074 H7, #1102). With none installed, an `https` request fails with
//! [`Error::Tls`] once connected, before any byte of it is sent.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use crate::grant::{Grant, Method, Url, allowed};
use crate::parse::{self, Limits, Parser};

/// Why a request got no response it may return (`spec/24 §1` rule 3), as `HttpError` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No grant allows the URL and method, or the key named for the credential is unknown.
    Denied,
    /// The URL is not an absolute `http` or `https` URL, or it has user info.
    BadUrl,
    /// A header of the request is malformed, or names one the client sets itself.
    BadRequest,
    /// Name resolution or the connection failed.
    Unreachable,
    /// The request passed its deadline.
    TimedOut,
    /// The response passed a limit.
    TooLarge,
    /// The TLS handshake or the certificate failed.
    Tls,
    /// The response is not valid HTTP/1.1, or its body is not UTF-8.
    Protocol,
    /// A header value has a control character other than a tab. It is not an `HttpError`: the
    /// value may be `internal`, so the engines abort the run with U6004 instead.
    HeaderValue,
}

/// A request, as an operation of `Http` makes it.
#[derive(Clone, Copy, Debug)]
pub struct Request<'r> {
    /// The method.
    pub method: Method,
    /// The URL, as the program wrote it.
    pub url: &'r str,
    /// The program's headers, in order.
    pub headers: &'r [(String, String)],
    /// The body; empty for `get` and `delete`.
    pub body: &'r [u8],
    /// The key that `key_name` names, sent as a bearer token.
    pub bearer: Option<&'r [u8]>,
}

/// A response, as `Response` holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The headers, as received, in order.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: String,
}

/// The settings of a run: limits and the deadline of each request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The limits of a response.
    pub limits: Limits,
    /// The total wall-clock time of a request, its redirects included.
    pub timeout: Duration,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            limits: Limits::default(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// The longest `--http-timeout`, a day, in seconds (`spec/24 §4`).
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

/// The most redirects followed in a row (`spec/24 §2` rule 3).
const MAX_REDIRECTS: usize = 5;

/// A TLS connection over a connected socket, as [`Tls::connect`] makes it.
pub trait TlsStream: Read + Write {
    /// The socket under the connection, whose timeouts bound each read and write.
    fn socket(&self) -> &TcpStream;
}

/// The TLS client of `https` requests (`spec/24 §3` rule 4), which a process installs once with
/// [`install_tls`].
pub trait Tls: Sync {
    /// The TLS connection to the server `name` over `stream`, its handshake done before
    /// `deadline`: [`Error::BadUrl`] when `name` is not a server name, [`Error::Tls`] when the
    /// handshake or the certificate fails, and [`Error::TimedOut`] past the deadline.
    fn connect(
        &self,
        name: &str,
        stream: TcpStream,
        deadline: Instant,
    ) -> Result<Box<dyn TlsStream>, Error>;

    /// Whether `error`, of a connection this client made, is a failure of TLS.
    fn failed(&self, error: &std::io::Error) -> bool;
}

/// The TLS client of the process, once installed.
static TLS: OnceLock<&'static dyn Tls> = OnceLock::new();

/// Installs `tls` as the TLS client of every later `https` request of the process. The first
/// installation stays: a later one changes nothing.
pub fn install_tls(tls: &'static dyn Tls) {
    let _ = TLS.set(tls);
}

/// Sends `request` where `grants` allow, within `settings` (`spec/24`).
pub fn send(
    request: &Request<'_>,
    grants: &[Grant],
    settings: &Settings,
) -> Result<Response, Error> {
    // Capped at a day, as the flags are, so the deadline never overflows the clock.
    let timeout = settings.timeout.min(Duration::from_secs(MAX_TIMEOUT_SECS));
    let deadline = Instant::now().checked_add(timeout).ok_or(Error::TimedOut)?;
    // First, so that the abort it leads to depends on the header values alone.
    if request
        .headers
        .iter()
        .any(|(_, value)| value.as_bytes().iter().any(control))
    {
        return Err(Error::HeaderValue);
    }
    let mut url = Url::parse(request.url).ok_or(Error::BadUrl)?;
    check_headers(request.headers, request.bearer)?;
    if !allowed(grants, &url, request.method) {
        return Err(Error::Denied);
    }
    let first_origin = (url.tls, url.host.clone(), url.port);
    let mut method = request.method;
    let mut body = request.body;
    // The headers for another origin: the program's own credentials stay with the first one.
    let elsewhere: Vec<(String, String)> = request
        .headers
        .iter()
        .filter(|(name, _)| !CREDENTIALS.iter().any(|c| name.eq_ignore_ascii_case(c)))
        .cloned()
        .collect();
    // Whether the chain has left the first origin: the credentials then stay dropped to its
    // end, even back at that origin, whose path the other origin chose.
    let mut left_first = false;
    for redirects in 0..=MAX_REDIRECTS {
        // The credentials go only to the origin they were given for.
        left_first |= (url.tls, url.host.clone(), url.port) != first_origin;
        let first = !left_first;
        let bearer = request.bearer.filter(|_| first);
        let headers = if first {
            request.headers
        } else {
            elsewhere.as_slice()
        };
        let response = exchange(method, &url, headers, body, bearer, deadline, settings)?;
        let next = matches!(response.status, 301 | 302 | 303 | 307 | 308)
            .then(|| location(&response, &url))
            .flatten();
        let Some(target) = next else {
            return Ok(response);
        };
        let follow_method = if response.status == 303 {
            Method::Get
        } else {
            method
        };
        if redirects == MAX_REDIRECTS
            || follow_method != Method::Get
            || !allowed(grants, &target, Method::Get)
        {
            return Ok(response);
        }
        url = target;
        method = Method::Get;
        body = &[];
    }
    Err(Error::Protocol)
}

/// The URL of a redirect's `Location`: absolute, or a path on the same origin.
fn location(response: &Response, base: &Url) -> Option<Url> {
    let value = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value.as_str())?;
    if value.starts_with('/') && !value.starts_with("//") {
        let scheme = if base.tls { "https" } else { "http" };
        return Url::parse(&format!("{scheme}://{}{value}", base.host_header()));
    }
    Url::parse(value)
}

/// A control character other than a tab, which no header value or key may hold.
fn control(b: &u8) -> bool {
    *b < 0x20 && *b != b'\t' || *b == 0x7f
}

/// The headers that carry a credential, which a redirect to another origin drops.
const CREDENTIALS: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];

/// The headers the program may send: tokens, none that the client sets itself, and no
/// `Authorization` beside a key; the key, free of control characters (`BadRequest`).
/// [`send`] checks the values first ([`Error::HeaderValue`]).
fn check_headers(headers: &[(String, String)], bearer: Option<&[u8]>) -> Result<(), Error> {
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
    if bearer.is_some_and(|key| key.iter().any(control)) {
        return Err(Error::BadRequest);
    }
    for (name, _) in headers {
        if name.is_empty() || !name.bytes().all(token) {
            return Err(Error::BadRequest);
        }
        if bearer.is_some() && name.eq_ignore_ascii_case("authorization") {
            return Err(Error::BadRequest);
        }
        let reserved = ["host", "content-length", "transfer-encoding", "connection"];
        if reserved.iter().any(|r| name.eq_ignore_ascii_case(r)) {
            return Err(Error::BadRequest);
        }
    }
    Ok(())
}

/// The time left before `deadline`, or `TimedOut`.
pub fn left(deadline: Instant) -> Result<Duration, Error> {
    let now = Instant::now();
    if now >= deadline {
        return Err(Error::TimedOut);
    }
    Ok(deadline - now)
}

/// The addresses of a host, resolved by the system within the deadline. The resolver has no
/// timeout of its own, so it runs on a thread of its own, which may outlive a late answer.
fn resolve(host: &str, port: u16, deadline: Instant) -> Result<Vec<SocketAddr>, Error> {
    let name = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("una-http-resolve".to_owned())
        .spawn(move || {
            let found = (name.as_str(), port)
                .to_socket_addrs()
                .map(Iterator::collect::<Vec<_>>);
            let _ = sender.send(found);
        })
        .map_err(|_| Error::Unreachable)?;
    match receiver.recv_timeout(left(deadline)?) {
        Ok(Ok(addrs)) if !addrs.is_empty() => Ok(addrs),
        Ok(_) => Err(Error::Unreachable),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::TimedOut),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Unreachable),
    }
}

/// A connection, over TCP or TLS.
enum Connection {
    Plain(TcpStream),
    Tls(Box<dyn TlsStream>),
}

impl Connection {
    fn socket(&self) -> &TcpStream {
        match self {
            Connection::Plain(stream) => stream,
            Connection::Tls(stream) => stream.socket(),
        }
    }

    /// Sets the socket's timeouts to the time left, so no read or write passes the deadline.
    fn bound(&self, deadline: Instant) -> Result<(), Error> {
        let left = left(deadline)?;
        let socket = self.socket();
        socket
            .set_read_timeout(Some(left))
            .and_then(|()| socket.set_write_timeout(Some(left)))
            .map_err(|_| Error::Unreachable)
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Error> {
        let mut rest = bytes;
        while !rest.is_empty() {
            self.bound(deadline)?;
            let written = match self {
                Connection::Plain(stream) => stream.write(rest),
                Connection::Tls(stream) => stream.write(rest),
            };
            match written {
                Ok(0) => return Err(Error::Unreachable),
                Ok(n) => rest = rest.get(n..).unwrap_or_default(),
                Err(error) => return Err(io_error(&error, deadline)),
            }
        }
        match self {
            Connection::Plain(stream) => stream.flush(),
            Connection::Tls(stream) => stream.flush(),
        }
        .map_err(|error| io_error(&error, deadline))
    }

    fn read(&mut self, buffer: &mut [u8], deadline: Instant) -> Result<usize, Error> {
        self.bound(deadline)?;
        let read = match self {
            Connection::Plain(stream) => stream.read(buffer),
            Connection::Tls(stream) => stream.read(buffer),
        };
        match read {
            Ok(n) => Ok(n),
            // A peer that closes without TLS's close_notify ends the stream; the parser decides
            // whether the response was complete.
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(0),
            Err(error) => Err(io_error(&error, deadline)),
        }
    }
}

/// The failure of an I/O error: past the deadline, a timeout; a TLS error, as the installed
/// TLS client tells it, `Tls`; else the connection failed.
pub fn io_error(error: &std::io::Error, deadline: Instant) -> Error {
    if Instant::now() >= deadline
        || matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        )
    {
        return Error::TimedOut;
    }
    if TLS.get().is_some_and(|tls| tls.failed(error)) {
        return Error::Tls;
    }
    Error::Unreachable
}

/// Connects to the URL's origin within the deadline, with TLS for `https`.
fn connect(url: &Url, deadline: Instant) -> Result<Connection, Error> {
    let addrs = resolve(&url.host, url.port, deadline)?;
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, left(deadline)?) {
            Ok(connected) => {
                stream = Some(connected);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                left(deadline)?;
            }
            Err(_) => {}
        }
    }
    let stream = stream.ok_or(Error::Unreachable)?;
    let _ = stream.set_nodelay(true);
    if !url.tls {
        return Ok(Connection::Plain(stream));
    }
    // No TLS client installed: the request fails, and is never sent in the clear.
    let tls = TLS.get().ok_or(Error::Tls)?;
    let name = url.host.trim_start_matches('[').trim_end_matches(']');
    tls.connect(name, stream, deadline).map(Connection::Tls)
}

/// The bytes of a request (`spec/24 §3` rule 2).
fn request_bytes(
    method: Method,
    url: &Url,
    headers: &[(String, String)],
    body: &[u8],
    bearer: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\n",
        method.token(),
        url.target,
        url.host_header()
    )
    .into_bytes();
    if matches!(method, Method::Post | Method::Put) || !body.is_empty() {
        out.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    }
    out.extend_from_slice(b"Accept-Encoding: identity\r\nConnection: close\r\n");
    for (name, value) in headers {
        out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    if let Some(key) = bearer {
        out.extend_from_slice(b"Authorization: Bearer ");
        out.extend_from_slice(key);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

/// One request and its response, on a connection of its own.
fn exchange(
    method: Method,
    url: &Url,
    headers: &[(String, String)],
    body: &[u8],
    bearer: Option<&[u8]>,
    deadline: Instant,
    settings: &Settings,
) -> Result<Response, Error> {
    let mut connection = connect(url, deadline)?;
    connection.write_all(&request_bytes(method, url, headers, body, bearer), deadline)?;
    let mut parser = Parser::new(settings.limits);
    let mut buffer = vec![0u8; 16 * 1024];
    let mut received = false;
    while !parser.done() {
        let n = match connection.read(&mut buffer, deadline) {
            Ok(n) => n,
            Err(Error::Unreachable) if received => return Err(Error::Protocol),
            Err(error) => return Err(error),
        };
        if n == 0 {
            break;
        }
        received = true;
        parser
            .feed(buffer.get(..n).unwrap_or_default())
            .map_err(parse_error)?;
    }
    let raw = parser.finish().map_err(parse_error)?;
    let body = String::from_utf8(raw.body).map_err(|_| Error::Protocol)?;
    Ok(Response {
        status: raw.status,
        headers: raw.headers,
        body,
    })
}

fn parse_error(error: parse::Error) -> Error {
    match error {
        parse::Error::Protocol => Error::Protocol,
        parse::Error::TooLarge => Error::TooLarge,
    }
}
