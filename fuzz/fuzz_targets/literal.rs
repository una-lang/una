//! The `literal` fuzz target (#766): the two entry points that read bytes the generated code
//! passes by pointer and length, `una_rt_int_literal` and `una_rt_text_literal`, on any bytes.
//! Each input runs on a thread of its own, whose runtime heap starts empty and ends with it,
//! so an input does the same on its own as in a run of the fuzzer.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_core::bigint::BigInt;
use una_rt::{ffi, values};

fuzz_target!(|bytes: &[u8]| {
    let bytes = bytes.to_vec();
    let checked = std::thread::spawn(move || {
        let (int, text) = ffi::fuzz_literals(&bytes);
        // The text literal holds the bytes when they are UTF-8, and the empty text otherwise.
        let expected = String::from_utf8(bytes).unwrap_or_default();
        assert_eq!(values::text_value(text), expected);
        // The integer literal is the integer the text writes in decimal, and 0 for any other.
        let value: BigInt = expected.parse().unwrap_or_default();
        assert_eq!(values::int_value(int), value);
    })
    .join();
    // A panic in the thread is a finding: libfuzzer-sys's panic hook has aborted already.
    assert!(checked.is_ok());
});
