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
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// What one look of a wait that parks outside the engine found ([`Port::look`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// The wait ends, as [`Port::wait`] would have ended it.
    Done(Waited),
    /// Nothing yet: the caller parks for at most `slice`, then looks again. When the look was
    /// given a wake, the engine holds it as `ticket` and calls it at most once, at a signal the
    /// waiter may claim; the caller gives the ticket back with [`Port::unpark`] after its park.
    Park {
        /// The wake's place among the engine's parked waiters, when one was given.
        ticket: Option<u64>,
        /// The longest the caller parks before its next look.
        slice: Duration,
    },
}

/// What wakes a waiter that parks outside the engine: called at most once per look.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

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

    /// Waits until something changed since `epoch` that may let the next attempt deliver,
    /// until `retry` on the engine's clock when a rate holds the next request, until
    /// `deadline`, or until `stopped` holds, which it reads at least every few milliseconds. It
    /// takes no request. An engine may end only one wait for an event that lets one request
    /// through, and every wait for a stop ([`super::live`]).
    fn wait(
        &self,
        epoch: u64,
        retry: Option<u64>,
        deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
    ) -> Waited;

    /// One look of a wait that parks its caller outside the engine, as a task parks on the
    /// native runtime's reactor without holding a thread (#1723): the same tests, in the same
    /// order, as one pass of [`Port::wait`], without its sleep. With `wake`, a look that finds
    /// nothing registers it, under the lock its signals are sent under, so a signal sent after
    /// the look calls it and none is lost between the look and the park. `None` when the engine
    /// has no such waits: its caller waits with [`Port::wait`].
    fn look(
        &self,
        epoch: u64,
        retry: Option<u64>,
        deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
        wake: Option<Waker>,
    ) -> Option<Look> {
        let _ = (epoch, retry, deadline, stopped, wake);
        None
    }

    /// Gives back the place of a parked waiter's wake ([`Look::Park`]), whether or not a signal
    /// called it. A spare call does nothing.
    fn unpark(&self, ticket: u64) {
        let _ = ticket;
    }

    /// Hands on a wake that a wait ended with [`Waited::Changed`] and that its caller will not
    /// use: a `next` that leaves without a further attempt, as at a cancellation, calls it so
    /// that the request that woke it wakes another waiter. A spare call costs one wake.
    fn pass(&self) {}

    /// `close` (ADR 0123 rule 1): the listeners close, the requests read and not delivered are
    /// answered `503` with `Connection: close`, and each connection finishes the exchange it
    /// has open and closes. The program's side refuses every later `next` with `Closed`.
    fn close(&self);

    /// The hard stop of the engine (ADR 0123 rule 18 item 6): every exchange still open is
    /// answered `503` in one non-blocking write and every connection closes, without waiting on
    /// a peer. From the call on, the engine discards every answer and reply of the program
    /// before it can take an exchange ([`super::run::Exchange::reply`]), so only the serve
    /// threads end the exchanges still open. The program's side calls it before it drops the
    /// exchanges its tasks hold at the hard stop. A reply already in progress when the hard stop
    /// comes, one that checked the stop before it and takes its exchange after it, may still be
    /// lost: its connection can close before the reply is written, with no `503` either. An
    /// engine with no peers to answer, as a script's, does nothing.
    fn hard_stop(&self) {}

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
