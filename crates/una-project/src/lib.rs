//! Project files (`una.toml` and `verification.lock` to edition `0.3`; `ita.una`, `ita.lock`
//! and `itaty.una` from edition `0.4`) and package discovery (`spec/13 §7`, `spec/12 §9`,
//! `spec/30`, ADR 0021, ADR 0122).
//!
//! Errors are located by byte offsets into the file's text; the caller turns them into
//! diagnostics. The grammar of the files of edition `0.4` is the syntax crate's.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

// The test registry of `tests/support/registry.rs` names this crate `una_project`; the owner
// model's unit tests render chains with it.
#[cfg(test)]
extern crate self as una_project;

pub mod authority;
pub mod client;
pub mod content;
mod document;
pub mod ita;
pub mod itaoca;
pub mod ledger;
pub mod literal;
mod lock;
mod manifest;
pub mod migrate;
pub mod mirrors;
pub mod names;
mod package;
pub mod provenance;
pub mod registry;
pub mod resolve;
pub mod tlog;
pub mod transport;
pub mod tree;
pub mod vendor;
pub mod version;

use std::ops::Range;

use una_diag::Code;

pub use lock::{DEFAULT_SOLVER_VERSION, Lock, parse_lock};
pub use manifest::{
    Boundary, Ceiling, Contract, Drive, Edition, Extents, Format, MAX_DRIVE_MILLISECONDS,
    MAX_MEMORY_CAP, MAX_SERVE_MILLISECONDS, Manifest, Mount, Profile, Serve, is_grant,
    is_module_path, parse_manifest,
};
pub use package::{
    LoadError, Loaded, LoadedFile, Role, discover, discover_dependency, project_file_bound,
    read_project_file,
};

/// A problem in a project file, located by byte offsets into its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectError {
    /// The registered code.
    pub code: Code,
    /// Where it is, in bytes of the file's text.
    pub range: Range<usize>,
    /// One sentence, lowercase start, no trailing period.
    pub message: String,
}

impl ProjectError {
    fn new(code: Code, range: Range<usize>, message: impl Into<String>) -> ProjectError {
        ProjectError {
            code,
            range,
            message: message.into(),
        }
    }
}

/// A string value of a project file and where it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    /// The value.
    pub value: String,
    /// Where it is written, in bytes of the file's text.
    pub range: Range<usize>,
}
