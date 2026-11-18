//! The registry's HTTP/1.1 endpoints (ADR 0122 rules 34, 36.2 and 43; `spec/30 §16`), on the
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
//! without progress and at least 64 KiB per second on average past its first minute (the
//! idle timeout), counted on the bytes that arrived, within a total of the idle timeout and a
//! second per 64 KiB declared. Chunked bodies, percent-encoding, queries, repeated
//! `Content-Length` and folded headers are refused. A body's buffer grows with the bytes that
//! arrive, doubling from 64 KiB, and takes its whole declared length at once only after its
//! first MiB arrived, so a reallocation never holds more than the length and that MiB; the
//! search for a head's end resumes where it stopped, so its work is linear in the head. A
//! request refused before its body was read gets its answer, then the service stops writing
//! and discards at most 64 KiB of the body for at most a second before it closes, so a client
//! that sent a small body reads the refusal rather than a reset.
//!
//! **Writing a response.** Every response is written within the write timeout and the least
//! rate, the mirror of a body's: no write waits more than the write timeout (60 seconds) for
//! the peer to take bytes, and past the first write timeout the peer has taken at least 64 KiB
//! per second on average, within a total of the write timeout and a second per 64 KiB of the
//! response; a peer that reads too slowly or not at all has its connection closed. The rate
//! is checked between writes, each of at most 64 KiB. A file of the store is sent from the
//! open file in parts of at most 64 KiB, never read whole into memory; it is the version that
//! was opened, whole, even if the path is replaced meanwhile (`store`), and a file that cannot
//! be read to its length ends the connection short of its `Content-Length`.
//!
//! **Connections.** Each connection serves one request. At most `max_connections` are served
//! at once, from the request's first byte to the response's last: a connection holds its slot
//! while its response is written. [`serve`] takes the slot before it starts the connection's
//! thread, so it runs at most `max_connections` connection threads; a connection past the
//! bound is answered 503 by one write that never waits, then closed, and starts no thread. A
//! thread that cannot start closes its connection and gives its slot back, and the loop goes
//! on accepting. An `accept` that fails for one connection goes on at once, and one that
//! fails for want of descriptors or memory goes on after a pause that doubles from 5 ms to a
//! second; any other failure of the listener ends [`serve`]. At most two
//! publications are read and checked at once, since each holds its archive in memory: a
//! publication takes its slot once its head is checked, before its body is read.
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
use crate::store::{Opened, Store, check_path};
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
/// The least average rate of a body past its first idle timeout, in bytes per second.
const MIN_BODY_RATE: u128 = 64 << 10;
/// The most bytes of an unread body discarded after a refusal, before the connection closes.
const DRAIN_MAX: usize = 64 << 10;
/// The longest wait for those bytes.
const DRAIN_TIME: Duration = Duration::from_secs(1);
/// The most bytes of a response given to one write, and the buffer a file is sent through.
pub const SEND_CHUNK: usize = 64 << 10;
/// The least average rate at which a peer takes a response past its first write timeout, in
/// bytes per second: the mirror of [`MIN_BODY_RATE`].
const MIN_SEND_RATE: u128 = 64 << 10;
/// The first pause after an `accept` that failed for want of descriptors or memory.
const ACCEPT_PAUSE_MIN: Duration = Duration::from_millis(5);
/// The longest such pause; each failure in a row doubles it up to this.
const ACCEPT_PAUSE_MAX: Duration = Duration::from_secs(1);

/// How the endpoints run.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The operator's bearer token for `/register`; registration is closed without one.
    pub register_token: Option<String>,
    /// How many connections are served at once, each from its request's first byte to its
    /// response's last; [`serve`] runs at most this many connection threads.
    pub max_connections: usize,
    /// The time to read a request's head.
    pub head_timeout: Duration,
    /// The longest wait for progress on a body.
    pub idle_timeout: Duration,
    /// The longest wait for the peer to take bytes of a response, and the time past which it
    /// takes them at the least rate.
    pub write_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig {
            register_token: None,
            max_connections: 256,
            head_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(60),
            write_timeout: Duration::from_secs(60),
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

/// What answers a request: a response held whole, or a file of the store, sent from the open
/// file in parts.
enum Answer {
    Whole(Response),
    File {
        content_type: &'static str,
        immutable: bool,
        file: Opened,
    },
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

/// A connection slot taken before its thread starts, released when dropped: by the thread
/// when its connection is done, or with the thread's work when the thread cannot start.
struct Held(Arc<Service>);

impl Held {
    fn take(service: &Arc<Service>) -> Option<Held> {
        let previous = service.connections.fetch_add(1, Ordering::SeqCst);
        if previous >= service.config.max_connections {
            service.connections.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Held(Arc::clone(service)))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.0.connections.fetch_sub(1, Ordering::SeqCst);
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

    /// How many connections hold a connection slot now, for an operator's tools and tests.
    pub fn connections_in_progress(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// How many publications hold a publishing slot now, for an operator's tools and tests.
    pub fn publications_in_progress(&self) -> usize {
        self.publishing.load(Ordering::SeqCst)
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

    /// Answers a request that was read and checked, with its body whole: a file served is
    /// read into the response, which a connection never does ([`Service::handle`] sends it
    /// in parts).
    pub fn route(&self, request: Request) -> Response {
        match self.dispatch(request) {
            Answer::Whole(response) => response,
            Answer::File {
                content_type,
                immutable,
                file,
            } => {
                let mut body = Vec::new();
                match file.reader.take(file.length).read_to_end(&mut body) {
                    Ok(_) if body.len() as u64 == file.length => Response {
                        status: 200,
                        content_type,
                        immutable,
                        body,
                    },
                    _ => text(503, "the store cannot be read"),
                }
            }
        }
    }

    fn dispatch(&self, request: Request) -> Answer {
        match request.method.as_str() {
            "GET" => self.read(&request.path),
            "POST" => Answer::Whole(self.write(request, None)),
            _ => Answer::Whole(text(405, "only GET and POST are served")),
        }
    }

    /// Opens the file of a `GET`; its bytes are read only as they are sent.
    fn read(&self, path: &str) -> Answer {
        let Some(immutable) = servable(path) else {
            return Answer::Whole(text(404, "not found"));
        };
        match self.store.reader(path, MAX_SERVED) {
            Ok(Some(file)) => Answer::File {
                content_type: if path.starts_with("tile/")
                    || path.starts_with("map/")
                    || path.starts_with("blobs/")
                {
                    "application/octet-stream"
                } else {
                    "text/plain; charset=utf-8"
                },
                immutable,
                file,
            },
            Ok(None) => Answer::Whole(text(404, "not found")),
            Err(_) => Answer::Whole(text(503, "the store cannot be read")),
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

    /// The publishing slot, or the response that refuses a publication for the lack of one.
    fn publishing_slot(&self) -> Result<Slot<'_>, Response> {
        take(&self.publishing, MAX_PUBLISHING)
            .ok_or_else(|| text(503, "too many publications are being checked; try again"))
    }

    /// Answers a `POST`; a publication uses `slot` when its reader took one already.
    fn write(&self, request: Request, slot: Option<Slot<'_>>) -> Response {
        match request.path.as_str() {
            "publish" => {
                let _slot = match slot {
                    Some(slot) => slot,
                    None => match self.publishing_slot() {
                        Ok(slot) => slot,
                        Err(response) => return response,
                    },
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

    /// Serves one connection: takes a connection slot, reads one request within the limits,
    /// answers it within the write limits, and closes; the slot is held to the response's
    /// last byte. Past `max_connections`, the connection is answered 503 by one write that
    /// never waits, and closed.
    pub fn handle(&self, stream: TcpStream) {
        match take(&self.connections, self.config.max_connections) {
            None => refuse_busy(stream),
            Some(_slot) => self.answer(stream),
        }
    }

    /// Serves one connection whose slot its caller holds, until the response's last byte.
    fn answer(&self, mut stream: TcpStream) {
        match self.serve_one(&mut stream) {
            Served::Read(answer) => {
                let _ = send(&mut stream, answer, &self.config);
            }
            Served::Refused(response) => {
                // The body may still be on its way: answer, stop writing, and discard a
                // bounded part of it within the connection's slot, so a client that sent a
                // small body reads the refusal rather than a reset. A peer that timed out
                // sends nothing worth waiting for.
                let status = response.status;
                let sent = send(&mut stream, Answer::Whole(response), &self.config);
                if sent.is_ok() && status != 408 {
                    let _ = stream.shutdown(std::net::Shutdown::Write);
                    drain(&mut stream);
                }
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

/// Answers a connection past the bound 503 by one write that never waits, and closes it: a
/// refusal of a few hundred bytes fits in a fresh connection's buffer, and a peer that does
/// not take it gets the close alone.
fn refuse_busy(mut stream: TcpStream) {
    let _ = stream.set_nonblocking(true);
    let _ = write_response(&mut stream, &text(503, "too many connections; try again"));
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

impl Service {
    /// Reads and answers one request. A publication takes its publishing slot once its head
    /// is checked and before its body is read, so at most two bodies of publications are held
    /// at once, however many connections declare one.
    fn serve_one(&self, stream: &mut TcpStream) -> Served {
        let head = match read_head(stream, &self.config) {
            Ok(head) => head,
            Err(response) => return Served::Refused(response),
        };
        let slot = if head.publishes() {
            match self.publishing_slot() {
                Ok(slot) => Some(slot),
                Err(response) => return Served::Refused(response),
            }
        } else {
            None
        };
        match head.read_body(stream, &self.config) {
            Ok(request) if slot.is_some() => Served::Read(Answer::Whole(self.write(request, slot))),
            Ok(request) => Served::Read(self.dispatch(request)),
            Err(response) => Served::Refused(response),
        }
    }
}

/// The answer to a connection's request, and whether its request was read whole.
enum Served {
    /// The request was read whole.
    Read(Answer),
    /// The request was refused before its body was read whole.
    Refused(Response),
}

/// Discards at most `DRAIN_MAX` bytes of `stream` within `DRAIN_TIME`, until it closes.
fn drain(stream: &mut impl Conn) {
    let started = stream.now();
    let mut left = DRAIN_MAX;
    let mut chunk = [0u8; 4096];
    while left > 0 {
        let Some(wait) = DRAIN_TIME
            .checked_sub(stream.now().saturating_duration_since(started))
            .filter(|d| !d.is_zero())
        else {
            return;
        };
        stream.wait_at_most(wait);
        match stream.read(chunk.get_mut(..left.min(4096)).unwrap_or_default()) {
            Ok(0) | Err(_) => return,
            Ok(n) => left = left.saturating_sub(n),
        }
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

/// A connection the requests are read from: bytes, a bound on the wait for each read, and the
/// clock its limits are measured with.
trait Conn: Read {
    fn wait_at_most(&mut self, wait: Duration);

    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl Conn for TcpStream {
    fn wait_at_most(&mut self, wait: Duration) {
        let _ = self.set_read_timeout(Some(wait));
    }
}

/// A request's head, read and checked, with the bytes of its body that came with it.
struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    already: Vec<u8>,
    /// The body's declared length, within the endpoint's bound; `None` for no body.
    length: Option<usize>,
}

/// Reads a request from `stream` within the limits of the module, or the response that
/// refuses it.
pub fn read_request(stream: &mut TcpStream, config: &ServerConfig) -> Result<Request, Response> {
    read_head(stream, config)?.read_body(stream, config)
}

#[cfg(test)]
thread_local! {
    /// The bytes the head's search looked at, for the tests of its linear work.
    static HEAD_SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The largest capacity a body's buffer had, for the tests of its reservation.
    static BODY_RESERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The most a reallocation of a body's buffer could hold at once, old and new.
    static BODY_PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Reads a request's head from `stream` and checks it: every refusal of the head and of the
/// body's framing comes before any byte of the body past the head's reads.
fn read_head(stream: &mut impl Conn, config: &ServerConfig) -> Result<Head, Response> {
    let started = stream.now();
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    // Where the search resumes: the bytes before it were searched already, and a delimiter
    // that ends in the new bytes starts at most three bytes before them, so each byte of the
    // head is searched a bounded number of times however the peer splits its writes.
    let mut from = 0usize;
    let end = loop {
        let searched = head.get(from..).unwrap_or_default();
        #[cfg(test)]
        HEAD_SCANNED.with(|c| c.set(c.get() + searched.len()));
        if let Some(at) = find(searched, b"\r\n\r\n") {
            break from + at;
        }
        from = head.len().saturating_sub(3);
        // A delimiter that ends a head of at most `MAX_HEAD` bytes ends within its first
        // `MAX_HEAD + 4`: only past them is the head too large whatever comes next, so the
        // answer does not depend on how the peer split its writes.
        if head.len() > MAX_HEAD + 3 {
            return Err(text(431, "the request's head is too large"));
        }
        let left = config
            .head_timeout
            .checked_sub(stream.now().saturating_duration_since(started))
            .filter(|d| !d.is_zero())
            .ok_or_else(|| text(408, "the request's head took too long"))?;
        stream.wait_at_most(left);
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
    let length = match (limit, length) {
        (None, None | Some(0)) if already.is_empty() => None,
        (None, _) => return Err(text(400, "this request has no body")),
        (Some(_), None) => return Err(text(411, "a body needs `Content-Length`")),
        (Some(limit), Some(length)) if length > limit => {
            return Err(text(
                413,
                format!("the body is larger than the {limit} bytes this endpoint takes"),
            ));
        }
        (Some(_), Some(length)) => Some(length),
    };
    Ok(Head {
        method,
        path,
        headers,
        already,
        length,
    })
}

impl Head {
    /// Whether the request is a publication, which needs a publishing slot before its body
    /// is read.
    fn publishes(&self) -> bool {
        self.method == "POST" && self.path == "publish"
    }

    /// Reads the body the head declared, and gives the request.
    fn read_body(self, stream: &mut impl Conn, config: &ServerConfig) -> Result<Request, Response> {
        let body = match self.length {
            None => Vec::new(),
            Some(length) => read_body(stream, self.already, length, config)?,
        };
        Ok(Request {
            method: self.method,
            path: self.path,
            headers: self.headers,
            body,
        })
    }
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn read_body(
    stream: &mut impl Conn,
    already: Vec<u8>,
    length: usize,
    config: &ServerConfig,
) -> Result<Vec<u8>, Response> {
    if already.len() > length {
        return Err(text(400, "the body is longer than `Content-Length`"));
    }
    let started = stream.now();
    let deadline = config.idle_timeout + Duration::from_secs((length / (64 << 10)) as u64);
    // The buffer grows with the bytes that arrived, never by the declared length alone, so a
    // peer that declares a large body and sends little holds little memory.
    let before = already.len();
    let mut body = already;
    let mut chunk = vec![0u8; 64 << 10];
    stream.wait_at_most(config.idle_timeout);
    while body.len() < length {
        #[cfg(test)]
        BODY_RESERVED.with(|c| c.set(c.get().max(body.capacity())));
        let elapsed = stream.now().saturating_duration_since(started);
        if elapsed > deadline {
            return Err(text(408, "the body took too long"));
        }
        // Past the first idle timeout the body keeps the least average rate on the bytes
        // that arrived, so a peer that sends a byte now and then loses its slot, however
        // large the length it declared.
        if let Some(past) = elapsed.checked_sub(config.idle_timeout) {
            let owed = past.as_nanos().saturating_mul(MIN_BODY_RATE) / 1_000_000_000;
            let received = body.len().saturating_sub(before) as u128;
            if received < owed {
                return Err(text(
                    408,
                    "the body arrived slower than 64 KiB per second on average",
                ));
            }
        }
        let want = (length - body.len()).min(chunk.len());
        match stream.read(chunk.get_mut(..want).unwrap_or_default()) {
            Ok(0) => return Err(text(400, "the connection closed inside the body")),
            Ok(n) => {
                grow(&mut body, n, length)?;
                body.extend_from_slice(chunk.get(..n).unwrap_or_default());
            }
            Err(e) if is_timeout(&e) => return Err(text(408, "the body made no progress")),
            Err(_) => return Err(text(400, "the body could not be read")),
        }
    }
    #[cfg(test)]
    BODY_RESERVED.with(|c| c.set(c.get().max(body.capacity())));
    Ok(body)
}

/// The first reservation of a body's buffer.
const FIRST_BODY_RESERVE: usize = 64 << 10;
/// The capacity past which a body's buffer takes its whole declared length at once.
const FULL_BODY_RESERVE_AFTER: usize = 1 << 20;

/// Makes room in `body` for `extra` more bytes of a body of `length`: the capacity doubles
/// from 64 KiB up to 1 MiB, so it stays within twice the bytes received, then takes `length`
/// at once, so the copies stay linear and a reallocation holds at most `length` and 1 MiB;
/// it never passes `length`. A reservation the allocator refuses refuses the request.
fn grow(body: &mut Vec<u8>, extra: usize, length: usize) -> Result<(), Response> {
    let needed = body.len().saturating_add(extra);
    if needed <= body.capacity() {
        return Ok(());
    }
    let doubled = body.capacity().saturating_mul(2).max(FIRST_BODY_RESERVE);
    let target = if doubled > FULL_BODY_RESERVE_AFTER {
        length
    } else {
        doubled.min(length)
    }
    .max(needed);
    #[cfg(test)]
    BODY_PEAK.with(|c| c.set(c.get().max(body.capacity().saturating_add(target))));
    body.try_reserve_exact(target - body.len())
        .map_err(|_| text(503, "the service cannot hold this body now; try again"))
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

/// A response's status line and headers.
fn response_head(status: u16, content_type: &str, immutable: bool, length: u64) -> String {
    let cache = if immutable && status == 200 {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nCache-Control: {cache}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        reason(status),
    )
}

/// Writes a response and its headers, with no limit of its own: a connection's responses go
/// through the write limits of the module instead.
pub fn write_response(stream: &mut impl Write, response: &Response) -> io::Result<()> {
    let head = response_head(
        response.status,
        response.content_type,
        response.immutable,
        response.body.len() as u64,
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// A connection's work, for the function that starts its thread.
pub type Work = Box<dyn FnOnce() + Send>;

/// Serves `listener` until it fails, one thread per connection and at most `max_connections`
/// of them (the module's rules).
pub fn serve(listener: &TcpListener, service: &Arc<Service>) -> io::Result<()> {
    serve_with(listener, service, |work| {
        std::thread::Builder::new()
            .name("itaty-connection".to_owned())
            .spawn(work)
            .map(drop)
    })
}

/// Serves `listener` as [`serve`] does, with `spawn` starting each connection's thread: an
/// operator's own threads, or a test's. The connection's slot is taken before `spawn` is
/// called; when `spawn` fails, it has dropped the work, which closes the connection and gives
/// the slot back, and the loop goes on.
pub fn serve_with(
    listener: &TcpListener,
    service: &Arc<Service>,
    spawn: impl Fn(Work) -> io::Result<()>,
) -> io::Result<()> {
    let mut pause = ACCEPT_PAUSE_MIN;
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => {
                pause = ACCEPT_PAUSE_MIN;
                stream
            }
            Err(e) if fails_one_connection(&e) => continue,
            Err(e) if fails_for_resources(&e) => {
                std::thread::sleep(pause);
                pause = pause.saturating_mul(2).min(ACCEPT_PAUSE_MAX);
                continue;
            }
            Err(e) => return Err(e),
        };
        let Some(held) = Held::take(service) else {
            refuse_busy(stream);
            continue;
        };
        let _ = spawn(Box::new(move || {
            held.0.answer(stream);
            drop(held);
        }));
    }
    Ok(())
}

/// Whether a failed `accept` concerns only the connection it would have given.
fn fails_one_connection(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
    )
}

/// Whether a failed `accept` lacked descriptors (`ENFILE`, `EMFILE`: 23 and 24 on Linux and
/// macOS) or memory, which the connections that end give back.
fn fails_for_resources(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::OutOfMemory || matches!(e.raw_os_error(), Some(23 | 24))
}

/// A connection a response is written to: bytes, a bound on the wait for each write, and the
/// clock its limits are measured with.
trait Out: Write {
    fn wait_to_send_at_most(&mut self, wait: Duration);

    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl Out for TcpStream {
    fn wait_to_send_at_most(&mut self, wait: Duration) {
        let _ = self.set_write_timeout(Some(wait));
    }
}

/// Writes bytes of a response within the write limits.
struct Sender<'a, O: Out> {
    out: &'a mut O,
    write_timeout: Duration,
    started: Instant,
    deadline: Duration,
    sent: u128,
}

impl<'a, O: Out> Sender<'a, O> {
    /// A sender of a response of `length` bytes, head included.
    fn new(out: &'a mut O, length: u64, config: &ServerConfig) -> Sender<'a, O> {
        let started = out.now();
        Sender {
            out,
            write_timeout: config.write_timeout,
            started,
            deadline: config
                .write_timeout
                .saturating_add(Duration::from_secs(length / (64 << 10))),
            sent: 0,
        }
    }

    /// Writes `bytes` whole, at most `SEND_CHUNK` per write; an error leaves the connection
    /// to be closed.
    fn put(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let elapsed = self.out.now().saturating_duration_since(self.started);
            let left = self
                .deadline
                .checked_sub(elapsed)
                .filter(|d| !d.is_zero())
                .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
            // Past the first write timeout the peer keeps the least average rate on what it
            // took, so a peer that takes a byte now and then loses its slot, however large
            // the response.
            if let Some(past) = elapsed.checked_sub(self.write_timeout) {
                let owed = past.as_nanos().saturating_mul(MIN_SEND_RATE) / 1_000_000_000;
                if self.sent < owed {
                    return Err(io::ErrorKind::TimedOut.into());
                }
            }
            self.out.wait_to_send_at_most(left.min(self.write_timeout));
            let part = bytes.get(..SEND_CHUNK.min(bytes.len())).unwrap_or_default();
            match self.out.write(part) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.sent = self.sent.saturating_add(n as u128);
                    bytes = bytes.get(n..).unwrap_or_default();
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// The largest buffer a file was read into while it was sent.
    static SEND_READ_MAX: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Writes an answer within the write limits: its head, then its body, a file of the store
/// read through one buffer of `SEND_CHUNK` bytes. A file that ends before its length, or
/// cannot be read, is an error, and the connection closes short of its `Content-Length`.
fn send(out: &mut impl Out, answer: Answer, config: &ServerConfig) -> io::Result<()> {
    match answer {
        Answer::Whole(response) => {
            let head = response_head(
                response.status,
                response.content_type,
                response.immutable,
                response.body.len() as u64,
            );
            let mut sender = Sender::new(out, (head.len() + response.body.len()) as u64, config);
            sender.put(head.as_bytes())?;
            sender.put(&response.body)?;
        }
        Answer::File {
            content_type,
            immutable,
            mut file,
        } => {
            let head = response_head(200, content_type, immutable, file.length);
            let mut sender =
                Sender::new(out, (head.len() as u64).saturating_add(file.length), config);
            sender.put(head.as_bytes())?;
            let mut buffer = vec![0u8; SEND_CHUNK];
            let mut left = file.length;
            while left > 0 {
                let want = usize::try_from(left).unwrap_or(SEND_CHUNK).min(SEND_CHUNK);
                let part = buffer.get_mut(..want).unwrap_or_default();
                #[cfg(test)]
                SEND_READ_MAX.with(|c| c.set(c.get().max(part.len())));
                let n = match file.reader.read(part) {
                    Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                sender.put(part.get(..n).unwrap_or_default())?;
                left = left.saturating_sub(n as u64);
            }
        }
    }
    out.flush()
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

    /// A connection that hands out `step` bytes per read, then stalls.
    struct Trickle {
        data: Vec<u8>,
        at: usize,
        step: usize,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.step.min(buf.len()).min(self.data.len() - self.at);
            if n == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    impl Conn for Trickle {
        fn wait_at_most(&mut self, _: Duration) {}
    }

    fn trickle(data: &[u8], step: usize) -> Trickle {
        Trickle {
            data: data.to_vec(),
            at: 0,
            step,
        }
    }

    fn head_of(size: usize) -> Vec<u8> {
        let mut head = b"GET /checkpoint HTTP/1.1\r\nX-Pad: ".to_vec();
        head.resize(size - 4, b'a');
        head.extend_from_slice(b"\r\n\r\n");
        head
    }

    #[test]
    fn a_head_sent_a_byte_at_a_time_is_searched_in_linear_work() {
        for size in [2 << 10, 4 << 10, 8 << 10, MAX_HEAD] {
            HEAD_SCANNED.with(|c| c.set(0));
            let head =
                read_head(&mut trickle(&head_of(size), 1), &ServerConfig::default()).unwrap();
            assert_eq!((head.path.as_str(), head.length), ("checkpoint", None));
            let scanned = HEAD_SCANNED.with(std::cell::Cell::get);
            assert!(
                scanned <= 8 * size,
                "{scanned} bytes searched for a head of {size} bytes"
            );
        }
    }

    #[test]
    fn the_end_of_a_head_is_found_across_reads_at_every_offset() {
        let config = ServerConfig::default();
        for pad in 0..4 {
            let mut data = b"POST /submit HTTP/1.1\r\nContent-Length: 4\r\nX-Pad: ".to_vec();
            data.resize(data.len() + pad, b'a');
            data.extend_from_slice(b"\r\n\r\nrest");
            let end = data.len() - 4;
            // Every read size puts the delimiter's bytes at every offset of a read.
            for step in 1..=9 {
                let head = read_head(&mut trickle(&data, step), &config).unwrap();
                let read = end.div_ceil(step) * step;
                assert_eq!(head.length, Some(4), "pad {pad} step {step}");
                assert_eq!(
                    head.already,
                    data[end..read.min(data.len())],
                    "pad {pad} step {step}"
                );
            }
        }
        // The earliest delimiter ends the head; what follows is the body's.
        let data = b"POST /submit HTTP/1.1\r\nContent-Length: 4\r\n\r\n\r\n\r\n";
        let head = read_head(&mut trickle(data, 1024), &config).unwrap();
        assert_eq!(head.already, b"\r\n\r\n");
        // A head past the bound is still refused.
        let mut long = b"GET /checkpoint HTTP/1.1\r\nX-Pad: ".to_vec();
        long.resize(MAX_HEAD + 2048, b'a');
        let refused = read_head(&mut trickle(&long, 1), &config).err().unwrap();
        assert_eq!(refused.status, 431);
    }

    #[test]
    fn a_body_reserves_only_what_arrived_not_what_it_declared() {
        let config = ServerConfig {
            idle_timeout: Duration::from_secs(1),
            ..ServerConfig::default()
        };
        // A huge declared length with a few bytes sent reserves about what was sent.
        BODY_RESERVED.with(|c| c.set(0));
        let refused = read_body(
            &mut trickle(&[7u8; 100], 100),
            vec![1; 10],
            MAX_PUBLISH_BODY,
            &config,
        )
        .err()
        .unwrap();
        assert_eq!(refused.status, 408);
        let reserved = BODY_RESERVED.with(std::cell::Cell::get);
        assert!(
            reserved <= 128 << 10,
            "{reserved} bytes reserved for 110 received"
        );
        // A whole body is read, its buffer growing with the bytes and never past the length.
        let length = (1 << 20) + 17;
        let data: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
        BODY_RESERVED.with(|c| c.set(0));
        let body = read_body(
            &mut trickle(&data[3..], 4096),
            data[..3].to_vec(),
            length,
            &config,
        )
        .unwrap();
        assert_eq!(body, data);
        let reserved = BODY_RESERVED.with(std::cell::Cell::get);
        assert!(reserved <= length, "{reserved} bytes reserved for {length}");
        // Bytes past the declared length are refused, as before.
        let refused = read_body(&mut trickle(b"", 1), vec![0; 5], 4, &config)
            .err()
            .unwrap();
        assert_eq!(refused.status, 400);
    }

    /// A connection that hands out `step` bytes per read, each read taking `tick` on a clock
    /// of its own, then closes or stalls: the limits on time, with no wall clock.
    struct Paced {
        data: Vec<u8>,
        at: usize,
        step: usize,
        start: Instant,
        tick: Duration,
        reads: u32,
        closes: bool,
    }

    impl Read for Paced {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            let n = self.step.min(buf.len()).min(self.data.len() - self.at);
            if n == 0 {
                return if self.closes {
                    Ok(0)
                } else {
                    Err(io::ErrorKind::WouldBlock.into())
                };
            }
            buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    impl Conn for Paced {
        fn wait_at_most(&mut self, _: Duration) {}

        fn now(&self) -> Instant {
            self.start + self.tick * self.reads
        }
    }

    fn paced(data: Vec<u8>, step: usize, tick: Duration) -> Paced {
        Paced {
            data,
            at: 0,
            step,
            start: Instant::now(),
            tick,
            reads: 0,
            closes: false,
        }
    }

    #[test]
    fn a_trickling_body_loses_its_slot_after_the_idle_timeout() {
        // The review's case: the largest body declared, a byte every 59 seconds, which never
        // trips the idle timeout of 60 seconds. It is refused once the first minute is past,
        // not after the 70 minutes the declared length alone would give it.
        let config = ServerConfig::default();
        let mut conn = paced(vec![b'x'; 4096], 1, Duration::from_secs(59));
        let refused = read_body(&mut conn, Vec::new(), MAX_PUBLISH_BODY, &config)
            .err()
            .unwrap();
        assert_eq!(refused.status, 408);
        assert!(String::from_utf8_lossy(&refused.body).contains("slower"));
        let held = conn.now().saturating_duration_since(conn.start);
        assert!(
            held <= config.idle_timeout + Duration::from_secs(2 * 59),
            "held for {held:?}"
        );
        // A short idle timeout and a byte every 0.7 seconds: refused within a few ticks.
        let config = ServerConfig {
            idle_timeout: Duration::from_secs(1),
            ..ServerConfig::default()
        };
        let mut conn = paced(vec![b'x'; 4096], 1, Duration::from_millis(700));
        let refused = read_body(&mut conn, Vec::new(), MAX_PUBLISH_BODY, &config)
            .err()
            .unwrap();
        assert_eq!(refused.status, 408);
        assert!(conn.reads <= 3, "{} reads before the refusal", conn.reads);
        // Bytes that came with the head do not count toward the rate.
        let mut conn = paced(vec![b'x'; 4096], 1, Duration::from_secs(59));
        let refused = read_body(
            &mut conn,
            vec![b'y'; MAX_HEAD],
            MAX_PUBLISH_BODY,
            &ServerConfig::default(),
        )
        .err()
        .unwrap();
        assert_eq!(refused.status, 408);
        assert!(conn.reads <= 3, "{} reads before the refusal", conn.reads);
        // A body at half the rate is refused by the rate, before the total deadline.
        let mut conn = paced(vec![b'x'; 8 << 20], 32 << 10, Duration::from_secs(1));
        let refused = read_body(&mut conn, Vec::new(), 8 << 20, &ServerConfig::default())
            .err()
            .unwrap();
        assert_eq!(refused.status, 408);
        assert!(String::from_utf8_lossy(&refused.body).contains("slower"));
        let held = conn.now().saturating_duration_since(conn.start);
        assert!(held <= Duration::from_secs(122), "held for {held:?}");
    }

    #[test]
    fn an_honest_body_keeps_its_slot() {
        let config = ServerConfig::default();
        let length = (8 << 20) + 17;
        let data: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
        // Fast: 64 KiB every half second.
        let mut conn = paced(data[5..].to_vec(), 64 << 10, Duration::from_millis(500));
        let body = read_body(&mut conn, data[..5].to_vec(), length, &config).unwrap();
        assert_eq!(body, data);
        // Slow but above the rate: 65 KiB a second.
        let mut conn = paced(data.clone(), 65 << 10, Duration::from_secs(1));
        assert_eq!(
            read_body(&mut conn, Vec::new(), length, &config).unwrap(),
            data
        );
        // A note sent a few bytes a second within its first minute is read.
        let note = vec![b'n'; 1000];
        let mut conn = paced(note.clone(), 20, Duration::from_secs(1));
        assert_eq!(
            read_body(&mut conn, Vec::new(), 1000, &config).unwrap(),
            note
        );
    }

    #[test]
    fn a_head_at_the_bound_is_answered_however_it_arrives() {
        let config = ServerConfig::default();
        for end in MAX_HEAD - 4..=MAX_HEAD + 4 {
            let mut data = b"GET /checkpoint HTTP/1.1\r\nX-Pad: ".to_vec();
            data.resize(end, b'a');
            data.extend_from_slice(b"\r\n\r\n");
            let answers: Vec<Option<u16>> = [1, 2, 3, 1024, data.len()]
                .into_iter()
                .map(|step| {
                    read_head(&mut trickle(&data, step), &config)
                        .err()
                        .map(|e| e.status)
                })
                .collect();
            let expected = if end <= MAX_HEAD { None } else { Some(431) };
            assert!(
                answers.iter().all(|a| *a == expected),
                "end {end}: {answers:?}"
            );
        }
        // No delimiter at all: refused once nothing within the bound can end the head.
        let long = vec![b'a'; MAX_HEAD + 4];
        for step in [1, 4096, MAX_HEAD + 4] {
            let refused = read_head(&mut trickle(&long, step), &config).err().unwrap();
            assert_eq!(refused.status, 431, "step {step}");
        }
    }

    #[test]
    fn a_body_reallocation_holds_at_most_its_length_and_a_mib() {
        let config = ServerConfig::default();
        for length in [
            (1 << 20) - 1,
            1 << 20,
            (1 << 20) + 1,
            (3 << 20) + 5,
            (16 << 20) + 17,
        ] {
            let data = vec![3u8; length];
            for step in [4096, 64 << 10] {
                BODY_RESERVED.with(|c| c.set(0));
                BODY_PEAK.with(|c| c.set(0));
                let mut conn = paced(data.clone(), step, Duration::from_millis(1));
                let body = read_body(&mut conn, Vec::new(), length, &config).unwrap();
                assert_eq!(body.len(), length);
                let reserved = BODY_RESERVED.with(std::cell::Cell::get);
                let peak = BODY_PEAK.with(std::cell::Cell::get);
                assert!(reserved <= length, "{reserved} reserved for {length}");
                assert!(
                    peak <= length + FULL_BODY_RESERVE_AFTER,
                    "{peak} held at once for {length}"
                );
            }
        }
        // Below 1 MiB received, the buffer stays within twice what arrived.
        BODY_RESERVED.with(|c| c.set(0));
        let mut conn = paced(vec![1u8; 300 << 10], 4096, Duration::from_millis(1));
        let refused = read_body(&mut conn, Vec::new(), MAX_PUBLISH_BODY, &config)
            .err()
            .unwrap();
        assert_eq!(refused.status, 408);
        let reserved = BODY_RESERVED.with(std::cell::Cell::get);
        assert!(
            reserved <= 2 * (300 << 10),
            "{reserved} reserved for 300 KiB"
        );
    }

    #[test]
    fn a_refused_body_is_drained_within_its_bounds() {
        // At most `DRAIN_MAX` bytes, however much the peer sends.
        let mut conn = paced(vec![0u8; 1 << 20], 4096, Duration::from_millis(1));
        drain(&mut conn);
        assert_eq!(conn.at, DRAIN_MAX);
        // At most `DRAIN_TIME`, however slowly it sends.
        let mut conn = paced(vec![0u8; 1 << 20], 1, Duration::from_millis(100));
        drain(&mut conn);
        assert!(conn.at <= 10, "{} bytes drained", conn.at);
        // Not past a close.
        let mut conn = paced(vec![0u8; 100], 4096, Duration::from_millis(1));
        conn.closes = true;
        drain(&mut conn);
        assert_eq!(conn.at, 100);
    }

    /// A peer that takes at most `step` bytes per write, each write taking `tick` on a clock of
    /// its own, and stalls once it took `capacity` bytes, as a socket whose buffers are full.
    struct Taker {
        taken: usize,
        step: usize,
        capacity: usize,
        start: Instant,
        tick: Duration,
        writes: u32,
        longest_wait: Duration,
    }

    impl Write for Taker {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            let n = self.step.min(buf.len()).min(self.capacity - self.taken);
            if n == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.taken += n;
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Out for Taker {
        fn wait_to_send_at_most(&mut self, wait: Duration) {
            self.longest_wait = self.longest_wait.max(wait);
        }

        fn now(&self) -> Instant {
            self.start + self.tick * self.writes
        }
    }

    fn taker(step: usize, capacity: usize, tick: Duration) -> Taker {
        Taker {
            taken: 0,
            step,
            capacity,
            start: Instant::now(),
            tick,
            writes: 0,
            longest_wait: Duration::ZERO,
        }
    }

    /// A file of `left` bytes whose contents are never written, counting what was read of it.
    struct Lazy {
        left: u64,
        read: u64,
    }

    impl Read for Lazy {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = usize::try_from(self.left)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            self.left -= n as u64;
            self.read += n as u64;
            Ok(n)
        }
    }

    /// The answer of a file of `length` bytes, of which `present` can be read, and the count
    /// of the bytes read from it.
    fn lazy_file(length: u64, present: u64) -> (Answer, Arc<Mutex<u64>>) {
        struct Counted(Lazy, Arc<Mutex<u64>>);
        impl Read for Counted {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.read(buf)?;
                *self.1.lock().unwrap() = self.0.read;
                Ok(n)
            }
        }
        let count = Arc::new(Mutex::new(0));
        let reader = Counted(
            Lazy {
                left: present,
                read: 0,
            },
            Arc::clone(&count),
        );
        let answer = Answer::File {
            content_type: "application/octet-stream",
            immutable: true,
            file: Opened {
                length,
                reader: Box::new(reader),
            },
        };
        (answer, count)
    }

    #[test]
    fn a_file_is_sent_through_one_bounded_buffer_whatever_its_size() {
        let config = ServerConfig::default();
        // The largest file served, to a peer that takes everything: every read of the file is
        // into the one buffer of `SEND_CHUNK` bytes, and every write gives at most that.
        let length = MAX_SERVED as u64;
        let (answer, count) = lazy_file(length, length);
        let mut peer = taker(usize::MAX, usize::MAX, Duration::from_micros(1));
        SEND_READ_MAX.with(|c| c.set(0));
        send(&mut peer, answer, &config).unwrap();
        assert_eq!(*count.lock().unwrap(), length);
        assert!(peer.taken as u64 > length);
        assert_eq!(SEND_READ_MAX.with(std::cell::Cell::get), SEND_CHUNK);
        assert!(peer.writes as u64 <= length / SEND_CHUNK as u64 + 3);
        assert!(peer.longest_wait <= config.write_timeout);
    }

    #[test]
    fn a_peer_that_never_reads_is_dropped_at_its_first_stalled_write() {
        let config = ServerConfig {
            write_timeout: Duration::from_secs(1),
            ..ServerConfig::default()
        };
        // Its buffers take 256 KiB, then nothing: the file is read no further than what was
        // written and one more part, and each write waits at most the write timeout.
        let length = MAX_SERVED as u64;
        let (answer, count) = lazy_file(length, length);
        let mut peer = taker(usize::MAX, 256 << 10, Duration::from_millis(1));
        let failed = send(&mut peer, answer, &config).unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(peer.taken, 256 << 10);
        let read = *count.lock().unwrap();
        assert!(read <= (256 << 10) + SEND_CHUNK as u64, "{read} bytes read");
        assert_eq!(peer.longest_wait, config.write_timeout);
    }

    #[test]
    fn a_slow_reader_loses_its_connection_past_the_first_write_timeout() {
        let config = ServerConfig {
            write_timeout: Duration::from_secs(1),
            ..ServerConfig::default()
        };
        // A byte every 0.7 seconds: refused within a few writes past the first second.
        let (answer, _) = lazy_file(1 << 20, 1 << 20);
        let mut peer = taker(1, usize::MAX, Duration::from_millis(700));
        let failed = send(&mut peer, answer, &config).unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::TimedOut);
        assert!(
            peer.writes <= 3,
            "{} writes before the refusal",
            peer.writes
        );
        // Half the rate, with the default timeout: refused by the rate, about two minutes in,
        // not after the deadline the length alone would give.
        let (answer, _) = lazy_file(8 << 20, 8 << 20);
        let mut peer = taker(32 << 10, usize::MAX, Duration::from_secs(1));
        let failed = send(&mut peer, answer, &ServerConfig::default()).unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::TimedOut);
        let held = peer.now().saturating_duration_since(peer.start);
        assert!(held <= Duration::from_secs(122), "held for {held:?}");
        // A small response is under the same rule.
        let mut peer = taker(1, usize::MAX, Duration::from_millis(700));
        let refusal = Answer::Whole(text(503, "x".repeat(100)));
        assert!(send(&mut peer, refusal, &config).is_err());
    }

    #[test]
    fn an_honest_reader_takes_the_whole_file() {
        let config = ServerConfig::default();
        // Just above the rate: 65 KiB a second for 8 MiB.
        let length = 8 << 20;
        let (answer, count) = lazy_file(length, length);
        let mut peer = taker(65 << 10, usize::MAX, Duration::from_secs(1));
        send(&mut peer, answer, &config).unwrap();
        assert_eq!(*count.lock().unwrap(), length);
        // A file that ends before its length is an error, so the connection closes short.
        let (answer, _) = lazy_file(length, length - 1);
        let mut peer = taker(usize::MAX, usize::MAX, Duration::from_micros(1));
        let failed = send(&mut peer, answer, &config).unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_failed_accept_goes_on_unless_the_listener_is_broken() {
        for kind in [
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::Interrupted,
        ] {
            assert!(fails_one_connection(&kind.into()));
        }
        for errno in [23, 24] {
            assert!(fails_for_resources(&io::Error::from_raw_os_error(errno)));
        }
        assert!(fails_for_resources(&io::ErrorKind::OutOfMemory.into()));
        let other = io::Error::from(io::ErrorKind::InvalidInput);
        assert!(!fails_one_connection(&other) && !fails_for_resources(&other));
    }

    #[test]
    fn secrets_compare_whole() {
        assert!(same_secret(b"token", b"token"));
        assert!(!same_secret(b"token", b"tokem"));
        assert!(!same_secret(b"token", b"toke"));
    }
}
