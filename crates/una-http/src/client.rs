//! Sending a request (`spec/24 §3`, §4): HTTP/1.1, directly to the origin, over TCP or TLS,
//! within one deadline for the whole request, its redirects included.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use crate::grant::{Grant, Method, Url, allowed};
use crate::limit::{Abandon, Governor};
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
    /// A host stop request came before the request started, or while it waited on its
    /// connection (ADR 0116 rule 12): it may or may not have reached its server. A wait at a
    /// limit that a host stop or a cancellation ends is one too, never sent (ADR 0117 rule 8).
    Interrupted,
    /// The queue of a limit the request had to wait at was full (ADR 0117 rule 8, `spec/27 §9`):
    /// it was not sent.
    RateLimited,
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

/// What a wait on a request's socket saw (ADR 0116 rule 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waited {
    /// The socket is ready.
    Ready,
    /// A host stop request has come.
    Stopped,
    /// The time given passed.
    TimedOut,
    /// The wait itself failed, as `poll` can: the request fails as an unreachable one.
    Failed,
}

/// What can interrupt a request (ADR 0116 rule 12): an engine gives one for an operation in
/// the body of a `for ever`, which a host stop request interrupts.
pub trait Interrupt {
    /// Whether a host stop request has come: a request then fails with `Interrupted` before it
    /// starts, and between the steps that do not wait on the socket.
    fn stopped(&self) -> bool;

    /// Waits until `socket` is ready to read, or to write when `write`, until a host stop
    /// request comes, or until `timeout` passes. It must really wait: the socket is then
    /// non-blocking, and each `WouldBlock` comes back here.
    fn wait(&self, socket: RawFd, write: bool, timeout: Duration) -> Waited;

    /// Whether [`Interrupt::wait`] waits. When it does, the request's socket is non-blocking,
    /// so no read, write or step of the TLS handshake blocks past one wait, and a stop is seen
    /// at the next one. It has no default: an implementor whose `wait` returns at once must
    /// say `false`, or the non-blocking loop would spin until the deadline.
    fn waits(&self) -> bool;
}

/// No interruption: every wait is left to the socket's own timeouts, and a wait at a limit to
/// the request's deadline and its [`crate::limit::Abandon`]. The engines use it for a request
/// outside the body of a `for ever` (ADR 0116 rule 12).
#[derive(Clone, Copy, Debug, Default)]
pub struct Uninterrupted;

impl Interrupt for Uninterrupted {
    fn stopped(&self) -> bool {
        false
    }

    fn wait(&self, _socket: RawFd, _write: bool, _timeout: Duration) -> Waited {
        Waited::Ready
    }

    fn waits(&self) -> bool {
        false
    }
}

/// The longest `--http-timeout`, a day, in seconds (`spec/24 §4`).
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

/// The most redirects followed in a row (`spec/24 §2` rule 3).
const MAX_REDIRECTS: usize = 5;

/// Sends `request` where `grants` allow, within `settings` (`spec/24`). Limits on `grants` hold
/// for this request only; a run shares them across its requests with [`send_governed`].
pub fn send(
    request: &Request<'_>,
    grants: &[Grant],
    settings: &Settings,
) -> Result<Response, Error> {
    send_interruptible(request, grants, settings, &Uninterrupted)
}

/// [`send_governed`] with a governor of `grants` for this request only: limits on `grants`
/// hold within it, and are not shared with any other request.
pub fn send_interruptible(
    request: &Request<'_>,
    grants: &[Grant],
    settings: &Settings,
    interrupt: &dyn Interrupt,
) -> Result<Response, Error> {
    let governor = Governor::new(grants.to_vec());
    send_governed(
        request,
        &governor,
        settings,
        interrupt,
        &Abandon::new(),
        None,
    )
}

/// Sends `request` where the grants of `governor` allow, within `settings` and their limits
/// (`spec/24`, `spec/27 §9`), which `interrupt` interrupts (ADR 0116 rule 12): after the header
/// values are checked, a request whose stop has come fails with `Interrupted` before anything is
/// sent.
///
/// Each hop, the first and each redirect followed, is charged to the limited grants that allow
/// its target before its exchange, waiting in their queues when it must, and releases its slots
/// once its response is read, before the next hop asks for its own (ADR 0117 rule 8). The wait
/// counts toward the request's deadline; `abandon` ends it when the engine cancels the task.
/// On the run's clock, the governor's, the request's deadline is its time-out from its start,
/// or `cover`, the moment of a deadline of `within` or `for ever within` that covers it, when
/// that comes first: under virtual time a wait for a refill moves the clock to it, at most
/// (ADR 0116 rule 19).
/// When `interrupt` waits, the socket is non-blocking from the connection on: every read,
/// write and step of the TLS handshake that would block waits through `interrupt` instead, and
/// a stop is checked before each try, so a slow or stalled peer cannot hold a stopped request.
/// Name resolution and the TCP connection are not waited on with `interrupt`: a stop that comes
/// during them takes effect when they return, within the request's deadline (ADR 0116,
/// implementation plan, item 6).
pub fn send_governed(
    request: &Request<'_>,
    governor: &Governor,
    settings: &Settings,
    interrupt: &dyn Interrupt,
    abandon: &Abandon,
    cover: Option<u64>,
) -> Result<Response, Error> {
    let grants = governor.grants();
    // Capped at a day, as the flags are, so the deadline never overflows the clock.
    let timeout = settings.timeout.min(Duration::from_secs(MAX_TIMEOUT_SECS));
    let deadline = Instant::now().checked_add(timeout).ok_or(Error::TimedOut)?;
    // The same deadline on the run's clock, for the waits at limits (ADR 0116 rule 19).
    let millis = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    let until = governor
        .now_millis()
        .saturating_add(millis)
        .min(cover.unwrap_or(u64::MAX));
    // First, so that the abort it leads to depends on the header values alone.
    if request
        .headers
        .iter()
        .any(|(_, value)| value.as_bytes().iter().any(control))
    {
        return Err(Error::HeaderValue);
    }
    if interrupt.stopped() {
        return Err(Error::Interrupted);
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
        // The hop's tokens and slots, held until its response is read (ADR 0117 rule 8).
        let permit = governor.acquire_until(&url, method, (deadline, until), interrupt, abandon)?;
        let response = exchange(
            method,
            &url,
            headers,
            body,
            bearer,
            settings,
            (deadline, interrupt),
        );
        drop(permit);
        let response = response?;
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

    /// Waits on the socket, to write when `write` and to read otherwise, within the deadline,
    /// unless `interrupt` ends the wait first.
    fn wait_on(
        &self,
        write: bool,
        deadline: Instant,
        interrupt: &dyn Interrupt,
    ) -> Result<(), Error> {
        match interrupt.wait(self.socket().as_raw_fd(), write, left(deadline)?) {
            Waited::Ready => Ok(()),
            Waited::Stopped => Err(Error::Interrupted),
            Waited::TimedOut => Err(Error::TimedOut),
            Waited::Failed => Err(Error::Unreachable),
        }
    }

    /// The direction a connection waits in after `WouldBlock`: TLS writes first when it has
    /// bytes to send, as a step of the handshake may; plain I/O waits as `write` says.
    fn direction(&self, write: bool) -> bool {
        match self {
            Connection::Plain(_) => write,
            Connection::Tls(stream) => stream.conn.wants_write(),
        }
    }

    /// Runs `step`, an I/O step on the connection, within the deadline (ADR 0116 rule 12).
    /// Without an interruption, the socket blocks, bounded by the time left. With one, the
    /// socket is non-blocking: each `WouldBlock` waits through `interrupt`, and a stop is
    /// checked before each try, so a stop is seen within one wait, whatever the peer sends.
    fn io<T>(
        &mut self,
        write: bool,
        deadline: Instant,
        interrupt: &dyn Interrupt,
        mut step: impl FnMut(&mut Connection) -> std::io::Result<T>,
    ) -> Result<std::io::Result<T>, Error> {
        if !interrupt.waits() {
            self.bound(deadline)?;
            return Ok(step(self));
        }
        loop {
            if interrupt.stopped() {
                return Err(Error::Interrupted);
            }
            left(deadline)?;
            match step(self) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.wait_on(self.direction(write), deadline, interrupt)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                done => return Ok(done),
            }
        }
    }

    fn write_all(
        &mut self,
        bytes: &[u8],
        deadline: Instant,
        interrupt: &dyn Interrupt,
    ) -> Result<(), Error> {
        let mut rest = bytes;
        while !rest.is_empty() {
            let written = self.io(true, deadline, interrupt, |connection| match connection {
                Connection::Plain(stream) => stream.write(rest),
                Connection::Tls(stream) => stream.write(rest),
            })?;
            match written {
                // A TLS stream whose buffer is full takes nothing until the socket drains. A
                // plain socket that takes nothing is a failed connection, never a wait.
                Ok(0) if interrupt.waits() && matches!(self, Connection::Tls(_)) => {
                    self.wait_on(true, deadline, interrupt)?;
                }
                Ok(0) => return Err(Error::Unreachable),
                Ok(n) => rest = rest.get(n..).unwrap_or_default(),
                Err(error) => return Err(io_error(&error, deadline)),
            }
        }
        self.io(true, deadline, interrupt, |connection| match connection {
            Connection::Plain(stream) => stream.flush(),
            Connection::Tls(stream) => stream.flush(),
        })?
        .map_err(|error| io_error(&error, deadline))
    }

    fn read(
        &mut self,
        buffer: &mut [u8],
        deadline: Instant,
        interrupt: &dyn Interrupt,
    ) -> Result<usize, Error> {
        let read = self.io(false, deadline, interrupt, |connection| match connection {
            Connection::Plain(stream) => stream.read(buffer),
            Connection::Tls(stream) => stream.read(buffer),
        })?;
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
fn connect(url: &Url, deadline: Instant, interrupt: &dyn Interrupt) -> Result<Connection, Error> {
    let addrs = resolve(&url.host, url.port, deadline)?;
    let mut stream = None;
    for addr in addrs {
        if interrupt.stopped() {
            return Err(Error::Interrupted);
        }
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
    if interrupt.stopped() {
        return Err(Error::Interrupted);
    }
    let _ = stream.set_nodelay(true);
    // An interruptible request never blocks on its socket (ADR 0116 rule 12).
    if interrupt.waits() {
        stream
            .set_nonblocking(true)
            .map_err(|_| Error::Unreachable)?;
    }
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
    // The handshake, within the deadline, each step through `io`.
    while matches!(&connection, Connection::Tls(tls) if tls.conn.is_handshaking()) {
        let stepped = connection.io(true, deadline, interrupt, |connection| match connection {
            Connection::Tls(tls) => tls.conn.complete_io(&mut tls.sock).map(drop),
            Connection::Plain(_) => Ok(()),
        })?;
        stepped.map_err(|error| match io_error(&error, deadline) {
            Error::TimedOut => Error::TimedOut,
            Error::Unreachable if error.kind() == std::io::ErrorKind::InvalidData => Error::Tls,
            other => other,
        })?;
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

/// One request and its response, on a connection of its own, by `deadline` unless `interrupt`
/// ends it first.
fn exchange(
    method: Method,
    url: &Url,
    headers: &[(String, String)],
    body: &[u8],
    bearer: Option<&[u8]>,
    settings: &Settings,
    (deadline, interrupt): (Instant, &dyn Interrupt),
) -> Result<Response, Error> {
    let mut connection = connect(url, deadline, interrupt)?;
    connection.write_all(
        &request_bytes(method, url, headers, body, bearer),
        deadline,
        interrupt,
    )?;
    let mut parser = Parser::new(settings.limits);
    let mut buffer = vec![0u8; 16 * 1024];
    let mut received = false;
    while !parser.done() {
        let n = match connection.read(&mut buffer, deadline, interrupt) {
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
