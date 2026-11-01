//! The linker's diagnostics (ADR 0124, Diagnostics): U8001 for a link that fails on Una's own
//! inputs, U7015 for an executable that cannot be laid out.

use std::fmt;

/// The registered code of a link failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// A link that fails on Una's own inputs: an unsupported or position-dependent relocation, a
    /// relocation or symbol outside its section, a duplicate or undefined symbol, a symbol
    /// outside libSystem's list, a refusal of the verifier (ADR 0124 rules 23, 24 and 34).
    U8001,
    /// The executable cannot be laid out: a branch beyond its target's range, or a section larger
    /// than its format allows.
    U7015,
}

impl Code {
    /// The code as `spec/13 §4` writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Code::U8001 => "U8001",
            Code::U7015 => "U7015",
        }
    }
}

/// Why a link failed: its code and a one-sentence message that names the input, the section,
/// the symbol and the relocation type where they apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The registered code.
    pub code: Code,
    /// The message, lowercase, without a trailing period.
    pub message: String,
}

impl Problem {
    /// A U8001 problem.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Problem {
        Problem {
            code: Code::U8001,
            message: message.into(),
        }
    }

    /// A U7015 problem.
    #[must_use]
    pub fn layout(message: impl Into<String>) -> Problem {
        Problem {
            code: Code::U7015,
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code.as_str())
    }
}

/// The result type of the linker.
pub type Result<T> = std::result::Result<T, Problem>;

/// A symbol's name for a message: its bytes as text, lossily.
pub(crate) fn shown(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}
