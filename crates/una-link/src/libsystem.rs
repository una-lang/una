//! libSystem's export list (ADR 0124 rule 23, decision D3): the symbols an executable may import,
//! generated once from a pinned SDK by `scripts/libsystem-exports.sh` and embedded. A symbol that
//! no input defines is bound to libSystem only if this list holds it.

use std::collections::BTreeSet;
use std::sync::OnceLock;

/// The list as `scripts/libsystem-exports.sh` wrote it.
const LIST: &str = include_str!("../../../supply-chain/libsystem/exports-arm64.txt");

/// The set of exported names.
pub(crate) fn exports() -> &'static BTreeSet<&'static [u8]> {
    static SET: OnceLock<BTreeSet<&'static [u8]>> = OnceLock::new();
    SET.get_or_init(|| {
        LIST.lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .map(str::as_bytes)
            .collect()
    })
}

/// libSystem's current version, packed as `xxxx.yy.zz`, from the list's header.
pub(crate) fn current_version() -> u32 {
    let text = LIST
        .lines()
        .find_map(|line| line.strip_prefix("# libsystem-current-version "))
        .unwrap_or("1");
    let mut parts = text.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major << 16) | ((minor & 0xff) << 8) | (patch & 0xff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_holds_the_c_library_and_no_linker_directives() {
        let set = exports();
        assert!(set.len() > 5000);
        for name in [
            &b"_write"[..],
            b"__tlv_bootstrap",
            b"___chkstk_darwin",
            b"_pthread_create",
        ] {
            assert!(set.contains(name), "{}", String::from_utf8_lossy(name));
        }
        assert!(!set.iter().any(|n| n.starts_with(b"$ld$")));
        assert!(!set.contains(&b"_una_rt_print"[..]));
        assert_eq!(current_version(), 1311 << 16);
    }

    #[test]
    fn the_list_is_the_one_its_lock_pins() {
        use sha2::{Digest, Sha256};
        let lock = include_str!("../../../supply-chain/libsystem.lock");
        let pinned = lock
            .lines()
            .find_map(|l| l.strip_prefix("exports_sha256 = \""))
            .and_then(|l| l.strip_suffix('"'))
            .unwrap_or_default();
        let digest: String = Sha256::digest(LIST.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest, pinned);
    }
}
