//! The `canary` fuzz target (#766): a crash planted behind an input the fuzzer must find,
//! `una!` at the start. scripts/sanitize.sh runs it first, as its self-check, and fails unless
//! the crash is reported, a reproducer kept and a minimized input made: otherwise a crash in
//! the other targets could pass unseen. It is never one of the targets the gate checks. Its
//! crash happens on a thread of its own, as the targets run each input, so the check covers a
//! panic there.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let bytes = bytes.to_vec();
    let checked = std::thread::spawn(move || {
        // One byte at a time, so the fuzzer's comparison tracing finds each in turn.
        if bytes.first() == Some(&b'u')
            && bytes.get(1) == Some(&b'n')
            && bytes.get(2) == Some(&b'a')
            && bytes.get(3) == Some(&b'!')
        {
            panic!("the canary's planted crash");
        }
    })
    .join();
    assert!(checked.is_ok());
});
