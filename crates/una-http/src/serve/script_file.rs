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
//! The times never decrease from a line to the next, and lines of equal `at` take effect in
//! their order. A connection is opened once, before its other lines. Reading is O(n) in the
//! bytes of the file, with one hash lookup per line.

use std::collections::HashMap;
use std::net::IpAddr;

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

/// The events of the script `text`, for a run whose `serve:` grants have the addresses
/// `listeners` (as `ServeGrant::listener` writes them), or why it cannot be followed: the
/// message names the line, counted from 1.
pub fn parse(text: &str, listeners: &[String]) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    let mut connections: HashMap<u64, State> = HashMap::new();
    let mut last = 0u64;
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let event = event(line, listeners, &mut connections, last)
            .map_err(|why| format!("line {}: {why}", index + 1))?;
        last = event.at;
        events.push(event);
    }
    Ok(events)
}

/// One line, checked against the connections before it and the time `last` of the line
/// before it.
fn event(
    line: &str,
    listeners: &[String],
    connections: &mut HashMap<u64, State>,
    last: u64,
) -> Result<Event, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| format!("not a JSON object: {error}"))?;
    let Value::Object(object) = value else {
        return Err("not a JSON object".to_owned());
    };
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
    let action = if object.contains_key("grant") {
        keys(&object, &["grant", "peer", "reads"])?;
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
        if object.contains_key("bytes") {
            keys(&object, &["bytes"])?;
            Action::Bytes(base64(text("bytes")?).ok_or("`bytes` is not base64 text")?)
        } else if object.contains_key("read") {
            keys(&object, &["read"])?;
            if reads != PeerReads::Paced {
                return Err(format!(
                    "connection {connection} reads eagerly, so it takes no `read`"
                ));
            }
            match number("read")? {
                0 => return Err("`read` is 0; give at least 1".to_owned()),
                n => Action::Read(n),
            }
        } else if object.contains_key("close") {
            keys(&object, &["close"])?;
            if object.get("close") != Some(&Value::Bool(true)) {
                return Err("`close` is not `true`".to_owned());
            }
            connections.insert(connection, State::Closed);
            Action::Close
        } else {
            return Err("the line has none of `grant`, `bytes`, `read` and `close`".to_owned());
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
