//! Links the Z3 front end that `scripts/build-z3.sh` builds into `una-z3`, and records the
//! solver's build (ADR 0124 rules 1, 18, 19 and 20). It reads three variables at build time,
//! which a built `una` never reads:
//!
//! - `UNA_EMBED_Z3` with `UNA_EMBED_Z3_SHA256`: a release build. The library's SHA-256 must be
//!   the one given, which `scripts/release.sh` takes from `supply-chain/z3.lock`; otherwise the
//!   build fails;
//! - `UNA_Z3_LIB`: a development build, with the library at that path, usually
//!   `~/.cache/una/z3/<key>/lib/libz3shell.a` (`scripts/z3-library.sh`); its digest is recorded,
//!   not checked;
//! - neither: a development build with no solver built in, in which `una check` is U5003; the
//!   build warns, naming `scripts/z3-library.sh --export`.
//!
//! The pin of `supply-chain/z3.lock` (version, commit, tree, source digest) goes into the crate
//! as environment constants. This script runs no program, downloads nothing and compiles
//! nothing: the library is built once per pin by `scripts/build-z3.sh`.
//!
//! The library is linked from a verified copy (#1925, `stage.rs`): it is copied into a directory
//! of `OUT_DIR` that holds the copy alone, the copy is hashed (and, in a release build, checked),
//! and the link names only that directory. The linker reads the checked bytes, never the given
//! file again, and searches no directory beside it, so no other library there is linked.

// A build script reads Cargo's and the build's variables; the library crates' rule against
// reading the environment (compiler/conventions.md §1) does not apply to it.
#![allow(clippy::disallowed_methods)]

use std::env;
use std::fs;
use std::path::PathBuf;

// The verified copy and its link directives, shared with tests/stage.rs; the test uses an
// accessor that the build does not.
#[allow(dead_code)]
mod stage;

/// A quoted string value of the lock, by key, up to the first table header.
fn setting(lock: &str, key: &str) -> Result<String, String> {
    lock.lines()
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
        })
        .ok_or_else(|| format!("supply-chain/z3.lock has no {key}"))
}

fn main() -> Result<(), String> {
    for variable in ["UNA_EMBED_Z3", "UNA_EMBED_Z3_SHA256", "UNA_Z3_LIB"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let lock_path = manifest.join("../../supply-chain/z3.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock =
        fs::read_to_string(&lock_path).map_err(|e| format!("{}: {e}", lock_path.display()))?;
    for (key, name) in [
        ("version", "UNA_Z3_PIN_VERSION"),
        ("commit", "UNA_Z3_PIN_COMMIT"),
        ("tree", "UNA_Z3_PIN_TREE"),
        ("source_sha256", "UNA_Z3_PIN_SOURCE_SHA256"),
    ] {
        println!("cargo:rustc-env={name}={}", setting(&lock, key)?);
    }

    let embed = env::var_os("UNA_EMBED_Z3").map(PathBuf::from);
    let dev = env::var_os("UNA_Z3_LIB").map(PathBuf::from);
    let (library, release) = match (embed, dev) {
        (Some(_), Some(_)) => {
            return Err(
                "set one of UNA_EMBED_Z3 (release) and UNA_Z3_LIB (development), not both".into(),
            );
        }
        (Some(path), None) => {
            let want = env::var("UNA_EMBED_Z3_SHA256")
                .map_err(|_| "UNA_EMBED_Z3 needs UNA_EMBED_Z3_SHA256, the library's digest from supply-chain/z3.lock".to_owned())?;
            (Some((path, Some(want))), true)
        }
        (None, Some(path)) => (Some((path, None)), false),
        (None, None) => {
            // Not an error: a development build without the solver is a supported build
            // (rule 19). The warning says how to get one with it, since its tier-1 tests fail.
            println!(
                "cargo:warning=UNA_Z3_LIB is not set: this una has no solver built in and its tier-1 tests fail with U5003; run `eval \"$(scripts/z3-library.sh --export)\"` first (docs/compiler/testing.md §3)"
            );
            (None, false)
        }
    };
    let mut library_sha256 = String::new();
    if let Some((path, want)) = library {
        println!("cargo:rerun-if-changed={}", path.display());
        let out = PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?);
        let staged = stage::stage(&path, &out.join("z3-link"), want.as_deref())?;
        let target_os = env::var("CARGO_CFG_TARGET_OS").ok();
        for directive in stage::directives(&staged, target_os.as_deref())? {
            println!("{directive}");
        }
        library_sha256 = staged.sha256;
        println!("cargo:rustc-cfg=una_z3_linked");
        if release {
            println!("cargo:rustc-cfg=una_z3_release");
        }
    }
    println!("cargo:rustc-env=UNA_Z3_LIBRARY_SHA256={library_sha256}");
    Ok(())
}
