//! The program's side of a run's serve engine (ADR 0123 rules 1 to 4, 14 and 18, `spec/31 §1`):
//! what the operations `next`, `reply` and `close` of `HttpServer` call, in both engines.
//!
//! A [`Port`] is the engine as the program meets it: one attempt to deliver a request
//! ([`Port::next`], which is [`super::run::Shared::next`] and so one atomic step: a request is
//! either delivered or left in its queue), a wait for a change that may let the next attempt
//! deliver ([`Port::wait`]), the run's first server operation ([`Port::begin`]) and `close`
//! ([`Port::close`]). The engine's own [`super::run::Exchange`] answers a reply, and
//! [`super::rights::Rights`] records which task may give it.
//!
//! A wait never takes a request: it only says that the next attempt may succeed, so a `next`
//! that fails while it waits, by its deadline or a host stop, leaves every request queued
//! (rule 4.7). The epoch protocol keeps a wake from being lost between an attempt that finds
//! nothing and the wait after it: the caller reads [`Port::epoch`] before the attempt and waits
//! for a change from that epoch.

use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

use super::run::Next;
use super::trace::TraceKey;

/// How a wait for the next request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waited {
    /// Something changed since the epoch waited from: the next attempt may deliver.
    Changed,
    /// The deadline passed first.
    TimedOut,
    /// The run's host stop came first.
    Stopped,
}

/// The serve engine of a run, as the program's operations meet it, from any thread of the run.
pub trait Port: Sync {
    /// The run's first server operation (ADR 0123, S4 amendment B.2): from here the listeners
    /// are registered and connections accepted. Once; later calls do nothing. `close` does not
    /// call it, so a `close` that comes first never begins accepting.
    fn begin(&self);

    /// A count that changes whenever a request is queued, an exchange ends or the run stops:
    /// read before [`Port::next`], and given to [`Port::wait`].
    fn epoch(&self) -> u64;

    /// One attempt to deliver the next request: [`super::run::Shared::next`].
    fn next(&self) -> Next;

    /// Waits until the epoch differs from `epoch`, until `retry` on the engine's clock when a
    /// rate holds the next request, until `deadline`, or until `stopped` holds, which it reads
    /// at least every few milliseconds. It takes no request.
    fn wait(
        &self,
        epoch: u64,
        retry: Option<u64>,
        deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
    ) -> Waited;

    /// `close` (ADR 0123 rule 1): the listeners close, the requests read and not delivered are
    /// answered `503` with `Connection: close`, and each connection finishes the exchange it
    /// has open and closes. The program's side refuses every later `next` with `Closed`.
    fn close(&self);

    /// The key of the run's reduced trace form (ADR 0123 rule 1, [`TraceKey`]): drawn at random
    /// when the run starts, all zeros under a script of connections.
    fn trace_key(&self) -> TraceKey;

    /// Whether a trace writes the values of the events of the host's `HttpServer` without the
    /// redaction of their labels, still in the reduced form: only the scripted engine of the
    /// toolchain's own tests says so, when a test asks it to, so that the tests compare the
    /// reduced form of both engines. Every other engine says no.
    fn reveals_trace(&self) -> bool {
        false
    }
}

/// The peer's address as a request names it (`spec/31 §1`, `peer`): its IP address without its
/// port, an IPv4-mapped IPv6 address written as its IPv4 address.
pub fn peer_text(address: SocketAddr) -> String {
    match address.ip() {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => v6.to_string(),
        },
        IpAddr::V4(v4) => v4.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_peer_is_its_address_without_the_port() {
        assert_eq!(peer_text("192.0.2.1:4000".parse().unwrap()), "192.0.2.1");
        assert_eq!(
            peer_text("[2001:db8::1]:4000".parse().unwrap()),
            "2001:db8::1"
        );
        assert_eq!(
            peer_text("[::ffff:192.0.2.9]:4000".parse().unwrap()),
            "192.0.2.9"
        );
    }
}
