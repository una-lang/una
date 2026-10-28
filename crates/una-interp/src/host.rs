//! Host capabilities (`spec/14 §3`): the world that performs input and output, the grants of
//! `una run`, the confinement of `Files` to its prefix, and HMAC-SHA-256 for `Keys`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

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

/// `path` with `.` removed and `..` applied, lexically.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The path that a `Files` operation on `relative` opens under `prefix`, or `None` when the
/// operation is `Denied` (`spec/14 §3`, ADR 0038 D9):
///
/// 1. `relative` is normalized lexically, without following links; an absolute path, a `..`
///    that leaves the prefix, and a path with no final component are denied;
/// 2. every intermediate directory, the prefix included, is resolved with its links followed
///    and must lie inside the canonical prefix; a dangling link there is denied, and a
///    component that does not exist leaves the rest to the operation, which fails with
///    `NotFound`;
/// 3. the final component is never followed: a symbolic link there, dangling or not, is
///    denied. The world opens the returned path without creating a file through a link.
///
/// Races between this check and the operation are outside the v0.1 model (D9 rule 4).
pub fn confine(world: &mut dyn World, prefix: &Path, relative: &str) -> Option<PathBuf> {
    // Rule 1: lexical normalization.
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
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
    // D-133: a path that names the prefix itself (``, `.`, `a/..`) has no final
    // component to open, and is denied.
    let (last, directories) = parts.split_last()?;
    // Rule 2: every intermediate directory resolves inside the canonical prefix.
    // D-134: each one is checked, not only the last, so a path never passes through
    // a directory outside the prefix even when it comes back inside.
    let mut lexical = prefix.to_path_buf();
    let Some(root) = world.canonical(prefix) else {
        return missing(world, &lexical, prefix, &parts);
    };
    let mut resolved = root.clone();
    for part in directories {
        lexical.push(part);
        match world.canonical(&lexical) {
            Some(directory) if directory.starts_with(&root) => resolved = directory,
            Some(_) => return None,
            None => return missing(world, &lexical, prefix, &parts),
        }
    }
    // Rule 3: the final component is never followed.
    let target = resolved.join(last);
    (!world.is_link(&target)).then_some(target)
}

/// The outcome of [`confine`] when the intermediate directory `lexical` does not resolve: a
/// dangling link is denied, and a missing directory yields the lexical path, whose operation
/// then fails with `NotFound` without touching anything outside the prefix.
fn missing(
    world: &mut dyn World,
    lexical: &Path,
    prefix: &Path,
    parts: &[&std::ffi::OsStr],
) -> Option<PathBuf> {
    if world.is_link(lexical) {
        return None;
    }
    let mut out = prefix.to_path_buf();
    out.extend(parts);
    Some(out)
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

/// HMAC-SHA-256 (RFC 2104) over `sha2`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
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

/// The deterministic `Hashed` value of `std.testing.fake_hash(seed)` (`spec/14 §4`).
pub fn fake_hash(seed: &str) -> Vec<u8> {
    Sha256::digest(format!("una.testing.fake_hash:{seed}").as_bytes()).to_vec()
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

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2 of RFC 4231.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block.
        let key = [0xaa_u8; 131];
        let mac = hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            hex(&mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
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
