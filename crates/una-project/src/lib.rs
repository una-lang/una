//! Project files (`una.toml`, `verification.lock`) and package discovery (`spec/13 §7`,
//! `spec/12 §9`, ADR 0021).
//!
//! Errors are located by byte offsets into the file's text, so this crate does not depend on
//! the syntax crate; the caller turns them into diagnostics.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod document;
mod lock;
mod manifest;
mod package;

use std::ops::Range;

use una_diag::Code;

pub use lock::{DEFAULT_SOLVER_VERSION, Lock, parse_lock};
pub use manifest::{Edition, Manifest, is_grant, is_module_path, parse_manifest};
pub use package::{LoadError, Loaded, LoadedFile, Role, discover};

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
