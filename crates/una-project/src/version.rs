//! Versions of packages (ADR 0122 rules 20 to 22): semantic versioning 2.0.0 without build
//! metadata, each numeric part at most 2^31 − 1, ordered by semver precedence.

use std::cmp::Ordering;
use std::fmt;

/// The largest numeric part of a version (ADR 0122 rule 20).
pub const MAX_PART: u64 = (1 << 31) - 1;

/// A pre-release identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pre {
    /// A numeric identifier, which orders below every alphanumeric one.
    Num(u64),
    /// An alphanumeric identifier, ordered by ASCII.
    Alpha(String),
}

/// A full version, `MAJOR.MINOR.PATCH` with an optional pre-release.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    /// `MAJOR`.
    pub major: u64,
    /// `MINOR`.
    pub minor: u64,
    /// `PATCH`.
    pub patch: u64,
    /// The pre-release identifiers; empty for a release.
    pub pre: Vec<Pre>,
}

/// A version as a need writes it: `2.1`, `0.3.1`, `1.0.0-rc.1` (ADR 0122 rule 22).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// The least version it admits: the missing `PATCH` is `0`.
    pub version: Version,
    /// How many numeric parts were written: 2 or 3.
    pub parts: usize,
}

/// The major line of a version: `MAJOR` when it is at least 1, `0.MINOR` otherwise
/// (ADR 0122 rule 21). Two versions of one package are one identity only on one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Line {
    /// `MAJOR`, at least 1.
    Major(u64),
    /// `0.MINOR`.
    Zero(u64),
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Line::Major(major) => write!(f, "{major}"),
            Line::Zero(minor) => write!(f, "0.{minor}"),
        }
    }
}

impl Line {
    /// The line that a trust line's major writes: `2`, or `0.3` for the line `0.3`.
    pub fn parse(text: &str) -> Option<Line> {
        match text.split_once('.') {
            None => number(text).filter(|major| *major >= 1).map(Line::Major),
            Some(("0", minor)) => number(minor).map(Line::Zero),
            Some(_) => None,
        }
    }
}

impl Version {
    /// Reads `text`, two or three numeric parts and an optional pre-release. `Err` says why
    /// it is not a version.
    pub fn parse(text: &str) -> Result<Written, String> {
        if text.contains('+') {
            return Err("a version has no build metadata (`+...`)".to_owned());
        }
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if !(2..=3).contains(&parts.len()) {
            return Err("a version is `MAJOR.MINOR.PATCH` or `MAJOR.MINOR`".to_owned());
        }
        let mut numbers = [0u64; 3];
        for (slot, part) in numbers.iter_mut().zip(&parts) {
            *slot = number(part).ok_or_else(|| {
                format!("`{part}` is not a numeric part of a version (no leading zero, at most {MAX_PART})")
            })?;
        }
        let mut identifiers = Vec::new();
        if let Some(pre) = pre {
            if parts.len() != 3 {
                return Err(
                    "a pre-release follows a full version, `MAJOR.MINOR.PATCH-pre`".to_owned(),
                );
            }
            for identifier in pre.split('.') {
                if identifier.is_empty() || !identifier.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return Err(format!("`{identifier}` is not a pre-release identifier"));
                }
                if identifier.bytes().all(|b| b.is_ascii_digit()) {
                    identifiers.push(Pre::Num(number(identifier).ok_or_else(|| {
                        format!("`{identifier}` is not a numeric pre-release identifier (no leading zero, at most {MAX_PART})")
                    })?));
                } else {
                    identifiers.push(Pre::Alpha(identifier.to_owned()));
                }
            }
        }
        let [major, minor, patch] = numbers;
        Ok(Written {
            version: Version {
                major,
                minor,
                patch,
                pre: identifiers,
            },
            parts: parts.len(),
        })
    }

    /// Reads a full version, three numeric parts.
    pub fn parse_full(text: &str) -> Result<Version, String> {
        let written = Version::parse(text)?;
        if written.parts == 3 {
            Ok(written.version)
        } else {
            Err(format!(
                "`{text}` is not a full version; write `MAJOR.MINOR.PATCH`, such as `{text}.0`"
            ))
        }
    }

    /// Its major line (ADR 0122 rule 21).
    pub fn line(&self) -> Line {
        if self.major >= 1 {
            Line::Major(self.major)
        } else {
            Line::Zero(self.minor)
        }
    }

    /// Whether it is a pre-release.
    pub fn is_pre(&self) -> bool {
        !self.pre.is_empty()
    }
}

/// A numeric part: `0`, or digits without a leading zero, at most [`MAX_PART`].
fn number(text: &str) -> Option<u64> {
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if !digits || (text.len() > 1 && text.starts_with('0')) || text.len() > 10 {
        return None;
    }
    text.parse::<u64>().ok().filter(|value| *value <= MAX_PART)
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (index, identifier) in self.pre.iter().enumerate() {
            f.write_str(if index == 0 { "-" } else { "." })?;
            match identifier {
                Pre::Num(value) => write!(f, "{value}")?,
                Pre::Alpha(text) => f.write_str(text)?,
            }
        }
        Ok(())
    }
}

impl Ord for Pre {
    fn cmp(&self, other: &Pre) -> Ordering {
        match (self, other) {
            (Pre::Num(a), Pre::Num(b)) => a.cmp(b),
            (Pre::Num(_), Pre::Alpha(_)) => Ordering::Less,
            (Pre::Alpha(_), Pre::Num(_)) => Ordering::Greater,
            (Pre::Alpha(a), Pre::Alpha(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Pre {
    fn partial_cmp(&self, other: &Pre) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Semver precedence: the numeric parts, then a pre-release below the release, then the
/// pre-release identifiers in order, a shorter list below a longer one it is a prefix of.
impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).map(|w| w.version).unwrap_or_else(|e| {
            // INVARIANT: the test inputs are versions.
            panic!("{text}: {e}")
        })
    }

    #[test]
    fn precedence_follows_semver() {
        let ordered = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.2.0",
            "2.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn written_forms() {
        assert_eq!(Version::parse("2.1").map(|w| w.parts), Ok(2));
        assert_eq!(v("2.1"), v("2.1.0"));
        assert!(Version::parse("2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
        assert!(Version::parse("01.2.3").is_err());
        assert!(Version::parse("1.2.3+build").is_err());
        assert!(Version::parse("1.2-rc.1").is_err());
        assert!(Version::parse("1.2.3-rc.01").is_err());
        assert!(Version::parse("2147483648.0.0").is_err());
        assert!(Version::parse("2147483647.0.0").is_ok());
        assert!(Version::parse_full("1.4").is_err());
    }

    #[test]
    fn major_lines() {
        assert_eq!(v("2.1.0").line(), Line::Major(2));
        assert_eq!(v("0.3.1").line(), Line::Zero(3));
        assert_eq!(Line::parse("2"), Some(Line::Major(2)));
        assert_eq!(Line::parse("0.3"), Some(Line::Zero(3)));
        assert_eq!(Line::parse("0"), None);
        assert_eq!(Line::parse("1.2"), None);
        assert_eq!(Line::Zero(3).to_string(), "0.3");
    }
}
