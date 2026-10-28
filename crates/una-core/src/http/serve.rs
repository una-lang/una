//! `serve:` grants (`spec/31 §2`, ADR 0123 rules 6 to 9): where a run may accept connections,
//! with the limits every server has, the names `Host` may carry, and the TLS files and protocol
//! of an `https` grant. Pure, so the manifest, the checker and both engines read grants alike;
//! the host code that opens files and binds sockets is `una-http`'s.
//!
//! Parsing is O(n) in the spec's length, and the overlap check O(g log g) for g grants.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The prefix of every `serve:` grant spec.
pub const PREFIX: &str = "serve:";

/// The largest count of the items whose range ends at 2^31 − 1.
pub const MAX_COUNT: u64 = (1 << 31) - 1;

/// The largest `connections` and `peer`.
pub const MAX_CONNECTIONS: u64 = 1 << 20;

/// The largest byte budget in MiB: `buffers`, `peer_buffers`, `replies`, `peer_replies` and
/// `delivered`.
pub const MAX_BUDGET_MIB: u64 = 1 << 16;

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;

/// A period of a rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    /// `N/second`.
    Second,
    /// `N/minute`.
    Minute,
    /// `N/hour`.
    Hour,
}

impl Period {
    /// The period's length in seconds.
    pub fn seconds(self) -> u64 {
        match self {
            Period::Second => 1,
            Period::Minute => 60,
            Period::Hour => 3600,
        }
    }

    /// The period's name, as an item writes it after `/`.
    pub fn name(self) -> &'static str {
        match self {
            Period::Second => "second",
            Period::Minute => "minute",
            Period::Hour => "hour",
        }
    }

    fn of(name: &str) -> Option<Period> {
        match name {
            "second" => Some(Period::Second),
            "minute" => Some(Period::Minute),
            "hour" => Some(Period::Hour),
            _ => None,
        }
    }
}

/// A rate of a grant: `count` exchanges delivered per `period`, a token bucket (ADR 0117
/// rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    /// The exchanges per period, 1 to 2^31 − 1.
    pub count: u64,
    /// The period.
    pub period: Period,
}

impl Rate {
    /// The interval between two tokens, in nanoseconds, rounded up and never zero (ADR 0123
    /// rule 7, "Arithmetic").
    pub fn interval_nanos(self) -> u64 {
        let span = self.period.seconds().saturating_mul(1_000_000_000);
        span.div_ceil(self.count.max(1)).max(1)
    }
}

/// The limits of a grant, every default and derived default applied (ADR 0123 rule 7). Sizes
/// are in bytes and times in seconds; `None` is the item's `none`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// `N/once`: exchanges delivered and not yet answered.
    pub once: u64,
    /// `N/second`, `N/minute` or `N/hour`, when the grant gives one.
    pub rate: Option<Rate>,
    /// `waiting`: requests read and waiting to be delivered.
    pub waiting: u64,
    /// `connections`: open connections.
    pub connections: u64,
    /// `peer`: open connections from one peer.
    pub peer: Option<u64>,
    /// `handshakes`: TLS handshakes started per second.
    pub handshakes: u64,
    /// `requests`: requests on one connection.
    pub requests: u64,
    /// `line`: bytes of the request line.
    pub line: u64,
    /// `headers`: header lines of a request, trailers included.
    pub headers: u64,
    /// `header`: bytes of the header section, trailers included.
    pub header: u64,
    /// `body`: bytes of a request's body.
    pub body: u64,
    /// `buffers`: bytes of request bodies held for the grant.
    pub buffers: u64,
    /// `peer_buffers`: the share of `buffers` of one peer.
    pub peer_buffers: Option<u64>,
    /// `replies`: bytes of replies not yet written.
    pub replies: u64,
    /// `peer_replies`: the share of `replies` of one peer under pressure.
    pub peer_replies: Option<u64>,
    /// `delivered`: bytes of the requests of open exchanges.
    pub delivered: u64,
    /// `idle`: seconds a connection waits for a request's first byte.
    pub idle: u64,
    /// `read`: seconds to read a whole request.
    pub read: u64,
    /// `write`: seconds to write a whole reply.
    pub write: u64,
    /// `answer`: seconds from the end of a request to the start of its reply.
    pub answer: u64,
}

impl Limits {
    /// The handshakes one peer may start per second: `max(4, handshakes / 16)`.
    pub fn peer_handshakes(&self) -> u64 {
        (self.handshakes / 16).max(4)
    }
}

/// The names that `Host` may carry on a grant (ADR 0123 rule 8, `spec/31 §2` rule 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hosts {
    /// No `hosts` item: `localhost` and IP literals only.
    Local,
    /// `hosts=*`: every name of valid syntax.
    Any,
    /// `hosts=a,b`: these names, in ASCII lower case without a trailing `.`, each either a
    /// name or `*.` and a name, in the order written.
    Names(Vec<String>),
}

impl Hosts {
    /// Whether a request whose `Host` names `name` passes, else `421`. `name` is the name as
    /// `una_http::request::host` reads it: in ASCII lower case, without its port and without
    /// one trailing `.`; `ip` says whether it is an IP literal. O(n) in the names listed.
    pub fn allows(&self, name: &str, ip: bool) -> bool {
        match self {
            Hosts::Local => ip || name == "localhost",
            Hosts::Any => true,
            Hosts::Names(names) => {
                !ip && names.iter().any(|listed| match listed.strip_prefix("*.") {
                    Some(suffix) => name
                        .split_once('.')
                        .is_some_and(|(first, rest)| !first.is_empty() && rest == suffix),
                    None => listed == name,
                })
            }
        }
    }
}

/// The TLS files of an `https` grant, as written; relative paths resolve as other grants'
/// paths do (`spec/13 §5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsFiles {
    /// `certificate`: the PEM chain, leaf first.
    pub certificate: String,
    /// `key`: the PEM private key.
    pub key: String,
}

/// The pinned configuration of the TLS server of an `https` grant (ADR 0123 rule 9). It
/// describes what the server is built with; building it waits for the decision on the TLS
/// provider (#1059), and no provider is bound here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TlsPolicy {
    /// Whether TLS 1.2 is accepted beside TLS 1.3: only with the grant's `tls12`.
    pub tls12: bool,
    /// Whether TLS 1.2 requires the extended master secret: always, when TLS 1.2 is on.
    pub require_ems: bool,
    /// The early data the server accepts, in bytes: none.
    pub max_early_data: u32,
    /// Whether stateless session tickets are issued: never.
    pub stateless_tickets: bool,
    /// The entries of the in-memory session cache, the only resumption.
    pub session_cache: usize,
    /// Whether a key log is written: never, so `SSLKEYLOGFILE` is not read.
    pub key_log: bool,
    /// The protocols ALPN offers: `http/1.1` only.
    pub alpn: &'static [&'static str],
    /// Whether client certificates are asked for: never.
    pub client_certificates: bool,
}

impl TlsPolicy {
    /// The pinned policy, with TLS 1.2 only when the grant says `tls12`.
    pub fn pinned(tls12: bool) -> TlsPolicy {
        TlsPolicy {
            tls12,
            require_ems: true,
            max_early_data: 0,
            stateless_tickets: false,
            session_cache: 256,
            key_log: false,
            alpn: &["http/1.1"],
            client_certificates: false,
        }
    }
}

/// A grant `serve:<scheme>://<address>:<port>` and its items (`spec/31 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeGrant {
    /// Whether the scheme is `https`.
    pub tls: bool,
    /// The address to listen on; `0.0.0.0` and `::` are every interface of their family.
    pub address: IpAddr,
    /// The port, 1 to 65535.
    pub port: u16,
    /// The limits, with their defaults.
    pub limits: Limits,
    /// The names `Host` may carry.
    pub hosts: Hosts,
    /// The TLS files, on an `https` grant.
    pub files: Option<TlsFiles>,
    /// Whether the grant says `tls12`, on an `https` grant only.
    pub tls12: bool,
}

impl ServeGrant {
    /// The grant's address and port as the grant writes them, `127.0.0.1:8080` or
    /// `[::1]:8080`: the `listener` of a request.
    pub fn listener(&self) -> String {
        match self.address {
            IpAddr::V4(v4) => format!("{v4}:{}", self.port),
            IpAddr::V6(v6) => format!("[{v6}]:{}", self.port),
        }
    }

    /// Whether the address is a wildcard, `0.0.0.0` or `::`.
    pub fn wildcard(&self) -> bool {
        self.address.is_unspecified()
    }

    /// The pinned TLS policy of an `https` grant.
    pub fn tls_policy(&self) -> Option<TlsPolicy> {
        self.files.as_ref().map(|_| TlsPolicy::pinned(self.tls12))
    }

    /// Reads a `serve:` spec. Every problem is reported, each with the index of its item
    /// among the `;` suffixes, or `None` for the origin and for what concerns the whole grant.
    pub fn parse(spec: &str) -> Result<ServeGrant, Vec<ServeError>> {
        let Some(rest) = spec.strip_prefix(PREFIX) else {
            return Err(vec![ServeError::grant(
                "a serve grant starts with `serve:`",
            )]);
        };
        let mut errors = Vec::new();
        let mut parts = rest.split(';');
        let origin = parts.next().unwrap_or_default();
        let origin = match parse_origin(origin) {
            Ok(origin) => Some(origin),
            Err(message) => {
                errors.push(ServeError::grant(message));
                None
            }
        };
        let mut seen: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut items: Vec<(usize, Item)> = Vec::new();
        for (index, text) in parts.enumerate() {
            match parse_item(text) {
                Ok(item) => {
                    let kind = item.kind();
                    if seen.insert(kind, index).is_some() {
                        let message = if kind == "rate" {
                            format!("`{text}` is a second rate; a serve grant has one rate")
                        } else {
                            format!("`{kind}` is given twice")
                        };
                        errors.push(ServeError::item(index, message));
                    } else {
                        items.push((index, item));
                    }
                }
                Err(message) => errors.push(ServeError::item(index, message)),
            }
        }
        let tls = origin.as_ref().is_some_and(|origin| origin.0);
        let mut builder = Builder::default();
        for (index, item) in items {
            if !tls && origin.is_some() && item.tls_only() {
                errors.push(ServeError::item(
                    index,
                    format!(
                        "`{}` is for https grants only; this grant is http",
                        item.kind()
                    ),
                ));
                continue;
            }
            builder.set(item);
        }
        if tls {
            for (name, present) in [
                ("certificate", builder.certificate.is_some()),
                ("key", builder.key.is_some()),
            ] {
                if !present {
                    errors.push(ServeError::grant(format!(
                        "an https grant names its `{name}` file"
                    )));
                }
            }
        }
        match origin {
            Some((tls, address, port)) if errors.is_empty() => {
                Ok(builder.finish(tls, address, port))
            }
            _ => Err(errors),
        }
    }
}

/// A problem of a `serve:` spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeError {
    /// The index of the item among the `;` suffixes, or `None` for the origin or the grant.
    pub item: Option<usize>,
    /// One sentence, lowercase start, no trailing period.
    pub message: String,
}

impl ServeError {
    fn grant(message: impl Into<String>) -> ServeError {
        ServeError {
            item: None,
            message: message.into(),
        }
    }

    fn item(index: usize, message: impl Into<String>) -> ServeError {
        ServeError {
            item: Some(index),
            message: message.into(),
        }
    }
}

/// The indexes of the grants, given as their addresses and ports in order, that overlap an
/// earlier one (`spec/31 §2` rule 6): the same address and port, or a wildcard sharing its
/// port with a grant of its family. O(g log g).
pub fn overlapping<I>(listeners: I) -> Vec<usize>
where
    I: IntoIterator<Item = (IpAddr, u16)>,
{
    #[derive(Default)]
    struct Slot {
        wildcard: bool,
        addresses: BTreeSet<IpAddr>,
    }
    let mut slots: BTreeMap<(bool, u16), Slot> = BTreeMap::new();
    let mut out = Vec::new();
    for (index, (address, port)) in listeners.into_iter().enumerate() {
        let slot = slots.entry((address.is_ipv6(), port)).or_default();
        let wildcard = address.is_unspecified();
        let overlaps = if wildcard {
            slot.wildcard || !slot.addresses.is_empty()
        } else {
            slot.wildcard || slot.addresses.contains(&address)
        };
        if overlaps {
            out.push(index);
        }
        if wildcard {
            slot.wildcard = true;
        } else {
            slot.addresses.insert(address);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The origin
// ---------------------------------------------------------------------------------------------

/// `<scheme>://<address>:<port>`: whether the scheme is `https`, the address and the port.
fn parse_origin(origin: &str) -> Result<(bool, IpAddr, u16), String> {
    let (tls, rest) = if let Some(rest) = origin.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = origin.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(format!(
            "`{origin}` does not start with `http://` or `https://`, in lower case"
        ));
    };
    let (address, after) = if let Some(inner) = rest.strip_prefix('[') {
        let Some((inside, after)) = inner.split_once(']') else {
            return Err(format!("`{origin}` opens `[` and does not close it"));
        };
        (IpAddr::V6(ipv6(inside)?), after)
    } else {
        let (text, after) = match rest.find(':') {
            Some(colon) => rest.split_at(colon),
            None => (rest, ""),
        };
        (IpAddr::V4(ipv4(text)?), after)
    };
    let Some(digits) = after.strip_prefix(':') else {
        return Err(format!(
            "`{origin}` names no port; a serve grant always writes its port"
        ));
    };
    let port = number(digits)
        .filter(|port| (1..=u64::from(u16::MAX)).contains(port))
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| {
            format!("the port of `{origin}` is a number from 1 to 65535 with no leading zero, and nothing follows it")
        })?;
    Ok((tls, address, port))
}

fn ipv4(text: &str) -> Result<Ipv4Addr, String> {
    let Ok(address) = text.parse::<Ipv4Addr>() else {
        return Err(format!(
            "`{text}` is not an IPv4 address; a serve grant names an IP address, never a host name, and IPv6 goes in brackets"
        ));
    };
    if address.to_string() != text {
        return Err(format!("write the address `{text}` as `{address}`"));
    }
    if address.is_multicast() || address.is_broadcast() {
        return Err(format!(
            "`{text}` is a multicast or broadcast address, where nothing can listen"
        ));
    }
    Ok(address)
}

fn ipv6(text: &str) -> Result<Ipv6Addr, String> {
    let Ok(address) = text.parse::<Ipv6Addr>() else {
        return Err(format!(
            "`[{text}]` is not an IPv6 address; zones are not written in a grant"
        ));
    };
    if let Some(v4) = address.to_ipv4_mapped() {
        return Err(format!(
            "`[{text}]` is an IPv4-mapped address; write `{v4}`, since an IPv6 listener receives no IPv4 connection"
        ));
    }
    if address.to_string() != text {
        return Err(format!("write the address `[{text}]` as `[{address}]`"));
    }
    if address.is_multicast() {
        return Err(format!(
            "`[{text}]` is a multicast address, where nothing can listen"
        ));
    }
    Ok(address)
}

/// A decimal number with no sign and no leading zero, `0` alone included.
fn number(text: &str) -> Option<u64> {
    if text.is_empty()
        || text.len() > 20
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok()
}

// ---------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------

/// One item, read but not yet combined with the others.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Once(u64),
    Rate(Rate),
    Count(&'static str, u64),
    Share(&'static str, Option<u64>),
    Hosts(Hosts),
    Certificate(String),
    Key(String),
    Tls12,
}

impl Item {
    /// The kind of the item, which a grant gives at most once.
    fn kind(&self) -> &'static str {
        match self {
            Item::Once(_) => "once",
            Item::Rate(_) => "rate",
            Item::Count(name, _) | Item::Share(name, _) => name,
            Item::Hosts(_) => "hosts",
            Item::Certificate(_) => "certificate",
            Item::Key(_) => "key",
            Item::Tls12 => "tls12",
        }
    }

    fn tls_only(&self) -> bool {
        matches!(self, Item::Certificate(_) | Item::Key(_) | Item::Tls12)
    }
}

/// The unit an item's number is written in.
#[derive(Clone, Copy)]
enum Unit {
    Count,
    KiB,
    MiB,
    Seconds,
}

impl Unit {
    fn describe(self) -> &'static str {
        match self {
            Unit::Count => "a number",
            Unit::KiB => "a number of KiB",
            Unit::MiB => "a number of MiB",
            Unit::Seconds => "a number of seconds",
        }
    }
}

/// The items written `name=value`: their range, unit, and whether `none` is allowed.
const NAMED: [(&str, u64, u64, Unit, bool); 18] = [
    ("waiting", 0, MAX_COUNT, Unit::Count, false),
    ("connections", 1, MAX_CONNECTIONS, Unit::Count, false),
    ("peer", 1, MAX_CONNECTIONS, Unit::Count, true),
    ("handshakes", 1, MAX_COUNT, Unit::Count, false),
    ("requests", 1, MAX_COUNT, Unit::Count, false),
    ("line", 1, 64, Unit::KiB, false),
    ("headers", 1, 1000, Unit::Count, false),
    ("header", 1, 256, Unit::KiB, false),
    ("body", 0, 1 << 20, Unit::KiB, false),
    ("buffers", 1, MAX_BUDGET_MIB, Unit::MiB, false),
    ("peer_buffers", 1, MAX_BUDGET_MIB, Unit::MiB, true),
    ("replies", 1, MAX_BUDGET_MIB, Unit::MiB, false),
    ("peer_replies", 1, MAX_BUDGET_MIB, Unit::MiB, true),
    ("delivered", 1, MAX_BUDGET_MIB, Unit::MiB, false),
    ("idle", 1, 3600, Unit::Seconds, false),
    ("read", 1, 3600, Unit::Seconds, false),
    ("write", 1, 3600, Unit::Seconds, false),
    ("answer", 1, 86_400, Unit::Seconds, false),
];

fn parse_item(text: &str) -> Result<Item, String> {
    if text == "tls12" {
        return Ok(Item::Tls12);
    }
    if let Some((name, value)) = text.split_once('=') {
        match name {
            "hosts" => return parse_hosts(value).map(Item::Hosts),
            "certificate" | "key" => {
                if value.is_empty() || value.contains('\0') {
                    return Err(format!("`{name}` names a file path, not `{value}`"));
                }
                return Ok(if name == "key" {
                    Item::Key(value.to_owned())
                } else {
                    Item::Certificate(value.to_owned())
                });
            }
            _ => {}
        }
        let Some(&(name, low, high, unit, none)) = NAMED.iter().find(|(known, ..)| *known == name)
        else {
            return Err(unknown(text));
        };
        if none && value == "none" {
            return Ok(Item::Share(name, None));
        }
        let Some(count) = number(value).filter(|count| (low..=high).contains(count)) else {
            let or_none = if none { ", or `none`" } else { "" };
            return Err(format!(
                "`{name}` is {} from {low} to {high}{or_none}, not `{value}`",
                unit.describe()
            ));
        };
        return Ok(if none {
            Item::Share(name, Some(count))
        } else {
            Item::Count(name, count)
        });
    }
    if let Some((count, per)) = text.split_once('/') {
        let in_range = |what: &str| {
            number(count)
                .filter(|count| (1..=MAX_COUNT).contains(count))
                .ok_or_else(|| format!("`{what}` counts from 1 to {MAX_COUNT}, not `{count}`"))
        };
        if per == "once" {
            return in_range("N/once").map(Item::Once);
        }
        if let Some(period) = Period::of(per) {
            let what = format!("N/{per}");
            return in_range(&what).map(|count| Item::Rate(Rate { count, period }));
        }
    }
    Err(unknown(text))
}

fn unknown(text: &str) -> String {
    format!(
        "`{text}` is not an item of a serve grant; see spec/31 §2 for the items and their forms"
    )
}

/// `hosts=a,b` or `hosts=*` (`spec/31 §2` rule 5).
fn parse_hosts(value: &str) -> Result<Hosts, String> {
    if value == "*" {
        return Ok(Hosts::Any);
    }
    let mut names: Vec<String> = Vec::new();
    for written in value.split(',') {
        if written == "*" {
            return Err("`*` accepts every name and is the only name of its `hosts`".to_owned());
        }
        let (wildcard, name) = match written.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, written),
        };
        let Some(name) = registered_name(name) else {
            return Err(format!(
                "`{written}` is not a host name: letters, digits, `-`, `_` and `.`, no empty label, a last label that is not a number, and an optional leftmost `*.`"
            ));
        };
        let name = if wildcard { format!("*.{name}") } else { name };
        if names.contains(&name) {
            return Err(format!("`{written}` is given twice in `hosts`"));
        }
        names.push(name);
    }
    Ok(Hosts::Names(names))
}

/// A registered name as `Host` writes one, in ASCII lower case without one trailing `.`, or
/// `None` when it is not one (`spec/31 §3` rule 5).
fn registered_name(text: &str) -> Option<String> {
    let name = text.strip_suffix('.').unwrap_or(text);
    if name.is_empty()
        || name.len() > 253
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        || name.split('.').any(str::is_empty)
    {
        return None;
    }
    let last = name.rsplit('.').next().unwrap_or_default();
    let numeric = last.bytes().all(|b| b.is_ascii_digit());
    let hex = last.len() > 2 && (last.starts_with("0x") || last.starts_with("0X"));
    (!numeric && !hex).then(|| name.to_ascii_lowercase())
}

/// The items of a grant, combined into its limits once all are read.
#[derive(Default)]
struct Builder {
    once: Option<u64>,
    rate: Option<Rate>,
    counts: BTreeMap<&'static str, u64>,
    shares: BTreeMap<&'static str, Option<u64>>,
    hosts: Option<Hosts>,
    certificate: Option<String>,
    key: Option<String>,
    tls12: bool,
}

impl Builder {
    fn set(&mut self, item: Item) {
        match item {
            Item::Once(count) => self.once = Some(count),
            Item::Rate(rate) => self.rate = Some(rate),
            Item::Count(name, count) => {
                self.counts.insert(name, count);
            }
            Item::Share(name, share) => {
                self.shares.insert(name, share);
            }
            Item::Hosts(hosts) => self.hosts = Some(hosts),
            Item::Certificate(path) => self.certificate = Some(path),
            Item::Key(path) => self.key = Some(path),
            Item::Tls12 => self.tls12 = true,
        }
    }

    fn finish(self, tls: bool, address: IpAddr, port: u16) -> ServeGrant {
        let count = |name: &str, default: u64| self.counts.get(name).copied().unwrap_or(default);
        let connections = count("connections", 1024);
        let buffers = count("buffers", 64);
        let replies = count("replies", 64);
        // A share given as a number or `none`, else its default, derived from the values the
        // grant gives (ADR 0123 rule 7).
        let share = |name: &str, default: u64| match self.shares.get(name) {
            Some(given) => *given,
            None => Some(default),
        };
        let limits = Limits {
            once: self.once.unwrap_or(256),
            rate: self.rate,
            waiting: count("waiting", 1024),
            connections,
            peer: share("peer", (connections / 16).max(16)),
            handshakes: count("handshakes", 512),
            requests: count("requests", 1000),
            line: count("line", 8).saturating_mul(KIB),
            headers: count("headers", 100),
            header: count("header", 16).saturating_mul(KIB),
            body: count("body", 1024).saturating_mul(KIB),
            buffers: buffers.saturating_mul(MIB),
            peer_buffers: share("peer_buffers", (buffers / 64).max(1))
                .map(|mib| mib.saturating_mul(MIB)),
            replies: replies.saturating_mul(MIB),
            peer_replies: share("peer_replies", (replies / 64).max(1))
                .map(|mib| mib.saturating_mul(MIB)),
            delivered: count("delivered", 256).saturating_mul(MIB),
            idle: count("idle", 5),
            read: count("read", 20),
            write: count("write", 30),
            answer: count("answer", 30),
        };
        let files = match (tls, self.certificate, self.key) {
            (true, Some(certificate), Some(key)) => Some(TlsFiles { certificate, key }),
            _ => None,
        };
        ServeGrant {
            tls,
            address,
            port,
            limits,
            hosts: self.hosts.unwrap_or(Hosts::Local),
            tls12: tls && self.tls12,
            files,
        }
    }
}

#[cfg(test)]
mod tests;
