//! The script of connections of `--serve-script <file>` (ADR 0123 rule 21, `spec/31 §8`): one
//! JSON object per line, read and checked in full before anything runs, so that a script the
//! run cannot follow is a usage error and never a fault in the middle of a run.
//!
//! The four forms, each with exactly these keys, `at` in virtual milliseconds:
//!
//! - `{"at", "connection", "grant", "peer", "reads"}` opens connection `connection` on the
//!   grant whose address is `grant`, from the IP address `peer`, with `reads` `"eager"` or
//!   `"paced"`;
//! - `{"at", "connection", "bytes"}` sends the bytes of the base64 text `bytes`;
//! - `{"at", "connection", "read"}` lets a `paced` connection receive `read` more bytes, at
//!   least 1;
//! - `{"at", "connection", "close": true}` closes the peer's side; nothing follows it on that
//!   connection.
//!
//! Lines end with LF, the last one optionally, and each is one JSON object, each key once, with
//! nothing before its `{` or after its `}`: an empty line, a line of spaces, a CR and a key
//! given twice are refused. A script with no lines opens no connection.
//!
//! The times never decrease from a line to the next, and lines of equal `at` take effect in
//! their order. A connection is opened once, before its other lines, and one peer address
//! opens at most 64,512 connections. Reading is O(n) in the
//! bytes of the file, with one hash lookup per line.

use std::collections::HashMap;
use std::net::IpAddr;

use serde::de::{DeserializeSeed, Deserializer, Error as _, MapAccess, Visitor};
use serde_json::{Map, Value};

/// How a scripted peer reads the bytes the host sends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerReads {
    /// It takes every byte as soon as the host sends it.
    Eager,
    /// It takes bytes only as its `read` lines allow.
    Paced,
}

/// What one line of a script does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Opens the connection on the grant at `grant`, the index of its address in the run's
    /// grants, from `peer`.
    Open {
        grant: usize,
        peer: IpAddr,
        reads: PeerReads,
    },
    /// Sends these bytes.
    Bytes(Vec<u8>),
    /// Lets a paced connection receive this many more bytes.
    Read(u64),
    /// Closes the peer's side.
    Close,
}

/// One line of a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// The virtual time, in milliseconds.
    pub at: u64,
    /// The connection, as the script numbers it.
    pub connection: u64,
    /// What happens.
    pub action: Action,
}

/// Where a connection of the script stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Open(PeerReads),
    Closed,
}

/// `--serve-script` without `--virtual-time` (ADR 0123 rule 21).
pub const NEEDS_VIRTUAL_TIME: &str =
    "`--serve-script` needs `--virtual-time`: its times are virtual milliseconds";

/// `--serve-out` without `--serve-script`.
pub const NEEDS_SCRIPT: &str =
    "`--serve-out` needs `--serve-script`: it writes what the scripted connections received";

/// `--serve-script` in a run with no `serve:` grant.
pub const NEEDS_GRANT: &str =
    "`--serve-script` replaces the listeners of the run's `serve:` grants, and the run has none";

/// A run whose script was read and checked: its connections do not run in this release yet
/// (`spec/31 §8`), as a program that serves does not (`spec/31 §1` rule 10).
pub const PENDING: &str =
    "the scripted connections of `--serve-script` do not run in this release yet";

/// The usage error of `--serve-script` and `--serve-out` as given, before anything else is
/// read: `--serve-script` needs `--virtual-time`, and `--serve-out` needs `--serve-script`.
pub fn check_flags(script: bool, out: bool, virtual_time: bool) -> Result<(), &'static str> {
    if script && !virtual_time {
        return Err(NEEDS_VIRTUAL_TIME);
    }
    if out && !script {
        return Err(NEEDS_SCRIPT);
    }
    Ok(())
}

/// The connections a script may open from one peer address: one per port from 1024 to 65535,
/// which the engine gives them in order.
pub const PEER_CONNECTIONS: u32 = 64_512;

/// The largest script `--serve-script` reads, in bytes: 64 MiB.
pub const MAX_SCRIPT: u64 = 64 * 1024 * 1024;

/// The script of the file at `path`, read and checked for a run whose `serve:` grants have the
/// addresses `listeners`, or the message of the usage error: [`NEEDS_GRANT`] when there are
/// none, a file that is not a regular file (a FIFO or a device would block or never end), one
/// larger than [`MAX_SCRIPT`], one that cannot be read or is not UTF-8, or the first line that
/// cannot be followed. The file is opened without blocking (a FIFO's open would wait for a
/// writer) and checked on its open descriptor, so what is read is what was checked, and it is
/// read through a bound, so a read takes at most `MAX_SCRIPT + 1` bytes.
pub fn load(path: &std::path::Path, listeners: &[String]) -> Result<Vec<Event>, String> {
    use std::io::Read;
    if listeners.is_empty() {
        return Err(NEEDS_GRANT.to_owned());
    }
    let shown = path.display();
    let cannot = |error: std::io::Error| {
        format!("cannot read the script `{shown}` of `--serve-script`: {error}")
    };
    let regular = |metadata: std::fs::Metadata| {
        if !metadata.is_file() {
            return Err(format!(
                "the script `{shown}` of `--serve-script` is not a regular file"
            ));
        }
        if metadata.len() > MAX_SCRIPT {
            return Err(format!(
                "the script `{shown}` of `--serve-script` is larger than {} MiB",
                MAX_SCRIPT / (1024 * 1024)
            ));
        }
        Ok(())
    };
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(una_sys::server::OPEN_FLAGS)
            .open(path)
            .map_err(cannot)?
    };
    regular(file.metadata().map_err(cannot)?)?;
    let mut bytes = Vec::new();
    file.take(MAX_SCRIPT + 1)
        .read_to_end(&mut bytes)
        .map_err(cannot)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SCRIPT {
        return Err(format!(
            "the script `{shown}` of `--serve-script` is larger than {} MiB",
            MAX_SCRIPT / (1024 * 1024)
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        format!("cannot read the script `{shown}` of `--serve-script`: it is not UTF-8")
    })?;
    parse(&text, listeners)
        .map_err(|why| format!("the script `{shown}` of `--serve-script`, {why}"))
}

/// Writes the text of `--serve-out` to `path`, which holds what the program sent, `internal`
/// data and credentials included, unredacted (`spec/31 §8` rule 6): the last component of the
/// path is never followed as a symbolic link, the open never blocks (a FIFO with no reader is
/// refused), a file that is not a regular file is refused, and the file's mode is 0600, an
/// existing file's included, before it is truncated and written.
pub fn write_out(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(una_sys::server::WRITE_FLAGS)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("it is not a regular file"));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.set_len(0)?;
    file.write_all(text.as_bytes())
}

/// The message of a `--serve-out` that cannot be written, the same in both engines.
pub fn out_error(path: &std::path::Path, error: &std::io::Error) -> String {
    format!(
        "cannot write the connections to `{}`: {error}",
        path.display()
    )
}

/// The events of the script `text`, for a run whose `serve:` grants have the addresses
/// `listeners` (as `ServeGrant::listener` writes them), or why it cannot be followed: the
/// message names the line, counted from 1.
///
/// Lines end with LF, the last one optionally; every line is one JSON object with nothing
/// before its `{` or after its `}`, so an empty line, a line of spaces and a CR, CRLF line
/// ends included, are refused. A script with no lines opens no connection.
pub fn parse(text: &str, listeners: &[String]) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    let mut connections: HashMap<u64, State> = HashMap::new();
    let mut per_peer: HashMap<IpAddr, u32> = HashMap::new();
    let mut last = 0u64;
    if text.is_empty() {
        return Ok(events);
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    for (index, line) in body.split('\n').enumerate() {
        let event = event(line, listeners, &mut connections, &mut per_peer, last)
            .map_err(|why| format!("line {}: {why}", index + 1))?;
        last = event.at;
        events.push(event);
    }
    Ok(events)
}

/// The keys a line of a script may have.
const KEYS: [&str; 8] = [
    "at",
    "connection",
    "grant",
    "peer",
    "reads",
    "bytes",
    "read",
    "close",
];

/// A JSON object whose keys are among [`KEYS`] and each appear once: the first key that is not
/// one of them, or that appears again, is refused at once, with its message in `refused`. The
/// object so holds at most 8 entries, and reading it is O(n) in the line.
struct Once<'a> {
    refused: &'a mut Option<String>,
}

impl<'de> DeserializeSeed<'de> for Once<'_> {
    type Value = Map<String, Value>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Once<'_> {
    type Value = Map<String, Value>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            let why = if !KEYS.contains(&key.as_str()) {
                Some(format!("`{key}` is not a key of this line"))
            } else if object.contains_key(&key) {
                Some(format!("`{key}` appears twice"))
            } else {
                None
            };
            if let Some(why) = why {
                let error = A::Error::custom(&why);
                *self.refused = Some(why);
                return Err(error);
            }
            let value: Value = access.next_value()?;
            object.insert(key, value);
        }
        Ok(object)
    }
}

/// The JSON object of `line`, its keys among [`KEYS`] and each once, with nothing before or
/// after it.
fn object(line: &str) -> Result<Map<String, Value>, String> {
    if line.contains('\r') {
        return Err("the line holds a CR; lines end with LF alone".to_owned());
    }
    if !line.starts_with('{') || !line.ends_with('}') {
        return Err("not a JSON object alone on its line".to_owned());
    }
    let mut refused = None;
    let mut deserializer = serde_json::Deserializer::from_str(line);
    let parsed = Once {
        refused: &mut refused,
    }
    .deserialize(&mut deserializer)
    .and_then(|object| deserializer.end().map(|()| object));
    match (parsed, refused) {
        (_, Some(why)) => Err(why),
        (Ok(object), None) => Ok(object),
        (Err(error), None) => Err(format!("not a JSON object: {error}")),
    }
}

/// One line, checked against the connections before it and the time `last` of the line
/// before it: its form and keys first, then its numbers, then its place in the script.
fn event(
    line: &str,
    listeners: &[String],
    connections: &mut HashMap<u64, State>,
    per_peer: &mut HashMap<IpAddr, u32>,
    last: u64,
) -> Result<Event, String> {
    let object = object(line)?;
    let form: &[&str] = if object.contains_key("grant") {
        &["grant", "peer", "reads"]
    } else if object.contains_key("bytes") {
        &["bytes"]
    } else if object.contains_key("read") {
        &["read"]
    } else if object.contains_key("close") {
        &["close"]
    } else {
        return Err("the line has none of `grant`, `bytes`, `read` and `close`".to_owned());
    };
    keys(&object, form)?;
    let number = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("`{key}` is not a whole number from 0 to {}", u64::MAX))
    };
    let at = number("at")?;
    let connection = number("connection")?;
    if at < last {
        return Err(format!("`at` is {at}, before the line above it, at {last}"));
    }
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("`{key}` is not a text"))
    };
    let state = connections.get(&connection).copied();
    let action = if form == ["grant", "peer", "reads"] {
        if state.is_some() {
            return Err(format!("connection {connection} is opened twice"));
        }
        let address = text("grant")?;
        let grant = listeners
            .iter()
            .position(|listener| listener == address)
            .ok_or_else(|| format!("`{address}` is the address of no `serve:` grant of the run"))?;
        let peer: IpAddr = text("peer")?
            .parse()
            .map_err(|_| "`peer` is not an IP address".to_owned())?;
        let reads = match text("reads")? {
            "eager" => PeerReads::Eager,
            "paced" => PeerReads::Paced,
            other => return Err(format!("`reads` is `{other}`, not `eager` or `paced`")),
        };
        let opened = per_peer.entry(peer).or_insert(0u32);
        if *opened == PEER_CONNECTIONS {
            return Err(format!(
                "`{peer}` opens more than {PEER_CONNECTIONS} connections, one per port it has"
            ));
        }
        *opened += 1;
        connections.insert(connection, State::Open(reads));
        Action::Open { grant, peer, reads }
    } else {
        let reads = match state {
            Some(State::Open(reads)) => reads,
            Some(State::Closed) => {
                return Err(format!("connection {connection} is closed already"));
            }
            None => return Err(format!("connection {connection} is not open")),
        };
        if form == ["bytes"] {
            Action::Bytes(base64(text("bytes")?).ok_or("`bytes` is not base64 text")?)
        } else if form == ["read"] {
            if reads != PeerReads::Paced {
                return Err(format!(
                    "connection {connection} reads eagerly, so it takes no `read`"
                ));
            }
            match number("read")? {
                0 => return Err("`read` is 0; give at least 1".to_owned()),
                n => Action::Read(n),
            }
        } else {
            if object.get("close") != Some(&Value::Bool(true)) {
                return Err("`close` is not `true`".to_owned());
            }
            connections.insert(connection, State::Closed);
            Action::Close
        }
    };
    Ok(Event {
        at,
        connection,
        action,
    })
}

/// Refuses a line whose keys are not `at`, `connection` and `form`.
fn keys(object: &Map<String, Value>, form: &[&str]) -> Result<(), String> {
    for key in object.keys() {
        if key != "at" && key != "connection" && !form.contains(&key.as_str()) {
            return Err(format!("`{key}` is not a key of this line"));
        }
    }
    match form.iter().find(|key| !object.contains_key(**key)) {
        Some(missing) => Err(format!("`{missing}` is missing")),
        None => Ok(()),
    }
}

/// The bytes of `text` in the standard base64 alphabet with its padding (RFC 4648 §4), or
/// `None` when it is not exactly that form: a length that is not a multiple of 4, a character
/// outside the alphabet, padding before the end, or bits left over that are not zero.
pub fn base64(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let value = |b: u8| match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let quads = bytes.chunks(4).count();
    for (index, quad) in bytes.chunks(4).enumerate() {
        let pad = quad.iter().rev().take_while(|b| **b == b'=').count();
        if pad > 2 || (pad > 0 && index + 1 != quads) {
            return None;
        }
        let mut n = 0u32;
        for &b in quad.get(..4 - pad)? {
            n = (n << 6) | u32::from(value(b)?);
        }
        n <<= 6 * pad;
        let [_, a, b, c] = n.to_be_bytes();
        let taken = [a, b, c];
        let keep = 3 - pad;
        if taken.get(keep..)?.iter().any(|byte| *byte != 0) {
            return None;
        }
        out.extend_from_slice(taken.get(..keep)?);
    }
    Some(out)
}

#[cfg(test)]
mod tests;
