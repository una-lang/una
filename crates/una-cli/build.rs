//! The build script of `una`: its link arguments (#1103) and, in a release build, the native
//! runtime built into it (ADR 0124 rules 12, 14, 18 and 19, step 2, `docs/plan/0124-step2-design.md`).
//!
//! **Link arguments.** On macOS, the `libc` crate declares `#[link(name = "iconv")]` for its
//! iconv functions, so rustc passes `-liconv` and the linker records a load command for
//! `/usr/lib/libiconv.2.dylib` although `una` calls no iconv function. `-dead_strip_dylibs`
//! makes ld64 drop every dylib from which the binary imports no symbol; libSystem, which it
//! uses, stays. scripts/load-commands.sh fails on any load command outside
//! scripts/load-commands.txt. The argument is set here rather than in a Cargo configuration
//! file: `RUSTFLAGS`, which scripts/release.sh sets, replaces the `rustflags` of a
//! configuration file, so the release build would not get it.
//!
//! **The embedded runtime.** `scripts/release.sh` builds `libuna_rt.a` first, from the same
//! commit, strips its debug sections, and passes its path and SHA-256 in `UNA_EMBED_RUNTIME`
//! and `UNA_EMBED_RUNTIME_SHA256`. With both, this script reads the file, checks its digest
//! (a mismatch fails the build), copies the bytes into `OUT_DIR/libuna_rt.a`, which
//! `src/runtime_archive.rs` embeds, and sets the cfg `una_runtime_embedded` and the variable
//! `UNA_EMBED_RUNTIME_SHA256` for the crate. With one of the two, the build fails. With
//! neither, it embeds nothing: a development build, which reads the runtime from a file. A
//! built `una` never reads these variables. This script runs no program, downloads nothing and
//! compiles nothing (rule 18).

// A build script reads Cargo's and the build's variables and speaks to Cargo on standard
// output; the library crates' rules (compiler/conventions.md §1) do not apply to it.
#![allow(clippy::disallowed_methods, clippy::print_stdout)]

use std::env;
use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    // The target's operating system, not the host's: the build script runs on the host.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bin=una=-Wl,-dead_strip_dylibs");
    }
    embed_runtime()
}

/// Embeds the runtime that `UNA_EMBED_RUNTIME` names, with its digest checked against
/// `UNA_EMBED_RUNTIME_SHA256`, or nothing in a development build.
fn embed_runtime() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=UNA_EMBED_RUNTIME");
    println!("cargo:rerun-if-env-changed=UNA_EMBED_RUNTIME_SHA256");
    let path = env::var_os("UNA_EMBED_RUNTIME").filter(|value| !value.is_empty());
    let want = env::var("UNA_EMBED_RUNTIME_SHA256")
        .ok()
        .filter(|value| !value.is_empty());
    let (path, want) = match (path, want) {
        (None, None) => return Ok(()),
        (Some(_), None) => {
            return Err(
                "UNA_EMBED_RUNTIME needs UNA_EMBED_RUNTIME_SHA256, the SHA-256 of the stripped runtime that scripts/release.sh built".into(),
            );
        }
        (None, Some(_)) => {
            return Err(
                "UNA_EMBED_RUNTIME_SHA256 is set without UNA_EMBED_RUNTIME, the runtime to embed"
                    .into(),
            );
        }
        (Some(path), Some(want)) => (PathBuf::from(path), want),
    };
    if want.len() != 64
        || !want
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "UNA_EMBED_RUNTIME_SHA256 must be 64 lowercase hexadecimal digits, not {want:?}"
        ));
    }
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let found = format!("{:x}", Sha256::digest(&bytes));
    if found != want {
        return Err(format!(
            "SHA-256 mismatch for {}: UNA_EMBED_RUNTIME_SHA256 is {want}, the file is {found}",
            path.display()
        ));
    }
    if !bytes.starts_with(b"!<arch>\n") {
        return Err(format!(
            "{} is not a static library (an `ar` archive)",
            path.display()
        ));
    }
    let out = PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let copy = out.join("libuna_rt.a");
    fs::write(&copy, &bytes).map_err(|e| format!("{}: {e}", copy.display()))?;
    println!("cargo:rustc-cfg=una_runtime_embedded");
    println!("cargo:rustc-env=UNA_EMBED_RUNTIME_SHA256={want}");
    Ok(())
}
