//! The call into the Z3 front end built into `una` (ADR 0124 rule 2): the declaration of its
//! entry, `una_z3_shell_main`, which is the shell's `main` renamed by
//! `supply-chain/z3/shell-library.patch`; the construction of its arguments; and the call.
//! Nothing else: this is the third place of the repository that may hold `unsafe` code (ADR
//! 0058, amendment of 2026-10-27), at most 150 lines, depending on no crate but `una-sys`.
//!
//! The entry runs only in a process that recorded a successful confinement
//! ([`una_sys::solver::confined`]); otherwise [`run`] aborts, so no path of `una` reaches Z3's
//! engine unconfined. Z3's static initializers run in every `una` process; they read no input.
#![allow(unsafe_code)]

use std::ffi::{CString, c_char, c_int};
use std::fmt;

/// The solver's build: the pin of `supply-chain/z3.lock`, and what this `una` linked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Build {
    /// Z3's version, as `verification.lock` pins it.
    pub version: &'static str,
    /// The commit of Z3's release tag.
    pub commit: &'static str,
    /// Its git tree hash.
    pub tree: &'static str,
    /// The SHA-256 of the source tarball.
    pub source_sha256: &'static str,
    /// The SHA-256 of the library linked in; empty when none is.
    pub library_sha256: &'static str,
    /// Whether a front end is linked in at all.
    pub linked: bool,
    /// Whether it is the release library, taken by digest (`UNA_EMBED_Z3`); a development
    /// build otherwise (ADR 0124 rule 19).
    pub release: bool,
}

/// This `una`'s solver build, fixed at build time by `build.rs`.
pub const BUILD: Build = Build {
    version: env!("UNA_Z3_PIN_VERSION"),
    commit: env!("UNA_Z3_PIN_COMMIT"),
    tree: env!("UNA_Z3_PIN_TREE"),
    source_sha256: env!("UNA_Z3_PIN_SOURCE_SHA256"),
    library_sha256: env!("UNA_Z3_LIBRARY_SHA256"),
    linked: cfg!(una_z3_linked),
    release: cfg!(una_z3_release),
};

/// Why the front end did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// This development build has no solver built in (U5003, ADR 0124 rule 19).
    NoSolver,
    /// An argument holds a NUL byte, which a C string cannot.
    Nul,
    /// More arguments than a C `argc` can count.
    TooMany,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSolver => {
                "this development build has no solver built in; run scripts/build-z3.sh"
            }
            Self::Nul => "a solver argument holds a NUL byte",
            Self::TooMany => "too many solver arguments",
        })
    }
}

impl std::error::Error for Refusal {}

#[cfg(una_z3_linked)]
unsafe extern "C" {
    /// Z3's shell front end: `main` renamed. `extern "C"`, not `"C-unwind"`, so a C++ exception
    /// that escapes aborts the process, which the parent sees as `crash` (rule 8).
    fn una_z3_shell_main(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

/// Runs the front end on `args`, in this process, as `z3 <args>` would: it reads stdin, writes
/// stdout and stderr, and may end the process itself. Returns the status it would exit with.
/// Aborts unless this process recorded a successful confinement (rule 2).
pub fn run(args: &[&str]) -> Result<i32, Refusal> {
    if !BUILD.linked {
        return Err(Refusal::NoSolver);
    }
    if !una_sys::solver::confined() {
        std::process::abort();
    }
    call(args)
}

#[cfg(una_z3_linked)]
fn call(args: &[&str]) -> Result<i32, Refusal> {
    let argc = c_int::try_from(args.len() + 1).map_err(|_| Refusal::TooMany)?;
    let owned: Vec<CString> = std::iter::once("z3")
        .chain(args.iter().copied())
        .map(|arg| CString::new(arg).map_err(|_| Refusal::Nul))
        .collect::<Result<_, _>>()?;
    let mut argv: Vec<*mut c_char> = owned.into_iter().map(CString::into_raw).collect();
    argv.push(std::ptr::null_mut());
    // SAFETY: `argv` holds `argc` pointers to NUL-terminated strings this function owns, then a
    // null, as C's `main` receives them; the front end reads them during the call and keeps
    // none. The strings are reclaimed below; a front end that ends the process never returns.
    let status = unsafe { una_z3_shell_main(argc, argv.as_mut_ptr()) };
    for pointer in argv.into_iter().take(args.len() + 1) {
        // SAFETY: each pointer came from `CString::into_raw` above and is reclaimed once.
        drop(unsafe { CString::from_raw(pointer) });
    }
    Ok(status)
}

#[cfg(not(una_z3_linked))]
fn call(_: &[&str]) -> Result<i32, Refusal> {
    Err(Refusal::NoSolver)
}
