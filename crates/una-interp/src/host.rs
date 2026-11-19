//! Host capabilities (`spec/14 §3`): the world that performs input and output, the grants of
//! `una run`, the confinement of `Files` to its prefix, and HMAC-SHA-256 for `Keys`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use una_core::bigint::{BigInt, Sign};
use una_hash::Encoder;

/// What a file operation of the world can fail with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileFault {
    /// The file does not exist.
    NotFound,
    /// Any other input or output error.
    Io,
    /// The file's content is not valid UTF-8: the run aborts with U6005, since the content,
    /// not the path or the file system's shape, would decide a failure (S-16, ADR 0086).
    NotText,
}

/// Why an intermediate directory of a confined path is not there, as [`World::absent`] tells
/// it and [`confine`] answers it ([`Confined::Absent`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Absence {
    /// A component does not exist (`ENOENT`): the operation fails with `NotFound`.
    Missing,
    /// A component that a later one is looked up in is not a directory (`ENOTDIR`): the
    /// operation fails with `Io`.
    NotDirectory,
}

impl Absence {
    /// The fault of the operation on a path with this absence: what the operation failed with
    /// when it was given the path (`ENOENT` is `NotFound`, `ENOTDIR` is `Io`), now answered
    /// without it (#1459).
    pub fn fault(self) -> FileFault {
        match self {
            Absence::Missing => FileFault::NotFound,
            Absence::NotDirectory => FileFault::Io,
        }
    }
}

/// What [`confine`] decides for a `Files` operation on a path (`spec/14 §3`). A caller handles
/// each case: only [`Confined::Path`] carries a path, so no other outcome can reach the world's
/// file operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confined {
    /// The operation fails with `Denied`: the world is given no path.
    Denied,
    /// An intermediate directory is confirmed absent: the operation fails with
    /// [`Absence::fault`], `NotFound` for a directory that does not exist, and the world is
    /// given no path (#1459). Confinement has no canonical directory for such a path, so there
    /// is nothing checked to open.
    Absent(Absence),
    /// The path the operation opens, without following its final component: the canonical
    /// form of its directory, which lies inside the canonical prefix, joined with the final
    /// component.
    Path(PathBuf),
}

/// A host value that is not valid UTF-8, which the run aborts on with U6005 (S-16, ADR 0086).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotText;

/// The world outside the program: the only place where host operations touch the machine
/// they run on. `una-cli` implements it; tests implement it in memory.
pub trait World {
    /// `Console.print`: writes `line` and a newline.
    fn print(&mut self, line: &str);
    /// `Clock.now`: milliseconds since the Unix epoch.
    fn now(&mut self) -> i64;
    /// `Env.get`: an environment variable, or [`NotText`] when its value is not valid UTF-8.
    fn env_var(&mut self, name: &str) -> Result<Option<String>, NotText>;
    /// Reads a file as text: [`FileFault::NotText`] when its content is not valid UTF-8.
    fn read_file(&mut self, path: &Path) -> Result<String, FileFault>;
    /// Writes a file. It never creates a file through a symbolic link (`spec/14 §3`): a file
    /// that does not exist is created only as a new file at `path` itself.
    fn write_file(&mut self, path: &Path, content: &str) -> Result<(), FileFault>;
    /// The canonical form of an existing path, following links; `None` when it does not
    /// exist.
    fn canonical(&mut self, path: &Path) -> Option<PathBuf>;
    /// Whether `path` itself is a symbolic link, dangling or not, without following it.
    fn is_link(&mut self, path: &Path) -> bool;
    /// One step of [`confine`]: the entry `name` of `directory`, a canonical path that
    /// [`World::canonical`] or an earlier step returned, as a path naming the same location as
    /// `canonical(directory/name)` (a link at `name` followed), or `None` when it does not
    /// resolve. The default is `canonical(directory/name)` itself. A host overrides it to look
    /// up only `name` in the directory already resolved, so that confinement makes O(c)
    /// lookups of components for a path of c components on its normal path (#1343); such a
    /// step may keep `name` as spelled where a case-insensitive file system spells it
    /// otherwise, and counts host limits (links followed, path length) from `directory` only.
    /// So a step alone is not a canonicalization: [`confine`] never trusts it alone, but
    /// confirms it with [`World::canonical`] and, when they disagree, decides by resolving
    /// every lexical prefix.
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        self.canonical(&directory.join(name))
    }
    /// Whether `path`, which [`World::canonical`] did not resolve, does not exist, and why: a
    /// component of it is missing or is not a directory. Any other failure to resolve it, a
    /// host limit (a resolved path longer than the host takes, more links than it follows) or
    /// a directory that may not be searched, is not absence, and [`confine`] denies the path
    /// (`spec/14 §3` rule 2, #1380). The default is `None`, so a world that does not tell
    /// denies every path whose directory does not resolve. Whatever a world answers here, a
    /// path whose directory did not resolve is never opened (#1459): the answer only chooses
    /// between `Denied` and the failure of [`Absence::fault`].
    fn absent(&mut self, _path: &Path) -> Option<Absence> {
        None
    }
    /// Takes `event` as it is written, as the trace file of `una run --trace` does (ADR 0117
    /// rule 10): `true` when the run need not keep it. The default keeps every event.
    fn trace_event(&mut self, _event: &crate::machine::Event) -> bool {
        false
    }
    /// A started task at `path` ended alone after `abort`, U6003 at the call that ran too
    /// deep (`spec/27 §7`): its line on stderr. The default writes nothing.
    fn task_aborted(&mut self, _path: &str, _abort: &crate::machine::Abort) {}
}

/// The world of examples: they cannot reach host capabilities (`spec/11 §6`), so nothing here
/// is ever called.
pub(crate) struct NoWorld;

impl World for NoWorld {
    fn print(&mut self, _line: &str) {}

    fn now(&mut self) -> i64 {
        0
    }

    fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
        Ok(None)
    }

    fn read_file(&mut self, _path: &Path) -> Result<String, FileFault> {
        Err(FileFault::NotFound)
    }

    fn write_file(&mut self, _path: &Path, _content: &str) -> Result<(), FileFault> {
        Err(FileFault::Io)
    }

    fn canonical(&mut self, _path: &Path) -> Option<PathBuf> {
        None
    }

    fn is_link(&mut self, _path: &Path) -> bool {
        false
    }
}

/// A grant of `una run` (`spec/13 §5`), with its paths already resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grant {
    /// `console`.
    Console,
    /// `clock`.
    Clock,
    /// `env`.
    Env,
    /// `files:<path>:<modes>`.
    Files {
        /// The prefix that paths are relative to.
        prefix: PathBuf,
        /// Whether reading is allowed.
        read: bool,
        /// Whether writing is allowed.
        write: bool,
    },
    /// `keys:<file>`.
    Keys {
        /// The key file.
        file: PathBuf,
    },
    /// `http:<origin>[:<methods>]` (`spec/24 §2`).
    Http(una_core::http::Grant),
    /// `serve:<scheme>://<address>:<port>` and its items (`spec/31 §2`), edition 0.4.
    Serve {
        /// The grant, as written.
        grant: Box<una_core::http::serve::ServeGrant>,
        /// The certificate chain's path, resolved, on an `https` grant.
        certificate: Option<PathBuf>,
        /// The private key's path, resolved, on an `https` grant.
        key: Option<PathBuf>,
    },
}

impl Grant {
    /// The host capability this grant gives.
    pub fn capability(&self) -> &'static str {
        match self {
            Grant::Console => "Console",
            Grant::Clock => "Clock",
            Grant::Env => "Env",
            Grant::Files { .. } => "Files",
            Grant::Keys { .. } => "Keys",
            Grant::Http(_) => "Http",
            Grant::Serve { .. } => "HttpServer",
        }
    }

    /// The bound of the `tasks` statement of a `serve` section's driver (`spec/31 §7` rule 4,
    /// decision S6-D6, #1724): the sum of `/once` over the `serve:` grants among `grants`,
    /// saturating, within the default bound of a `tasks` statement for `max_memory` MiB and
    /// `cap` ([`crate::concurrency::default_bound`], ADR 0117 rule 7), and at least 1. The
    /// grants' `/once` bounds the exchanges open; this bound makes it bound the started tasks
    /// too, so a task's place is given back when its `handle` ends, not when its exchange ends,
    /// as when the peer leaves or `answer` passes; the clamp keeps the driver waiting at
    /// `start` below the run's cap on tasks, where it waited before, instead of aborting the
    /// run. The same in both engines. O(g) for the g grants.
    pub fn serve_driver_bound(grants: &[Grant], max_memory: usize, cap: Option<usize>) -> usize {
        let once = grants
            .iter()
            .filter_map(|grant| match grant {
                Grant::Serve { grant, .. } => Some(grant.limits.once),
                _ => None,
            })
            .fold(0u64, u64::saturating_add);
        usize::try_from(once)
            .unwrap_or(usize::MAX)
            .min(crate::concurrency::default_bound(max_memory, cap))
            .max(1)
    }

    /// Whether the grant carries limits (ADR 0117 rule 8, `spec/27 §9`).
    pub fn limited(&self) -> bool {
        matches!(self, Grant::Http(http) if http.limits().any())
    }

    /// Parses a grant spec of `spec/13 §5`, resolving relative paths against `base`.
    pub fn parse(spec: &str, base: &Path) -> Option<Grant> {
        match spec {
            "console" => Some(Grant::Console),
            "clock" => Some(Grant::Clock),
            "env" => Some(Grant::Env),
            _ => {
                // With the limit suffixes of edition 0.4 (`spec/27 §9`): a spec whose limits are
                // invalid is no grant. Whether a run may take limits is the caller's to check.
                if spec.starts_with("http:") {
                    return una_core::http::Grant::parse_limited(spec)
                        .and_then(Result::ok)
                        .map(Grant::Http);
                }
                if spec.starts_with(una_core::http::serve::PREFIX) {
                    let grant = una_core::http::serve::ServeGrant::parse(spec).ok()?;
                    let (certificate, key) = match &grant.files {
                        Some(files) => (
                            Some(base.join(&files.certificate)),
                            Some(base.join(&files.key)),
                        ),
                        None => (None, None),
                    };
                    return Some(Grant::Serve {
                        grant: Box::new(grant),
                        certificate,
                        key,
                    });
                }
                if let Some(file) = spec.strip_prefix("keys:") {
                    return (!file.is_empty()).then(|| Grant::Keys {
                        file: base.join(file),
                    });
                }
                let rest = spec.strip_prefix("files:")?;
                let (path, modes) = rest.rsplit_once(':')?;
                let (read, write) = match modes {
                    "read" => (true, false),
                    "write" => (false, true),
                    "read,write" => (true, true),
                    _ => return None,
                };
                (!path.is_empty()).then(|| Grant::Files {
                    prefix: normalize(&base.join(path)),
                    read,
                    write,
                })
            }
        }
    }
}

/// Why `spec` is not a grant, for the usage error of `--grant` (`spec/13 §5`): every problem
/// of a `serve:` spec (`spec/31 §2`), or the list of the grant specs.
pub fn invalid_grant(spec: &str) -> String {
    if spec.starts_with(una_core::http::serve::PREFIX)
        && let Err(problems) = una_core::http::serve::ServeGrant::parse(spec)
    {
        let messages: Vec<String> = problems.into_iter().map(|p| p.message).collect();
        return format!(
            "`{spec}` is not a valid serve grant: {}",
            messages.join("; ")
        );
    }
    format!(
        "`{spec}` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]"
    )
}

/// What a run knows of its program's `L_lim` (ADR 0117 IF4, `spec/27 §9` rule 5): the label as
/// a message writes it, and whether it flows to `internal`. `None` for a program of an edition
/// before `0.4`, or an executable built without it, which may take no limits.
pub type LimitLabel<'l> = Option<(&'l str, bool)>;

/// Refuses each `--grant` spec of `flags` that carries limits, unless the program's `limit`
/// lets it: a usage error, exit code 2, before `main` starts (ADR 0117 IF4, `spec/27 §9`), so
/// limits given at start-up cannot bypass the check that `una check` makes of `run grants`.
pub fn check_limited_flags(flags: &[String], limit: LimitLabel<'_>) -> Result<(), String> {
    for spec in flags {
        if !Grant::parse(spec, Path::new("")).is_some_and(|grant| grant.limited()) {
            continue;
        }
        match limit {
            Some((_, true)) => {}
            Some((label, false)) => {
                return Err(format!(
                    "the grant `{spec}` has limits, but this program's requests are timed by `{label}` data"
                ));
            }
            None => {
                return Err(format!(
                    "the grant `{spec}` has limits, which only a program of edition 0.4 may take"
                ));
            }
        }
    }
    Ok(())
}

/// `path` with `.` removed and `..` applied, lexically: a `..` removes the named component
/// before it, and is kept when there is none, so `../..` stays two levels up and a `..` at
/// the root stays `/..`, which every host resolves to the root (#2144: a `..` once removed
/// a `..` before it, so `../../x` was `x`). A relative path with no component left (`.`,
/// `a/..`, the empty path) is `.`, the directory it is relative to, and never the empty
/// path, which names no directory to the operating system (#2144).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(Component::CurDir);
    }
    out
}

/// What a `Files` operation on `relative` under `prefix` does: the path it opens
/// ([`Confined::Path`]), or the failure it ends with, the world given no path (`spec/14 §3`,
/// ADR 0038 D9):
///
/// 1. `relative` is normalized lexically, without following links; an absolute path, a `..`
///    that leaves the prefix, and a path with no final component are denied;
/// 2. every intermediate directory, the prefix included, is resolved with its links followed
///    and must lie inside the canonical prefix; a dangling link there is denied; a directory
///    that does not exist ([`World::absent`]) is [`Confined::Absent`]: the operation fails
///    with `NotFound` (or `Io` where a component is not a directory), which confinement
///    answers, since it has no canonical directory to open (#1459); a directory that does
///    not resolve for any other reason, a host limit included, is denied (#1380);
/// 3. the final component is never followed: a symbolic link there, dangling or not, is
///    denied. The world opens the returned path without creating a file through a link. A
///    final component that does not exist in a directory that resolves is not absence: the
///    path is returned, so a write creates the file and a read fails with `NotFound` at the
///    operation.
///
/// So every path returned is the canonical form of a directory that confinement resolved,
/// joined with one component: a path as the program wrote it never reaches the world's
/// file operations.
///
/// The outcome is that of canonicalizing each lexical prefix `prefix/d1/../dk` of the path in
/// turn (`confine_by_prefixes`), in O(c) lookups of components for c components (#1343):
/// each directory is resolved from the one before it with [`World::step`], which looks up only
/// its own component, to find where that resolution stops or leaves the prefix; then one
/// canonicalization of the lexical prefix there confirms it, and the next one decides as the
/// prefix-by-prefix resolution does. Where they disagree, which takes a host limit (more links
/// than the host follows in one path, a lexical path longer than the host takes) or a name
/// that a case-insensitive file system spells otherwise, the prefix-by-prefix resolution
/// decides, in O(c²) lookups: the bound of O(c) holds on the normal path, and the fallback,
/// which any path can reach, keeps the cost the earlier algorithm had for every path.
///
/// An empty `prefix` is `.`, the directory that a relative path is relative to, as it is for
/// every relative path ([`normalize`]): it is resolved and checked as any other prefix, and
/// is not a prefix that does not exist (#2144). The grants of [`Grant::parse`] never hold one.
///
/// Races between this check and the operation are outside the v0.1 model (D9 rule 4). An
/// absence is decided at the check: a directory made after it does not turn the answer into
/// a late success.
pub fn confine(world: &mut dyn World, prefix: &Path, relative: &str) -> Confined {
    let prefix = if prefix.as_os_str().is_empty() {
        Path::new(".")
    } else {
        prefix
    };
    let Some(parts) = normalized(relative) else {
        return Confined::Denied;
    };
    // D-133: a path that names the prefix itself (``, `.`, `a/..`) has no final
    // component to open, and is denied.
    let Some((last, directories)) = parts.split_last() else {
        return Confined::Denied;
    };
    let Some(root) = world.canonical(prefix) else {
        return unresolved(world, prefix);
    };
    // Rule 2, located: `resolved` is the canonical form of `prefix/d1/../d(depth)`, since
    // `canonical(a/b)` is `canonical(canonical(a)/b)` for a normal component `b`.
    let mut resolved = root.clone();
    let mut depth = 0;
    for part in directories {
        match world.step(&resolved, part) {
            Some(directory) if directory.starts_with(&root) => resolved = directory,
            _ => break,
        }
        depth += 1;
    }
    // Confirmed: the lexical prefix of `depth` directories resolves to `resolved`, so every
    // shorter one resolves too, inside the prefix, as each step was checked.
    let mut lexical = prefix.to_path_buf();
    lexical.extend(directories.iter().take(depth));
    if depth > 0 && world.canonical(&lexical).as_ref() != Some(&resolved) {
        return confine_by_prefixes(world, prefix, relative);
    }
    let Some(next) = directories.get(depth) else {
        return unfollowed(world, resolved.join(last));
    };
    // The resolution stops at the next directory, or leaves the prefix there: decided as by
    // the prefix-by-prefix resolution. D-134: each directory is checked, not only the last,
    // so a path never passes through a directory outside the prefix even when it comes back
    // inside.
    lexical.push(next);
    match world.canonical(&lexical) {
        None => unresolved(world, &lexical),
        Some(directory) if !directory.starts_with(&root) => Confined::Denied,
        Some(_) => confine_by_prefixes(world, prefix, relative),
    }
}

/// [`confine`] by canonicalizing every lexical prefix of the path from the grant prefix, in
/// O(c²) lookups of components for c components: the definition that [`confine`] keeps, and
/// what decides when its located resolution and a canonicalization disagree.
fn confine_by_prefixes(world: &mut dyn World, prefix: &Path, relative: &str) -> Confined {
    // Rule 1: lexical normalization.
    let Some(parts) = normalized(relative) else {
        return Confined::Denied;
    };
    // D-133: no final component, denied.
    let Some((last, directories)) = parts.split_last() else {
        return Confined::Denied;
    };
    // Rule 2: every intermediate directory resolves inside the canonical prefix.
    // D-134: each one is checked, not only the last.
    let mut lexical = prefix.to_path_buf();
    let Some(root) = world.canonical(prefix) else {
        return unresolved(world, &lexical);
    };
    let mut resolved = root.clone();
    for part in directories {
        lexical.push(part);
        match world.canonical(&lexical) {
            Some(directory) if directory.starts_with(&root) => resolved = directory,
            Some(_) => return Confined::Denied,
            None => return unresolved(world, &lexical),
        }
    }
    unfollowed(world, resolved.join(last))
}

/// Rule 3 of [`confine`]: `target`, a resolved directory joined with the final component, is
/// opened unless that component is a symbolic link, which is never followed.
fn unfollowed(world: &mut dyn World, target: PathBuf) -> Confined {
    if world.is_link(&target) {
        Confined::Denied
    } else {
        Confined::Path(target)
    }
}

/// Rule 1 of [`confine`]: the normal components of `relative` with `.` removed and `..`
/// applied lexically, or `None` when it is absolute or a `..` leaves the prefix.
fn normalized(relative: &str) -> Option<Vec<&OsStr>> {
    let mut parts = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts)
}

/// [`World::step`] on the machine's file system, for the worlds of `una run` and of native
/// executables: one `lstat` of `directory/name`, and a canonicalization only when `name` is a
/// link. Since `directory` is canonical and `name` a normal component, `directory/name` is
/// canonical when it is not a link.
pub fn disk_step(directory: &Path, name: &OsStr) -> Option<PathBuf> {
    let path = directory.join(name);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() {
        std::fs::canonicalize(&path).ok()
    } else {
        Some(path)
    }
}

/// [`World::absent`] on the machine's file system, for the worlds of `una run` and of native
/// executables: the canonicalization of `path` fails because a component is missing
/// (`ENOENT`) or is not a directory (`ENOTDIR`). Any other failure (`ENAMETOOLONG`, `ELOOP`,
/// `EACCES`, ...), or a canonicalization that now succeeds, is not absence.
pub fn disk_absent(path: &Path) -> Option<Absence> {
    match std::fs::canonicalize(path).err()?.kind() {
        std::io::ErrorKind::NotFound => Some(Absence::Missing),
        std::io::ErrorKind::NotADirectory => Some(Absence::NotDirectory),
        _ => None,
    }
}

/// The outcome of [`confine`] when `lexical`, the grant prefix or an intermediate directory
/// as written, does not resolve: a dangling link is denied; a directory that does not exist
/// is [`Confined::Absent`], the failure the operation had on such a path, answered here; and
/// any other failure to resolve is denied (#1380). No path is returned (#1459): none of the
/// directories from `lexical` on was resolved, so a path through them would be opened through
/// links that were never checked, whatever the world told of its absence.
fn unresolved(world: &mut dyn World, lexical: &Path) -> Confined {
    if world.is_link(lexical) {
        return Confined::Denied;
    }
    match world.absent(lexical) {
        Some(absence) => Confined::Absent(absence),
        None => Confined::Denied,
    }
}

/// The keys of a key file: a TOML table of key names to base64 keys. The
/// error names what is wrong, in one sentence.
pub fn parse_keys(text: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let table: BTreeMap<String, toml::Value> = toml::from_str(text)
        .map_err(|error| format!("it is not valid TOML: {}", error.message()))?;
    let mut keys = BTreeMap::new();
    for (name, value) in table {
        let toml::Value::String(encoded) = value else {
            return Err(format!("the key `{name}` is not a base64 string"));
        };
        match base64(&encoded) {
            Some(key) if !key.is_empty() => {
                keys.insert(name, key);
            }
            _ => return Err(format!("the key `{name}` is not a nonempty base64 string")),
        }
    }
    Ok(keys)
}

/// HMAC-SHA-256 (RFC 2104, FIPS 198-1) of `data` under `key`, the routine of `Keys.hmac` in
/// both engines and of the reduced bodies of a serve trace. Time O(length of the key + length
/// of the data); space O(1) beyond the result, and no allocation but the result.
///
/// It reads the key where its caller holds it and copies it nowhere but into its own scratch,
/// fixed arrays on its stack, which it overwrites with zeros before it returns (#2120). The
/// computation and the wipe are one function of the private module [`hmac`], which exports
/// nothing else: no routine of this crate can compute the MAC and skip the wipe. Overwritten,
/// on every return:
///
/// - the key padded to the block, and the same bytes under the inner and the outer pad (one
///   buffer, changed in place);
/// - the digest of a key longer than the block, and the last bytes of such a key, which the
///   hash's padding copies;
/// - the chaining value of each of the hashes, which after the block of a pad stands for the
///   key, and the inner digest.
///
/// How it is overwritten, and what that is worth: `fill(0)` and then `std::hint::black_box`,
/// the technique of `una_project::registry::keyfile::wipe`. It is a wipe WITHOUT a volatile
/// write: this crate forbids `unsafe`, which a volatile write needs, and it does not depend on
/// `zeroize` or `una-crypto` (`docs/compiler/architecture.md` §7). ADR 0133 rule 7 calls such a
/// wipe removable by an optimizer, and it is: `black_box` promises nothing, and a compiler may
/// drop the four fills as stores that nothing reads. No test in the tree can see that happen.
/// That the zeros are written rests on a measurement (a scan of the dead stack after a call,
/// on aarch64-apple-darwin with the pinned toolchain, in the release and dev profiles; not
/// measured on Linux or x86-64) and on the toolchain pin that keeps the measured compiler.
///
/// Not overwritten, and outside what this routine can reach:
///
/// - what the SHA-256 compression function (`sha2::compress256`) keeps in its own frame for
///   the time of a call. Its software path, which every aarch64 target takes and x86-64 takes
///   on processors without the SHA extensions, makes a copy of the chaining value, which after
///   the block of a pad is the same key-equivalent value as the scratch's `state`, and a
///   message schedule, which for that block is the key under the pad. Whether they stay in
///   registers or reach the stack is the compiler's choice; nothing here wipes them;
/// - any copy the compiler makes of the scratch in registers or spill slots;
/// - the key itself where the caller holds it (the run's table of keys, for the whole run), the
///   key file's text and its decoding at start-up, and the bearer key of an `Http` request,
///   which the operations of `Http` copy out of the table and into the request they send;
/// - the result, which is the value asked for.
///
/// The hash state is this routine's own so that it can be overwritten: only the compression
/// function is `sha2`'s (its feature `compress`), and the padding and length encoding of
/// SHA-256 (FIPS 180-4 §5.1.1) are code of this module, tested against the hasher of `sha2`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::mac(key, data).to_vec()
}

/// The computation of [`hmac_sha256`] with its wipe (#2120). It exports [`hmac::mac`] alone:
/// the scratch, its fields and the steps of the computation are private to it, so the MAC
/// cannot be computed from outside without the wipe.
mod hmac {
    /// The block length of SHA-256, in bytes.
    const BLOCK: usize = 64;

    /// The output length of SHA-256, in bytes.
    const OUTPUT: usize = 32;

    /// The initial hash value of SHA-256 (FIPS 180-4 §5.3.3).
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    /// Every buffer in which the HMAC holds its key or a value made from it, while it runs.
    /// Fixed arrays in one place, never a vector: nothing grows, so no reallocation leaves a
    /// copy behind, and one [`Scratch::wipe`] covers them all.
    struct Scratch {
        /// The key, padded with zeros to the block length (a key longer than the block: its
        /// SHA-256), then the same bytes under the inner pad's XOR, then under the outer
        /// pad's. One buffer, changed in place: the two pads are never separate copies.
        block: [u8; BLOCK],
        /// The chaining value of the hash in progress. After the block of a pad it is a
        /// function of the key alone, and whoever holds it can compute MACs without the key.
        state: [u32; 8],
        /// The end of the message being hashed, with its padding: one or two blocks. For a key
        /// longer than the block it holds the last bytes of the key itself.
        tail: [u8; 2 * BLOCK],
        /// The digest of a key longer than the block, then the inner digest.
        digest: [u8; OUTPUT],
    }

    impl Scratch {
        /// A scratch of zeros.
        const fn new() -> Self {
            Self {
                block: [0; BLOCK],
                state: [0; 8],
                tail: [0; 2 * BLOCK],
                digest: [0; OUTPUT],
            }
        }

        /// Overwrites every buffer with zeros, then passes the scratch through an optimization
        /// barrier, as `una_project::registry::keyfile::wipe` does. Nothing reads the zeros
        /// afterwards, so only the barrier keeps a compiler from dropping the writes as dead
        /// stores, and `std::hint::black_box` promises nothing: a wipe without a volatile
        /// write, removable by an optimizer in the words of ADR 0133 rule 7. See
        /// [`super::hmac_sha256`] for what it rests on.
        fn wipe(&mut self) {
            self.block.fill(0);
            self.state.fill(0);
            self.tail.fill(0);
            self.digest.fill(0);
            let _ = std::hint::black_box(self);
        }

        /// The fields, as one sequence of bytes.
        #[cfg(test)]
        fn bytes(&self) -> Vec<u8> {
            let mut bytes = self.block.to_vec();
            bytes.extend(self.state.iter().flat_map(|word| word.to_be_bytes()));
            bytes.extend_from_slice(&self.tail);
            bytes.extend_from_slice(&self.digest);
            bytes
        }
    }

    #[cfg(test)]
    thread_local! {
        /// The scratch of this thread's last HMAC as it was just before its wipe, in tests: it
        /// shows that the wipe had something to overwrite.
        static BEFORE_WIPE: std::cell::RefCell<Vec<u8>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// One step of the SHA-256 compression function over `block`, read where it is.
    fn sha256_block(state: &mut [u32; 8], block: &[u8; BLOCK]) {
        sha2::compress256(state, std::slice::from_ref(block.into()));
    }

    /// Ends a SHA-256 whose first `before` bytes (a multiple of the block) are already in
    /// `state`: absorbs `data`, pads (FIPS 180-4 §5.1.1) in `tail` and writes the digest to
    /// `digest`. The whole blocks of `data` are read where they are; only its last, partial
    /// block is copied, to `tail`. Time O(length of `data`), no allocation.
    fn sha256_finish(
        state: &mut [u32; 8],
        tail: &mut [u8; 2 * BLOCK],
        digest: &mut [u8; OUTPUT],
        before: u64,
        data: &[u8],
    ) {
        let (blocks, rest) = data.as_chunks::<BLOCK>();
        for block in blocks {
            sha256_block(state, block);
        }
        // The message's length in bits, modulo 2^64 as the standard counts it.
        let bits = before.wrapping_add(data.len() as u64).wrapping_mul(8);
        tail.fill(0);
        // The rest, the byte 0x80 and the 8 bytes of the length fit in one block, or take two.
        let padded = if rest.len() + 9 <= BLOCK {
            tail.as_mut_slice().split_at_mut(BLOCK).0
        } else {
            tail.as_mut_slice()
        };
        for (to, from) in padded.iter_mut().zip(rest.iter().chain(&[0x80])) {
            *to = *from;
        }
        // The length is big-endian at the end: its lowest byte is the last byte.
        for (to, from) in padded.iter_mut().rev().zip(bits.to_le_bytes()) {
            *to = from;
        }
        for block in padded.as_chunks::<BLOCK>().0 {
            sha256_block(state, block);
        }
        for (to, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(state.iter()) {
            *to = word.to_be_bytes();
        }
    }

    /// The HMAC of `data` under `key`, computed in `scratch`, which is overwritten with zeros
    /// before this returns. The computation and the wipe are this one function: there is no
    /// routine that computes and leaves the scratch as it was. There is one return and the
    /// wipe is before it; nothing before the wipe can panic (it indexes nothing, and the one
    /// split is at a constant inside its array), so no unwinding passes the wipe by.
    fn mac_in(key: &[u8], data: &[u8], scratch: &mut Scratch) -> [u8; OUTPUT] {
        let mut mac = [0u8; OUTPUT];
        {
            let Scratch {
                block,
                state,
                tail,
                digest,
            } = &mut *scratch;
            // Nothing of what the scratch held before reaches the result.
            block.fill(0);
            if key.len() > BLOCK {
                *state = INITIAL;
                sha256_finish(state, tail, digest, 0, key);
                for (to, from) in block.iter_mut().zip(digest.iter()) {
                    *to = *from;
                }
            } else {
                for (to, from) in block.iter_mut().zip(key) {
                    *to = *from;
                }
            }
            // The inner hash: the key under the inner pad, then the data.
            for byte in block.iter_mut() {
                *byte ^= 0x36;
            }
            *state = INITIAL;
            sha256_block(state, block);
            sha256_finish(state, tail, digest, BLOCK as u64, data);
            // The outer hash: the key under the outer pad, then the inner digest.
            for byte in block.iter_mut() {
                *byte ^= 0x36 ^ 0x5c;
            }
            *state = INITIAL;
            sha256_block(state, block);
            sha256_finish(state, tail, &mut mac, BLOCK as u64, digest.as_slice());
        }
        #[cfg(test)]
        BEFORE_WIPE.with(|seen| *seen.borrow_mut() = scratch.bytes());
        scratch.wipe();
        mac
    }

    /// The HMAC-SHA-256 of `data` under `key`, in a scratch of its own, on its stack, which is
    /// overwritten before it returns.
    pub(super) fn mac(key: &[u8], data: &[u8]) -> [u8; OUTPUT] {
        let mut scratch = Scratch::new();
        mac_in(key, data, &mut scratch)
    }

    #[cfg(test)]
    mod tests {
        use super::super::{hex, hmac_sha256};
        use super::*;
        use sha2::{Digest, Sha256};

        /// The seven test cases of RFC 4231 §4 for HMAC-SHA-256: key, data, expected MAC in
        /// hexadecimal (case 5 gives the first 128 bits only).
        fn rfc_4231() -> Vec<(Vec<u8>, Vec<u8>, &'static str)> {
            vec![
                (
                    vec![0x0b; 20],
                    b"Hi There".to_vec(),
                    "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
                ),
                (
                    b"Jefe".to_vec(),
                    b"what do ya want for nothing?".to_vec(),
                    "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
                ),
                (
                    vec![0xaa; 20],
                    vec![0xdd; 50],
                    "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
                ),
                (
                    (1..=25).collect(),
                    vec![0xcd; 50],
                    "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
                ),
                (
                    vec![0x0c; 20],
                    b"Test With Truncation".to_vec(),
                    "a3b6167473100ee06e0c796c2955552b",
                ),
                (
                    vec![0xaa; 131],
                    b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
                    "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
                ),
                (
                    vec![0xaa; 131],
                    b"This is a test using a larger than block-size key and a larger than \
                      block-size data. The key needs to be hashed before being used by the HMAC \
                      algorithm."
                        .to_vec(),
                    "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
                ),
            ]
        }

        /// HMAC-SHA-256 as this crate computed it before #2120, on the hasher of `sha2` and
        /// vectors: the reference the routine must agree with bit for bit.
        fn reference(key: &[u8], data: &[u8]) -> Vec<u8> {
            let mut block = if key.len() > BLOCK {
                Sha256::digest(key).to_vec()
            } else {
                key.to_vec()
            };
            block.resize(BLOCK, 0);
            let inner_pad: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
            let outer_pad: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
            let inner = Sha256::new()
                .chain_update(&inner_pad)
                .chain_update(data)
                .finalize();
            Sha256::new()
                .chain_update(&outer_pad)
                .chain_update(inner)
                .finalize()
                .to_vec()
        }

        /// Bytes that differ from one position to the next and are never zero.
        fn pattern(length: usize, seed: usize) -> Vec<u8> {
            (0..length)
                .map(|i| ((i * 7 + seed * 13) % 255 + 1) as u8)
                .collect()
        }

        /// The scratch of this thread's last HMAC, as it was just before its wipe.
        fn before_wipe() -> Vec<u8> {
            BEFORE_WIPE.with(|seen| seen.borrow().clone())
        }

        #[test]
        fn hmac_matches_rfc_4231() {
            let cases = rfc_4231();
            assert_eq!(cases.len(), 7);
            for (number, (key, data, expected)) in cases.iter().enumerate() {
                let mac = hex(&hmac_sha256(key, data));
                assert_eq!(mac.len(), 64);
                assert_eq!(
                    &mac[..expected.len()],
                    *expected,
                    "test case {}",
                    number + 1
                );
            }
        }

        /// The edges the cases of RFC 4231 do not have: the empty key, the empty message and
        /// a key of exactly one block. The expected values are literals from two independent
        /// implementations that agree (Python's `hmac` module and `openssl dgst -sha256
        /// -mac HMAC`), not from this crate.
        #[test]
        fn hmac_matches_known_answers_at_the_edges() {
            let block_key: Vec<u8> = (0..64).collect();
            let cases: [(&[u8], &[u8], &str); 5] = [
                (
                    b"",
                    b"",
                    "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad",
                ),
                (
                    b"",
                    b"what do ya want for nothing?",
                    "76d9e7194e7dbc3aa00bbe8ffb9f6fcb5a932170f971f948bb2ab61607d2b9d6",
                ),
                (
                    b"Jefe",
                    b"",
                    "923598ca6d64af2a5dba79dcd021a8a0fe5c5f557519adaaf0ad532d4506dd30",
                ),
                (
                    &block_key,
                    b"Hi There",
                    "e311769a0a9a3af1ad9da74c1933bab5ac0aa48367b55ab6ec995508bdab1db6",
                ),
                (
                    &block_key,
                    b"",
                    "3499f163f48604c0b15ac89e4e7c66f314fb3b203b8ac2f564828e62f6be9d9d",
                ),
            ];
            for (key, data, expected) in cases {
                assert_eq!(
                    hex(&hmac_sha256(key, data)),
                    expected,
                    "a key of {} bytes, data of {}",
                    key.len(),
                    data.len()
                );
            }
        }

        #[test]
        fn hmac_is_bit_identical_to_the_hmac_over_the_hasher_of_sha2() {
            // Every key length around the block (shorter, equal, longer: hashed first) with
            // every data length around one and two blocks, where the padding takes one block
            // or two.
            for key_length in (0..=70).chain([127, 128, 129, 131, 200, 1000]) {
                let key = pattern(key_length, 1);
                for data_length in (0..=130).chain([191, 192, 193, 1000, 4096]) {
                    let data = pattern(data_length, 2);
                    assert_eq!(
                        hmac_sha256(&key, &data),
                        reference(&key, &data),
                        "a key of {key_length} bytes, data of {data_length}"
                    );
                }
            }
            // The result does not depend on what the scratch held before.
            let mut scratch = Scratch::new();
            for key_length in [0, 4, 64, 131] {
                scratch.block = [0xa5; BLOCK];
                scratch.state = [0xa5a5_a5a5; 8];
                scratch.tail = [0xa5; 2 * BLOCK];
                scratch.digest = [0xa5; OUTPUT];
                let key = pattern(key_length, 3);
                assert_eq!(
                    mac_in(&key, b"data", &mut scratch).to_vec(),
                    reference(&key, b"data")
                );
            }
        }

        /// The routine leaves nothing in its scratch, which held key material just before the
        /// wipe (#2120). The first half shows the second is not vacuous: before the wipe the
        /// scratch holds the key under a fixed XOR, a chaining value and the inner digest.
        #[test]
        fn hmac_wipes_every_buffer_that_held_key_material() {
            let data = pattern(100, 5);
            // A key shorter than the block, one of exactly a block, and longer ones, which are
            // hashed first: 131 bytes leave 3 bytes of the key in the tail, 190 leave 62 and
            // take two blocks of padding.
            for key_length in [1, 4, 32, 63, 64, 65, 131, 190] {
                let key = pattern(key_length, 4);
                let padded = if key_length > BLOCK {
                    Sha256::digest(&key).to_vec()
                } else {
                    key.clone()
                };
                // The scratch held other bytes before.
                let mut scratch = Scratch::new();
                scratch.tail = [0xa5; 2 * BLOCK];
                let mac = mac_in(&key, &data, &mut scratch);
                assert_eq!(mac.to_vec(), reference(&key, &data));

                // Just before the wipe: the padded key under the outer pad, ...
                let dirty = before_wipe();
                assert_eq!(dirty.len(), BLOCK + 32 + 2 * BLOCK + OUTPUT);
                let (block, after) = dirty.split_at(BLOCK);
                let (state, after) = after.split_at(32);
                let (tail, digest) = after.split_at(2 * BLOCK);
                for (i, byte) in block.iter().enumerate() {
                    assert_eq!(byte ^ 0x5c, padded.get(i).copied().unwrap_or(0));
                }
                // ... the chaining value of the outer hash, which is the MAC, and the inner
                // digest, in its buffer and in the padding block of the outer hash.
                assert_eq!(state, mac);
                let inner_pad: Vec<u8> = (0..BLOCK)
                    .map(|i| padded.get(i).copied().unwrap_or(0) ^ 0x36)
                    .collect();
                let inner = Sha256::new()
                    .chain_update(&inner_pad)
                    .chain_update(&data)
                    .finalize();
                assert_eq!(digest, &inner[..]);
                assert_eq!(tail[..OUTPUT], inner[..]);

                // After it: nothing.
                let left = scratch.bytes();
                assert_eq!(left.len(), dirty.len());
                assert!(
                    left.iter().all(|byte| *byte == 0),
                    "a key of {key_length} bytes left {left:02x?}"
                );
            }
        }

        /// The public routine goes through the same function: its own scratch held the key
        /// under the outer pad just before the wipe that function ends with.
        #[test]
        fn the_public_routine_computes_in_the_scratch_that_is_wiped() {
            let key = pattern(40, 7);
            BEFORE_WIPE.with(|seen| seen.borrow_mut().clear());
            assert_eq!(hmac_sha256(&key, b"data"), reference(&key, b"data"));
            let dirty = before_wipe();
            assert_eq!(dirty.len(), BLOCK + 32 + 2 * BLOCK + OUTPUT);
            for (i, byte) in dirty[..BLOCK].iter().enumerate() {
                assert_eq!(byte ^ 0x5c, key.get(i).copied().unwrap_or(0));
            }
        }

        /// While a key longer than the block is hashed, its last bytes are in the tail and its
        /// digest in the digest buffer: both are in the scratch the routine wipes, not
        /// elsewhere.
        #[test]
        fn hmac_hashes_a_long_key_in_the_scratch() {
            let key = pattern(131, 6);
            let mut scratch = Scratch::new();
            scratch.state = INITIAL;
            let Scratch {
                state,
                tail,
                digest,
                ..
            } = &mut scratch;
            sha256_finish(state, tail, digest, 0, &key);
            assert_eq!(scratch.digest, Sha256::digest(&key)[..]);
            assert_eq!(scratch.tail[..3], key[128..]);
            scratch.wipe();
            assert!(scratch.bytes().iter().all(|byte| *byte == 0));
        }
    }
}

#[cfg(test)]
thread_local! {
    /// The pairs of bytes the `Hashed` comparisons of this thread folded, in tests (#1481).
    pub(crate) static HASHED_BYTES_FOLDED: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Whether two `Hashed` values hold the same bytes, the `==` of both engines on them (#1481).
///
/// Its time does not depend on where the two differ, or on whether they do: every pair of bytes
/// is read and the differences are folded into one byte, which is tested once, after an
/// optimization barrier that keeps a compiler from turning the fold into an early exit. Time
/// O(length) always. The lengths are public (a keyed hash has its algorithm's length), so two
/// values of different lengths differ at once. Defense in depth: a `Hashed` is a pseudonymous
/// key, not a tag under verification, and timing stays outside the verification model.
#[must_use]
pub fn hashed_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
        #[cfg(test)]
        HASHED_BYTES_FOLDED.with(|folded| folded.set(folded.get() + 1));
    }
    std::hint::black_box(difference) == 0
}

/// The domain of `fake_hash`'s preimages: a fixed prefix whose final zero byte ends it before the
/// seed's encoding starts, so a seed never shifts into the prefix.
const FAKE_HASH_DOMAIN: &[u8] = b"una.testing.fake_hash.int\0";

/// The deterministic `Hashed` value of `std.testing.fake_hash(seed)` (`spec/14 §4`): SHA-256 of
/// the seed's preimage (`fake_hash_preimage`). Equal seeds give equal values, and distinct seeds
/// distinct preimages, so distinct values with the probability of SHA-256. O(d) in the seed's
/// limbs (#1342): the seed is never written in decimal.
pub fn fake_hash(seed: &BigInt) -> Vec<u8> {
    Sha256::digest(fake_hash_preimage(seed)).to_vec()
}

/// What `fake_hash` hashes: the domain, then the seed's hash encoding (`spec/28 §1` rule 2),
/// which is self-delimiting and injective on integers: the byte 0 and 8 bytes of two's
/// complement for a seed in -2^63 to 2^63 - 1, otherwise the byte 1, a sign byte, the length of
/// the magnitude in bytes and the magnitude, little-endian. O(d).
pub fn fake_hash_preimage(seed: &BigInt) -> Vec<u8> {
    let (sign, magnitude) = seed.to_bytes_le();
    let mut preimage = Vec::with_capacity(FAKE_HASH_DOMAIN.len() + 10 + magnitude.len());
    preimage.extend_from_slice(FAKE_HASH_DOMAIN);
    Encoder::new(&mut preimage).big(sign == Sign::Minus, &magnitude);
    preimage
}

/// Lowercase hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes standard base64, with or without padding.
pub fn base64(text: &str) -> Option<Vec<u8>> {
    let mut bits: u32 = 0;
    let mut count = 0;
    let mut out = Vec::new();
    for c in text.trim().chars() {
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ => return None,
        };
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push(((bits >> count) & 0xff) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `2^(64 * limbs - 1) + 1`: an `Int` of exactly `limbs` 64-bit limbs.
    fn int_of_limbs(limbs: usize) -> BigInt {
        (BigInt::from(1) << (64 * limbs - 1)) + BigInt::from(1)
    }

    #[test]
    fn fake_hash_gives_equal_values_for_equal_seeds_only() {
        let big = int_of_limbs(4);
        let seeds = [
            BigInt::from(0),
            BigInt::from(1),
            BigInt::from(-1),
            BigInt::from(255),
            BigInt::from(256),
            BigInt::from(-256),
            BigInt::from(i64::MAX),
            BigInt::from(i64::MIN),
            BigInt::from(i64::MAX) + BigInt::from(1),
            BigInt::from(i64::MIN) - BigInt::from(1),
            BigInt::from(u64::MAX),
            -BigInt::from(u64::MAX),
            BigInt::from(1) << 64,
            big.clone(),
            -big.clone(),
            big.clone() + BigInt::from(1),
            big.clone() << 8,
        ];
        for (i, a) in seeds.iter().enumerate() {
            // Equal seeds, made separately, give equal values.
            let again: BigInt = a.to_string().parse().unwrap();
            assert_eq!(fake_hash(a), fake_hash(&again), "seed {a}");
            assert_eq!(fake_hash(a).len(), 32);
            for b in seeds.iter().skip(i + 1) {
                assert_ne!(fake_hash_preimage(a), fake_hash_preimage(b), "{a} and {b}");
                assert_ne!(fake_hash(a), fake_hash(b), "{a} and {b}");
            }
        }
        // Zero has no sign: `-0` is `0`.
        assert_eq!(fake_hash(&-BigInt::from(0)), fake_hash(&BigInt::from(0)));
        // The preimage is the domain and the seed's hash encoding (`spec/28 §1` rule 2).
        let mut expected = FAKE_HASH_DOMAIN.to_vec();
        expected.push(0);
        expected.extend_from_slice(&(-2i64).to_le_bytes());
        assert_eq!(fake_hash_preimage(&BigInt::from(-2)), expected);
    }

    #[test]
    fn fake_hash_is_linear_in_its_seeds_limbs() {
        // `spec/14 §4`, ADR 0132 rule 16, #1342: seeds of d and 10d limbs. The deterministic check is the
        // preimage: exactly the domain, 10 bytes of framing and 8 bytes per limb, so linear.
        let (d, ten_d) = (2_000, 20_000);
        let small = int_of_limbs(d);
        let large = int_of_limbs(ten_d);
        let framing = FAKE_HASH_DOMAIN.len() + 10;
        assert_eq!(fake_hash_preimage(&small).len(), framing + 8 * d);
        assert_eq!(fake_hash_preimage(&large).len(), framing + 8 * ten_d);
        // The secondary check is the time: at most 30 times as long, each size the best of
        // three batches, so scheduler noise does not fail it (#341).
        let best = |seed: &BigInt| {
            (0..3)
                .map(|_| {
                    let started = std::time::Instant::now();
                    for _ in 0..20 {
                        std::hint::black_box(fake_hash(std::hint::black_box(seed)));
                    }
                    started.elapsed().as_secs_f64()
                })
                .fold(f64::INFINITY, f64::min)
        };
        let floor = 1e-4;
        let growth = best(&large).max(floor) / best(&small).max(floor);
        assert!(growth <= 30.0, "10x the limbs took {growth:.1}x as long");
    }

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(base64("*"), None);
    }

    #[test]
    fn key_files_are_tables_of_base64_keys() {
        let keys = parse_keys("phone_pepper = \"aGVsbG8=\"\n").unwrap();
        assert_eq!(keys.get("phone_pepper"), Some(&b"hello".to_vec()));
        assert!(parse_keys("phone_pepper = 3\n").is_err());
        assert!(parse_keys("phone_pepper = \"*\"\n").is_err());
        assert!(parse_keys("= broken").is_err());
    }

    #[test]
    fn the_serve_driver_is_bounded_by_the_sum_of_once() {
        // #1724, decision S6-D6: the driver's `tasks` statement holds at most the sum of the
        // grants' `/once`; other grants add nothing, and a run with none still has a bound.
        let base = Path::new("/work/app");
        let grants: Vec<Grant> = [
            "serve:http://127.0.0.1:8080;2/once",
            "console",
            "serve:http://127.0.0.1:8081;5/once",
        ]
        .iter()
        .map(|spec| Grant::parse(spec, base).unwrap())
        .collect();
        assert_eq!(Grant::serve_driver_bound(&grants, 4096, None), 7);
        let plain = [Grant::parse("serve:http://127.0.0.1:8080", base).unwrap()];
        assert_eq!(Grant::serve_driver_bound(&plain, 4096, None), 256);
        assert_eq!(Grant::serve_driver_bound(&[Grant::Console], 4096, None), 1);
        // Within the default bound of a `tasks` statement (ADR 0117 rule 7): a sum above
        // `max(64, M)` gives it, and so does a sum above the cap on tasks.
        let large = [Grant::parse("serve:http://127.0.0.1:8080;20000/once", base).unwrap()];
        assert_eq!(Grant::serve_driver_bound(&large, 4096, None), 4096);
        assert_eq!(Grant::serve_driver_bound(&large, 64, None), 64);
        assert_eq!(Grant::serve_driver_bound(&plain, 64, None), 64);
        assert_eq!(Grant::serve_driver_bound(&large, 32768, Some(16382)), 16382);
        assert_eq!(Grant::serve_driver_bound(&grants, 4096, Some(3)), 3);
    }

    #[test]
    fn serve_grants_resolve_their_tls_files() {
        let base = Path::new("/work/app");
        let Some(Grant::Serve {
            grant,
            certificate,
            key,
        }) = Grant::parse(
            "serve:https://127.0.0.1:8443;certificate=tls/c.pem;key=/etc/k.pem",
            base,
        )
        else {
            panic!("a serve grant")
        };
        assert_eq!(grant.port, 8443);
        assert_eq!(certificate, Some(PathBuf::from("/work/app/tls/c.pem")));
        assert_eq!(key, Some(PathBuf::from("/etc/k.pem")));
        let plain = Grant::parse("serve:http://[::1]:8080", base).unwrap();
        assert_eq!(plain.capability(), "HttpServer");
        assert_eq!(Grant::parse("serve:http://[::1]:8080;peer=0", base), None);
        assert_eq!(
            invalid_grant("serve:http://localhost:80;peer=0"),
            "`serve:http://localhost:80;peer=0` is not a valid serve grant: `localhost` is not an IPv4 address; a serve grant names an IP address, never a host name, and IPv6 goes in brackets; `peer` is a number from 1 to 1048576, or `none`, not `0`"
        );
        assert_eq!(
            invalid_grant("network"),
            "`network` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]"
        );
    }

    #[test]
    fn grants_parse_and_resolve_their_paths() {
        let base = Path::new("/work/app");
        assert_eq!(Grant::parse("console", base), Some(Grant::Console));
        assert_eq!(
            Grant::parse("files:./data:read,write", base),
            Some(Grant::Files {
                prefix: PathBuf::from("/work/app/data"),
                read: true,
                write: true
            })
        );
        assert_eq!(
            Grant::parse("keys:keys.toml", base),
            Some(Grant::Keys {
                file: PathBuf::from("/work/app/keys.toml")
            })
        );
        assert_eq!(Grant::parse("files:./data:execute", base), None);
        assert_eq!(Grant::parse("network", base), None);
        // Limits on `http` grants (`spec/27 §9`), valid or not; none on other grants.
        let limited = Grant::parse("http:https://h:get;3/second;waiting=5", base).unwrap();
        assert!(limited.limited());
        assert!(!Grant::parse("http:https://h", base).unwrap().limited());
        assert_eq!(Grant::parse("http:https://h;0/second", base), None);
        assert_eq!(Grant::parse("http:https://h;1/once;2/once", base), None);
        assert_eq!(Grant::parse("console;1/once", base), None);
    }

    #[test]
    fn the_prefix_of_a_files_grant_is_never_the_empty_path_2144() {
        // #2144: `files:.:<modes>` relative to the current directory (a `--grant` flag, the
        // embedded grants of an executable: the base ``) or to a package root written `.`
        // (`una run .`) held the empty path, which no host resolves, so every operation under
        // the grant ended with `NotFound`. The prefix is `.` there, and every other prefix is
        // as it was.
        let prefix = |spec: &str, base: &str| match Grant::parse(spec, Path::new(base)) {
            Some(Grant::Files { prefix, .. }) => prefix,
            other => panic!("{spec} in `{base}`: {other:?}"),
        };
        for base in ["", ".", "./", "./."] {
            for path in [".", "./", "./.", "data/..", "./data/.."] {
                let spec = format!("files:{path}:write");
                assert_eq!(prefix(&spec, base), Path::new("."), "{spec} in `{base}`");
            }
            for path in ["data", "./data", "data/", "./data/.", "x/../data"] {
                let spec = format!("files:{path}:read");
                assert_eq!(prefix(&spec, base), Path::new("data"), "{spec} in `{base}`");
            }
            assert_eq!(prefix("files:..:read", base), Path::new(".."));
            assert_eq!(prefix("files:../data:read", base), Path::new("../data"));
            assert_eq!(prefix("files:/srv/data:read", base), Path::new("/srv/data"));
            assert_eq!(prefix("files:/:read", base), Path::new("/"));
        }
        assert_eq!(prefix("files:.:read", "/work/app"), Path::new("/work/app"));
        assert_eq!(prefix("files:.:read", "app"), Path::new("app"));
        assert_eq!(prefix("files:..:read", "app"), Path::new("."));
        // A `..` never removes a `..` before it: a spec that goes up two or more levels names
        // that directory, not the current one or one in it.
        for base in ["", ".", "./"] {
            for (path, expected) in [
                ("../..", "../.."),
                ("../../", "../.."),
                ("../../x", "../../x"),
                ("../../../x", "../../../x"),
                ("a/../..", ".."),
                ("a/../../..", "../.."),
                ("../a/../..", "../.."),
                ("../../a/..", "../.."),
            ] {
                let spec = format!("files:{path}:read");
                assert_eq!(
                    prefix(&spec, base),
                    Path::new(expected),
                    "{spec} in `{base}`"
                );
            }
        }
        assert_eq!(prefix("files:..:read", ".."), Path::new("../.."));
        assert_eq!(prefix("files:.:read", "../.."), Path::new("../.."));
        assert_eq!(prefix("files:../..:read", "../pkg"), Path::new("../.."));
        assert_eq!(prefix("files:../x:read", "../pkg"), Path::new("../x"));
        // More `..` than an absolute base has components: the rest stays after the root,
        // where the host reads `/..` as `/`, so the prefix is a path from the root and never
        // a relative one.
        assert_eq!(prefix("files:../..:read", "/work"), Path::new("/.."));
        assert_eq!(prefix("files:../../x:read", "/work"), Path::new("/../x"));
        assert_eq!(prefix("files:..:read", "/"), Path::new("/.."));
        assert_eq!(prefix("files:/../../x:read", ""), Path::new("/../../x"));
        assert_eq!(prefix("files:..:read", "/work"), Path::new("/"));
        assert_eq!(normalize(Path::new("../..")), Path::new("../.."));
        assert_eq!(normalize(Path::new("")), Path::new("."));
        assert_eq!(normalize(Path::new("/")), Path::new("/"));
        // No path is a spec without a path, as before.
        assert_eq!(Grant::parse("files::read", Path::new("")), None);
    }

    #[test]
    fn limited_flags_need_a_label_at_most_internal() {
        let flags = vec![
            "console".to_owned(),
            "http:https://h".to_owned(),
            "http:https://h:get;1/once".to_owned(),
        ];
        assert_eq!(
            check_limited_flags(&flags, Some(("internal", true))),
            Ok(())
        );
        assert_eq!(
            check_limited_flags(&flags, Some(("secret", false))),
            Err(
                "the grant `http:https://h:get;1/once` has limits, but this program's requests are timed by `secret` data"
                    .to_owned()
            )
        );
        assert!(check_limited_flags(&flags, None).is_err());
        // Without limits, nothing is refused.
        assert_eq!(check_limited_flags(&flags[..2], None), Ok(()));
    }
}
