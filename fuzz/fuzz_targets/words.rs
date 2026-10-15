//! The `words` fuzz target (#766): sequences of calls of the runtime's entry points, with words
//! the generated code never passes, as the word tests of `una-rt`'s `ffi.rs` make them, but
//! chosen by the fuzzer. Outside a run, as those tests: an abort returns 0 rather than ending
//! the process. Every entry point of the table but the host operations, whose effects need a
//! run's grants; `una_rt_range` takes small bounds, since outside a run no memory budget bounds
//! its list. Each input runs on a thread of its own, whose runtime heap starts empty and ends
//! with it, so an input does the same on its own as in a run of the fuzzer.
//!
//! An input is read as calls: one byte picks the entry point, then each argument is either a
//! word of the pool (a byte below 0x80) or the next eight bytes as a word (any other byte).
#![no_main]

use libfuzzer_sys::fuzz_target;
use num_bigint::BigInt;
use una_rt::{ffi, values};

/// The most calls one input makes.
const MOST_CALLS: usize = 64;

/// The words of the pool: boundary words, live handles of each kind, a freed handle, and
/// handles one slot off a live one, made on the input's thread.
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
        let table: Vec<_> = ffi::exported_table()
            .into_iter()
            .filter(|(_, function, _, _)| !function.starts_with("host"))
            .collect();
        let pool = pool();
        let small: Vec<i64> = (-3..=3)
            .map(|value| values::int_word(&BigInt::from(value)))
            .collect();
        let mut rest = bytes.as_slice();
        for _ in 0..MOST_CALLS {
            let Some((&pick, after)) = rest.split_first() else {
                break;
            };
            rest = after;
            let Some((name, _, arity, call)) = table.get(usize::from(pick) % table.len()) else {
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
                } else if choice < 0x80 {
                    pool[usize::from(choice) % pool.len()]
                } else if let Some((raw, after)) = rest.split_first_chunk::<8>() {
                    rest = after;
                    i64::from_le_bytes(*raw)
                } else {
                    0
                };
                words.push(word);
            }
            call(&words);
        }
        // The runtime still works.
        let text = values::text_literal("still");
        assert_eq!(values::text_len(text), values::int_word(&BigInt::from(5)));
    })
    .join();
    // A panic in the thread is a finding: libfuzzer-sys's panic hook has aborted already.
    assert!(checked.is_ok());
});
