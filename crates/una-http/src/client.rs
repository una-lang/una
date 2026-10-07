//! Sending a request (`spec/24 §3`, §4): HTTP/1.1, directly to the origin, over TCP or TLS,
//! within one deadline for the whole request, its redirects included.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock, mpsc};
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
    /// A header of the request is malformed, or names one the client sets itself (N-28).
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

/// The longest `--http-timeout`, a day, in seconds (`spec/24 §4`, N-30).
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

/// The most redirects followed in a row (`spec/24 §2` rule 3).
const MAX_REDIRECTS: usize = 5;

/// Sends `request` where `grants` allow, within `settings` (`spec/24`).
pub fn send(
    request: &Request<'_>,
    grants: &[Grant],
    settings: &Settings,
) -> Result<Response, Error> {
    // Capped at a day, as the flags are, so the deadline never overflows the clock.
    let timeout = settings.timeout.min(Duration::from_secs(MAX_TIMEOUT_SECS));
    let deadline = Instant::now().checked_add(timeout).ok_or(Error::TimedOut)?;
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
    for redirects in 0..=MAX_REDIRECTS {
        // The credentials go only to the origin they were given for.
        let first = (url.tls, url.host.clone(), url.port) == first_origin;
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

/// The headers that carry a credential, which a redirect to another origin drops (N-30).
const CREDENTIALS: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];

/// The headers the program may send: tokens with values free of control characters, none that
/// the client sets itself, and no `Authorization` beside a key; the key, free of control
/// characters too (`BadRequest`, N-28, N-30).
fn check_headers(headers: &[(String, String)], bearer: Option<&[u8]>) -> Result<(), Error> {
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
    let control = |b: &u8| *b < 0x20 && *b != b'\t' || *b == 0x7f;
    if bearer.is_some_and(|key| key.iter().any(control)) {
        return Err(Error::BadRequest);
    }
    for (name, value) in headers {
        if name.is_empty() || !name.bytes().all(token) {
            return Err(Error::BadRequest);
        }
        if value.as_bytes().iter().any(control) {
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
fn left(deadline: Instant) -> Result<Duration, Error> {
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
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Connection {
    fn socket(&self) -> &TcpStream {
        match self {
            Connection::Plain(stream) => stream,
            Connection::Tls(stream) => &stream.sock,
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

/// The failure of an I/O error: past the deadline, a timeout; a TLS error, `Tls`; else the
/// connection failed.
fn io_error(error: &std::io::Error, deadline: Instant) -> Error {
    if Instant::now() >= deadline
        || matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        )
    {
        return Error::TimedOut;
    }
    if error
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
    {
        return Error::Tls;
    }
    Error::Unreachable
}

/// The TLS settings: the Mozilla roots compiled in (`spec/24 §3` rule 4); in debug builds only,
/// also the roots of the PEM file that `UNA_HTTP_TEST_ROOTS` names, for the tests' local
/// servers. Release builds have no such hook.
fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    Arc::clone(CONFIG.get_or_init(|| {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        #[cfg(debug_assertions)]
        let roots = {
            let mut roots = roots;
            test_roots(&mut roots);
            roots
        };
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map(|builder| builder.with_root_certificates(roots).with_no_client_auth());
        match config {
            Ok(config) => Arc::new(config),
            // INVARIANT: the ring provider supports the default versions; an empty config
            // would refuse every handshake, which fails safe.
            Err(_) => Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(rustls::RootCertStore::empty())
                    .with_no_client_auth(),
            ),
        }
    }))
}

/// The PEM file of test roots that [`trust_test_roots`] names, in debug builds only.
#[cfg(debug_assertions)]
static TEST_ROOTS: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Debug builds only: trusts the roots of the PEM file at `path` too, for tests that serve TLS
/// locally. It takes effect only before the first TLS connection of the process.
#[cfg(debug_assertions)]
pub fn trust_test_roots(path: &std::path::Path) {
    let _ = TEST_ROOTS.set(path.to_owned());
}

#[cfg(debug_assertions)]
#[allow(clippy::disallowed_methods)]
fn test_roots(roots: &mut rustls::RootCertStore) {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;
    let path = match TEST_ROOTS.get() {
        Some(path) => path.clone(),
        None => match std::env::var_os("UNA_HTTP_TEST_ROOTS") {
            Some(path) => std::path::PathBuf::from(path),
            None => return,
        },
    };
    let Ok(certs) = CertificateDer::pem_file_iter(path) else {
        return;
    };
    for cert in certs.flatten() {
        let _ = roots.add(cert);
    }
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
    let name = url
        .host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let server = rustls::pki_types::ServerName::try_from(name).map_err(|_| Error::BadUrl)?;
    let client = rustls::ClientConnection::new(tls_config(), server).map_err(|_| Error::Tls)?;
    let mut connection = Connection::Tls(Box::new(rustls::StreamOwned::new(client, stream)));
    // The handshake, within the deadline.
    if let Connection::Tls(tls) = &mut connection {
        loop {
            if !tls.conn.is_handshaking() {
                break;
            }
            let left = left(deadline)?;
            let _ = tls.sock.set_read_timeout(Some(left));
            let _ = tls.sock.set_write_timeout(Some(left));
            tls.conn.complete_io(&mut tls.sock).map_err(|error| {
                match io_error(&error, deadline) {
                    Error::TimedOut => Error::TimedOut,
                    Error::Unreachable if error.kind() == std::io::ErrorKind::InvalidData => {
                        Error::Tls
                    }
                    other => other,
                }
            })?;
        }
    }
    Ok(connection)
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
