//! The link arguments of the `una` binary (#1103).
//!
//! On macOS, the `libc` crate declares `#[link(name = "iconv")]` for its iconv functions, so
//! rustc passes `-liconv` and the linker records a load command for `/usr/lib/libiconv.2.dylib`
//! although `una` calls no iconv function. `-dead_strip_dylibs` makes ld64 drop every dylib
//! from which the binary imports no symbol; libSystem, which it uses, stays.
//! scripts/load-commands.sh fails on any load command outside scripts/load-commands.txt.
//!
//! The argument is set here rather than in a Cargo configuration file: `RUSTFLAGS`, which
//! scripts/release.sh sets, replaces the `rustflags` of a configuration file, so the release
//! build would not get it.

// A build script reads Cargo's variables and speaks to Cargo on standard output; the library
// crates' rules (compiler/conventions.md §1) do not apply to it.
#[allow(clippy::disallowed_methods, clippy::print_stdout)]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // The target's operating system, not the host's: the build script runs on the host.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bin=una=-Wl,-dead_strip_dylibs");
    }
}
