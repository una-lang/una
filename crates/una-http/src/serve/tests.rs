//! The work before `main` for `serve:` grants: key files checked on the open descriptor,
//! listeners bound per grant, overlaps and the descriptor limit refused. Certificates are made
//! at test time; the repository holds no private key.

use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::*;

/// A fresh directory of this test process for `name`.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-http-serve-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A certificate for `localhost` and its key, as PEM.
fn pem_pair() -> (String, String) {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    (cert.pem(), key.serialize_pem())
}

fn write(path: &Path, content: &[u8], mode: u32) {
    std::fs::write(path, content).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// A port the kernel had free a moment ago.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn grant(spec: &str) -> ServeGrant {
    ServeGrant::parse(spec).unwrap()
}

fn plain(grant: &ServeGrant) -> ServeInput<'_> {
    ServeInput {
        grant,
        certificate: None,
        key: None,
    }
}

fn me() -> Identity {
    una_sys::server::identity().unwrap()
}

#[test]
fn key_files_are_checked_by_owner_group_and_mode() {
    let who = Identity {
        uid: 501,
        gid: 20,
        groups: vec![20, 12, 61],
    };
    for (mode, uid, gid) in [
        (0o100400, 501, 20),
        (0o100600, 501, 99),
        (0o100400, 0, 0),
        (0o100440, 0, 12),
        (0o100440, 501, 20),
        (0o100640, 0, 61),
        (0o104400, 501, 20),
    ] {
        assert_eq!(key_file_problem(mode, uid, gid, &who), None, "{mode:o} {uid} {gid}");
    }
    for (mode, uid, gid, problem) in [
        (
            0o100644,
            501,
            20,
            "has mode 0644: a key file gives others no access and its group no write access, such as 0400, 0600 or 0440",
        ),
        (0o100604, 501, 20, "has mode 0604: a key file gives others no access and its group no write access, such as 0400, 0600 or 0440"),
        (0o100620, 501, 20, "has mode 0620: a key file gives others no access and its group no write access, such as 0400, 0600 or 0440"),
        (0o100401, 501, 20, "has mode 0401: a key file gives others no access and its group no write access, such as 0400, 0600 or 0440"),
        (0o100400, 502, 20, "is owned by user 502, neither the run's user (501) nor root"),
        (0o100440, 0, 99, "is readable by group 99, which is not a group of the run"),
    ] {
        assert_eq!(
            key_file_problem(mode, uid, gid, &who).as_deref(),
            Some(problem),
            "{mode:o} {uid} {gid}"
        );
    }
    // A key owned by another user is refused even for a run of root's.
    let root = Identity {
        uid: 0,
        gid: 0,
        groups: vec![],
    };
    assert!(key_file_problem(0o100400, 501, 20, &root).is_some());
}

#[test]
fn no_grant_needs_no_work() {
    assert!(prepare(&[]).unwrap().listeners.is_empty());
}

#[test]
fn each_grant_binds_its_listener_before_main() {
    let port = free_port();
    let g = grant(&format!("serve:http://127.0.0.1:{port};connections=8"));
    let prepared = prepare(&[plain(&g)]).unwrap();
    let [listener] = prepared.listeners.as_slice() else {
        panic!("one listener")
    };
    assert_eq!(listener.listener, format!("127.0.0.1:{port}"));
    assert!(listener.tls.is_none());
    // A peer's connection waits in the backlog until the serve threads accept it.
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
    // The address stays the run's: a second run cannot bind it.
    let error = prepare(&[plain(&g)]).unwrap_err();
    assert!(
        error.starts_with(&format!("cannot listen on `127.0.0.1:{port}` for its serve grant: ")),
        "{error}"
    );
}

#[test]
fn overlapping_grants_of_a_run_are_refused() {
    let port = free_port();
    let a = grant(&format!("serve:http://127.0.0.1:{port}"));
    let b = grant(&format!("serve:http://0.0.0.0:{port}"));
    assert_eq!(
        prepare(&[plain(&a), plain(&b)]).unwrap_err(),
        format!(
            "the serve grant on `0.0.0.0:{port}` overlaps another serve grant of the run: each address and port is granted once, and a wildcard shares its port with no grant of its family"
        )
    );
}

#[test]
fn a_descriptor_limit_the_run_cannot_reach_is_refused() {
    let a = grant(&format!("serve:http://127.0.0.1:{};connections=1048576", free_port()));
    let b = grant(&format!("serve:http://127.0.0.1:{};connections=1048576", free_port()));
    let error = prepare(&[plain(&a), plain(&b)]).unwrap_err();
    assert!(
        error.starts_with("the serve grants need 2097216 open descriptors"),
        "{error}"
    );
}

#[test]
fn tls_files_are_read_from_checked_descriptors() {
    let dir = scratch("tls");
    let (cert, key) = pem_pair();
    let chain = dir.join("chain.pem");
    let key_file = dir.join("key.pem");
    write(&chain, cert.as_bytes(), 0o644);
    write(&key_file, key.as_bytes(), 0o600);
    let g = grant(&format!(
        "serve:https://127.0.0.1:{};certificate=c;key=k",
        free_port()
    ));
    let input = |certificate: &Path, key: &Path| -> Result<Prepared, String> {
        prepare(&[ServeInput {
            grant: &g,
            certificate: Some(certificate),
            key: Some(key),
        }])
    };
    let prepared = input(&chain, &key_file).unwrap();
    let tls = prepared.listeners[0].tls.as_ref().unwrap();
    assert_eq!(tls.chain.len(), 1);
    assert_eq!(tls.policy, TlsPolicy::pinned(false));
    // The key is in no message: its `Debug` is redacted.
    let shown = format!("{prepared:?}");
    assert!(shown.contains("<redacted>"), "{shown}");
    assert!(!shown.contains("PRIVATE"), "{shown}");
    drop(prepared);

    // A secret volume's link to a key of mode 0400 is followed.
    let linked = dir.join("linked.pem");
    let target = dir.join("target.pem");
    write(&target, key.as_bytes(), 0o400);
    std::os::unix::fs::symlink(&target, &linked).unwrap();
    input(&chain, &linked).unwrap();

    // Mode 0644, the default of a Kubernetes secret, is refused, and so is any bit for others.
    for mode in [0o644, 0o604, 0o620, 0o401] {
        let open = dir.join(format!("open-{mode:o}.pem"));
        write(&open, key.as_bytes(), mode);
        let error = input(&chain, &open).unwrap_err();
        assert!(error.contains(&format!("has mode {mode:04o}")), "{error}");
        assert!(!error.contains("PRIVATE"), "{error}");
    }

    // A directory, a device and a FIFO are not regular files; the FIFO does not block the open.
    let error = input(&chain, &dir).unwrap_err();
    assert!(error.ends_with("is not a regular file"), "{error}");
    let error = input(&chain, Path::new("/dev/null")).unwrap_err();
    assert!(error.ends_with("is not a regular file"), "{error}");
    let fifo = dir.join("fifo");
    if std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .is_ok_and(|status| status.success())
    {
        let error = input(&chain, &fifo).unwrap_err();
        assert!(error.ends_with("is not a regular file"), "{error}");
        let error = input(&fifo, &key_file).unwrap_err();
        assert!(error.ends_with("is not a regular file"), "{error}");
    }

    // Too large, missing, or not what it should hold.
    let large = dir.join("large.pem");
    write(&large, &vec![b'#'; 1 << 20 | 1], 0o600);
    let error = input(&chain, &large).unwrap_err();
    assert!(error.ends_with("is larger than 1 MiB"), "{error}");
    let error = input(&chain, &dir.join("missing.pem")).unwrap_err();
    assert!(error.starts_with("cannot open the private key"), "{error}");
    let error = input(&key_file, &key_file).unwrap_err();
    assert!(error.ends_with("holds no `CERTIFICATE` block"), "{error}");
    let error = input(&chain, &chain_as_key(&dir, &cert)).unwrap_err();
    assert!(
        error.ends_with("holds no PEM private key in PKCS#8, SEC1 or PKCS#1 form"),
        "{error}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The certificate written where a key should be, with a key file's mode.
fn chain_as_key(dir: &Path, cert: &str) -> PathBuf {
    let path = dir.join("not-a-key.pem");
    write(&path, cert.as_bytes(), 0o600);
    path
}

#[test]
fn the_runs_identity_owns_the_files_it_writes() {
    let dir = scratch("identity");
    let file = dir.join("mine");
    write(&file, b"x", 0o600);
    let metadata = std::fs::metadata(&file).unwrap();
    assert_eq!(
        key_file_problem(metadata.mode(), metadata.uid(), metadata.gid(), &me()),
        None
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
