//! `file` URIs, as LSP names documents.

use std::path::{Path, PathBuf};

/// The path of a `file` URI, percent-decoded.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%'
            && let (Some(high), Some(low)) = (bytes.get(index + 1), bytes.get(index + 2))
            && let Ok(value) = u8::from_str_radix(&format!("{}{}", *high as char, *low as char), 16)
        {
            decoded.push(value);
            index += 3;
            continue;
        }
        decoded.push(byte);
        index += 1;
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// The `file` URI of an absolute path, with the bytes outside the unreserved set and `/`
/// percent-encoded.
pub fn uri_of(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_round_trips_through_its_uri() {
        let path = Path::new("/tmp/a b/caf\u{e9}.una");
        assert_eq!(uri_of(path), "file:///tmp/a%20b/caf%C3%A9.una");
        assert_eq!(path_of(&uri_of(path)).as_deref(), Some(path));
    }
}
