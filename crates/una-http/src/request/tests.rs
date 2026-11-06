//! The request reader: every refusal of `spec/31 §3` and §4 and ADR 0123 rules 8, 10 and 11,
//! a corpus of request-smuggling, hop-by-hop and path-confusion cases (RFC 9112 §11 and
//! published desynchronization research), every split at every byte boundary, a differential
//! test against the client's framing, a deterministic fuzz, and scaling tests.

use super::*;
use crate::parse;
use Refusal::{
    BadRequest, ContentTooLarge, ExpectationFailed, HeaderFieldsTooLarge, NotImplemented,
    UnsupportedMediaType, UriTooLong, VersionNotSupported,
};

/// A small deterministic generator, so the property tests need no crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n as u64).max(1)).unwrap()
    }
}

/// One request read from `bytes` fed in pieces at `splits`: the request and the bytes read.
fn read_split(
    bytes: &[u8],
    splits: &[usize],
    limits: RequestLimits,
    scheme: Scheme,
) -> Result<(Request, usize), Refusal> {
    let mut reader = RequestReader::new(limits, scheme);
    let mut from = 0;
    for &at in splits.iter().chain(std::iter::once(&bytes.len())) {
        let at = at.clamp(from, bytes.len());
        let progress = reader.feed(&bytes[from..at])?;
        if progress.done {
            return Ok((reader.finish()?, from + progress.used));
        }
        assert_eq!(
            progress.used,
            at - from,
            "an unfinished request reads its whole piece"
        );
        from = at;
    }
    // The connection ended before the request did: the host closes without an answer.
    Err(BadRequest)
}

fn read(bytes: &[u8]) -> Result<(Request, usize), Refusal> {
    read_split(bytes, &[], RequestLimits::default(), Scheme::Http)
}

fn refusal(bytes: &[u8]) -> Option<Refusal> {
    read(bytes).err()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn a_request_reads_whole() {
    let bytes = b"POST /orders/%C3%A9/7?a=1&b=x+y&&c HTTP/1.1\r\nHost: Shop.Example.:8080\r\nX-Trace: t1\r\nContent-Type: application/json\r\nConnection: keep-alive, TE\r\nTE: trailers\r\nContent-Length: 7\r\n\r\n{\"n\":1}GET / HTTP/1.1\r\n";
    let (request, used) = read(bytes).unwrap();
    assert_eq!(used, bytes.len() - 16);
    assert_eq!(
        request,
        Request {
            method: Method::Post,
            version: Version::Http11,
            target: "/orders/%C3%A9/7?a=1&b=x+y&&c".to_owned(),
            segments: vec!["orders".to_owned(), "\u{e9}".to_owned(), "7".to_owned()],
            query: pairs(&[("a", "1"), ("b", "x y"), ("c", "")]),
            headers: pairs(&[
                ("host", "Shop.Example.:8080"),
                ("x-trace", "t1"),
                ("content-type", "application/json"),
            ]),
            body: "{\"n\":1}".to_owned(),
            host: Host {
                name: "shop.example".to_owned(),
                port: Some(8080),
                ip: false
            },
            close: false,
        }
    );
}

#[test]
fn bodies_and_connections() {
    let chunked = b"PUT /a HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: Chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\nX-Sum: 1\r\n\r\n";
    let (request, used) = read(chunked).unwrap();
    assert_eq!((request.body.as_str(), used), ("abcde", chunked.len()));
    assert!(
        request
            .headers
            .iter()
            .all(|(n, _)| n != "transfer-encoding")
    );
    let get = b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let (request, _) = read(get).unwrap();
    assert_eq!(
        (request.segments.clone(), request.close),
        (vec![String::new()], false)
    );
    assert_eq!(
        request.host,
        Host {
            name: "127.0.0.1".to_owned(),
            port: None,
            ip: true
        }
    );
    let close = b"GET / HTTP/1.1\r\nHost: [::1]:80\r\nConnection: Close\r\n\r\n";
    let (request, _) = read(close).unwrap();
    assert!(request.close);
    assert_eq!(
        request.host,
        Host {
            name: "[::1]".to_owned(),
            port: Some(80),
            ip: true
        }
    );
    // HTTP/1.0 closes after one reply, and still needs Host.
    let old = b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n";
    assert!(read(old).unwrap().0.close);
    assert_eq!(refusal(b"GET / HTTP/1.0\r\n\r\n"), Some(BadRequest));
    // Every method of `Method`.
    for (word, method) in [
        ("GET", Method::Get),
        ("HEAD", Method::Head),
        ("POST", Method::Post),
        ("PUT", Method::Put),
        ("PATCH", Method::Patch),
        ("DELETE", Method::Delete),
        ("OPTIONS", Method::Options),
    ] {
        let bytes = format!("{word} /x HTTP/1.1\r\nHost: h.example\r\n\r\n");
        assert_eq!(read(bytes.as_bytes()).unwrap().0.method, method);
    }
}

#[test]
fn absolute_form_is_rewritten_only_for_its_own_host() {
    let bytes = b"GET http://Shop.example:8080/a/b?q=1 HTTP/1.1\r\nHost: shop.example:8080\r\n\r\n";
    let (request, _) = read(bytes).unwrap();
    assert_eq!(request.target, "/a/b?q=1");
    assert_eq!(request.segments, vec!["a".to_owned(), "b".to_owned()]);
    let empty = b"GET http://h.example HTTP/1.1\r\nHost: h.example\r\n\r\n";
    assert_eq!(read(empty).unwrap().0.target, "/");
    let query = b"GET HTTP://h.example?x HTTP/1.1\r\nHost: h.example\r\n\r\n";
    assert_eq!(read(query).unwrap().0.target, "/?x");
    let tls = b"GET https://h.example/ HTTP/1.1\r\nHost: h.example\r\n\r\n";
    assert_eq!(
        read_split(tls, &[], RequestLimits::default(), Scheme::Https)
            .unwrap()
            .0
            .target,
        "/"
    );
    for bad in [
        // Another authority than Host: the classic routing confusion.
        &b"GET http://evil.example/ HTTP/1.1\r\nHost: h.example\r\n\r\n"[..],
        b"GET http://h.example:81/ HTTP/1.1\r\nHost: h.example\r\n\r\n",
        b"GET http://user@h.example/ HTTP/1.1\r\nHost: h.example\r\n\r\n",
        b"GET http:///a HTTP/1.1\r\nHost: h.example\r\n\r\n",
        // The scheme is the grant's.
        b"GET https://h.example/ HTTP/1.1\r\nHost: h.example\r\n\r\n",
        b"GET ftp://h.example/ HTTP/1.1\r\nHost: h.example\r\n\r\n",
        // Asterisk-form and authority-form.
        b"OPTIONS * HTTP/1.1\r\nHost: h.example\r\n\r\n",
        b"CONNECT h.example:443 HTTP/1.1\r\nHost: h.example\r\n\r\n",
        b"GET h.example:443 HTTP/1.1\r\nHost: h.example\r\n\r\n",
        b"GET http://h.example/../a HTTP/1.1\r\nHost: h.example\r\n\r\n",
        b"GET http://h.example/a#f HTTP/1.1\r\nHost: h.example\r\n\r\n",
    ] {
        assert!(refusal(bad).is_some(), "{}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn host_values() {
    for (value, name, port, ip) in [
        ("localhost", "localhost", None, false),
        ("LocalHost:8080", "localhost", Some(8080), false),
        ("shop.example.", "shop.example", None, false),
        ("a-b_c.example:1", "a-b_c.example", Some(1), false),
        ("10.0.0.1", "10.0.0.1", None, true),
        ("10.0.0.1:65535", "10.0.0.1", Some(65535), true),
        ("[::1]", "[::1]", None, true),
        (
            "[::FFFF:127.0.0.1]:443",
            "[::ffff:127.0.0.1]",
            Some(443),
            true,
        ),
        ("x1.example", "x1.example", None, false),
        ("a.x0", "a.x0", None, false),
        ("0x.example", "0x.example", None, false),
    ] {
        assert_eq!(
            host(value),
            Ok(Host {
                name: name.to_owned(),
                port,
                ip
            }),
            "{value}"
        );
    }
    for bad in [
        "",
        ":80",
        "a/b@c",
        "user@h.example",
        "h.example/a",
        "h.example:",
        "h.example:0x50",
        "h.example:123456",
        "h.example:65536",
        "h.example:-1",
        "h example",
        "h.example,evil.example",
        "h..example",
        ".h.example",
        "h.example..",
        "caf\u{e9}.example",
        "[::1",
        "[::1]x",
        "[::1]:",
        "[fe80::1%25eth0]",
        "[h.example]",
        "::1",
        "127.1",
        "127.0.0.1.",
        "0x7f.0.0.1",
        "1.2.3.0x4",
        "1.2.3.0x",
        "a.0x",
        "a.0X.",
        "01.02.03.04",
        "256.0.0.1",
        "1.2.3.4.5",
    ] {
        assert_eq!(host(bad), Err(BadRequest), "{bad:?}");
    }
}

/// Wraps header lines into a request with a valid line and `Host`.
fn with_headers(lines: &str) -> Vec<u8> {
    format!("POST /a HTTP/1.1\r\nHost: h.example\r\n{lines}\r\n").into_bytes()
}

#[test]
fn every_refusal_has_its_status() {
    let cases: Vec<(Vec<u8>, Refusal)> = vec![
        // Request line (rule 10).
        (
            b"\r\nGET / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"\r\n\r\nGET / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (b" GET / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(), BadRequest),
        (b"GET  / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(), BadRequest),
        (b"GET / HTTP/1.1 \r\nHost: h\r\n\r\n".to_vec(), BadRequest),
        (b"G@T / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(), BadRequest),
        (
            b"GET / HTTP/2.0\r\nHost: h\r\n\r\n".to_vec(),
            VersionNotSupported,
        ),
        (
            b"GET / HTTP/1.2\r\nHost: h\r\n\r\n".to_vec(),
            VersionNotSupported,
        ),
        (b"GET /\r\n\r\n".to_vec(), BadRequest),
        (
            b"CONNECT h:443 HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            NotImplemented,
        ),
        (
            b"TRACE / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            NotImplemented,
        ),
        (
            b"get / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            NotImplemented,
        ),
        (
            b"BREW / HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            NotImplemented,
        ),
        // Line ends.
        (b"GET / HTTP/1.1\nHost: h\r\n\r\n".to_vec(), BadRequest),
        (b"GET / HTTP/1.1\r\nHost: h\n\r\n".to_vec(), BadRequest),
        (b"GET / HTTP/1.1\r\nHost: h\r\n\n".to_vec(), BadRequest),
        (b"GET / HTTP/1.1\r\nHost: h\r\r\n\r\n".to_vec(), BadRequest),
        (
            b"GET / HTTP/1.1\r\nX: a\rb\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (b"GET /\r HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(), BadRequest),
        // Header lines.
        (with_headers("X: a\r\n folded\r\n"), BadRequest),
        (with_headers(" X: a\r\n"), BadRequest),
        (with_headers("X : a\r\n"), BadRequest),
        (with_headers("X\t: a\r\n"), BadRequest),
        (with_headers("X: a\x00b\r\n"), BadRequest),
        (with_headers("X: a\x1bb\r\n"), BadRequest),
        (with_headers("X: a\x7f\r\n"), BadRequest),
        (
            b"GET / HTTP/1.1\r\nHost: h\r\nX: \xff\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (with_headers("no colon\r\n"), BadRequest),
        (with_headers(": empty\r\n"), BadRequest),
        // Host (rule 8).
        (b"GET / HTTP/1.1\r\n\r\n".to_vec(), BadRequest),
        (
            b"GET / HTTP/1.1\r\nHost: a\r\nHost: a\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET / HTTP/1.1\r\nHost: a/b@c\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (b"GET / HTTP/1.1\r\nHost:\r\n\r\n".to_vec(), BadRequest),
        // Framing.
        (
            with_headers("Content-Length: 3\r\nTransfer-Encoding: chunked\r\n"),
            BadRequest,
        ),
        (
            with_headers("Transfer-Encoding: chunked\r\nContent-Length: 3\r\n"),
            BadRequest,
        ),
        (
            with_headers("Content-Length: 3\r\nContent-Length: 3\r\n"),
            BadRequest,
        ),
        (with_headers("Content-Length: 3, 3\r\n"), BadRequest),
        (with_headers("Content-Length: +3\r\n"), BadRequest),
        (with_headers("Content-Length: 0x3\r\n"), BadRequest),
        (with_headers("Transfer-Encoding: gzip\r\n"), NotImplemented),
        (
            with_headers("Transfer-Encoding: gzip, chunked\r\n"),
            NotImplemented,
        ),
        (
            with_headers("Transfer-Encoding: chunked, chunked\r\n"),
            BadRequest,
        ),
        (
            with_headers("Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n"),
            BadRequest,
        ),
        (with_headers("Transfer-Encoding: \r\n"), BadRequest),
        (
            b"POST / HTTP/1.0\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (with_headers("Content-Length: 1048577\r\n"), ContentTooLarge),
        // Connection.
        (
            with_headers("Connection: close, x-forwarded-for\r\n"),
            BadRequest,
        ),
        (with_headers("Connection: host\r\n"), BadRequest),
        (with_headers("Connection: authorization\r\n"), BadRequest),
        (with_headers("Connection: close,\r\n"), BadRequest),
        // Encodings and expectations.
        (
            with_headers("Content-Encoding: gzip\r\n"),
            UnsupportedMediaType,
        ),
        (
            with_headers("Content-Encoding: identity\r\nContent-Encoding: br\r\n"),
            UnsupportedMediaType,
        ),
        (
            with_headers("Expect: 100-continue-please\r\n"),
            ExpectationFailed,
        ),
        (with_headers("Expect: 200-ok\r\n"), ExpectationFailed),
        // The target (rule 11).
        (
            b"GET /a/../b HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET /a/%2e%2e/b HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET /a%2fb HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET /a%5cb HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET /a%00 HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET /a?q=| HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET /a?q={} HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (
            b"GET /a?q=\"x\" HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (b"GET /a#f HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(), BadRequest),
        (
            b"GET /a?q=%ff HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            BadRequest,
        ),
        (b"GET * HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(), BadRequest),
        // A body that is not UTF-8.
        (
            with_headers("Content-Length: 1\r\n")
                .into_iter()
                .chain(*b"\xff")
                .collect(),
            BadRequest,
        ),
    ];
    for (bytes, expected) in cases {
        assert_eq!(
            refusal(&bytes),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        // The refusal does not depend on how the bytes arrive.
        for at in 0..=bytes.len() {
            assert_eq!(
                read_split(&bytes, &[at], RequestLimits::default(), Scheme::Http).err(),
                Some(expected),
                "{} at {at}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }
}

#[test]
fn http_10_ignores_expect() {
    let bytes = b"POST / HTTP/1.0\r\nHost: h\r\nExpect: something\r\nContent-Length: 1\r\n\r\nx";
    assert_eq!(read(bytes).unwrap().0.body, "x");
}

#[test]
fn expect_continue_is_reported_once_the_head_is_read() {
    let head = b"POST / HTTP/1.1\r\nHost: h\r\nExpect: 100-Continue\r\nContent-Length: 2\r\n\r\n";
    let mut reader = RequestReader::new(RequestLimits::default(), Scheme::Http);
    assert!(!reader.started());
    reader.feed(&head[..10]).unwrap();
    assert!(reader.started());
    assert!(!reader.expects_continue());
    let progress = reader.feed(&head[10..]).unwrap();
    assert_eq!(
        progress,
        Progress {
            used: head.len() - 10,
            done: false
        }
    );
    assert!(reader.expects_continue());
    assert_eq!(reader.framing(), Some(BodyFraming::Length(2)));
    assert_eq!(
        reader.feed(b"okGET").unwrap(),
        Progress {
            used: 2,
            done: true
        }
    );
    assert_eq!(reader.finish().unwrap().body, "ok");
    // No body, no 100 Continue.
    let empty = b"GET / HTTP/1.1\r\nHost: h\r\nExpect: 100-continue\r\n\r\n";
    let mut reader = RequestReader::new(RequestLimits::default(), Scheme::Http);
    assert!(reader.feed(empty).unwrap().done);
    assert!(!reader.expects_continue());
}

#[test]
fn hop_by_hop_headers_are_removed() {
    let bytes = with_headers(
        "Connection: keep-alive, Upgrade, TE, Trailer, Proxy-Connection\r\nKeep-Alive: timeout=5\r\nUpgrade: h2c\r\nTE: trailers\r\nTrailer: X\r\nProxy-Connection: x\r\nX-Kept: 1\r\n",
    );
    let (request, _) = read(&bytes).unwrap();
    assert_eq!(
        request.headers,
        pairs(&[("host", "h.example"), ("x-kept", "1")])
    );
    assert!(!request.close);
}

#[test]
fn smuggling_corpus() {
    // Request smuggling and desynchronization vectors (RFC 9112 §11.2 and published research:
    // CL.TE, TE.CL, TE.TE obfuscations, header-name confusion, line-end confusion). Each is
    // refused, so a front proxy and this reader can never disagree on where a request ends.
    let vectors: &[&[u8]] = &[
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 6\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nG",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nContent-Length: 4\r\n\r\n5c\r\nGPOST",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: xchunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding : chunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding:\x0bchunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\x0c\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding\r\n : chunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nX: x\r\n Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nTransfer-encoding: x\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: identity, chunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked;foo=bar\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: \"chunked\"\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer_Encoding: chunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent_Length: 5\r\n\r\nhello",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\nContent_Length: 0\r\n\r\nhello",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 0\r\nContent-Length: 5\r\n\r\nhello",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 5 5\r\n\r\nhello",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 5\x00\r\n\r\nhello",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 05e0\r\n\r\nhello",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length:\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length\r\n : 5\r\n\r\nhello",
        // Chunk-size confusion: extensions, oversized digits, whitespace, bare LF.
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n2;\nxx\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n2;a=\"\r\n\"\r\nxx\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n0000000000000000002\r\nxx\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n10000000000000002\r\nxx\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n2 \r\nxx\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n2\nxx\n0\n\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nxxx\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nxx0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nContent-Length: 5\r\nX : y\r\n\r\n",
        // Line-end confusion between the request line and the headers.
        b"GET / HTTP/1.1\nHost: h\nContent-Length: 5\n\nhello",
        b"GET / HTTP/1.1\r\nHost: h\rContent-Length: 5\r\n\r\nhello",
        b"GET / HTTP/1.1\r\nHost: h\r\n\r\rContent-Length: 5\r\n\r\nhello",
        b"\nGET / HTTP/1.1\r\nHost: h\r\n\r\n",
        b"\r\nGET / HTTP/1.1\r\nHost: h\r\n\r\n",
        // HTTP/1.0 framing and versions.
        b"POST / HTTP/1.0\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nConnection: close, Transfer-Encoding, X-Front\r\n\r\n0\r\n\r\n",
        b"GET / HTTP/0.9\r\n\r\n",
        b"GET /\r\n\r\n",
    ];
    for vector in vectors {
        assert!(
            refusal(vector).is_some(),
            "{}",
            String::from_utf8_lossy(vector)
        );
    }
}

#[test]
fn the_next_request_is_left_unread() {
    // A body framed by length is read to its length, and what follows is the next request,
    // never part of this one.
    let first = b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\n\r\nhello";
    let second = b"GET /admin HTTP/1.1\r\nHost: h\r\n\r\n";
    let bytes = [&first[..], &second[..]].concat();
    for at in 0..=bytes.len() {
        let (request, used) =
            read_split(&bytes, &[at], RequestLimits::default(), Scheme::Http).unwrap();
        assert_eq!((request.body.as_str(), used), ("hello", first.len()));
        let (next, used) = read(&bytes[used..]).unwrap();
        assert_eq!((next.target.as_str(), used), ("/admin", second.len()));
    }
}

#[test]
fn path_confusion_corpus() {
    for target in [
        "/..",
        "/.",
        "/a/..",
        "/a/./b",
        "/%2e%2e/etc/passwd",
        "/%2E%2E/",
        "/.%2e/",
        "/a/%2e/b",
        "/..%2f..%2fetc",
        "/a%2F..",
        "/%5c..%5c",
        "/a\\b",
        "/a%00.html",
        "/a%0d%0aSet-Cookie:x",
        "/%c0%ae%c0%ae/",
        "/%e0%80%ae",
        "/a%7f",
        "/a%09b",
        "/a b",
        "/a\tb",
    ] {
        let bytes = format!("GET {target} HTTP/1.1\r\nHost: h\r\n\r\n");
        assert_eq!(refusal(bytes.as_bytes()), Some(BadRequest), "{target:?}");
    }
    // Forms other servers read as special are ordinary segments here (ADR 0123, Security).
    for (target, segments) in [
        ("/..;/x", vec!["..;", "x"]),
        ("/%252e%252e/x", vec!["%2e%2e", "x"]),
        ("/a;b/c", vec!["a;b", "c"]),
        ("/...", vec!["..."]),
        ("//a", vec!["", "a"]),
    ] {
        let bytes = format!("GET {target} HTTP/1.1\r\nHost: h\r\n\r\n");
        let (request, _) = read(bytes.as_bytes()).unwrap();
        assert_eq!(request.segments, segments, "{target}");
        // Joining the segments again gives back the decoded path, with no new segment.
        assert_eq!(
            request.segments.join("/").split('/').count(),
            request.segments.len()
        );
    }
}

#[test]
fn limits_are_checked_before_the_bytes_are_kept() {
    let limits = RequestLimits {
        line: 20,
        headers: 2,
        header: 40,
        body: 4,
    };
    let at = |bytes: &[u8]| read_split(bytes, &[], limits, Scheme::Http).map(|(r, _)| r);
    // The request line: 18 bytes and its CRLF fit 20; one more is 414.
    assert!(at(b"GET /0123 HTTP/1.1\r\nHost: h\r\n\r\n").is_ok());
    assert_eq!(
        at(b"GET /01234 HTTP/1.1\r\nHost: h\r\n\r\n").err(),
        Some(UriTooLong)
    );
    // A long line is refused before its end arrives.
    let mut reader = RequestReader::new(limits, Scheme::Http);
    assert_eq!(reader.feed(&[b'G'; 1 << 16]), Err(UriTooLong));
    // Header lines: two fit, three are 431.
    assert!(at(b"GET / HTTP/1.1\r\nHost: h\r\nA: 1\r\n\r\n").is_ok());
    assert_eq!(
        at(b"GET / HTTP/1.1\r\nHost: h\r\nA: 1\r\nB: 2\r\n\r\n").err(),
        Some(HeaderFieldsTooLarge)
    );
    // Header bytes: 9 + 29 + 2 = 40 fit; one more is 431.
    let fits = format!("GET / HTTP/1.1\r\nHost: h\r\nA: {}\r\n\r\n", "x".repeat(24));
    assert!(at(fits.as_bytes()).is_ok());
    let over = format!("GET / HTTP/1.1\r\nHost: h\r\nA: {}\r\n\r\n", "x".repeat(25));
    assert_eq!(at(over.as_bytes()).err(), Some(HeaderFieldsTooLarge));
    let mut reader = RequestReader::new(limits, Scheme::Http);
    let mut flood = b"GET / HTTP/1.1\r\nX: ".to_vec();
    flood.extend(std::iter::repeat_n(b'y', 1 << 16));
    assert_eq!(reader.feed(&flood), Err(HeaderFieldsTooLarge));
    // Trailers spend what the head left: the head uses both lines, so a trailer is 431.
    let roomy = RequestLimits {
        header: 64,
        ..limits
    };
    let at = |bytes: &[u8]| read_split(bytes, &[], roomy, Scheme::Http).map(|(r, _)| r);
    let chunked =
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nT: 1\r\n\r\n";
    assert_eq!(at(chunked).err(), Some(HeaderFieldsTooLarge));
    let chunked = b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
    assert!(at(chunked).is_ok());
    // With 39 bytes of head, the empty line after the last chunk no longer fits 40.
    let at = |bytes: &[u8]| read_split(bytes, &[], limits, Scheme::Http).map(|(r, _)| r);
    assert_eq!(at(chunked).err(), Some(HeaderFieldsTooLarge));
    // The body: a declared length over the limit is 413 before any byte of it.
    let mut reader = RequestReader::new(limits, Scheme::Http);
    assert_eq!(
        reader.feed(b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\n\r\n"),
        Err(ContentTooLarge)
    );
    let grows =
        b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\n";
    assert_eq!(at(grows).err(), Some(ContentTooLarge));
    assert_eq!(
        at(b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\n\r\nabcd")
            .unwrap()
            .body,
        "abcd"
    );
}

#[test]
fn an_unfinished_request_is_not_a_request() {
    let reader = RequestReader::new(RequestLimits::default(), Scheme::Http);
    assert_eq!(reader.finish(), Err(BadRequest));
    let mut reader = RequestReader::new(RequestLimits::default(), Scheme::Http);
    reader
        .feed(b"POST / HTTP/1.1\r\nHost: h\r\nContent-Length: 3\r\n\r\nab")
        .unwrap();
    assert_eq!(reader.body_length(), 2);
    assert_eq!(reader.finish(), Err(BadRequest));
}

/// A random well-formed request: its bytes, and its body.
fn request(rng: &mut Rng) -> (Vec<u8>, String) {
    let body: String = (0..rng.below(40))
        .map(|_| ['a', ' ', '\r', '\n', '{', '0', '\u{e9}', ';'][rng.below(8)])
        .collect();
    let methods = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
    let targets = [
        "/",
        "/a",
        "/a/b%20c",
        "/x?y=1&z",
        "/caf%C3%A9?q=a+b",
        "//a/",
    ];
    let mut out = format!(
        "{} {} HTTP/1.1\r\nHost: h{}.example\r\n",
        methods[rng.below(methods.len())],
        targets[rng.below(targets.len())],
        rng.below(9)
    )
    .into_bytes();
    for i in 0..rng.below(4) {
        out.extend_from_slice(format!("X-H{i}: \tv{} \r\n", rng.below(100)).as_bytes());
    }
    let bytes = body.as_bytes();
    match rng.below(3) {
        0 if bytes.is_empty() => out.extend_from_slice(b"\r\n"),
        0 | 1 => {
            out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes());
            out.extend_from_slice(bytes);
        }
        _ => {
            out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
            let mut rest = bytes;
            while !rest.is_empty() {
                let (chunk, after) = rest.split_at(rng.below(rest.len()) + 1);
                out.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                out.extend_from_slice(chunk);
                out.extend_from_slice(b"\r\n");
                rest = after;
            }
            out.extend_from_slice(b"0\r\n");
            if rng.below(2) == 0 {
                out.extend_from_slice(b"X-Trailer: t\r\n");
            }
            out.extend_from_slice(b"\r\n");
        }
    }
    (out, body)
}

#[test]
fn every_split_reads_as_the_whole() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..300 {
        let (mut bytes, body) = request(&mut rng);
        let len = bytes.len();
        bytes.extend_from_slice(b"GET /next HTTP/1.1\r\n");
        let whole = read(&bytes);
        let (request, used) = whole.as_ref().unwrap();
        // A body cut inside a UTF-8 sequence by chunking still reads whole.
        assert_eq!((request.body.as_str(), *used), (body.as_str(), len));
        for at in 0..=bytes.len() {
            assert_eq!(
                read_split(&bytes, &[at], RequestLimits::default(), Scheme::Http),
                whole
            );
        }
        let mut splits: Vec<usize> = (0..rng.below(8))
            .map(|_| rng.below(bytes.len() + 1))
            .collect();
        splits.sort_unstable();
        assert_eq!(
            read_split(&bytes, &splits, RequestLimits::default(), Scheme::Http),
            whole
        );
    }
}

#[test]
fn mutated_requests_never_panic_and_split_alike() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut refused = 0usize;
    for _ in 0..5000 {
        let (mut bytes, _) = request(&mut rng);
        for _ in 0..rng.below(4) + 1 {
            if bytes.is_empty() {
                break;
            }
            let at = rng.below(bytes.len());
            match rng.below(5) {
                0 => bytes[at] = b"0aZ \r\n:;-_%./\x00\xff\t"[rng.below(16)],
                1 => bytes.insert(at, b"0\r\n: %"[rng.below(6)]),
                2 => {
                    bytes.remove(at);
                }
                3 => bytes.truncate(at),
                _ => {
                    let from = rng.below(bytes.len());
                    let piece = bytes[from..].to_vec();
                    drop(bytes.splice(at..at, piece.into_iter().take(12)));
                }
            }
        }
        let whole = read(&bytes);
        refused += usize::from(whole.is_err());
        for _ in 0..4 {
            let at = rng.below(bytes.len() + 1);
            assert_eq!(
                read_split(&bytes, &[at], RequestLimits::default(), Scheme::Http),
                whole
            );
        }
    }
    assert!(
        refused > 1000,
        "the mutations reach the refusals: {refused}"
    );
}

/// The client's reading of the same framing as a response: the head's header lines after a
/// status line, then the same bytes.
fn client_reads(headers: &[u8], rest: &[u8]) -> Result<Vec<u8>, parse::Error> {
    let mut parser = parse::Parser::new(parse::Limits::default());
    parser.feed(b"HTTP/1.1 200 OK\r\n")?;
    parser.feed(headers)?;
    parser.feed(rest)?;
    parser.finish().map(|response| response.body)
}

#[test]
fn the_client_and_the_server_frame_alike_where_both_accept() {
    // One corpus of framings, read by the client's response parser and by this reader: on
    // every input both accept and that ends where the bytes end, the bodies are equal.
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    let mut both = 0usize;
    for _ in 0..4000 {
        let (bytes, _) = request(&mut rng);
        let Some(line_end) = bytes.windows(2).position(|w| w == b"\r\n") else {
            continue;
        };
        let (line, after) = bytes.split_at(line_end + 2);
        let mut after = after.to_vec();
        for _ in 0..rng.below(3) {
            if after.is_empty() {
                break;
            }
            let at = rng.below(after.len());
            match rng.below(4) {
                0 => after[at] = b"0aF \r\n:;\t,"[rng.below(10)],
                1 => after.insert(at, b"0\r\n;,"[rng.below(5)]),
                2 => {
                    after.remove(at);
                }
                _ => after.truncate(at),
            }
        }
        let server = read(&[line, &after[..]].concat());
        let client = client_reads(&after, b"");
        if let (Ok((request, used)), Ok(body)) = (&server, &client)
            && *used == line.len() + after.len()
        {
            assert_eq!(
                request.body.as_bytes(),
                body.as_slice(),
                "{}",
                String::from_utf8_lossy(&after)
            );
            both += 1;
        }
    }
    assert!(
        both > 1000,
        "the corpus exercises the differential test: {both}"
    );
}

#[test]
fn reading_is_linear() {
    // Headers from 1 to 1000 lines, and bodies of one-byte chunks: the time per byte does not
    // grow with the size. A quadratic step multiplies the time by 16 more than the bytes do.
    let limits = RequestLimits {
        line: 1 << 20,
        headers: 1 << 20,
        header: 1 << 26,
        body: 1 << 26,
    };
    let best = |bytes: &[u8]| {
        let mut best = std::time::Duration::MAX;
        for _ in 0..3 {
            let start = std::time::Instant::now();
            assert!(read_split(bytes, &[], limits, Scheme::Http).is_ok());
            best = best.min(start.elapsed());
        }
        best
    };
    let headers = |n: usize| {
        let mut bytes = b"GET / HTTP/1.1\r\nHost: h\r\n".to_vec();
        for i in 0..n {
            bytes.extend_from_slice(format!("X-{i}: value {i}\r\n").as_bytes());
        }
        bytes.extend_from_slice(b"\r\n");
        bytes
    };
    let chunks = |n: usize| {
        let mut bytes =
            b"POST / HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        bytes.extend_from_slice(&b"1\r\nx\r\n".repeat(n));
        bytes.extend_from_slice(b"0\r\n\r\n");
        bytes
    };
    for make in [&headers as &dyn Fn(usize) -> Vec<u8>, &chunks] {
        let small = best(&make(1 << 12));
        let large = best(&make(1 << 16));
        assert!(
            large < small * 16 * 8,
            "16x the bytes took {large:?} against {small:?}"
        );
    }
    // One request split at every byte boundary: each split costs the same.
    let bytes = headers(1000);
    let start = std::time::Instant::now();
    let mut reader = RequestReader::new(limits, Scheme::Http);
    for byte in bytes.chunks(1) {
        reader.feed(byte).unwrap();
    }
    assert!(reader.finish().is_ok());
    assert!(start.elapsed() < best(&bytes) * 64 + std::time::Duration::from_millis(50));
}
