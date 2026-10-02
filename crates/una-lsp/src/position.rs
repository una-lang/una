//! Positions: Una counts 1-based lines and columns in Unicode scalar values (`spec/13 §1`);
//! LSP counts 0-based lines and characters in the encoding the client and the server agreed
//! on, UTF-32 or UTF-16 (ADR 0063 rule 7).

use serde_json::{Value, json};

/// The unit of an LSP character offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Unicode scalar values, as Una counts columns.
    Utf32,
    /// UTF-16 code units, LSP's default.
    Utf16,
}

impl Encoding {
    /// Its name in LSP's `positionEncoding`.
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf32 => "utf-32",
            Encoding::Utf16 => "utf-16",
        }
    }
}

/// The member of `value` at `path`, or `null` when there is none.
pub(crate) fn field<'v>(value: &'v Value, path: &[&str]) -> &'v Value {
    path.iter()
        .fold(value, |value, key| value.get(key).unwrap_or(&Value::Null))
}

/// The text of line `line` (0-based) of `text`, without its line break.
fn line_of(text: &str, line: usize) -> &str {
    text.split('\n')
        .nth(line)
        .unwrap_or("")
        .trim_end_matches('\r')
}

/// The LSP position of Una's 1-based `line` and `col` in `text`.
pub fn to_lsp(text: &str, line: u32, col: u32, encoding: Encoding) -> Value {
    let line = line.saturating_sub(1) as usize;
    let col = col.saturating_sub(1) as usize;
    let character = match encoding {
        Encoding::Utf32 => col,
        Encoding::Utf16 => line_of(text, line)
            .chars()
            .take(col)
            .map(char::len_utf16)
            .sum(),
    };
    json!({ "line": line, "character": character })
}

/// Una's 1-based line and column of an LSP position in `text`.
pub fn from_lsp(text: &str, position: &Value, encoding: Encoding) -> (u32, u32) {
    let line = field(position, &["line"]).as_u64().unwrap_or(0) as usize;
    let character = field(position, &["character"]).as_u64().unwrap_or(0) as usize;
    let col = match encoding {
        Encoding::Utf32 => character,
        Encoding::Utf16 => {
            let mut units = 0;
            let mut col = 0;
            for c in line_of(text, line).chars() {
                if units >= character {
                    break;
                }
                units += c.len_utf16();
                col += 1;
            }
            col
        }
    };
    (
        u32::try_from(line + 1).unwrap_or(u32::MAX),
        u32::try_from(col + 1).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_counts_code_units_and_utf32_scalar_values() {
        let text = "a\n\u{1F600}x = 1\n";
        assert_eq!(
            to_lsp(text, 2, 2, Encoding::Utf16),
            json!({ "line": 1, "character": 2 })
        );
        assert_eq!(
            to_lsp(text, 2, 2, Encoding::Utf32),
            json!({ "line": 1, "character": 1 })
        );
        assert_eq!(
            from_lsp(text, &json!({ "line": 1, "character": 2 }), Encoding::Utf16),
            (2, 2)
        );
    }
}
