//! The per-type oracle of the ELF relocation engine against `ld.lld` (ADR 0124 rule 33, step
//! 4b): the fixtures of tests/elf-oracle, one place or sequence per supported relocation type
//! and per form the engine treats differently, are assembled by `clang` and linked by the Rust
//! toolchain's `rust-lld` (`ld.lld` of LLVM) into static-pie executables for both musl targets;
//! at every place, the engine, given the addresses `ld.lld` chose, must write `ld.lld`'s bytes
//! (`una_link::elf_oracle`). Every type of each target's list must be compared at least once.
//! Tools of the test, not of `una`: the macOS gate machine has both (Apple's `clang` assembles
//! ELF for both architectures). GNU `ld` is the second oracle, in the Linux container
//! (scripts/link-oracle-elf.sh).
#![cfg(target_os = "macos")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::print_stdout
)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The toolchain's `rust-lld`.
fn rust_lld() -> PathBuf {
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("rustc runs");
    let host = Command::new("rustc")
        .args(["--print", "host-tuple"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("rustc runs");
    let lld = PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim())
        .join("lib/rustlib")
        .join(String::from_utf8(host.stdout).unwrap().trim())
        .join("bin/rust-lld");
    assert!(lld.is_file(), "{} is missing", lld.display());
    lld
}

fn run(command: &mut Command) {
    let output = command.output().expect("the tool runs");
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Assembles `fixture` and absolute.s for `target` and links them with `ld.lld`, with `extra`
/// flags; returns the fixture object's and the executable's bytes.
fn build(dir: &Path, fixture: &str, target: &str, extra: &[&str]) -> (Vec<u8>, Vec<u8>) {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/elf-oracle")
        .join(fixture);
    let object = dir.join(format!("{fixture}.o"));
    let absolute = dir.join(format!("{fixture}.absolute.o"));
    let linked = dir.join(format!("{fixture}.lld"));
    run(Command::new("clang")
        .args(["-target", target, "-c"])
        .arg(&source)
        .arg("-o")
        .arg(&object));
    run(Command::new("clang")
        .args(["-target", target, "-c"])
        .arg(source.with_file_name("absolute.s"))
        .arg("-o")
        .arg(&absolute));
    run(Command::new(rust_lld())
        .args(["-flavor", "gnu", "-static", "-pie", "--no-dynamic-linker"])
        .args([
            "-z",
            "nodynamic-undefined-weak",
            "-z",
            "text",
            "-nostdlib",
            "-e",
            "_start",
        ])
        .args(extra)
        .arg("-o")
        .arg(&linked)
        .arg(&object)
        .arg(&absolute));
    (
        std::fs::read(&object).unwrap(),
        std::fs::read(&linked).unwrap(),
    )
}

#[test]
fn every_type_writes_the_bytes_of_ld_lld() {
    let dir = std::env::temp_dir().join(format!("una-link-elf-oracle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let types = una_link::elf_relocation_types();
    // On arm64, `--no-relax` keeps `ld.lld` from its optional `adrp`/`add` and GOT
    // optimizations, which the engine does not make; its TLS relaxations stay on.
    for (fixture, target, extra) in [
        ("x86_64.s", "x86_64-unknown-linux-musl", &[][..]),
        (
            "aarch64.s",
            "aarch64-unknown-linux-musl",
            &["--no-relax"][..],
        ),
    ] {
        let (object, linked) = build(&dir, fixture, target, extra);
        let report = una_link::elf_oracle(&object, &linked).unwrap();
        for (key, count) in &report.equal {
            println!("{target}: {key}: {count}");
        }
        assert!(
            report.mismatches.is_empty(),
            "{target}: {:#?}",
            report.mismatches
        );
        // `ld.lld` makes the engine's choices everywhere: every place is byte for byte.
        assert!(
            report.equivalent.is_empty(),
            "{target}: {:#?}",
            report.equivalent
        );
        for name in &types[target] {
            assert!(
                report
                    .equal
                    .keys()
                    .any(|key| key.split(' ').next() == Some(*name)),
                "{target}: the fixture has no place of {name}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
