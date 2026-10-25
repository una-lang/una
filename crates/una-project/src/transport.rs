//! How the registry client reads a registry's files (ADR 0122 rules 34, 39 and 40): a
//! [`Transport`] gives the bytes at a path below a registry's base, and nothing else. Integrity
//! never rests on the transport: every byte it gives is verified by digests, signatures and
//! proofs ([`crate::client`]), whatever the transport is, a local directory included.
//!
//! **Locations.** A registry or a mirror is named by a location ([`Location::parse`]):
//! - an `https://` URL: the network location of a source. This release has no HTTPS transport,
//!   so reading from one fails with a refusal that says so, never a silent skip;
//! - a `file://` URL with an absolute path, or a directory path: a mirror on the local file
//!   system, read by [`Directory`];
//! - any other scheme, `http://` included, is refused when it is parsed: plain HTTP is never
//!   used, and neither is any scheme this client does not know.
//!
//! No transport starts another program.
//!
//! Costs: a read is O(b) for the b bytes of the file, bounded by the limit its caller gives.

use std::path::{Path, PathBuf};

use crate::tree;

/// Why a read fails: one sentence.
pub type Refusal = String;

/// Reads the files of one registry, by their paths below its base (rule 34).
pub trait Transport {
    /// The bytes of the file at `path`, of at most `limit` bytes, or `None` when the registry
    /// has no file there. `path` is relative, of `/`-separated segments, none empty, `.` or
    /// `..`. A file larger than `limit` is a refusal, never a truncation.
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, Refusal>;

    /// Where the transport reads from, for messages.
    fn describe(&self) -> String;
}

/// Where a registry or a mirror is (rule 40).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// An `https://` base URL, without a trailing `/`.
    Https(String),
    /// A directory on the local file system.
    Directory(PathBuf),
}

impl Location {
    /// Reads a location: an `https://` URL; a `file://` URL whose path is absolute; or, with no
    /// scheme, a directory path. A text with any other scheme (`http:`, `ftp:`, ...), and a URL
    /// with a host, a query, a fragment or credentials where a file URL has none, is refused,
    /// so plain HTTP is never used.
    pub fn parse(text: &str) -> Result<Location, Refusal> {
        if text.is_empty() || text.chars().any(char::is_control) {
            return Err("a registry location is not empty and has no control character".to_owned());
        }
        if let Some(rest) = text.strip_prefix("https://") {
            let host_ok = !rest.is_empty()
                && !rest.starts_with('/')
                && rest.chars().all(|c| c.is_ascii_graphic())
                && !rest.contains(['?', '#', '@']);
            if !host_ok {
                return Err(format!("`{text}` is not an https URL of a registry"));
            }
            return Ok(Location::Https(text.trim_end_matches('/').to_owned()));
        }
        if let Some(rest) = text.strip_prefix("file://") {
            // `file:///abs/path`: an empty host, then an absolute path.
            if !rest.starts_with('/') || rest.contains(['?', '#']) {
                return Err(format!(
                    "`{text}` is not a file URL of a local directory: write `file:///absolute/path`"
                ));
            }
            return Ok(Location::Directory(PathBuf::from(rest)));
        }
        if let Some((scheme, _)) = text.split_once("://") {
            return Err(format!(
                "`{scheme}://` is not a scheme a registry is read with; a location is an `https://` URL, a `file://` URL or a directory, and plain `http://` is never used"
            ));
        }
        // A scheme without `//`, as `http:example`, is refused as well: a directory path with a
        // colon before its first `/` would be ambiguous.
        if let Some((head, _)) = text.split_once(':')
            && !head.contains('/')
            && !head.contains('\\')
            && head.len() > 1
        {
            return Err(format!(
                "`{text}` names a scheme; a location is an `https://` URL, a `file://` URL or a directory"
            ));
        }
        Ok(Location::Directory(PathBuf::from(text)))
    }
}

/// A transport over a directory on the local file system: a mirror, or a copy of a registry's
/// files (rule 40). Each file is found with no symbolic link followed below the directory, and
/// read once through a descriptor opened with no link followed. What it gives is verified as
/// what any transport gives: a local directory is trusted for nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    /// The directory.
    pub root: PathBuf,
}

impl Directory {
    /// The transport over `root`.
    pub fn new(root: &Path) -> Directory {
        Directory {
            root: root.to_path_buf(),
        }
    }
}

/// The segments of a registry path, each checked: not empty, not `.` or `..`, of ASCII
/// letters, digits and `.`, `_`, `-`; at most 16 segments.
pub fn segments(path: &str) -> Result<Vec<&str>, Refusal> {
    let parts: Vec<&str> = path.split('/').collect();
    let valid = !parts.is_empty()
        && parts.len() <= 16
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part.len() <= 255
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        });
    if valid {
        Ok(parts)
    } else {
        Err(format!("`{path}` is not a path of a registry"))
    }
}

impl Transport for Directory {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
        let parts = segments(path)?;
        tree::not_a_link(&self.root)?;
        match tree::locate(&self.root, &parts)? {
            Some(found) => tree::read_file(&found, limit).map(Some),
            None => Ok(None),
        }
    }

    fn describe(&self) -> String {
        format!("the mirror `{}`", self.root.display())
    }
}

/// The transport of an `https://` location. This release has no HTTPS transport: every read
/// is refused with a message that says so and names the way to read the registry's files from
/// a mirror directory. It is never skipped silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    /// The URL.
    pub url: String,
}

impl Transport for Unavailable {
    fn get(&self, path: &str, _limit: u64) -> Result<Option<Vec<u8>>, Refusal> {
        segments(path)?;
        Err(format!(
            "`{}` is reached over https, and this release has no https transport; read the registry from a mirror directory with `--mirror <source>=<directory>`",
            self.url
        ))
    }

    fn describe(&self) -> String {
        format!("`{}`", self.url)
    }
}

/// The transport of a location.
pub fn open(location: &Location) -> Box<dyn Transport> {
    match location {
        Location::Https(url) => Box::new(Unavailable { url: url.clone() }),
        Location::Directory(dir) => Box::new(Directory::new(dir)),
    }
}
