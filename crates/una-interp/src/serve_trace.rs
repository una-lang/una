//! The reduced form of the values of the events of the host's `HttpServer` (ADR 0123 rule 1,
//! `spec/31 §1` rule 7), one implementation for both engines: the interpreter's renderer and
//! the native runtime's call it on the fields of the server's kinds of `std.host`, where the
//! event is built, so that no path writes the clear form. The redaction of labels applies on
//! top, as for every event (`spec/11 §5`).
//!
//! - The value of a header named `authorization`, `proxy-authorization`, `cookie`, `set-cookie`,
//!   `x-api-key` or `x-auth-token`, in any letter case, in a request or in a reply, is
//!   `<credential>`.
//! - A body, of a request or of a reply, is `{"length": n, "hmac": "<hex>"}`: its length in
//!   bytes and an HMAC-SHA-256 of its bytes under the run's [`TraceKey`].
//! - The value of a query parameter is written as a body is: in `params`, the decoded value;
//!   in `target`, the value as it was received, before its percent-decoding, inside the
//!   target's text.
//! - A reply right is `<reply-right>`, as everywhere.
//!
//! Path segments, header names, other header values, the method, the peer, the listener and
//! statuses stay as they are.
//!
//! Costs: O(b) for b bytes of a body or of a target, one HMAC over them; O(1) for the others.

use una_http::serve::trace::TraceKey;

/// The headers whose values are credentials, in ASCII lower case (ADR 0123 rule 1).
pub const CREDENTIALS: [&str; 6] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
];

/// Whether a header named `name` holds a credential: one of [`CREDENTIALS`] in any letter
/// case, or a name that is not a token (RFC 9110 §5.6.2), such as `"set-cookie "` or one with
/// a tab or a control byte, which the host refuses to send and whose value is still written
/// as a credential's, failing closed: no spelling of a credential's name escapes the reduction.
pub fn is_credential(name: &str) -> bool {
    !is_token(name)
        || CREDENTIALS
            .iter()
            .any(|credential| credential.eq_ignore_ascii_case(name))
}

/// Whether `name` is a token: one or more of RFC 9110's `tchar`.
fn is_token(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// The reduced form of a body `text`: `{"length": n, "hmac": "<hex>"}`.
pub fn body(key: &TraceKey, text: &str) -> String {
    format!(
        "{{\"length\": {}, \"hmac\": \"{}\"}}",
        text.len(),
        crate::host::hex(&crate::host::hmac_sha256(key.bytes(), text.as_bytes()))
    )
}

/// The reduced form of a request target: its path as it is, and in its query, after the first
/// `?`, each value of a part `name=value`, split at its first `=`, written as a body is. A part
/// with no `=` is a name, and an empty part stays empty, so the target keeps its shape.
pub fn target(key: &TraceKey, target: &str) -> String {
    reduce_target(target, &mut |value| body(key, value))
}

/// [`target`], with each value of its query written by `value`.
fn reduce_target(target: &str, value: &mut dyn FnMut(&str) -> String) -> String {
    let Some((path, query)) = target.split_once('?') else {
        return target.to_owned();
    };
    let mut out = String::with_capacity(target.len());
    out.push_str(path);
    out.push('?');
    for (i, part) in query.split('&').enumerate() {
        if i > 0 {
            out.push('&');
        }
        match part.split_once('=') {
            Some((name, text)) => {
                out.push_str(name);
                out.push('=');
                out.push_str(&value(text));
            }
            None => out.push_str(part),
        }
    }
    out
}

/// How a field of a kind of the server is written in the reduced form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduced {
    /// `<credential>`.
    Credential,
    /// As a body.
    Body,
    /// As a target.
    Target,
}

/// How field `index` of a value of the kind `kind` of `std.host` is written in the reduced
/// form, `None` when it is written as it is. `header` is the name of a `Header`, its first
/// field. The kinds are `Request`, `Reply`, `Header` and `Param`, in their declared order of
/// fields (`spec/31 §1` rule 2, `spec/24 §1`).
pub fn field(kind: &str, index: usize, header: Option<&str>) -> Option<Reduced> {
    match (kind, index) {
        ("Request", 1) => Some(Reduced::Target),
        ("Request", 5) | ("Reply", 2) | ("Param", 1) => Some(Reduced::Body),
        ("Header", 1) if header.is_some_and(is_credential) => Some(Reduced::Credential),
        _ => None,
    }
}

/// Whether `kind`, of the prelude's module, is a kind of the server whose fields the reduced
/// form may change.
pub fn is_server_kind(kind: &str) -> bool {
    matches!(kind, "Request" | "Reply" | "Header" | "Param")
}

/// The text that field `reduced`, whose value is the text `text`, is written as, in its Una
/// form: a target stays a quoted text, the others are written bare, as `<reply-right>` is.
pub fn write(reduced: Reduced, key: &TraceKey, text: &str) -> String {
    match reduced {
        Reduced::Credential => "<credential>".to_owned(),
        Reduced::Body => body(key, text),
        Reduced::Target => una_core::quote(&target(key, text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: TraceKey = TraceKey::ZERO;

    #[test]
    fn credentials_are_named_in_any_letter_case() {
        for name in CREDENTIALS {
            assert!(is_credential(name));
            assert!(is_credential(&name.to_ascii_uppercase()));
        }
        assert!(is_credential("Set-Cookie"));
        assert!(!is_credential("x-thing"));
        assert!(!is_credential("cookies"));
        // A name that is not a token is written as a credential's, whatever it spells.
        for name in [
            "",
            "set-cookie ",
            " cookie",
            "\tauthorization",
            "cookie\u{0}",
            "x-api-key\r",
            "x thing",
            "x:y",
            "caf\u{e9}",
        ] {
            assert!(is_credential(name), "{name:?}");
        }
        assert!(!is_credential("x-a.b_c~1!#$%&'*+^`|"));
    }

    #[test]
    fn a_body_is_its_length_and_keyed_hash() {
        // HMAC-SHA-256 under 32 zero bytes of the empty message, RFC 2104 over SHA-256: the
        // key is padded with zeros to the block, so it is the HMAC under the empty key.
        assert_eq!(
            body(&ZERO, ""),
            "{\"length\": 0, \"hmac\": \"b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad\"}"
        );
        let shown = body(&ZERO, "body");
        assert!(shown.starts_with("{\"length\": 4, \"hmac\": \""), "{shown}");
        assert!(!shown.contains("body\""), "{shown}");
        // The length is in bytes.
        assert!(body(&ZERO, "é").starts_with("{\"length\": 2,"));
    }

    #[test]
    fn a_body_depends_on_the_key() {
        let other = una_http::serve::trace::TraceKey::from_host().unwrap_or(ZERO);
        assert_ne!(body(&ZERO, "secret"), body(&other, "secret"));
        assert_eq!(body(&ZERO, "secret"), body(&ZERO, "secret"));
    }

    #[test]
    fn a_target_keeps_its_path_and_names_and_hides_its_values() {
        assert_eq!(target(&ZERO, "/a/b%20c"), "/a/b%20c");
        let reduced = target(&ZERO, "/a?x=1&&y=two&flag&z=");
        let one = body(&ZERO, "1");
        let two = body(&ZERO, "two");
        let empty = body(&ZERO, "");
        assert_eq!(reduced, format!("/a?x={one}&&y={two}&flag&z={empty}"));
        // A value split at its first `=` only.
        assert_eq!(
            target(&ZERO, "/?k=a=b"),
            format!("/?k={}", body(&ZERO, "a=b"))
        );
        // Only the first `?` starts the query.
        assert_eq!(
            target(&ZERO, "/p?q=?v"),
            format!("/p?q={}", body(&ZERO, "?v"))
        );
    }

    #[test]
    fn the_fields_of_the_server_kinds() {
        assert_eq!(field("Request", 1, None), Some(Reduced::Target));
        assert_eq!(field("Request", 5, None), Some(Reduced::Body));
        assert_eq!(field("Reply", 2, None), Some(Reduced::Body));
        assert_eq!(field("Param", 1, None), Some(Reduced::Body));
        assert_eq!(
            field("Header", 1, Some("Cookie")),
            Some(Reduced::Credential)
        );
        assert_eq!(field("Header", 1, Some("x-thing")), None);
        assert_eq!(field("Header", 0, Some("cookie")), None);
        for index in [0, 2, 3, 4, 6, 7] {
            assert_eq!(field("Request", index, None), None, "{index}");
        }
        assert_eq!(field("Reply", 0, None), None);
        assert_eq!(field("Param", 0, None), None);
        assert_eq!(write(Reduced::Credential, &ZERO, "abc"), "<credential>");
        assert_eq!(write(Reduced::Target, &ZERO, "/a"), "\"/a\"");
    }

    #[test]
    fn the_reduction_of_a_target_is_linear_in_its_bytes() {
        // A scaling test (CLAUDE.md, Performance), by counts rather than time, so load cannot
        // fail it: as the target grows 16 times, the reduction hashes each value once, exactly
        // the value's bytes, and writes an output whose length grows by the same factor; a
        // reduction that hashed or copied a prefix or the rest of the target per part would
        // hash a quadratic number of bytes. `body` is one HMAC over its bytes, O(b).
        let counts = |parts: usize| {
            let query: Vec<String> = (0..parts).map(|i| format!("k{i}=v{i}")).collect();
            let text = format!("/p?{}", query.join("&"));
            let (mut calls, mut hashed) = (0usize, 0usize);
            let out = reduce_target(&text, &mut |value| {
                calls += 1;
                hashed += value.len();
                body(&ZERO, value)
            });
            let values: usize = (0..parts).map(|i| format!("v{i}").len()).sum();
            assert_eq!(calls, parts);
            assert_eq!(hashed, values);
            assert_eq!(out, target(&ZERO, &text));
            (text.len(), hashed, out.len())
        };
        let (small_in, small_hashed, small_out) = counts(2_000);
        let (large_in, large_hashed, large_out) = counts(32_000);
        // Each bound is the input's own growth, about 16 to 20 times, never its square.
        let grew = large_in.div_ceil(small_in) + 1;
        assert!(
            large_hashed <= small_hashed * grew,
            "{small_hashed} {large_hashed}"
        );
        assert!(large_out <= small_out * grew, "{small_out} {large_out}");
    }
}
