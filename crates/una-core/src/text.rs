//! The representation of a text in both engines (`spec/22 §6` rule 1).

use std::cmp::Ordering;

/// How many scalar values apart the entries of a text's index are (`spec/22 §6` rule 1).
const STRIDE: usize = 64;

/// A text (`spec/22 §6` rule 1): its UTF-8 bytes, with its count of scalar values and whether
/// every byte is ASCII, computed when it is made and updated when it grows, and the byte offset
/// of every 64th scalar value, built by the first positional operation on a text that is not
/// ASCII and extended when it grows. None of these is observable; they bound the cost: `.len`
/// is O(1), a position is O(1) once the index is built, in O(b) at most once, and an append
/// costs O(appended part).
#[derive(Clone, Debug)]
pub struct Text {
    text: String,
    scalars: usize,
    ascii: bool,
    index: std::cell::OnceCell<Vec<usize>>,
}

impl Text {
    /// The text of `text`, in O(b).
    pub fn new(text: String) -> Text {
        Text {
            scalars: text.chars().count(),
            ascii: text.is_ascii(),
            text,
            index: std::cell::OnceCell::new(),
        }
    }

    /// The text as a `str`.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The number of scalar values, `.len`: O(1).
    pub fn scalars(&self) -> usize {
        self.scalars
    }

    /// Appends `part` in place (#261), in O(length of `part`): the count and the flag are
    /// updated, and a built index extended, not recomputed (`spec/22 §6` rule 2).
    pub fn push_str(&mut self, part: &str) {
        if let Some(index) = self.index.get_mut() {
            let base = self.text.len();
            for (at, (offset, _)) in part.char_indices().enumerate() {
                if (self.scalars + at).is_multiple_of(STRIDE) {
                    index.push(base + offset);
                }
            }
        }
        self.scalars += part.chars().count();
        self.ascii &= part.is_ascii();
        self.text.push_str(part);
    }

    /// The byte offset of the scalar value at `position`, at most the count, which is the
    /// offset of the end: O(1) on an ASCII text, else O(1) after the index is built.
    pub fn offset(&self, position: usize) -> usize {
        if position >= self.scalars {
            return self.text.len();
        }
        if self.ascii {
            return position;
        }
        let index = self.index.get_or_init(|| {
            self.text
                .char_indices()
                .step_by(STRIDE)
                .map(|(offset, _)| offset)
                .collect()
        });
        let start = index.get(position / STRIDE).copied().unwrap_or(0);
        self.text
            .get(start..)
            .and_then(|rest| rest.char_indices().nth(position % STRIDE))
            .map_or(self.text.len(), |(offset, _)| start + offset)
    }

    /// The scalar values from `start` up to `end`: O(result) after the index.
    pub fn slice(&self, start: usize, end: usize) -> &str {
        let (from, to) = (self.offset(start), self.offset(end.max(start)));
        self.text.get(from..to).unwrap_or("")
    }

    /// The number of scalar values before the byte offset `byte`: O(1) on an ASCII text.
    pub fn scalars_before(&self, byte: usize) -> usize {
        if self.ascii {
            return byte;
        }
        self.text
            .get(..byte)
            .map_or(0, |before| before.chars().count())
    }
}

impl std::ops::Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        self.text == other.text
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    fn partial_cmp(&self, other: &Text) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// UTF-8 byte order is the order of scalar values (`spec/03 §1`).
impl Ord for Text {
    fn cmp(&self, other: &Text) -> Ordering {
        self.text.cmp(&other.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_agree_with_the_scalar_values_however_the_text_was_built() {
        // An index built and then extended by appends gives the offsets of one built at once,
        // and both those of a plain scan, across ASCII and multi-byte scalar values.
        let parts = [
            "ab",
            "é",
            "日本",
            "",
            "x",
            "😀😀",
            "plain ascii text that is long",
        ];
        let mut grown = Text::new(String::new());
        let _ = grown.offset(0);
        for round in 0..40 {
            let part = parts[round % parts.len()];
            grown.push_str(part);
            if round % 7 == 0 {
                // Build the index midway, so later appends extend it.
                let _ = grown.offset(grown.scalars() / 2);
            }
        }
        let fresh = Text::new(grown.as_str().to_owned());
        let expected: Vec<usize> = grown
            .as_str()
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([grown.as_str().len()])
            .collect();
        assert_eq!(grown.scalars(), fresh.scalars());
        assert_eq!(grown.scalars(), expected.len() - 1);
        for (position, offset) in expected.iter().enumerate() {
            assert_eq!(grown.offset(position), *offset, "grown at {position}");
            assert_eq!(fresh.offset(position), *offset, "fresh at {position}");
        }
        assert_eq!(grown.slice(1, 4), fresh.slice(1, 4));
        assert_eq!(grown.scalars_before(grown.offset(9)), 9);
    }
}
