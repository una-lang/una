//! The `json` fuzz target (#766): the runtime's JSON parser, which reads text a host operation
//! returns (`parse_json`, spec/23), on any text. For text that parses, its canonical text
//! (spec/23 §4) must parse again to the same canonical text, and the runtime's entry points,
//! `una_rt_parse_json` then `una_rt_to_json`, must give that same text. Each input runs on a
//! thread of its own, whose runtime heap starts empty and ends with it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_core::json;
use una_rt::{ffi, values};

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let text = text.to_owned();
    let checked = std::thread::spawn(move || {
        let parsed = json::parse(&text);
        // Outside a run no memory budget applies, and the site is not read.
        let word = ffi::una_rt_parse_json(values::text_word(text), 0);
        if let Ok(tree) = parsed {
            let canonical = json::write(&tree);
            let again = json::parse(&canonical).map(|tree| json::write(&tree));
            assert_eq!(again.as_deref(), Ok(canonical.as_str()));
            let written = values::text_value(ffi::una_rt_to_json(word, 0));
            assert_eq!(written, canonical);
        }
    })
    .join();
    // A panic in the thread is a finding: libfuzzer-sys's panic hook has aborted already.
    assert!(checked.is_ok());
});
