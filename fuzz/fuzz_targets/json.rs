//! The `json` fuzz target (#766): the runtime's JSON parser, which reads text a host operation
//! returns (`parse_json`, spec/23), on any text: the input's bytes as UTF-8, a byte that is not
//! replaced by U+FFFD. Each input runs on a thread of its own, whose runtime heap starts empty
//! and ends with it.
//!
//! The runtime's entry points must agree with the parser of `una-core` that both the
//! interpreter and the runtime use: for text that parses, `una_rt_parse_json` then
//! `una_rt_to_json` give its canonical text (spec/23 §4), which parses back to itself; for text
//! that does not, `una_rt_parse_json` gives the `JsonError` of the same variant and position.
#![no_main]

use libfuzzer_sys::fuzz_target;
use num_bigint::BigInt;
use una_core::json;
use una_rt::{ffi, values};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let checked = std::thread::spawn(move || {
        let parsed = json::parse(&text);
        // Outside a run no memory budget applies, and the site is not read.
        let word = ffi::una_rt_parse_json(values::text_word(text), 0);
        match parsed {
            Ok(tree) => {
                let canonical = json::write(&tree);
                let again = json::parse(&canonical).map(|tree| json::write(&tree));
                assert_eq!(again.as_deref(), Ok(canonical.as_str()));
                let written = values::text_value(ffi::una_rt_to_json(word, 0));
                assert_eq!(written, canonical);
            }
            Err(error) => {
                // The failure word of `parse_json`: the variant of `JsonError`, with the position.
                let (variant, position) = match error {
                    json::Error::Syntax(at) => (0, at),
                    json::Error::TooDeep(at) => (1, at),
                    json::Error::DuplicateKey(at) => (2, at),
                    json::Error::NumberTooLong(at) => (3, at),
                };
                let expected = values::data(variant);
                values::push(expected, values::int_word(&BigInt::from(position)));
                assert_eq!(values::equal(word, expected), 1, "{error:?}");
            }
        }
    })
    .join();
    // A panic in the thread is a finding: libfuzzer-sys's panic hook has aborted already.
    assert!(checked.is_ok());
});
