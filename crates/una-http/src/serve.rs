//! What the host does for the `serve:` grants of a run before `main` (`spec/31 §2` rules 6 to
//! 8, ADR 0123 rules 6, 7, 9 and 19), for both engines: the overlap check of the final grants,
//! the limit of open descriptors, the TLS files opened once and checked on the open
//! descriptor, and one listening socket per grant.
//!
//! The sockets are held until the run ends. Nothing accepts on them yet: the serve threads of
//! ADR 0123 step 4, on the reactor of ADR 0117, take them over. The TLS files are loaded and
//! checked, and the TLS server is described by [`TlsPolicy`], but no provider is bound and no
//! server configuration is built, pending the decision on the TLS provider (#1059); that step
//! also checks that the key matches the certificate.

pub mod fault;
pub mod gate;
pub mod gcra;
pub mod grant;
pub mod live;
pub mod net;
pub mod peer;
pub mod place;
pub mod pressure;
pub mod program;
pub mod rights;
pub mod run;
#[cfg(feature = "test-hooks")]
pub mod script;
pub mod script_file;
pub mod server;
#[cfg(feature = "test-hooks")]
pub mod sim;
pub mod thread;
pub mod trace;

use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use una_core::http::serve::{ServeGrant, TlsPolicy, overlapping};
use una_sys::server::Identity;

/// The line a run that serves writes on stderr for each `serve:` grant once its serve threads
/// have started, before its first request, the same in both engines (`spec/31 §6` rule 14):
/// the address and port it listens on, and whether `run grants` of the package or a `--grant`
/// flag granted it, so a package never opens a listener unseen.
pub fn serving_notice(grant: &una_core::http::serve::ServeGrant, by_package: bool) -> String {
    let by = if by_package {
        "the package"
    } else {
        "a `--grant` flag"
    };
    format!("serving on {} (granted by {by})", grant.listener())
}

/// Installs, once per process, the panic hook of the serve engine, which keeps the panics it
/// contains off standard error (`spec/31 §6` rule 13), chained to the hook in place. A run calls
/// it on the thread that starts it, before any serve thread or worker starts, after any hook of
/// its own ([`fault`]'s precondition).
pub fn install_panic_hook() {
    fault::install_hook();
}

/// The largest TLS file the host reads, certificate chain or key.
pub const MAX_TLS_FILE: u64 = 1 << 20;

/// The descriptors a run keeps for `Console`, the reactor and its pair, the wake pipe and the
/// runtime (`spec/31 §6` rule 14).
pub const SPARE_DESCRIPTORS: u64 = 64;

/// The sockets a serving run keeps for its own `Http` requests (`spec/31 §6` rule 14).
pub const HTTP_DESCRIPTORS: u64 = 256;

/// The descriptors a run takes besides its connections and its `Http` sockets, set by
/// [`prepare`]: what the server and the runtime hold, so the cap on the sockets of `Http`
/// requests leaves them (ADR 0117 slice S7). 0 in a run that serves nothing.
static RESERVED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The descriptors the serve grants and the runtime hold of the process, besides the
/// [`HTTP_DESCRIPTORS`] kept for `Http` requests; 0 before [`prepare`] and in a run with no
/// `serve:` grant.
pub fn reserved_descriptors() -> u64 {
    RESERVED.load(std::sync::atomic::Ordering::Acquire)
}

/// The descriptors a run with these `serve:` grants needs open at once (`spec/31 §6` rule 14):
/// the `connections` of every grant, one listener per grant, the accept poller, a poller and a
/// wake pair for each of the `serve_threads`, [`HTTP_DESCRIPTORS`] for `Http` requests, the
/// `file_threads` that `Files` helpers may hold open, and [`SPARE_DESCRIPTORS`]. Saturating:
/// the sum never wraps.
pub fn descriptor_target(
    grants: &[ServeInput<'_>],
    serve_threads: usize,
    file_threads: usize,
) -> u64 {
    let connections = grants.iter().fold(0u64, |sum, g| {
        sum.saturating_add(g.grant.limits.connections)
    });
    let listeners = grants.len() as u64;
    let threads = (serve_threads as u64).saturating_mul(3);
    connections
        .saturating_add(listeners)
        .saturating_add(1)
        .saturating_add(threads)
        .saturating_add(HTTP_DESCRIPTORS)
        .saturating_add(file_threads as u64)
        .saturating_add(SPARE_DESCRIPTORS)
}

/// The largest backlog asked of the kernel, which caps it lower on most systems.
const MAX_BACKLOG: u64 = 65_535;

/// A `serve:` grant of a run, with the paths of its TLS files resolved.
#[derive(Clone, Copy, Debug)]
pub struct ServeInput<'g> {
    /// The grant.
    pub grant: &'g ServeGrant,
    /// The certificate chain's path, on an `https` grant.
    pub certificate: Option<&'g Path>,
    /// The private key's path, on an `https` grant.
    pub key: Option<&'g Path>,
}

/// The certificate chain and the private key of an `https` grant, read and parsed. The key
/// never leaves the host: this type prints neither.
pub struct TlsMaterial {
    /// The chain, leaf first.
    pub chain: Vec<CertificateDer<'static>>,
    /// The private key.
    pub key: PrivateKeyDer<'static>,
    /// The pinned configuration the TLS server is built with (#1059).
    pub policy: TlsPolicy,
}

impl std::fmt::Debug for TlsMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsMaterial")
            .field("chain", &self.chain.len())
            .field("key", &"<redacted>")
            .field("policy", &self.policy)
            .finish()
    }
}

/// A grant's listening socket, and its TLS material on an `https` grant.
#[derive(Debug)]
pub struct Listener {
    /// The grant's address and port, as the grant writes them.
    pub listener: String,
    /// The bound, listening socket.
    pub socket: TcpListener,
    /// The TLS material, on an `https` grant.
    pub tls: Option<TlsMaterial>,
}

/// What [`prepare`] made ready: one listener per grant, in the order of the grants.
#[derive(Debug, Default)]
pub struct Prepared {
    /// The listeners.
    pub listeners: Vec<Listener>,
}

/// Prepares the `serve:` grants of a run before `main`, in the order of `spec/31 §2` rule 7,
/// for `serve_threads` serve threads and `file_threads` helpers of `Files`.
/// Every failure is a usage error, its message one sentence. A run without `serve:` grants
/// makes no system call here.
pub fn prepare(
    grants: &[ServeInput<'_>],
    serve_threads: usize,
    file_threads: usize,
) -> Result<Prepared, String> {
    if grants.is_empty() {
        return Ok(Prepared::default());
    }
    if let Some(&index) =
        overlapping(grants.iter().map(|g| (g.grant.address, g.grant.port))).first()
        && let Some(input) = grants.get(index)
    {
        return Err(format!(
            "the serve grant on `{}` overlaps another serve grant of the run: each address and port is granted once, and a wildcard shares its port with no grant of its family",
            input.grant.listener()
        ));
    }
    let wanted = descriptor_target(grants, serve_threads, file_threads);
    let limit = una_sys::server::raise_descriptor_limit(wanted)
        .map_err(|error| format!("cannot read the limit of open descriptors: {error}"))?;
    if limit < wanted {
        return Err(format!(
            "the serve grants need {wanted} open descriptors, their `connections`, the listeners, the serve threads, `Http` requests, `Files` and the runtime together, and the run cannot raise its limit above {limit}"
        ));
    }
    // What the cap on `Http` sockets leaves for the server and the runtime.
    RESERVED.store(
        wanted.saturating_sub(HTTP_DESCRIPTORS),
        std::sync::atomic::Ordering::Release,
    );
    // The run's user and groups, read once when a key file is to be checked.
    let identity = if grants.iter().any(|input| input.grant.tls) {
        Some(
            una_sys::server::identity()
                .map_err(|error| format!("cannot read the run's user and groups: {error}"))?,
        )
    } else {
        None
    };
    let mut materials = Vec::with_capacity(grants.len());
    for input in grants {
        materials.push(match input.grant.tls_policy() {
            None => None,
            Some(policy) => {
                let (Some(certificate), Some(key), Some(who)) =
                    (input.certificate, input.key, &identity)
                else {
                    return Err(format!(
                        "the https grant on `{}` names no certificate or key",
                        input.grant.listener()
                    ));
                };
                // The key first: its checks are the ones that keep a secret.
                let key = load_key(key, who)?;
                let chain = load_chain(certificate)?;
                Some(TlsMaterial { chain, key, policy })
            }
        });
    }
    let mut listeners = Vec::with_capacity(grants.len());
    for (input, tls) in grants.iter().zip(materials) {
        let address = SocketAddr::new(input.grant.address, input.grant.port);
        let backlog =
            u32::try_from(input.grant.limits.connections.min(MAX_BACKLOG)).unwrap_or(u32::MAX);
        let socket = una_sys::server::listen(address, backlog).map_err(|error| {
            format!(
                "cannot listen on `{}` for its serve grant: {error}",
                input.grant.listener()
            )
        })?;
        listeners.push(Listener {
            listener: input.grant.listener(),
            socket,
            tls,
        });
    }
    Ok(Prepared { listeners })
}

/// Opens `path` once, following symbolic links and without blocking on a FIFO, and checks the
/// open descriptor: a regular file of at most [`MAX_TLS_FILE`] bytes.
fn open_checked(path: &Path, what: &str) -> Result<(File, Metadata), String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(una_sys::server::OPEN_FLAGS)
        .open(path)
        .map_err(|error| format!("cannot open the {what} `{}`: {error}", path.display()))?;
    // `fstat` on the descriptor, never the path, so the file checked is the file read.
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot read the {what} `{}`: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "the {what} `{}` is not a regular file",
            path.display()
        ));
    }
    if metadata.len() > MAX_TLS_FILE {
        return Err(format!(
            "the {what} `{}` is larger than 1 MiB",
            path.display()
        ));
    }
    Ok((file, metadata))
}

/// Reads at most [`MAX_TLS_FILE`] bytes from the open file, and fails when it holds more.
fn read_bounded(file: File, path: &Path, what: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(MAX_TLS_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read the {what} `{}`: {error}", path.display()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TLS_FILE {
        clear(&mut bytes);
        return Err(format!(
            "the {what} `{}` is larger than 1 MiB",
            path.display()
        ));
    }
    Ok(bytes)
}

/// The certificate chain of `path`: its `CERTIFICATE` blocks, leaf first, at least one.
fn load_chain(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let what = "certificate chain";
    let (file, _) = open_checked(path, what)?;
    let bytes = read_bounded(file, path, what)?;
    let chain = CertificateDer::pem_slice_iter(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("the {what} `{}` is not valid PEM: {error}", path.display()))?;
    if chain.is_empty() {
        return Err(format!(
            "the {what} `{}` holds no `CERTIFICATE` block",
            path.display()
        ));
    }
    Ok(chain)
}

/// The private key of `path`, after the checks of `spec/31 §2` rule 8 on its open descriptor.
fn load_key(path: &Path, who: &Identity) -> Result<PrivateKeyDer<'static>, String> {
    let what = "private key";
    let (file, metadata) = open_checked(path, what)?;
    if let Some(problem) = key_file_problem(metadata.mode(), metadata.uid(), metadata.gid(), who) {
        return Err(format!("the {what} `{}` {problem}", path.display()));
    }
    let mut bytes = read_bounded(file, path, what)?;
    let key = PrivateKeyDer::from_pem_slice(&bytes);
    clear(&mut bytes);
    // The message names the file, never its content.
    key.map_err(|_| {
        format!(
            "the {what} `{}` holds no PEM private key in PKCS#8, SEC1 or PKCS#1 form",
            path.display()
        )
    })
}

/// Why a key file with this mode, owner and group may not be used by `who`, or `None` when it
/// may (`spec/31 §2` rule 8): owned by the run's effective user or by root, no bit for others
/// and no write bit for the group, and the group's read bit only for a group of the run.
pub fn key_file_problem(mode: u32, uid: u32, gid: u32, who: &Identity) -> Option<String> {
    if uid != who.uid && uid != 0 {
        return Some(format!(
            "is owned by user {uid}, neither the run's user ({}) nor root",
            who.uid
        ));
    }
    let permissions = mode & 0o7777;
    if permissions & 0o027 != 0 {
        return Some(format!(
            "has mode {permissions:04o}: a key file gives others no access and its group no write access, such as 0400, 0600 or 0440"
        ));
    }
    if permissions & 0o040 != 0 && gid != who.gid && !who.groups.contains(&gid) {
        return Some(format!(
            "is readable by group {gid}, which is not a group of the run"
        ));
    }
    None
}

/// Overwrites `bytes` with zeros, in a way the compiler keeps.
fn clear(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&*bytes);
}

#[cfg(test)]
mod tests;
