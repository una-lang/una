//! The real-clock timing oracles of the amendment of ADR 0116 (section G): for each channel a
//! fix closes, a program whose observable timing, through a deadline, a race, a limit or a
//! tick, turns a difference of time into a difference the outside sees. Their timing needs a
//! machine without another heavy job, so they are ignored in the ordinary suite and run in the
//! release gates' quiet slot and in the pull request of each fix, where their output is
//! recorded. The deterministic halves of their differentials run in the ordinary suite.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/history.rs"]
mod history;

#[test]
#[ignore = "real-clock timing oracle, run in the release gates' quiet slot"]
fn the_history_of_a_limit_amplifies_time_on_this_machine() {
    // The positive control of the differential of #1147 (ADR 0116 P8, slice T1), whose secret
    // half is in `http_limits.rs`: the twin whose `key` is `public` is accepted under the same
    // grant, one request per second, so the limits work, and its two runs differ, the server
    // receiving one request when `key` is 0 and two when it is large, in each engine. So the
    // amplifier, the limit and the 300 ms cut in `fetch`, turns the time of `crunch` into the
    // number of requests sent on this machine, and the `secret` program, before the fix, leaked
    // `key` the same way.
    let server = history::CountingServer::start();
    for native in [false, true] {
        let large = history::large_key(native);
        let control = [
            history::observe_history(&server, 0, false, native),
            history::observe_history(&server, large, false, native),
        ];
        assert_eq!(
            control,
            [(Some(0), String::new(), 1), (Some(0), String::new(), 2)],
            "the control, native {native}"
        );
    }
}
