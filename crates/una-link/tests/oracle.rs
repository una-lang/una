//! The internal linker against Apple's `ld` on a Rust program that unwinds (ADR 0124 rules 25
//! and 33): a panic caught by `catch_unwind`, which needs `__unwind_info`, `__eh_frame` and the
//! personality routine, a thread-local variable read on another thread, and output through the
//! standard library. The program is compiled by `rustc` into a static library, a tool of the
//! test, not of `una`; both linkers link it, and both executables must give the same output and
//! exit status.
#![cfg(all(target_os = "macos", target_arch = "aarch64"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod common;

use std::process::Command;

const PROGRAM: &str = r#"
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<u32> = const { Cell::new(5) };
}

#[inline(never)]
fn deep(n: u32) -> u32 {
    if n == 0 {
        panic!("deliberate panic at the bottom");
    }
    deep(n - 1) + 1
}

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    std::panic::set_hook(Box::new(|info| eprintln!("hook: {info}")));
    let caught = std::panic::catch_unwind(|| deep(std::env::args().count() as u32 + 3));
    COUNT.with(|c| c.set(c.get() + 1));
    let other = std::thread::spawn(|| COUNT.with(|c| c.get())).join().unwrap();
    let here = COUNT.with(|c| c.get());
    let mut words: Vec<String> = (0..1000).map(|i| format!("w{i}")).collect();
    words.sort();
    println!("caught={} other={other} here={here} first={} last={}", caught.is_err(), words[0], words[999]);
    if caught.is_err() { 3 } else { 0 }
}
"#;

#[test]
fn a_rust_program_that_unwinds_runs_alike_with_both_linkers() {
    let dir = std::env::temp_dir().join(format!("una-link-oracle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("program.rs"), PROGRAM).unwrap();
    for (name, flags) in [
        ("debug", &["-C", "opt-level=0"][..]),
        (
            "release",
            &["-C", "opt-level=3", "-C", "codegen-units=4"][..],
        ),
    ] {
        let archive = dir.join(format!("lib{name}.a"));
        let built = Command::new("rustc")
            .args([
                "--edition",
                "2024",
                "--crate-type",
                "staticlib",
                "-C",
                "panic=unwind",
            ])
            .args(flags)
            .arg(dir.join("program.rs"))
            .arg("-o")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let runtime = std::fs::read(&archive).unwrap();
        // The program's object only asks for `_main`, which the archive defines.
        let program = common::Object {
            sections: Vec::new(),
            symbols: vec![common::undefined("_main")],
        };
        let bytes = una_link::link(
            &program.bytes(),
            &runtime,
            &una_link::Options {
                identifier: "oracle",
                debuggable: false,
            },
        )
        .unwrap();
        let ours = dir.join(format!("ours-{name}"));
        std::fs::write(&ours, &bytes).unwrap();
        assert!(
            Command::new("chmod")
                .arg("755")
                .arg(&ours)
                .status()
                .unwrap()
                .success()
        );
        let theirs = dir.join(format!("theirs-{name}"));
        let linked = Command::new("cc")
            .arg("-Wl,-u,_main")
            .arg(&archive)
            .args(["-Wl,-S", "-Wl,-dead_strip", "-o"])
            .arg(&theirs)
            .output()
            .unwrap();
        assert!(
            linked.status.success(),
            "{}",
            String::from_utf8_lossy(&linked.stderr)
        );
        let run = |path: &std::path::Path| Command::new(path).output().unwrap();
        let (a, b) = (run(&ours), run(&theirs));
        assert_eq!(
            String::from_utf8_lossy(&a.stdout),
            String::from_utf8_lossy(&b.stdout),
            "{name}"
        );
        assert_eq!(
            String::from_utf8_lossy(&a.stderr),
            String::from_utf8_lossy(&b.stderr),
            "{name}"
        );
        assert_eq!(a.status.code(), b.status.code(), "{name}");
        assert_eq!(
            a.status.code(),
            Some(3),
            "{name}: {}",
            String::from_utf8_lossy(&a.stderr)
        );
        assert!(
            String::from_utf8_lossy(&a.stdout)
                .contains("caught=true other=5 here=6 first=w0 last=w999")
        );
        assert!(String::from_utf8_lossy(&a.stderr).contains("deliberate panic at the bottom"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
