//! The TLS client of `https` requests (`spec/24 §3` rule 4, ADR 0074): `rustls` with its `ring`
//! provider and the Mozilla roots of `webpki-roots`, which [`install_tls`] installs into
//! `una-http-core`'s client.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use una_http_core::client::{self, Error, Tls, TlsStream};

/// The TLS client over `rustls`.
struct Rustls;

/// The one TLS client of the process.
static RUSTLS: Rustls = Rustls;

/// Installs the TLS client of this crate for every later `https` request of the process. It is
/// idempotent and costs one atomic load once installed.
pub fn install_tls() {
    client::install_tls(&RUSTLS);
}

/// A TLS connection of `rustls` over its socket.
struct Stream(rustls::StreamOwned<rustls::ClientConnection, TcpStream>);

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl TlsStream for Stream {
    fn socket(&self) -> &TcpStream {
        &self.0.sock
    }
}

impl Tls for Rustls {
    fn connect(
        &self,
        name: &str,
        stream: TcpStream,
        deadline: Instant,
    ) -> Result<Box<dyn TlsStream>, Error> {
        let server =
            rustls::pki_types::ServerName::try_from(name.to_owned()).map_err(|_| Error::BadUrl)?;
        let connection =
            rustls::ClientConnection::new(tls_config(), server).map_err(|_| Error::Tls)?;
        let mut tls = rustls::StreamOwned::new(connection, stream);
        // The handshake, within the deadline.
        while tls.conn.is_handshaking() {
            let left = client::left(deadline)?;
            let _ = tls.sock.set_read_timeout(Some(left));
            let _ = tls.sock.set_write_timeout(Some(left));
            tls.conn.complete_io(&mut tls.sock).map_err(|error| {
                match client::io_error(&error, deadline) {
                    Error::TimedOut => Error::TimedOut,
                    Error::Unreachable if error.kind() == std::io::ErrorKind::InvalidData => {
                        Error::Tls
                    }
                    other => other,
                }
            })?;
        }
        Ok(Box::new(Stream(tls)))
    }

    fn failed(&self, error: &std::io::Error) -> bool {
        error
            .get_ref()
            .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
    }
}

/// The TLS settings: the Mozilla roots compiled in (`spec/24 §3` rule 4); in debug builds only,
/// also the roots of the PEM file that `UNA_HTTP_TEST_ROOTS` names, for the tests' local
/// servers. Release builds have no such hook.
fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    Arc::clone(CONFIG.get_or_init(|| {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        #[cfg(debug_assertions)]
        let roots = {
            let mut roots = roots;
            test_roots(&mut roots);
            roots
        };
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map(|builder| builder.with_root_certificates(roots).with_no_client_auth());
        match config {
            Ok(config) => Arc::new(config),
            // INVARIANT: the ring provider supports the default versions; an empty config
            // would refuse every handshake, which fails safe.
            Err(_) => Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(rustls::RootCertStore::empty())
                    .with_no_client_auth(),
            ),
        }
    }))
}

/// The PEM file of test roots that [`trust_test_roots`] names, in debug builds only.
#[cfg(debug_assertions)]
static TEST_ROOTS: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Debug builds only: trusts the roots of the PEM file at `path` too, for tests that serve TLS
/// locally. It takes effect only before the first TLS connection of the process.
#[cfg(debug_assertions)]
pub fn trust_test_roots(path: &std::path::Path) {
    let _ = TEST_ROOTS.set(path.to_owned());
}

#[cfg(debug_assertions)]
#[allow(clippy::disallowed_methods)]
fn test_roots(roots: &mut rustls::RootCertStore) {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;
    let path = match TEST_ROOTS.get() {
        Some(path) => path.clone(),
        None => match std::env::var_os("UNA_HTTP_TEST_ROOTS") {
            Some(path) => std::path::PathBuf::from(path),
            None => return,
        },
    };
    let Ok(certs) = CertificateDer::pem_file_iter(path) else {
        return;
    };
    for cert in certs.flatten() {
        let _ = roots.add(cert);
    }
}
