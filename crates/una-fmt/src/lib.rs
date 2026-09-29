//! The canonical formatter (ADR 0011, ADR 0020).
//!
//! [`format`] prints a file that lexed and parsed without errors in its canonical form, and
//! [`differences`] reports U7001 wherever a file differs from it. A file with syntax errors has
//! no canonical form (DECISION D-027).
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod diff;
mod layout;
mod lines;
mod rewrite;
mod spacing;

pub use diff::differences;
pub use layout::format;
