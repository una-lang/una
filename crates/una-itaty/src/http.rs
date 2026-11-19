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
//! within 30 seconds. Its first byte arrives within 5 seconds (the first-byte timeout) of the
//! moment the connection starts to be read, which is when its thread starts, right after its
//! acceptance; and at every moment the head so far has arrived within the first-byte timeout
//! and a second per KiB received. So a connection that sends nothing gives its slot back
//! after 5 seconds, one that sends a byte and stops does a millisecond later, and holding a
//! slot for the 21 seconds the largest head allows costs its 16 KiB; a head that an honest
//! client sends whole, in one write, is not concerned. A body needs `Content-Length`, at most
//! the endpoint's bound (64 KiB for a note, the archive's bound for a publication), and is
//! read under the head's rule, with a body's numbers: at every moment the bytes of the body
//! read since the head have arrived within the first-byte timeout (or the idle timeout, when
//! that is shorter) and a second per 64 KiB of them, with no more than 60 seconds (the idle
//! timeout) without progress, within a total of the idle timeout and a second per 64 KiB
//! declared. So a head that declares a body and sends none of it gives its slot back after 5
//! seconds, and the longest a note's body holds a slot is the 6 seconds that cost its 64 KiB;
//! a publication's body holds its slot only while it keeps 64 KiB per second on average past
//! those 5 seconds, and at most two publications are read at once. Chunked bodies, percent-encoding, queries,
//! repeated `Content-Length` and folded headers are refused, so no byte of a body's framing
//! is outside the rule; `Expect` is not answered, so a client that waits for `100 Continue`
//! before it sends its body has the body's first-byte time to give up waiting. A body's
//! buffer grows with the bytes that arrive, doubling from 64 KiB, and takes its whole
//! declared length at once only after its first MiB arrived, so a reallocation never holds
//! more than the length and that MiB; the search for a head's end resumes where it stopped,
//! so its work is linear in the head.
//!
//! **Refused before the body.** What the head alone decides is answered with no read for the
//! body (only the bytes of it that came with the head's reads were taken): a head or a framing that is refused, a body past the endpoint's bound (413, by
//! its `Content-Length`), a publication with no publishing slot (503), and a registration
//! whose head does not carry the operator's token (403 when registration is closed, 401 for a
//! credential that is absent, malformed or wrong). A request refused so gets its answer, then
//! the service stops writing and discards what the body still declares, at most 64 KiB of it
//! for at most a second, before it closes, so a client that sent a small body reads the
//! refusal rather than a reset; the discarding ends with the body, so it waits for nothing
//! once a small body is whole. The other endpoints carry their credential, a signature, in
//! the body, which is therefore read first, under the rule above.
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
//! at once, each from its acceptance to its response's last byte: a connection takes its slot
//! when it is accepted, before any byte of its request, and holds it while its response is
//! written. Of those, at most `max_connections_per_peer` come from one peer, which is an IPv4
//! address or an IPv6 /64 prefix, so one such peer cannot hold every slot. That is the whole
//! of the protection: at the defaults four peers hold every slot between them, and a client
//! with four addresses or a wider IPv6 prefix is not bounded by it. Connections from a
//! loopback address are not bounded per peer, since they are a terminator's on the same
//! host; a terminator on another host is one peer (see [`ServerConfig`]). [`serve`] takes the
//! slot before it starts the connection's thread, so it runs at most `max_connections`
//! connection threads; a connection past either bound is answered 503 by one write that never
//! waits, then closed, and starts no thread. A thread that cannot start closes its connection
//! and gives its slot back, and the loop goes on accepting.
//!
//! **Timeouts are never absent.** A configuration with a zero timeout is refused
//! ([`ServerConfig::check`]), since the operating system reads a zero socket timeout as none.
//! A read or a write happens only after its timeout was set on the socket: when the socket
//! refuses a timeout, the request is not read further, the response is not written further,
//! and the connection closes.
//!
//! **The accept loop.** [`serve`] puts the listener in blocking mode, so an `accept` waits for
//! a connection. Only an error that says the listener itself is unusable (`EBADF`, `EINVAL`,
//! `ENOTSOCK`, `EFAULT`) ends [`serve`] at once. An `accept` that fails for the connection it
//! would have given (aborted, reset, interrupted) goes on at once, up to 16 times in a row,
//! then after a fixed pause of 5 ms each. Every other failure goes on after a pause that
//! doubles from 5 ms to a second, so an error that persists costs at most one `accept` a
//! second. The pauses start over at the next connection accepted. A listener that only fails
//! is not served for ever in silence: once the pauses since the last connection accepted add
//! up to 30 seconds, [`serve`] ends with the last error, for the operator's process to see.
//! Three kinds of failure do not count toward those 30 seconds, since they pass by themselves:
//! a want of descriptors, buffers or memory (`EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM`), which
//! the connections that end give back, a wait for a connection that timed out, and a failure
//! of one connection, which a peer can cause and so must not be a way to end [`serve`].
//!
//! **Descriptors.** A connection holds its socket and, while a file is sent, that file's open
//! descriptor, for as long as its response is written: up to 2 x `max_connections`
//! descriptors, and 16 more for the listener, the standard streams, a connection being
//! refused and the store's own files ([`descriptors_needed`]; 528 at the default). Below that,
//! a publication's write could fail for want of a descriptor, which stops the registry until
//! it is reopened. The service does not read the process's limit of open descriptors: the
//! standard library has no such read, so the operator sets a limit of at least the need
//! (`ulimit -n`) or lowers `max_connections` before starting it.
//!
//! **Publications.** At most two publications are read and checked at once, since each holds
//! its archive in memory: a publication takes its slot once its head is checked, before its
//! body is read.
//!
//! TLS is not here: the service is meant to sit behind a terminator that serves HTTPS
//! (rule 39: TLS keeps a private registry's credentials confidential; integrity rests on
//! digests, signatures and proofs).

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
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
/// The least average rate of a request's body past its first-byte time, in bytes per second:
/// the rate the body's total time already assumed (a second per 64 KiB declared), and the
/// mirror of [`MIN_SEND_RATE`]. A note of 64 KiB has one second more than its first byte.
const MIN_BODY_RATE: u64 = 64 << 10;
/// The most bytes of an unread body discarded after a refusal, before the connection closes.
const DRAIN_MAX: usize = 64 << 10;
/// The longest wait for those bytes.
const DRAIN_TIME: Duration = Duration::from_secs(1);
/// The most bytes of a response given to one write, and the buffer a file is sent through.
pub const SEND_CHUNK: usize = 64 << 10;
/// The least average rate at which a peer takes a response past its first write timeout, in
/// bytes per second: the mirror of [`MIN_BODY_RATE`].
const MIN_SEND_RATE: u128 = 64 << 10;
/// The first pause after an `accept` that failed and is tried again after a pause.
const ACCEPT_PAUSE_MIN: Duration = Duration::from_millis(5);
/// The longest such pause; each failure in a row doubles it up to this.
const ACCEPT_PAUSE_MAX: Duration = Duration::from_secs(1);
/// How many failures of one connection in a row an `accept` is tried again after at once;
/// past them, each is followed by a pause of `ACCEPT_PAUSE_MIN`, which does not double: such
/// a failure used up the connection it concerned, so it cannot repeat by itself.
const ACCEPT_FREE_RETRIES: u32 = 16;
/// The most time paused on failures of `accept` in a row, with no connection accepted, before
/// [`serve`] ends with the error; wants of resources, waits that timed out and failures of
/// one connection do not count.
const ACCEPT_FAILING_MAX: Duration = Duration::from_secs(30);
/// The least average rate of a request's head past the first-byte timeout, in bytes per
/// second: a head of `MAX_HEAD` bytes has 16 seconds more than its first byte.
const MIN_HEAD_RATE: u64 = 1 << 10;
/// The descriptors the service needs besides its connections': the standard streams, the
/// listener, the store's lock, a connection being refused, the files the registry reads and
/// writes (one at a time, under its lock), and a margin for the operator's own.
pub const RESERVED_DESCRIPTORS: u64 = 16;

/// How the endpoints run.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The operator's bearer token for `/register`; registration is closed without one.
    pub register_token: Option<String>,
    /// How many connections are served at once, each from its acceptance to its response's
    /// last byte; [`serve`] runs at most this many connection threads. At least 1. Each
    /// connection can hold two descriptors, its socket and the file it is sent, so the process
    /// needs a limit of open descriptors of at least [`descriptors_needed`], which the
    /// operator sets: the service does not check it.
    pub max_connections: usize,
    /// How many of those connections one peer holds at once; a peer is an IPv4 address or an
    /// IPv6 /64 prefix. At least 1. The default, 64, is a quarter of the default
    /// `max_connections`, so one address cannot hold every slot (four can). Connections from
    /// a loopback address are not counted against it: they are a terminator's on the same
    /// host, which bounds its clients itself. A terminator on another host is one peer: for
    /// it, set this to `max_connections` and bound the clients at the terminator.
    pub max_connections_per_peer: usize,
    /// The longest wait for a request's first byte, counted from the moment the connection
    /// starts to be read, right after its acceptance, within the head's time; past it, the
    /// head has a second more for each KiB of it that arrived. A client sends its request as
    /// soon as it is connected, so the default, 5 seconds, covers a retransmission of a lost
    /// first segment (1 second, then 2) with a margin, while a connection that sends nothing
    /// holds its slot a sixth of the head's 30 seconds. It is also the default wait for a
    /// request's first byte of the language's own servers (`spec/31`, `idle`). A body has the
    /// same wait for its first byte, counted from the end of the head, and past it a second
    /// more for each 64 KiB of it that arrived; when the idle timeout is shorter than this,
    /// the body's wait is the idle timeout.
    pub first_byte_timeout: Duration,
    /// The longest time to read a request's head, however fast it arrives.
    pub head_timeout: Duration,
    /// The longest wait for progress on a body, however much of it arrived, and with a second
    /// per 64 KiB declared the longest time to read it.
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
            max_connections_per_peer: 64,
            first_byte_timeout: Duration::from_secs(5),
            head_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(60),
            write_timeout: Duration::from_secs(60),
        }
    }
}

impl ServerConfig {
    /// Refuses a configuration the service cannot run safely: a zero timeout, which a socket
    /// takes as no timeout at all, or a bound of zero connections. [`Service::new`] calls it.
    pub fn check(&self) -> Result<(), String> {
        for (name, timeout) in [
            ("first_byte_timeout", self.first_byte_timeout),
            ("head_timeout", self.head_timeout),
            ("idle_timeout", self.idle_timeout),
            ("write_timeout", self.write_timeout),
        ] {
            if timeout.is_zero() {
                return Err(format!(
                    "`{name}` is zero, which would leave the socket with no timeout"
                ));
            }
        }
        for (name, bound) in [
            ("max_connections", self.max_connections),
            ("max_connections_per_peer", self.max_connections_per_peer),
        ] {
            if bound == 0 {
                return Err(format!(
                    "`{name}` is zero, so no connection would be served"
                ));
            }
        }
        Ok(())
    }
}

/// The descriptors a service with `config` needs: two for each of its `max_connections` (the
/// socket, and the file it is sent) and [`RESERVED_DESCRIPTORS`].
pub fn descriptors_needed(config: &ServerConfig) -> u64 {
    u64::try_from(config.max_connections)
        .unwrap_or(u64::MAX)
        .saturating_mul(2)
        .saturating_add(RESERVED_DESCRIPTORS)
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
    connections: Places,
    publishing: AtomicUsize,
}

/// The connection slots: how many are taken, and by which peers.
struct Places {
    /// The most slots taken at once.
    limit: usize,
    /// The most of them one peer takes.
    per_peer: usize,
    /// How many are taken; it changes only under the lock of `peers`.
    taken: AtomicUsize,
    /// How many each peer holds: an entry exists only while its count is positive, so the map
    /// holds at most `limit` entries.
    peers: Mutex<BTreeMap<Peer, usize>>,
}

/// Whose connections are counted together: an IPv4 address, or the /64 prefix of an IPv6
/// address, since one host commonly holds a whole /64. An IPv4-mapped IPv6 address counts as
/// its IPv4 address. The loopback addresses (`127.0.0.0/8`, `::1`) are this host, whose
/// connections are not bounded per peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Peer {
    Loopback,
    V4([u8; 4]),
    V6([u8; 8]),
}

impl Peer {
    fn of(address: IpAddr) -> Peer {
        match address.to_canonical() {
            address if address.is_loopback() => Peer::Loopback,
            IpAddr::V4(v4) => Peer::V4(v4.octets()),
            IpAddr::V6(v6) => {
                let [a, b, c, d, e, f, g, h, ..] = v6.octets();
                Peer::V6([a, b, c, d, e, f, g, h])
            }
        }
    }
}

/// Which bound refuses a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Busy {
    /// `max_connections` are being served.
    Service,
    /// `max_connections_per_peer` of them are the peer's.
    Peer,
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
struct Held(Arc<Service>, Peer);

impl Held {
    fn take(service: &Arc<Service>, peer: Peer) -> Result<Held, Busy> {
        service.connections.enter(peer)?;
        Ok(Held(Arc::clone(service), peer))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.0.connections.leave(self.1);
    }
}

/// A connection slot of a caller that holds the service, released when dropped.
struct Entered<'a>(&'a Places, Peer);

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        self.0.leave(self.1);
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

/// The response that refuses a registration by its `Authorization` header, `None` when the
/// header carries the operator's token: 403 while registration is closed, 401 for a
/// credential that is absent, malformed or wrong.
fn registration_refused(config: &ServerConfig, authorization: Option<&str>) -> Option<Response> {
    let Some(token) = &config.register_token else {
        return Some(text(403, "registration is closed"));
    };
    let given = authorization
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !same_secret(given.as_bytes(), token.as_bytes()) {
        return Some(text(401, "registration needs the operator's token"));
    }
    None
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
    /// The endpoints of `registry`, serving the files of `store`, which is the registry's;
    /// refused when `config` is one [`ServerConfig::check`] refuses.
    pub fn new(
        store: Arc<dyn Store>,
        registry: Registry,
        config: ServerConfig,
    ) -> Result<Service, String> {
        config.check()?;
        Ok(Service {
            store,
            registry: Arc::new(Mutex::new(registry)),
            connections: Places::new(config.max_connections, config.max_connections_per_peer),
            config,
            publishing: AtomicUsize::new(0),
        })
    }
}

impl Places {
    fn new(limit: usize, per_peer: usize) -> Places {
        Places {
            limit,
            per_peer,
            taken: AtomicUsize::new(0),
            peers: Mutex::new(BTreeMap::new()),
        }
    }

    fn taken(&self) -> usize {
        self.taken.load(Ordering::SeqCst)
    }

    /// The counts of the peers. The map is whole after every statement made under its lock
    /// (no code that can panic runs between them), so a poisoned lock still guards counts
    /// that are right.
    fn peers(&self) -> MutexGuard<'_, BTreeMap<Peer, usize>> {
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a connection slot for `peer`, or says which bound refuses it. Both counts change
    /// under the lock of the peers, so a refusal leaves no trace in either. A loopback peer
    /// is under the bound of the service alone, and has no entry among the peers.
    fn enter(&self, peer: Peer) -> Result<(), Busy> {
        let mut peers = self.peers();
        let held = peers.get(&peer).copied().unwrap_or(0);
        if peer != Peer::Loopback && held >= self.per_peer {
            return Err(Busy::Peer);
        }
        if self.taken() >= self.limit {
            return Err(Busy::Service);
        }
        self.taken.fetch_add(1, Ordering::SeqCst);
        if peer != Peer::Loopback {
            peers.insert(peer, held.saturating_add(1));
        }
        Ok(())
    }

    /// Gives back a slot [`Places::enter`] took for `peer`: the peer's first, so a caller
    /// that sees the count of connections fall finds the peer's count fallen too.
    fn leave(&self, peer: Peer) {
        let mut peers = self.peers();
        match peers.get(&peer).copied() {
            Some(held) if held > 1 => {
                peers.insert(peer, held - 1);
            }
            _ => {
                peers.remove(&peer);
            }
        }
        self.taken.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Service {
    /// The registry, for an operator's tools and tests.
    pub fn registry(&self) -> &Mutex<Registry> {
        &self.registry
    }

    /// How many connections hold a connection slot now, for an operator's tools and tests.
    pub fn connections_in_progress(&self) -> usize {
        self.connections.taken()
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
                // A connection was refused by its head already; a request given whole to
                // `route` is checked here.
                if let Some(refusal) =
                    registration_refused(&self.config, request.header("authorization"))
                {
                    return refusal;
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
    /// last byte. Past `max_connections`, or `max_connections_per_peer` for its peer, the
    /// connection is answered 503 by one write that never waits, and closed; a connection
    /// whose peer cannot be read is closed.
    pub fn handle(&self, stream: TcpStream) {
        let Ok(address) = stream.peer_addr() else {
            return;
        };
        let peer = Peer::of(address.ip());
        match self.connections.enter(peer) {
            Err(busy) => refuse_busy(stream, busy),
            Ok(()) => {
                let _entered = Entered(&self.connections, peer);
                self.answer(stream);
            }
        }
    }

    /// Serves one connection whose slot its caller holds, until the response's last byte.
    fn answer(&self, mut stream: TcpStream) {
        match self.serve_one(&mut stream) {
            Served::Read(answer) => {
                let _ = send(&mut stream, answer, &self.config);
            }
            Served::Refused(response, unread) => {
                // The body may still be on its way: answer, stop writing, and discard a
                // bounded part of it within the connection's slot, so a client that sent a
                // small body reads the refusal rather than a reset. A peer that timed out
                // sends nothing worth waiting for.
                let status = response.status;
                let sent = send(&mut stream, Answer::Whole(response), &self.config);
                if sent.is_ok() && status != 408 {
                    let _ = stream.shutdown(std::net::Shutdown::Write);
                    drain(&mut stream, unread);
                }
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

/// Answers a connection past the bound 503 by one write that never waits, and closes it: a
/// refusal of a few hundred bytes fits in a fresh connection's buffer, and a peer that does
/// not take it gets the close alone.
fn refuse_busy(mut stream: TcpStream, busy: Busy) {
    let refusal = match busy {
        Busy::Service => "too many connections; try again",
        Busy::Peer => "too many connections from this address; try again",
    };
    // A socket that cannot be made non-blocking is closed unanswered: a write could wait.
    if stream.set_nonblocking(true).is_ok() {
        let _ = write_response(&mut stream, &text(503, refusal));
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

impl Service {
    /// Reads and answers one request. A publication takes its publishing slot once its head
    /// is checked and before its body is read, so at most two bodies of publications are held
    /// at once, however many connections declare one.
    fn serve_one(&self, stream: &mut TcpStream) -> Served {
        // A head that was refused says nothing sure of a body: the most is discarded.
        let head = match read_head(stream, &self.config) {
            Ok(head) => head,
            Err(response) => return Served::Refused(response, DRAIN_MAX),
        };
        // What the head alone refuses is answered with no byte of the body read.
        let unread = head.unread();
        if let Some(response) = head.refused(&self.config) {
            return Served::Refused(response, unread);
        }
        let slot = if head.publishes() {
            match self.publishing_slot() {
                Ok(slot) => Some(slot),
                Err(response) => return Served::Refused(response, unread),
            }
        } else {
            None
        };
        match head.read_body(stream, &self.config) {
            Ok(request) if slot.is_some() => Served::Read(Answer::Whole(self.write(request, slot))),
            Ok(request) => Served::Read(self.dispatch(request)),
            Err(response) => Served::Refused(response, DRAIN_MAX),
        }
    }
}

/// The answer to a connection's request, and whether its request was read whole.
enum Served {
    /// The request was read whole.
    Read(Answer),
    /// The request was refused before its body was read whole; the most bytes of the body
    /// that may still come, to discard.
    Refused(Response, usize),
}

/// Discards at most `most` bytes of `stream`, and at most `DRAIN_MAX`, within `DRAIN_TIME`,
/// until it closes: it waits for nothing once `most` bytes were discarded.
fn drain(stream: &mut impl Conn, most: usize) {
    let started = stream.now();
    let mut left = most.min(DRAIN_MAX);
    let mut chunk = [0u8; 4096];
    while left > 0 {
        let Some(wait) = DRAIN_TIME
            .checked_sub(stream.now().saturating_duration_since(started))
            .filter(|d| !d.is_zero())
        else {
            return;
        };
        if stream.wait_at_most(wait).is_err() {
            return;
        }
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
/// clock its limits are measured with. A bound that cannot be set is an error, and its caller
/// does not read.
trait Conn: Read {
    fn wait_at_most(&mut self, wait: Duration) -> io::Result<()>;

    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl Conn for TcpStream {
    fn wait_at_most(&mut self, wait: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(wait))
    }
}

/// The refusal of a request whose socket took no read timeout: it is not read without one.
fn no_timeout() -> Response {
    text(503, "the connection's timeout could not be set")
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
/// refuses it; a request its head alone refuses is refused with no byte of its body read.
pub fn read_request(stream: &mut TcpStream, config: &ServerConfig) -> Result<Request, Response> {
    let head = read_head(stream, config)?;
    match head.refused(config) {
        Some(response) => Err(response),
        None => head.read_body(stream, config),
    }
}

/// The longest a part of a request may have taken when `received` bytes of it arrived: the
/// time `first` for its first byte and a second per `rate` bytes received, at most `most`.
/// It is the one rule of the head and of the body: nothing received has `first` alone, and a
/// byte now and then buys its share of a second, never the whole of `most`.
fn time_earned(first: Duration, received: usize, rate: u64, most: Duration) -> Duration {
    let earned = (received as u64).saturating_mul(1_000_000_000) / rate.max(1);
    first.saturating_add(Duration::from_nanos(earned)).min(most)
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
        // The head so far has the first-byte timeout and a second per `MIN_HEAD_RATE` bytes
        // of it, within the head's time: nothing received has the first-byte timeout alone,
        // and a byte now and then buys a millisecond each, not the head's whole time.
        let allowed = time_earned(
            config.first_byte_timeout,
            head.len(),
            MIN_HEAD_RATE,
            config.head_timeout,
        );
        let late = if head.is_empty() {
            "the request did not start in time"
        } else {
            "the request's head took too long"
        };
        let left = allowed
            .checked_sub(stream.now().saturating_duration_since(started))
            .filter(|d| !d.is_zero())
            .ok_or_else(|| text(408, late))?;
        if stream.wait_at_most(left).is_err() {
            return Err(no_timeout());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(text(400, "the connection closed inside the request's head")),
            Ok(n) => head.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e) if is_timeout(&e) => return Err(text(408, late)),
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

    /// The response that refuses the request by its head alone, before any byte of its body
    /// is read: a registration whose head does not carry the operator's token. The other
    /// endpoints carry their credential, a signature, in the body.
    fn refused(&self, config: &ServerConfig) -> Option<Response> {
        if self.method == "POST" && self.path == "register" {
            let authorization = self
                .headers
                .iter()
                .find(|(name, _)| name == "authorization")
                .map(|(_, value)| value.as_str());
            return registration_refused(config, authorization);
        }
        None
    }

    /// How many bytes of the body the head declared were not read with it.
    fn unread(&self) -> usize {
        self.length.unwrap_or(0).saturating_sub(self.already.len())
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
    let deadline = config
        .idle_timeout
        .saturating_add(Duration::from_secs((length / (64 << 10)) as u64));
    // The wait for the body's first byte is the request's, and never past the idle timeout,
    // the longest wait for a body's progress.
    let first = config.first_byte_timeout.min(config.idle_timeout);
    // The buffer grows with the bytes that arrived, never by the declared length alone, so a
    // peer that declares a large body and sends little holds little memory.
    let before = already.len();
    let mut body = already;
    let mut chunk = vec![0u8; 64 << 10];
    while body.len() < length {
        #[cfg(test)]
        BODY_RESERVED.with(|c| c.set(c.get().max(body.capacity())));
        // The head's rule, with the body's numbers: the bytes read since the head have the
        // first-byte time and a second per `MIN_BODY_RATE` of them, within the body's total
        // time. So a body that does not come loses its slot at the first-byte time, and one
        // that sends a byte now and then a moment later, however large the length declared.
        // The bytes that came with the head earn nothing: they cost the peer no time here.
        let received = body.len().saturating_sub(before);
        let allowed = time_earned(first, received, MIN_BODY_RATE, deadline);
        let late = if body.is_empty() {
            "the body did not start in time"
        } else if allowed >= deadline {
            "the body took too long"
        } else {
            "the body arrived slower than 64 KiB per second on average"
        };
        let left = allowed
            .checked_sub(stream.now().saturating_duration_since(started))
            .filter(|d| !d.is_zero())
            .ok_or_else(|| text(408, late))?;
        // No read waits past the idle timeout, however much time the bytes received earned.
        let wait = left.min(config.idle_timeout);
        if stream.wait_at_most(wait).is_err() {
            return Err(no_timeout());
        }
        let want = (length - body.len()).min(chunk.len());
        match stream.read(chunk.get_mut(..want).unwrap_or_default()) {
            Ok(0) => return Err(text(400, "the connection closed inside the body")),
            Ok(n) => {
                grow(&mut body, n, length)?;
                body.extend_from_slice(chunk.get(..n).unwrap_or_default());
            }
            Err(e) if is_timeout(&e) && wait < left => {
                return Err(text(408, "the body made no progress"));
            }
            Err(e) if is_timeout(&e) => return Err(text(408, late)),
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
/// of them (the module's rules). It puts `listener` in blocking mode. It ends at once with the
/// error of a listener that is unusable, and with the last error once its `accept` only
/// failed, with no connection accepted, through 30 seconds of pauses (the module's rules):
/// it returns `Err` when the listener keeps failing, and its caller decides whether to start
/// it again.
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
/// the slot back, and the loop goes on. Like [`serve`], it returns `Err` when the listener
/// keeps failing, and its caller decides whether to start it again.
pub fn serve_with(
    listener: &TcpListener,
    service: &Arc<Service>,
    spawn: impl Fn(Work) -> io::Result<()>,
) -> io::Result<()> {
    // An `accept` waits for a connection: on a non-blocking listener it would fail at once,
    // again and again. The loop pauses on that failure too, should the mode change under it.
    listener.set_nonblocking(false)?;
    serve_from(|| listener.accept(), service, spawn)
}

/// Serves the connections `accept` gives as [`serve_with`] serves a listener's, under the same
/// rules for a failed `accept`: for an operator whose connections do not come from one
/// listener of the standard library, and for tests. A connection is counted under the peer
/// address `accept` gives with it, the one `max_connections_per_peer` bounds: [`serve_with`]
/// gives the address the listener reported, and a caller that gives another answers for it.
/// `accept` waits for a connection; one that fails at once, again and again, is paused on as
/// the module says.
pub fn serve_from(
    accept: impl FnMut() -> io::Result<(TcpStream, SocketAddr)>,
    service: &Arc<Service>,
    spawn: impl Fn(Work) -> io::Result<()>,
) -> io::Result<()> {
    accept_loop(
        accept,
        std::thread::sleep,
        |stream, address| match Held::take(service, Peer::of(address.ip())) {
            Err(busy) => refuse_busy(stream, busy),
            Ok(held) => {
                let _ = spawn(Box::new(move || {
                    held.0.answer(stream);
                    drop(held);
                }));
            }
        },
    )
}

/// The accept loop of [`serve_with`], over what accepts a connection, what pauses, and what
/// serves a connection accepted. It ends only with an error: that of a listener that is
/// unusable, or the last of the failures that went on too long.
fn accept_loop(
    mut accept: impl FnMut() -> io::Result<(TcpStream, SocketAddr)>,
    mut pause: impl FnMut(Duration),
    mut connection: impl FnMut(TcpStream, SocketAddr),
) -> io::Result<()> {
    let mut retries = Retries::new();
    loop {
        let (stream, address) = match accept() {
            Ok(accepted) => {
                retries = Retries::new();
                accepted
            }
            Err(e) => match retries.after(&e) {
                Retry::Never => return Err(e),
                Retry::AtOnce => continue,
                Retry::After(wait) => {
                    pause(wait);
                    continue;
                }
            },
        };
        connection(stream, address);
    }
}

/// What a failed `accept` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    /// Only the connection it would have given failed.
    OneConnection,
    /// The listener is unusable, and no later `accept` can succeed.
    Listener,
    /// A want of descriptors, buffers or memory, which the connections that end give back.
    Resources,
    /// No connection came within the wait: a listener with a timeout, or one that does not
    /// block.
    Waiting,
    /// Anything else: a pending network error of a connection, or an error this module does
    /// not know.
    Other,
}

/// `EBADF`, `EFAULT` and `EINVAL`, the same numbers on Linux and macOS, and `ENOTSOCK`: the
/// errors of `accept(2)` that say the listener is not a listening socket any more.
const LISTENER_ERRORS: [i32; 4] = [
    9,
    14,
    22,
    if cfg!(any(target_os = "linux", target_os = "android")) {
        88
    } else {
        38
    },
];

/// `ENOMEM`, `ENFILE` and `EMFILE`, the same numbers on Linux and macOS, and `ENOBUFS`.
const RESOURCE_ERRORS: [i32; 4] = [
    12,
    23,
    24,
    if cfg!(any(target_os = "linux", target_os = "android")) {
        105
    } else {
        55
    },
];

/// Sorts a failed `accept`. Only the errors that name the listener itself end the loop at
/// once; `accept(2)` documents the others (`ECONNABORTED`, `EMFILE`, `ENFILE`, `ENOBUFS`,
/// `ENOMEM`, `EPERM`, `EPROTO`, `ENETDOWN`, `ENOPROTOOPT`, `EHOSTDOWN`, `ENONET`,
/// `EHOSTUNREACH`, `EOPNOTSUPP`, `ENETUNREACH`, `EAGAIN`, `ETIMEDOUT`) as passing or as one
/// connection's.
fn accept_failure(e: &io::Error) -> Failure {
    let numbered = |errors: &[i32]| {
        e.raw_os_error()
            .is_some_and(|errno| errors.contains(&errno))
    };
    if e.kind() == io::ErrorKind::InvalidInput || numbered(&LISTENER_ERRORS) {
        return Failure::Listener;
    }
    if numbered(&RESOURCE_ERRORS) {
        return Failure::Resources;
    }
    match e.kind() {
        io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::Interrupted => Failure::OneConnection,
        io::ErrorKind::OutOfMemory => Failure::Resources,
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Failure::Waiting,
        _ => Failure::Other,
    }
}

/// When a failed `accept` is tried again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retry {
    AtOnce,
    After(Duration),
    Never,
}

/// The failures of `accept` since the last connection it gave.
struct Retries {
    /// The failures of one connection tried again at once.
    free: u32,
    /// The next pause that doubles.
    pause: Duration,
    /// The pauses made on the failures that count toward `ACCEPT_FAILING_MAX`.
    failing: Duration,
}

impl Retries {
    fn new() -> Retries {
        Retries {
            free: 0,
            pause: ACCEPT_PAUSE_MIN,
            failing: Duration::ZERO,
        }
    }

    /// When to try again after the failure `e`. Never when the listener is unusable. At once
    /// for the first failures in a row of one connection, then after `ACCEPT_PAUSE_MIN` each.
    /// Otherwise after a pause that doubles up to `ACCEPT_PAUSE_MAX`, so no error that
    /// persists makes the loop spin. And never once the pauses on failures that do not pass
    /// by themselves add up to `ACCEPT_FAILING_MAX`, so a listener that only fails ends the
    /// loop with its error.
    fn after(&mut self, e: &io::Error) -> Retry {
        let failure = accept_failure(e);
        let wait = match failure {
            Failure::Listener => return Retry::Never,
            Failure::OneConnection if self.free < ACCEPT_FREE_RETRIES => {
                self.free += 1;
                return Retry::AtOnce;
            }
            Failure::OneConnection => ACCEPT_PAUSE_MIN,
            Failure::Resources | Failure::Waiting | Failure::Other => {
                let wait = self.pause;
                self.pause = wait.saturating_mul(2).min(ACCEPT_PAUSE_MAX);
                wait
            }
        };
        if failure == Failure::Other {
            if self.failing >= ACCEPT_FAILING_MAX {
                return Retry::Never;
            }
            self.failing = self.failing.saturating_add(wait);
        }
        Retry::After(wait)
    }
}

/// A connection a response is written to: bytes, a bound on the wait for each write, and the
/// clock its limits are measured with. A bound that cannot be set is an error, and its caller
/// does not write.
trait Out: Write {
    fn wait_to_send_at_most(&mut self, wait: Duration) -> io::Result<()>;

    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl Out for TcpStream {
    fn wait_to_send_at_most(&mut self, wait: Duration) -> io::Result<()> {
        self.set_write_timeout(Some(wait))
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
            // A socket that takes no timeout is not written to: the error closes it.
            self.out
                .wait_to_send_at_most(left.min(self.write_timeout))?;
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
        fn wait_at_most(&mut self, _: Duration) -> io::Result<()> {
            Ok(())
        }
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
        /// The bound of each wait asked for, in order.
        waits: Vec<Duration>,
        /// Whether setting a bound fails, as on a socket that refuses a timeout.
        refuses_timeouts: bool,
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
        fn wait_at_most(&mut self, wait: Duration) -> io::Result<()> {
            self.waits.push(wait);
            if self.refuses_timeouts {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            Ok(())
        }

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
            waits: Vec::new(),
            refuses_timeouts: false,
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
        // Exactly the least rate, 64 KiB a second from the first second on, is read too.
        let mut conn = paced(data.clone(), 64 << 10, Duration::from_secs(1));
        assert_eq!(
            read_body(&mut conn, Vec::new(), length, &config).unwrap(),
            data
        );
        // A small body sent slowly is read while it ends within the first-byte time and the
        // time its bytes earned: the largest note at 11 KiB a second takes 6 seconds, and a
        // cosignature of 2 KiB at 512 bytes a second takes 4.
        let note = vec![b'n'; MAX_NOTE_BODY];
        let mut conn = paced(note.clone(), 11 << 10, Duration::from_secs(1));
        assert_eq!(
            read_body(&mut conn, Vec::new(), MAX_NOTE_BODY, &config).unwrap(),
            note
        );
        let line = vec![b'c'; MAX_COSIGN_BODY];
        let mut conn = paced(line.clone(), 512, Duration::from_secs(1));
        assert_eq!(
            read_body(&mut conn, Vec::new(), MAX_COSIGN_BODY, &config).unwrap(),
            line
        );
    }

    #[test]
    fn a_body_that_does_not_come_loses_its_slot_at_the_first_byte_timeout() {
        let config = ServerConfig::default();
        assert!(config.first_byte_timeout < config.idle_timeout);
        let silent = |length: usize, config: &ServerConfig| {
            let mut conn = paced(Vec::new(), 1, Duration::from_secs(1));
            let refused = read_body(&mut conn, Vec::new(), length, config)
                .err()
                .unwrap();
            assert_eq!(refused.status, 408);
            assert!(String::from_utf8_lossy(&refused.body).contains("did not start"));
            conn.waits
        };
        // The review's case: a head that declares the largest note, then nothing. One wait,
        // of the first-byte timeout, not of the idle timeout's 60 seconds; the same for the
        // largest publication and for one byte declared.
        for length in [1, MAX_COSIGN_BODY, MAX_NOTE_BODY, MAX_PUBLISH_BODY] {
            assert_eq!(
                silent(length, &config),
                [config.first_byte_timeout],
                "{length}"
            );
        }
        // A first-byte timeout past the idle timeout is the idle timeout.
        let long = ServerConfig {
            first_byte_timeout: Duration::from_secs(3600),
            ..ServerConfig::default()
        };
        assert_eq!(silent(MAX_NOTE_BODY, &long), [long.idle_timeout]);
        // The bytes that came with the head earn no time: the wait is the same.
        let mut conn = paced(Vec::new(), 1, Duration::from_secs(1));
        let refused = read_body(&mut conn, vec![b'y'; 1024], MAX_NOTE_BODY, &config)
            .err()
            .unwrap();
        assert_eq!(refused.status, 408);
        assert_eq!(conn.waits, [config.first_byte_timeout]);
    }

    #[test]
    fn a_body_sent_slowly_loses_its_slot_soon_after_the_first_byte_timeout() {
        let config = ServerConfig::default();
        let first = config.first_byte_timeout;
        let refused_slow = |conn: &mut Paced, length: usize| {
            let refused = read_body(conn, Vec::new(), length, &config).err().unwrap();
            assert_eq!(refused.status, 408);
            assert!(String::from_utf8_lossy(&refused.body).contains("slower"));
        };
        // One byte at once, then nothing: the second wait ends a moment past the first-byte
        // timeout, not at the idle timeout.
        let mut conn = paced(b"x".to_vec(), 1, Duration::from_millis(1));
        refused_slow(&mut conn, MAX_NOTE_BODY);
        assert_eq!(conn.waits.len(), 2);
        assert!(conn.waits[1] <= first, "{:?}", conn.waits);
        // A byte a second: refused at the first-byte timeout, however large the body.
        for length in [MAX_NOTE_BODY, MAX_PUBLISH_BODY] {
            let mut conn = paced(vec![b'x'; 4096], 1, Duration::from_secs(1));
            refused_slow(&mut conn, length);
            assert!(
                held(&conn) <= first + Duration::from_secs(1),
                "{:?}",
                held(&conn)
            );
            assert!(conn.at <= 6, "{} bytes read", conn.at);
        }
        // Half the least rate: refused at twice the first-byte timeout, not after the two
        // minutes the idle timeout gave it.
        let mut conn = paced(vec![b'x'; 8 << 20], 32 << 10, Duration::from_secs(1));
        refused_slow(&mut conn, 8 << 20);
        assert!(
            held(&conn) <= 2 * first + Duration::from_secs(1),
            "{:?}",
            held(&conn)
        );
        // The longest hold of a note's slot: all of the largest note but its last byte, sent
        // in the first second, earns a second less its share: the wait that follows ends
        // within 6 seconds of the body's start, so the hold costs 64 KiB.
        let second = Duration::from_secs(1);
        let mut conn = paced(vec![b'n'; MAX_NOTE_BODY - 1], MAX_NOTE_BODY, second);
        refused_slow(&mut conn, MAX_NOTE_BODY);
        assert_eq!(conn.waits.len(), 2);
        let longest = second + conn.waits[1];
        assert!(
            longest > first + Duration::from_millis(999) && longest < first + second,
            "{longest:?}"
        );
        // No wait is past the idle timeout, however much time the bytes received earned: a
        // publication that sends 32 MiB at once and stalls is refused for its lack of
        // progress after one wait of the idle timeout.
        let mut conn = paced(vec![b'p'; 32 << 20], 32 << 20, Duration::from_millis(1));
        let refused = read_body(&mut conn, Vec::new(), MAX_PUBLISH_BODY, &config)
            .err()
            .unwrap();
        assert_eq!(refused.status, 408);
        assert!(String::from_utf8_lossy(&refused.body).contains("no progress"));
        assert!(conn.waits.iter().all(|w| *w <= config.idle_timeout));
        assert_eq!(conn.waits.last(), Some(&config.idle_timeout));
    }

    #[test]
    fn the_head_and_the_body_earn_their_time_by_one_rule() {
        let first = Duration::from_secs(5);
        let most = Duration::from_secs(30);
        // Nothing received has the first-byte time alone; each `rate` bytes add a second.
        assert_eq!(time_earned(first, 0, 1024, most), first);
        assert_eq!(time_earned(first, 1024, 1024, most), Duration::from_secs(6));
        assert_eq!(
            time_earned(first, 1, 1024, most),
            first + Duration::from_nanos(976_562)
        );
        assert_eq!(
            time_earned(first, 3 * (64 << 10), 64 << 10, most),
            Duration::from_secs(8)
        );
        // Never past the most, whatever arrived, and no overflow at the largest counts.
        assert_eq!(time_earned(first, 1 << 20, 1024, most), most);
        assert_eq!(time_earned(first, usize::MAX, 1, most), most);
        assert_eq!(time_earned(Duration::MAX, usize::MAX, 1, most), most);
        assert_eq!(time_earned(most, 0, 1024, first), first);
    }

    fn register_head(lines: &str) -> Vec<u8> {
        format!("POST /register HTTP/1.1\r\nContent-Length: 65536\r\n{lines}\r\n").into_bytes()
    }

    #[test]
    fn a_registration_without_the_token_is_refused_by_its_head() {
        let open = ServerConfig {
            register_token: Some("operator".to_owned()),
            ..ServerConfig::default()
        };
        let closed = ServerConfig::default();
        let refusal = |lines: &str, config: &ServerConfig| {
            let mut conn = trickle(&register_head(lines), 1024);
            let head = read_head(&mut conn, config).unwrap();
            assert_eq!(head.unread(), 65536);
            head.refused(config).map(|response| response.status)
        };
        // No credential, a malformed one, a wrong one: 401 from the head alone.
        for lines in [
            "",
            "Authorization: operator\r\n",
            "Authorization: Basic operator\r\n",
            "Authorization: bearer operator\r\n",
            "Authorization: Bearer\r\n",
            "Authorization: Bearer operator2\r\n",
            "Authorization: Bearer operato\r\n",
            "Authorization: Bearer  operator\r\n",
            "Expect: 100-continue\r\n",
        ] {
            assert_eq!(refusal(lines, &open), Some(401), "{lines:?}");
        }
        // The first `Authorization` header is the one read, as when the request is routed.
        let two = "Authorization: Bearer x\r\nAuthorization: Bearer operator\r\n";
        assert_eq!(refusal(two, &open), Some(401));
        // Registration that is closed is 403, whatever the head carries.
        for lines in ["", "Authorization: Bearer operator\r\n"] {
            assert_eq!(refusal(lines, &closed), Some(403), "{lines:?}");
        }
        // The operator's token is not refused, and its body is then read.
        assert_eq!(refusal("Authorization: Bearer operator\r\n", &open), None);
        assert_eq!(refusal("authorization:Bearer operator\r\n", &open), None);
        // The endpoints whose credential is in the body are not refused by their heads.
        for path in ["submit", "cosign", "publish"] {
            let data = format!("POST /{path} HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc");
            let head = read_head(&mut trickle(data.as_bytes(), 1024), &closed).unwrap();
            assert_eq!((head.unread(), head.refused(&closed)), (7, None), "{path}");
        }
        let head = read_head(
            &mut trickle(b"GET /register HTTP/1.1\r\n\r\n", 1024),
            &closed,
        )
        .unwrap();
        assert_eq!((head.unread(), head.refused(&closed)), (0, None));
    }

    #[test]
    fn a_request_refused_by_its_head_reads_no_byte_of_its_body() {
        let config = ServerConfig {
            register_token: Some("operator".to_owned()),
            ..ServerConfig::default()
        };
        // The whole request of a reader that holds no service: the head's reads and no more,
        // though a body is declared and its first bytes are there to read.
        let mut data = register_head("");
        let head_length = data.len();
        data.extend_from_slice(&[b'b'; 4096]);
        let mut conn = paced(data, head_length, Duration::from_secs(1));
        let head = read_head(&mut conn, &config).unwrap();
        let unread = head.unread();
        assert_eq!(head.refused(&config).map(|r| r.status), Some(401));
        assert_eq!((conn.reads, conn.at, unread), (1, head_length, 65536));
        // With the token, the body is read, under the body's rule.
        let mut data = register_head("Authorization: Bearer operator\r\n");
        let head_length = data.len();
        data.extend_from_slice(&[b'b'; 65536]);
        let mut conn = paced(data, 65536, Duration::from_millis(1));
        let head = read_head(&mut conn, &config).unwrap();
        assert_eq!(head.refused(&config), None);
        let request = head.read_body(&mut conn, &config).unwrap();
        assert_eq!(request.body.len(), 65536);
        assert_eq!(conn.at, head_length + 65536);
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
        drain(&mut conn, DRAIN_MAX);
        assert_eq!(conn.at, DRAIN_MAX);
        // At most `DRAIN_TIME`, however slowly it sends.
        let mut conn = paced(vec![0u8; 1 << 20], 1, Duration::from_millis(100));
        drain(&mut conn, DRAIN_MAX);
        assert!(conn.at <= 10, "{} bytes drained", conn.at);
        // Not past a close.
        let mut conn = paced(vec![0u8; 100], 4096, Duration::from_millis(1));
        conn.closes = true;
        drain(&mut conn, DRAIN_MAX);
        assert_eq!(conn.at, 100);
        // Not past what the body still declared: once it is discarded, nothing is waited
        // for, and with nothing declared nothing is read.
        let mut conn = paced(vec![0u8; 100], 4096, Duration::from_millis(1));
        drain(&mut conn, 100);
        assert_eq!((conn.at, conn.reads), (100, 1));
        let mut conn = paced(vec![0u8; 1 << 20], 4096, Duration::from_millis(1));
        drain(&mut conn, 5000);
        assert_eq!((conn.at, conn.reads), (5000, 2));
        let mut conn = paced(vec![0u8; 100], 4096, Duration::from_millis(1));
        drain(&mut conn, 0);
        assert_eq!((conn.at, conn.reads, conn.waits.len()), (0, 0, 0));
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
        /// Whether setting a bound fails, as on a socket that refuses a timeout.
        refuses_timeouts: bool,
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
        fn wait_to_send_at_most(&mut self, wait: Duration) -> io::Result<()> {
            if self.refuses_timeouts {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            self.longest_wait = self.longest_wait.max(wait);
            Ok(())
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
            refuses_timeouts: false,
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

    /// The numbers of the errors `accept(2)` documents as passing or as one connection's,
    /// besides those the standard library names: `EPERM`, `EAGAIN`, `ENOBUFS`, `EPROTO`,
    /// `ENETDOWN`, `ENETUNREACH`, `ENOPROTOOPT`, `EHOSTDOWN`, `EHOSTUNREACH`, `EOPNOTSUPP`,
    /// `ETIMEDOUT`, and on Linux `ENONET`.
    #[cfg(target_os = "linux")]
    const PASSING: &[i32] = &[1, 11, 105, 71, 100, 101, 92, 112, 113, 95, 110, 64];
    #[cfg(not(target_os = "linux"))]
    const PASSING: &[i32] = &[1, 35, 55, 100, 50, 51, 42, 64, 65, 102, 60];

    #[test]
    fn a_failed_accept_goes_on_unless_the_listener_is_broken() {
        for kind in [
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::Interrupted,
        ] {
            assert_eq!(accept_failure(&kind.into()), Failure::OneConnection);
        }
        // A want of descriptors (`ENFILE`, `EMFILE`), of memory or of buffers goes on, after
        // a pause.
        for errno in [23, 24, 12, PASSING[2]] {
            let e = io::Error::from_raw_os_error(errno);
            assert_eq!(accept_failure(&e), Failure::Resources, "errno {errno}");
            assert!(matches!(Retries::new().after(&e), Retry::After(_)));
        }
        assert_eq!(
            accept_failure(&io::ErrorKind::OutOfMemory.into()),
            Failure::Resources
        );
        // So do the pending network errors, a wait that timed out, a listener that does not
        // block, and an error with no number at all.
        for errno in PASSING {
            let e = io::Error::from_raw_os_error(*errno);
            assert!(
                matches!(
                    accept_failure(&e),
                    Failure::Other | Failure::Resources | Failure::Waiting
                ),
                "errno {errno}"
            );
            assert!(matches!(Retries::new().after(&e), Retry::After(_)));
        }
        for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut] {
            assert_eq!(accept_failure(&kind.into()), Failure::Waiting);
        }
        assert_eq!(accept_failure(&io::ErrorKind::Other.into()), Failure::Other);
        // Only the listener's own errors end the loop at once.
        let other = io::Error::from(io::ErrorKind::InvalidInput);
        assert_eq!(accept_failure(&other), Failure::Listener);
        assert_eq!(Retries::new().after(&other), Retry::Never);
        for errno in LISTENER_ERRORS {
            let e = io::Error::from_raw_os_error(errno);
            assert_eq!(accept_failure(&e), Failure::Listener, "errno {errno}");
        }
        assert!(PASSING.iter().all(|n| !LISTENER_ERRORS.contains(n)));
        assert!(RESOURCE_ERRORS.iter().all(|n| !LISTENER_ERRORS.contains(n)));
    }

    /// Runs the accept loop over `failures`, then `EBADF`, which ends it if nothing did
    /// before; the pauses it made, how many times it called `accept`, and the error it ended
    /// with.
    fn looped(failures: Vec<io::Error>) -> (Vec<Duration>, usize, io::Error) {
        let mut script = failures.into_iter();
        let mut pauses = Vec::new();
        let mut calls = 0;
        let ended = accept_loop(
            || {
                calls += 1;
                Err(script
                    .next()
                    .unwrap_or_else(|| io::Error::from_raw_os_error(9)))
            },
            |wait| pauses.push(wait),
            |_, _| panic!("no connection was accepted"),
        )
        .unwrap_err();
        (pauses, calls, ended)
    }

    fn total(pauses: &[Duration]) -> Duration {
        pauses.iter().sum()
    }

    #[test]
    fn a_failure_of_accept_that_persists_cannot_spin() {
        // A thousand failures in a row for want of buffers (`ENOBUFS`): each is followed by a
        // pause, from 5 ms doubling to a second, so they take more than 16 minutes, not none.
        // A want of resources passes when connections end, so it never ends the loop.
        for errno in [PASSING[2], 23, 24, 12] {
            let failures = (0..1000).map(|_| io::Error::from_raw_os_error(errno));
            let (pauses, calls, ended) = looped(failures.collect());
            assert_eq!((pauses.len(), calls), (1000, 1001), "errno {errno}");
            assert_eq!(ended.raw_os_error(), Some(9));
            assert_eq!(pauses[..3], [5, 10, 20].map(Duration::from_millis));
            assert!(pauses.windows(2).all(|w| w[0] <= w[1]));
            assert_eq!(pauses.iter().max(), Some(&ACCEPT_PAUSE_MAX));
            assert!(total(&pauses) > Duration::from_secs(990));
        }
        // A listener that does not block, or whose wait times out, fails every `accept` with
        // no connection waiting: paused from the first, and no reason to end either.
        for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut] {
            let (pauses, _, ended) = looped((0..1000).map(|_| kind.into()).collect());
            assert_eq!((pauses.len(), ended.raw_os_error()), (1000, Some(9)));
            assert!(total(&pauses) > Duration::from_secs(990));
        }
    }

    #[test]
    fn a_listener_that_only_fails_ends_the_loop_with_its_error() {
        // `EPERM` for ever, which no rule of the module knows to pass: tried again through 30
        // seconds of pauses, at most a second apart, then the loop ends with that error.
        let failures = (0..1000).map(|_| io::Error::from_raw_os_error(1));
        let (pauses, calls, ended) = looped(failures.collect());
        assert_eq!(ended.raw_os_error(), Some(1));
        assert!(total(&pauses) >= ACCEPT_FAILING_MAX);
        assert!(total(&pauses) < ACCEPT_FAILING_MAX + ACCEPT_PAUSE_MAX);
        assert_eq!(calls, pauses.len() + 1);
        assert!(calls < 50, "{calls} calls of `accept`");
        // An error with no number is the same.
        let failures = (0..1000).map(|_| io::Error::other("unknown"));
        let (pauses, _, ended) = looped(failures.collect());
        assert_eq!(ended.kind(), io::ErrorKind::Other);
        assert!(total(&pauses) >= ACCEPT_FAILING_MAX);
        // Wants of resources between them do not count, and do not start the count over.
        let failures =
            (0..2000).map(|i| io::Error::from_raw_os_error(if i % 2 == 0 { 1 } else { 24 }));
        let (pauses, calls, ended) = looped(failures.collect());
        assert_eq!(ended.raw_os_error(), Some(1));
        assert!(total(&pauses) >= 2 * ACCEPT_FAILING_MAX - 2 * ACCEPT_PAUSE_MAX);
        assert!(calls < 100, "{calls} calls of `accept`");
    }

    #[test]
    fn failures_of_one_connection_pause_briefly_and_do_not_escalate() {
        // Aborted connections go on at once 16 times in a row, then after 5 ms each, never
        // more: a peer that queues aborted connections does not buy a second apiece.
        let aborted = |n: usize| -> Vec<io::Error> {
            (0..n)
                .map(|_| io::ErrorKind::ConnectionAborted.into())
                .collect()
        };
        let (pauses, calls, ended) = looped(aborted(1000));
        assert_eq!(ended.raw_os_error(), Some(9));
        assert_eq!(
            (pauses.len(), calls),
            (1000 - ACCEPT_FREE_RETRIES as usize, 1001)
        );
        assert!(pauses.iter().all(|wait| *wait == ACCEPT_PAUSE_MIN));
        // They do not raise the pause of the other failures either.
        let mut failures = aborted(100);
        failures.push(io::Error::from_raw_os_error(24));
        let (pauses, _, _) = looped(failures);
        assert_eq!(pauses.last(), Some(&ACCEPT_PAUSE_MIN));
        // They never end the loop, however many a peer causes: far past 30 seconds of those
        // pauses, it ends only with the scripted `EBADF`.
        let (pauses, _, ended) = looped(aborted(100_000));
        assert_eq!(ended.raw_os_error(), Some(9));
        assert!(total(&pauses) > 10 * ACCEPT_FAILING_MAX);
    }

    #[test]
    fn an_accepted_connection_starts_the_pauses_over() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let enobufs = PASSING[2];
        let mut step = 0;
        let mut pauses = Vec::new();
        let mut accepted = 0;
        let ended = accept_loop(
            || {
                step += 1;
                match step {
                    1..=3 | 5 => Err(io::Error::from_raw_os_error(enobufs)),
                    4 => listener.accept(),
                    _ => Err(io::Error::from_raw_os_error(9)),
                }
            },
            |wait| pauses.push(wait),
            |_, _| accepted += 1,
        )
        .unwrap_err();
        assert_eq!(ended.raw_os_error(), Some(9));
        assert_eq!(accepted, 1);
        assert_eq!(pauses, [5, 10, 20, 5].map(Duration::from_millis));
        // The 30 seconds start over too: 23 seconds of pauses on each side of a connection
        // accepted do not end the loop, which the scripted `EBADF` does.
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let eperm = || io::Error::from_raw_os_error(1);
        let mut step = 0;
        let mut pauses = Vec::new();
        let ended = accept_loop(
            || {
                step += 1;
                match step {
                    1..=30 | 32..=61 => Err(eperm()),
                    31 => listener.accept(),
                    _ => Err(io::Error::from_raw_os_error(9)),
                }
            },
            |wait| pauses.push(wait),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!((ended.raw_os_error(), pauses.len()), (Some(9), 60));
    }

    fn v4(last: u8) -> Peer {
        Peer::of(IpAddr::from([192, 0, 2, last]))
    }

    #[test]
    fn one_peer_holds_at_most_its_bound_of_slots() {
        let places = Places::new(4, 2);
        assert_eq!(places.enter(v4(1)), Ok(()));
        assert_eq!(places.enter(v4(1)), Ok(()));
        // The peer's third is refused and leaves no trace; another peer is served.
        assert_eq!(places.enter(v4(1)), Err(Busy::Peer));
        assert_eq!(places.taken(), 2);
        assert_eq!(places.enter(v4(2)), Ok(()));
        assert_eq!(places.enter(v4(3)), Ok(()));
        // The bound of the service refuses a peer that holds nothing, and counts nothing.
        assert_eq!(places.enter(v4(4)), Err(Busy::Service));
        assert_eq!(places.enter(v4(2)), Err(Busy::Service));
        assert_eq!((places.taken(), places.peers().len()), (4, 3));
        // A slot given back is the peer's again.
        places.leave(v4(1));
        assert_eq!(places.enter(v4(1)), Ok(()));
        // The map holds a peer only while it holds a slot.
        for peer in [v4(1), v4(1), v4(2), v4(3)] {
            places.leave(peer);
        }
        assert_eq!((places.taken(), places.peers().len()), (0, 0));
    }

    #[test]
    fn loopback_peers_are_bounded_by_the_service_alone() {
        let of = |text: &str| Peer::of(text.parse().unwrap());
        for local in ["127.0.0.1", "127.8.9.10", "::1", "::ffff:127.0.0.1"] {
            assert_eq!(of(local), Peer::Loopback, "{local}");
        }
        for remote in [
            "192.0.2.1",
            "126.255.255.255",
            "128.0.0.1",
            "::2",
            "2001:db8::1",
        ] {
            assert_ne!(of(remote), Peer::Loopback, "{remote}");
        }
        // Past the bound of a peer, up to the bound of the service, with no entry kept.
        let places = Places::new(4, 1);
        for _ in 0..4 {
            assert_eq!(places.enter(Peer::Loopback), Ok(()));
        }
        assert_eq!(places.enter(Peer::Loopback), Err(Busy::Service));
        assert_eq!((places.taken(), places.peers().len()), (4, 0));
        // Another address is still bounded while they hold slots, and they give theirs back.
        places.leave(Peer::Loopback);
        assert_eq!(places.enter(v4(1)), Ok(()));
        places.leave(Peer::Loopback);
        assert_eq!(places.enter(v4(1)), Err(Busy::Peer));
        assert_eq!(places.enter(Peer::Loopback), Ok(()));
        for peer in [Peer::Loopback, Peer::Loopback, Peer::Loopback, v4(1)] {
            places.leave(peer);
        }
        assert_eq!((places.taken(), places.peers().len()), (0, 0));
    }

    #[test]
    fn a_peer_is_an_ipv4_address_or_an_ipv6_prefix() {
        let of = |text: &str| Peer::of(text.parse().unwrap());
        assert_ne!(of("192.0.2.1"), of("192.0.2.2"));
        // An IPv4-mapped address is its IPv4 address.
        assert_eq!(of("::ffff:192.0.2.1"), of("192.0.2.1"));
        // The addresses of one /64 are one peer; the next /64 is another.
        assert_eq!(
            of("2001:db8:0:1::1"),
            of("2001:db8:0:1:ffff:ffff:ffff:ffff")
        );
        assert_ne!(of("2001:db8:0:1::1"), of("2001:db8:0:2::1"));
    }

    #[test]
    fn a_configuration_without_a_timeout_or_a_connection_is_refused() {
        assert_eq!(ServerConfig::default().check(), Ok(()));
        let zero = Duration::ZERO;
        let default = ServerConfig::default;
        for (name, config) in [
            (
                "first_byte_timeout",
                ServerConfig {
                    first_byte_timeout: zero,
                    ..default()
                },
            ),
            (
                "head_timeout",
                ServerConfig {
                    head_timeout: zero,
                    ..default()
                },
            ),
            (
                "idle_timeout",
                ServerConfig {
                    idle_timeout: zero,
                    ..default()
                },
            ),
            (
                "write_timeout",
                ServerConfig {
                    write_timeout: zero,
                    ..default()
                },
            ),
            (
                "max_connections",
                ServerConfig {
                    max_connections: 0,
                    ..default()
                },
            ),
            (
                "max_connections_per_peer",
                ServerConfig {
                    max_connections_per_peer: 0,
                    ..default()
                },
            ),
        ] {
            let refused = config.check().unwrap_err();
            assert!(
                refused.starts_with(&format!("`{name}` is zero")),
                "{refused}"
            );
        }
        // The smallest timeout that is not zero is a timeout.
        let least = ServerConfig {
            write_timeout: Duration::from_nanos(1),
            ..default()
        };
        assert_eq!(least.check(), Ok(()));
    }

    #[test]
    fn a_socket_refuses_a_zero_timeout_and_the_refusal_is_an_error() {
        // What the standard library does with a zero timeout: an error, and no timeout set.
        // The module no longer ignores it.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        assert!(stream.wait_at_most(Duration::ZERO).is_err());
        assert!(stream.wait_to_send_at_most(Duration::ZERO).is_err());
        assert!(stream.wait_at_most(Duration::from_nanos(1)).is_ok());
        assert!(stream.wait_to_send_at_most(Duration::from_nanos(1)).is_ok());
    }

    #[test]
    fn a_connection_that_takes_no_timeout_is_neither_read_nor_written() {
        let config = ServerConfig::default();
        let refusing = || {
            let mut conn = paced(
                b"GET /checkpoint HTTP/1.1\r\n\r\n".to_vec(),
                1024,
                Duration::ZERO,
            );
            conn.refuses_timeouts = true;
            conn
        };
        // The head, a body and the discarding of a refused body: no read without a timeout.
        let mut conn = refusing();
        let refused = read_head(&mut conn, &config).err().unwrap();
        assert_eq!((refused.status, conn.reads), (503, 0));
        let mut conn = refusing();
        let refused = read_body(&mut conn, Vec::new(), 10, &config).err().unwrap();
        assert_eq!((refused.status, conn.reads), (503, 0));
        let mut conn = refusing();
        drain(&mut conn, DRAIN_MAX);
        assert_eq!(conn.reads, 0);
        // A response, whole or a file: no write without a timeout, and the file is not sent.
        let mut peer = taker(usize::MAX, usize::MAX, Duration::from_millis(1));
        peer.refuses_timeouts = true;
        let failed = send(&mut peer, Answer::Whole(text(200, "x")), &config).unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::InvalidInput);
        let (answer, _) = lazy_file(1 << 20, 1 << 20);
        assert!(send(&mut peer, answer, &config).is_err());
        assert_eq!((peer.writes, peer.taken), (0, 0));
    }

    /// How long `conn` was read, on its own clock.
    fn held(conn: &Paced) -> Duration {
        conn.now().saturating_duration_since(conn.start)
    }

    #[test]
    fn a_request_starts_within_the_first_byte_timeout() {
        let config = ServerConfig::default();
        assert!(config.first_byte_timeout < config.head_timeout);
        // Nothing arrives: one wait, of the first-byte timeout, then the refusal.
        let mut conn = paced(Vec::new(), 1, Duration::from_secs(1));
        let refused = read_head(&mut conn, &config).err().unwrap();
        assert_eq!(refused.status, 408);
        assert!(String::from_utf8_lossy(&refused.body).contains("did not start"));
        assert_eq!(conn.waits, [config.first_byte_timeout]);
        // A first-byte timeout past the head's is the head's.
        let long = ServerConfig {
            first_byte_timeout: Duration::from_secs(3600),
            ..ServerConfig::default()
        };
        let mut conn = paced(Vec::new(), 1, Duration::from_secs(1));
        assert_eq!(read_head(&mut conn, &long).err().unwrap().status, 408);
        assert_eq!(conn.waits, [long.head_timeout]);
    }

    #[test]
    fn a_head_sent_slowly_loses_its_slot_soon_after_the_first_byte_timeout() {
        let config = ServerConfig::default();
        let first = config.first_byte_timeout;
        let refused_late = |conn: &mut Paced| {
            let refused = read_head(conn, &config).err().unwrap();
            assert_eq!(refused.status, 408);
            assert!(String::from_utf8_lossy(&refused.body).contains("head took too long"));
        };
        // The review's case: one byte at once, then nothing. The second wait ends a
        // millisecond past the first-byte timeout, not at the head's 30 seconds.
        let mut conn = paced(b"G".to_vec(), 1, Duration::from_millis(1));
        refused_late(&mut conn);
        assert_eq!(conn.waits.len(), 2);
        assert!(conn.waits[1] <= first, "{:?}", conn.waits);
        // A byte a second: refused at the first-byte timeout, a few milliseconds late.
        let mut conn = paced(vec![b'a'; 4096], 1, Duration::from_secs(1));
        refused_late(&mut conn);
        assert!(
            held(&conn) <= first + Duration::from_secs(1),
            "{:?}",
            held(&conn)
        );
        assert!(conn.at <= 6, "{} bytes read", conn.at);
        // Half the least rate, 512 bytes a second: refused within twice the timeout or so,
        // long before the 32 seconds the largest head would take at that pace.
        let mut conn = paced(head_of(MAX_HEAD), 512, Duration::from_secs(1));
        refused_late(&mut conn);
        assert!(
            held(&conn) <= 2 * first + Duration::from_secs(1),
            "{:?}",
            held(&conn)
        );
        // No wait is ever past the head's time or the time the bytes received earned.
        assert!(conn.waits.iter().all(|w| *w <= config.head_timeout));
    }

    #[test]
    fn a_head_at_the_least_rate_is_read_within_the_head_timeout() {
        let config = ServerConfig::default();
        // The largest head at 1 KiB a second, the least rate: read whole in 16 seconds.
        let mut conn = paced(head_of(MAX_HEAD), 1 << 10, Duration::from_secs(1));
        assert_eq!(read_head(&mut conn, &config).unwrap().path, "checkpoint");
        assert_eq!(held(&conn), Duration::from_secs(16));
        // A small head sent in a few parts within the first-byte timeout is read.
        let head = b"GET /checkpoint HTTP/1.1\r\n\r\n";
        let mut conn = paced(head.to_vec(), 7, Duration::from_secs(1));
        assert_eq!(read_head(&mut conn, &config).unwrap().path, "checkpoint");
        // The head's time bounds a head however many bytes it earned: with a head timeout of
        // 8 seconds, the largest head at the least rate is refused at 8 seconds.
        let short = ServerConfig {
            head_timeout: Duration::from_secs(8),
            ..ServerConfig::default()
        };
        let mut conn = paced(head_of(MAX_HEAD), 1 << 10, Duration::from_secs(1));
        let refused = read_head(&mut conn, &short).err().unwrap();
        assert_eq!((refused.status, held(&conn)), (408, Duration::from_secs(8)));
    }

    #[test]
    fn the_descriptors_needed_are_two_a_connection_and_the_reserve() {
        assert_eq!(descriptors_needed(&ServerConfig::default()), 2 * 256 + 16);
        let small = ServerConfig {
            max_connections: 100,
            ..ServerConfig::default()
        };
        assert_eq!(descriptors_needed(&small), 216);
        // The largest bound does not overflow.
        let huge = ServerConfig {
            max_connections: usize::MAX,
            ..ServerConfig::default()
        };
        assert_eq!(descriptors_needed(&huge), u64::MAX);
    }

    #[test]
    fn secrets_compare_whole() {
        assert!(same_secret(b"token", b"token"));
        assert!(!same_secret(b"token", b"tokem"));
        assert!(!same_secret(b"token", b"toke"));
    }
}
