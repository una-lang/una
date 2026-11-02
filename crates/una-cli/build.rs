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
//!
//! **The musl inputs** (ADR 0124 rules 12 and 20, step 4a, `docs/plan/0124-step4-design.md`
//! §2). For a Linux target, `scripts/release.sh` also passes the pinned toolchain's
//! `self-contained` directory of the musl target of the same architecture in
//! `UNA_EMBED_MUSL_DIR`, and in `UNA_EMBED_MUSL_SHA256S` the digest of each of its inputs, as
//! `file=sha256` separated by spaces, in the order of [`MUSL_INPUTS`]. This script checks each
//! file against its digest, checks that the archives carry their symbol table (the index by
//! which the linker finds a member, rule 12) and that the objects are ELF, copies them into
//! `OUT_DIR/musl/`, which `src/musl_inputs.rs` embeds, and sets the cfg `una_musl_embedded` and
//! one variable per digest. A release build for Linux (one with the runtime embedded) needs
//! them; any other build is refused them, so they are carried for the target of the `una`
//! binary only, never another one.

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
    embed_runtime()?;
    embed_musl()
}

/// The musl inputs, in their fixed order, with the variable that carries each one's digest into
/// the crate.
const MUSL_INPUTS: [(&str, &str); 7] = [
    ("libc.a", "UNA_EMBED_MUSL_SHA256_LIBC"),
    ("rcrt1.o", "UNA_EMBED_MUSL_SHA256_RCRT1"),
    ("crti.o", "UNA_EMBED_MUSL_SHA256_CRTI"),
    ("crtn.o", "UNA_EMBED_MUSL_SHA256_CRTN"),
    ("crtbeginS.o", "UNA_EMBED_MUSL_SHA256_CRTBEGINS"),
    ("crtendS.o", "UNA_EMBED_MUSL_SHA256_CRTENDS"),
    ("libunwind.a", "UNA_EMBED_MUSL_SHA256_LIBUNWIND"),
];

/// Embeds the musl inputs of `UNA_EMBED_MUSL_DIR`, each checked against its digest in
/// `UNA_EMBED_MUSL_SHA256S`, into a release build for Linux; nothing into any other build.
fn embed_musl() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=UNA_EMBED_MUSL_DIR");
    println!("cargo:rerun-if-env-changed=UNA_EMBED_MUSL_SHA256S");
    let dir = env::var_os("UNA_EMBED_MUSL_DIR").filter(|value| !value.is_empty());
    let list = env::var("UNA_EMBED_MUSL_SHA256S")
        .ok()
        .filter(|value| !value.is_empty());
    let linux = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    let release = env::var_os("UNA_EMBED_RUNTIME").is_some_and(|value| !value.is_empty());
    let (dir, list) = match (dir, list) {
        (None, None) if linux && release => {
            return Err(
                "a release build for Linux embeds the musl inputs (ADR 0124 rule 12): set UNA_EMBED_MUSL_DIR and UNA_EMBED_MUSL_SHA256S, as scripts/release.sh does".into(),
            );
        }
        (None, None) => return Ok(()),
        (Some(_), None) => {
            return Err("UNA_EMBED_MUSL_DIR needs UNA_EMBED_MUSL_SHA256S, the digests of its inputs that scripts/release.sh computed".into());
        }
        (None, Some(_)) => {
            return Err("UNA_EMBED_MUSL_SHA256S is set without UNA_EMBED_MUSL_DIR, the directory of the inputs to embed".into());
        }
        (Some(dir), Some(list)) => (PathBuf::from(dir), list),
    };
    if !linux || !release {
        return Err("the musl inputs are embedded only into a release build for Linux, with the runtime (ADR 0124 rule 12)".into());
    }
    let entries: Vec<&str> = list.split(' ').collect();
    if entries.len() != MUSL_INPUTS.len() {
        return Err(format!(
            "UNA_EMBED_MUSL_SHA256S must list {} inputs, `file=sha256` in this order: {}",
            MUSL_INPUTS.len(),
            MUSL_INPUTS.map(|(file, _)| file).join(" ")
        ));
    }
    let out = PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?).join("musl");
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    for (entry, (file, variable)) in entries.iter().zip(MUSL_INPUTS) {
        let want = entry
            .strip_prefix(file)
            .and_then(|rest| rest.strip_prefix('='))
            .ok_or_else(|| {
                format!("UNA_EMBED_MUSL_SHA256S lists {entry:?} where `{file}=<sha256>` belongs")
            })?;
        if !hex_digest(want) {
            return Err(format!(
                "the digest of {file} in UNA_EMBED_MUSL_SHA256S must be 64 lowercase hexadecimal digits, not {want:?}"
            ));
        }
        let path = dir.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let found = format!("{:x}", Sha256::digest(&bytes));
        if found != want {
            return Err(format!(
                "SHA-256 mismatch for {}: UNA_EMBED_MUSL_SHA256S says {want}, the file is {found}",
                path.display()
            ));
        }
        if file.ends_with(".a") {
            // A GNU archive whose first member is its symbol table, named `/`.
            if !bytes.starts_with(b"!<arch>\n/ ") {
                return Err(format!(
                    "{} is not an `ar` archive with a symbol table",
                    path.display()
                ));
            }
        } else if !bytes.starts_with(b"\x7fELF\x02\x01") {
            return Err(format!(
                "{} is not a 64-bit little-endian ELF object",
                path.display()
            ));
        }
        let copy = out.join(file);
        fs::write(&copy, &bytes).map_err(|e| format!("{}: {e}", copy.display()))?;
        println!("cargo:rustc-env={variable}={want}");
    }
    println!("cargo:rustc-cfg=una_musl_embedded");
    Ok(())
}

/// Whether `text` is a SHA-256 digest in lowercase hexadecimal.
fn hex_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
    if !hex_digest(&want) {
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
