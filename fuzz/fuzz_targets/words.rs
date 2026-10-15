//! The `words` fuzz target (#766): sequences of calls of the runtime's entry points, with words
//! the generated code never passes, as the word tests of `una-rt`'s `ffi.rs` make them, but
//! chosen by the fuzzer. Outside a run, as those tests: an abort returns 0 rather than ending
//! the process. Each input runs on a thread of its own, whose runtime heap starts empty and
//! ends with it, so an input does the same on its own as in a run of the fuzzer.
//!
//! The calls: every entry point of the export table but the host's effects, which need a run's
//! grants (EFFECTS); `una_rt_range` takes small bounds, since outside a run no memory budget
//! bounds its list. And the entry points outside the table that take only words (EXTRA): the
//! reference counts, building objects, the call depth, the budget and the operation and event
//! state. Not called: `una_rt_closure` and `una_rt_push_op`, which take the address of
//! compiled code, `together` and `together for`, which run such code on tasks, `una_rt_start`
//! and `una_rt_main_failed`, which start and end a run, and the literals, the `literal`
//! target's.
//!
//! An input is read as calls: one byte picks the entry point, then each argument is a byte that
//! picks a word of the pool (below 0xF0) or announces the next eight bytes as a word (0xF0 and
//! above). The pool starts with boundary words and live handles of each kind, and every word a
//! call returns joins it, up to POOL_CAP words, so later calls take the handles earlier ones
//! made: lists and maps the input built, values after a release, copies and shares.
#![no_main]

use libfuzzer_sys::fuzz_target;
use num_bigint::BigInt;
use una_rt::{ffi, values};

/// The most calls one input makes.
const MOST_CALLS: usize = 64;

/// The most words the pool holds: the selector bytes below 0xF0 reach every one.
const POOL_CAP: usize = 0xF0;

/// The entry points of the table that are host effects, not called.
const EFFECTS: [&str; 10] = [
    "una_rt_console_print",
    "una_rt_clock_now",
    "una_rt_env_get",
    "una_rt_files_read",
    "una_rt_files_write",
    "una_rt_keys_hmac",
    "una_rt_http_get",
    "una_rt_http_post",
    "una_rt_http_put",
    "una_rt_http_delete",
];

/// An entry point to call: its name, its arity, and its function on words.
type Call = (&'static str, usize, fn(&[i64]) -> i64);

/// A word of `words` at `at`, or 0.
fn at(words: &[i64], at: usize) -> i64 {
    words.get(at).copied().unwrap_or(0)
}

/// The entry points outside the table that are called, with their arities. Those that return
/// nothing give 0.
fn extra() -> Vec<Call> {
    vec![
        ("una_rt_retain", 1, |w| {
            ffi::una_rt_retain(at(w, 0));
            0
        }),
        ("una_rt_release", 1, |w| {
            ffi::una_rt_release(at(w, 0));
            0
        }),
        ("una_rt_push", 2, |w| {
            ffi::una_rt_push(at(w, 0), at(w, 1));
            0
        }),
        ("una_rt_enter", 1, |w| ffi::una_rt_enter(at(w, 0))),
        ("una_rt_leave", 0, |_| {
            ffi::una_rt_leave();
            0
        }),
        ("una_rt_budget", 1, |w| ffi::una_rt_budget(at(w, 0))),
        ("una_rt_poll", 0, |_| ffi::una_rt_poll()),
        ("una_rt_stopped", 0, |_| ffi::una_rt_stopped()),
        ("una_rt_operation_begin", 0, |_| {
            ffi::una_rt_operation_begin();
            0
        }),
        ("una_rt_operation_end", 0, |_| {
            ffi::una_rt_operation_end();
            0
        }),
        ("una_rt_event_arg", 4, |w| {
            ffi::una_rt_event_arg(at(w, 0), at(w, 1), at(w, 2), at(w, 3));
            0
        }),
        ("una_rt_event_end", 4, |w| {
            ffi::una_rt_event_end(at(w, 0), at(w, 1), at(w, 2), at(w, 3));
            0
        }),
        ("una_rt_together_status", 0, |_| {
            ffi::una_rt_together_status()
        }),
    ]
}

/// The words the pool starts with: boundary words, live handles of each kind, a freed handle,
/// and handles one slot off a live one, made on the input's thread.
fn pool() -> Vec<i64> {
    let int = |value: i64| values::int_word(&BigInt::from(value));
    let freed = values::text_word("freed".to_owned());
    values::release(freed);
    let text = values::text_literal("abc");
    let some_list = values::append(values::list(), text);
    vec![
        0,
        1,
        2,
        3,
        -1,
        -2,
        i64::MIN,
        i64::MAX,
        i64::MIN + 1,
        i64::MAX - 1,
        1 << 32,
        1 << 62,
        -(1 << 62),
        1 << 40,
        freed,
        some_list.wrapping_add(2),
        some_list.wrapping_sub(2),
        int(7),
        int(-7),
        values::int_word(&(BigInt::from(i64::MAX) * 4)),
        text,
        values::text_word(String::new()),
        values::list(),
        some_list,
        values::data(3),
        values::map_empty(),
    ]
}

fuzz_target!(|bytes: &[u8]| {
    let bytes = bytes.to_vec();
    let checked = std::thread::spawn(move || {
        let mut calls: Vec<Call> = ffi::exported_table()
            .into_iter()
            .filter(|(name, _, _, _)| !EFFECTS.contains(name))
            .map(|(name, _, arity, call)| (name, arity, call))
            .collect();
        calls.extend(extra());
        let mut pool = pool();
        let small: Vec<i64> = (-3..=3)
            .map(|value| values::int_word(&BigInt::from(value)))
            .collect();
        let mut rest = bytes.as_slice();
        for _ in 0..MOST_CALLS {
            let Some((&pick, after)) = rest.split_first() else {
                break;
            };
            rest = after;
            let Some((name, arity, call)) = calls.get(usize::from(pick) % calls.len()) else {
                break;
            };
            let mut words = Vec::with_capacity(*arity);
            for _ in 0..*arity {
                let Some((&choice, after)) = rest.split_first() else {
                    break;
                };
                rest = after;
                let word = if *name == "una_rt_range" {
                    small[usize::from(choice) % small.len()]
                } else if usize::from(choice) < POOL_CAP {
                    pool[usize::from(choice) % pool.len()]
                } else if let Some((raw, after)) = rest.split_first_chunk::<8>() {
                    rest = after;
                    i64::from_le_bytes(*raw)
                } else {
                    0
                };
                words.push(word);
            }
            let result = call(&words);
            if result != 0 && pool.len() < POOL_CAP {
                pool.push(result);
            }
        }
        // The runtime still works.
        let text = values::text_literal("still");
        assert_eq!(values::text_len(text), values::int_word(&BigInt::from(5)));
    })
    .join();
    // A panic in the thread is a finding: libfuzzer-sys's panic hook has aborted already.
    assert!(checked.is_ok());
});
