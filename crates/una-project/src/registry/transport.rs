//! Transports: how the client reads a registry's static files and sends a submission
//! (ADR 0122 rules 34, 40 and 43).
//!
//! A registry is a set of files at fixed paths below a base (rule 34). A [`Transport`] reads the
//! document at a path, and writes one at a path; it is trusted for nothing, since every byte it
//! returns is verified by digests, signatures and proofs before it is used. This release has
//! one transport, [`Directory`], a registry or mirror laid out in a local directory, such as a
//! shared file system inside an organization. A URL is read by [`location`], which accepts
//! `https://` and `file:///` only: plain `http://` and every other scheme are refused when the
//! URL is read, never at use. No transport here opens a connection or starts a program; an
//! `https://` base has no transport in this release.
//!
//! Costs: a read or a write is O(bytes), through a descriptor opened with no link followed
//! ([`crate::tree`]).

use std::path::{Path, PathBuf};

use crate::content;
use crate::tree;

/// The most bytes of one document a transport reads: a package's archive at its largest, with
/// its listing (rule 27), and room for the listing's lines.
pub const MAX_DOCUMENT: u64 = content::MAX_BYTES + (64 << 20);

/// Where a registry or a mirror is, as a URL names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// `https://<host>/<path>`: served over HTTPS, which this release has no transport for.
    Https(String),
    /// `file:///<absolute path>`: a directory of the local file system.
    Directory(PathBuf),
}

/// Reads a URL of a registry or a mirror: `https://` followed by a host and an optional path,
/// or `file://` followed by an absolute path. Plain `http://` is refused, so a registry's
/// documents and a private source's credentials never travel in clear, and so is every other
/// scheme and a URL with none. The text has no space, no control character and no `\`.
pub fn location(url: &str) -> Result<Location, String> {
    if url.is_empty() || url.len() > 2048 || !url.chars().all(|c| c.is_ascii_graphic()) {
        return Err(format!(
            "`{url}` is not a URL: it is empty, too long, or has a space or a character outside ASCII"
        ));
    }
    if url.contains('\\') {
        return Err(format!("`{url}` has a `\\`; a URL separates its parts with `/`"));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(format!(
            "`{url}` has no scheme; write an `https://` or a `file:///` URL"
        ));
    };
    match scheme.to_ascii_lowercase().as_str() {
        "https" if scheme == "https" => {
            if rest.is_empty() || rest.starts_with('/') {
                return Err(format!("`{url}` names no host"));
            }
            Ok(Location::Https(url.to_owned()))
        }
        "file" if scheme == "file" => {
            let Some(path) = rest.strip_prefix('/') else {
                return Err(format!(
                    "`{url}` names a host; a `file` URL is `file:///` and an absolute path"
                ));
            };
            let valid = !path.is_empty()
                && path
                    .split('/')
                    .all(|segment| segment != "." && segment != "..")
                && !path.contains("//");
            if !valid {
                return Err(format!(
                    "`{url}` is not an absolute path without empty, `.` or `..` segments"
                ));
            }
            Ok(Location::Directory(PathBuf::from(format!("/{path}"))))
        }
        "http" => Err(format!(
            "`{url}` is plain HTTP, which is never used; write an `https://` URL"
        )),
        _ => Err(format!(
            "`{url}` has the scheme `{scheme}`; a registry or mirror is an `https://` or a `file:///` URL"
        )),
    }
}

/// How a transport fails: the document is not there, or the transport could not serve it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No document at the path.
    Absent,
    /// The transport refused or failed, with the reason: a refusal of service.
    Unavailable(String),
}

/// Reads and writes the documents of one registry or mirror, by path below its base. A path is
/// relative, of segments that rule 27 admits ([`content::check_path`]).
pub trait Transport {
    /// The URL it was made from, which messages name.
    fn name(&self) -> &str;

    /// The document at `path`, read once, of at most `limit` bytes.
    fn get(&self, path: &str, limit: u64) -> Result<Vec<u8>, Failure>;

    /// Writes `bytes` as the document at `path`, once: when a document is already there, it
    /// must be the same bytes.
    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), Failure>;
}

/// A registry or mirror laid out in a directory: each document is the file at its path below
/// the base. Reads follow no link below the base; writes go into a fresh private directory and
/// are renamed into place, read-only ([`tree::store_file`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    base: PathBuf,
    name: String,
}

impl Directory {
    /// The directory transport at `base`, named `name` in messages.
    pub fn new(base: &Path, name: &str) -> Directory {
        Directory {
            base: base.to_path_buf(),
            name: name.to_owned(),
        }
    }

    /// The transport of a `file:///` URL, or why there is none: an `https://` URL has no
    /// transport in this release.
    pub fn of(url: &str) -> Result<Directory, String> {
        match location(url)? {
            Location::Directory(base) => Ok(Directory::new(&base, url)),
            Location::Https(_) => Err(format!(
                "`{url}` is served over HTTPS, and this release has no network transport; give a `file:///` URL of a directory"
            )),
        }
    }

    /// The segments of `path`, checked.
    fn segments(path: &str) -> Result<Vec<&str>, Failure> {
        content::check_path(path).map_err(Failure::Unavailable)?;
        Ok(path.split('/').collect())
    }
}

impl Transport for Directory {
    fn name(&self) -> &str {
        &self.name
    }

    fn get(&self, path: &str, limit: u64) -> Result<Vec<u8>, Failure> {
        let segments = Directory::segments(path)?;
        tree::not_a_link(&self.base).map_err(Failure::Unavailable)?;
        match tree::locate(&self.base, &segments).map_err(Failure::Unavailable)? {
            Some(file) => tree::read_file(&file, limit).map_err(Failure::Unavailable),
            None => Err(Failure::Absent),
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), Failure> {
        let segments = Directory::segments(path)?;
        let Some((file, parents)) = segments.split_last() else {
            return Err(Failure::Unavailable("an empty path".to_owned()));
        };
        match self.get(path, bytes.len() as u64) {
            Ok(there) if there == bytes => return Ok(()),
            Ok(_) => {
                return Err(Failure::Unavailable(format!(
                    "another document is already at `{path}` of `{}`",
                    self.name
                )));
            }
            Err(Failure::Absent) => {}
            Err(Failure::Unavailable(reason)) => {
                return Err(Failure::Unavailable(format!(
                    "`{path}` of `{}` cannot be compared: {reason}",
                    self.name
                )));
            }
        }
        tree::not_a_link(&self.base).map_err(Failure::Unavailable)?;
        let parent = tree::make_dirs(&self.base, parents).map_err(Failure::Unavailable)?;
        tree::store_file(&parent, file, bytes, true).map_err(Failure::Unavailable)
    }
}
