//! The strict framing, case by case: every refusal of `spec/31 §3` and §4 that the framing
//! decides, every split of the incremental readers, a differential test against the client's
//! lenient reader, and a deterministic fuzz of mutated bodies.

use super::*;
use crate::parse;

use Refusal::{
    BadRequest, ContentTooLarge, HeaderFieldsTooLarge, NotImplemented, UriTooLong,
    VersionNotSupported,
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

/// Reads one line from `bytes` fed in pieces at `splits`: the line, and the bytes read.
fn read_line(bytes: &[u8], splits: &[usize], room: usize) -> Result<(Vec<u8>, usize), Refusal> {
    let mut reader = LineReader::new();
    let mut from = 0;
    for &at in splits.iter().chain(std::iter::once(&bytes.len())) {
        let at = at.clamp(from, bytes.len());
        let mut piece = &bytes[from..at];
        while !piece.is_empty() {
            let (used, ended) = reader.feed(piece, room, UriTooLong)?;
            if ended {
                return Ok((reader.take(), from + used));
            }
            assert_eq!(
                used,
                piece.len(),
                "an unfinished line reads its whole piece"
            );
            piece = &piece[used..];
        }
        from = at;
    }
    Err(BadRequest)
}

#[test]
fn lines_end_with_crlf_only() {
    assert_eq!(
        read_line(b"GET / HTTP/1.1\r\nrest", &[], 64),
        Ok((b"GET / HTTP/1.1".to_vec(), 16))
    );
    assert_eq!(read_line(b"\r\n", &[], 64), Ok((Vec::new(), 2)));
    for bad in [
        &b"GET / HTTP/1.1\n"[..],
        b"GET\r/ HTTP/1.1\r\n",
        b"GET / HTTP/1.1\r\r\n",
        b"\n",
        b"\r\r\n",
        b"\rx",
        b"a\r \n",
    ] {
        for at in 0..=bad.len() {
            assert_eq!(
                read_line(bad, &[at], 64).map(|_| ()),
                Err(BadRequest),
                "{bad:?} at {at}"
            );
        }
    }
}

#[test]
fn a_line_is_refused_once_it_cannot_fit() {
    // The room counts the CRLF: 6 bytes admit a line of 4.
    assert_eq!(read_line(b"abcd\r\n", &[], 6), Ok((b"abcd".to_vec(), 6)));
    assert_eq!(read_line(b"abcde\r\n", &[], 6), Err(UriTooLong));
    // Refused before the end of the line comes, and before its bytes are kept.
    let mut reader = LineReader::new();
    assert_eq!(reader.feed(b"abcde", 6, UriTooLong), Err(UriTooLong));
    assert_eq!(reader.pending(), 0);
    let mut reader = LineReader::new();
    assert_eq!(reader.feed(b"abcd", 6, UriTooLong), Ok((4, false)));
    assert_eq!(reader.feed(b"e", 6, UriTooLong), Err(UriTooLong));
    // Every split gives the same line.
    let bytes = b"Host: example\r\nX";
    for a in 0..=bytes.len() {
        for b in a..=bytes.len() {
            assert_eq!(
                read_line(bytes, &[a, b], 64),
                Ok((b"Host: example".to_vec(), 15))
            );
        }
    }
}

#[test]
fn request_lines() {
    let ok = request_line(b"GET /a?b HTTP/1.1").unwrap();
    assert_eq!(
        ok,
        RequestLine {
            method: Method::Get,
            target: "/a?b",
            version: Version::Http11
        }
    );
    for (word, method) in [
        ("GET", Method::Get),
        ("HEAD", Method::Head),
        ("POST", Method::Post),
        ("PUT", Method::Put),
        ("PATCH", Method::Patch),
        ("DELETE", Method::Delete),
        ("OPTIONS", Method::Options),
    ] {
        let line = format!("{word} / HTTP/1.0");
        let read = request_line(line.as_bytes()).unwrap();
        assert_eq!((read.method, read.version), (method, Version::Http10));
    }
    for (line, refusal) in [
        // An empty line before the request line is refused, never skipped.
        (&b""[..], BadRequest),
        (b" GET / HTTP/1.1", BadRequest),
        (b"GET  / HTTP/1.1", BadRequest),
        (b"GET /  HTTP/1.1", BadRequest),
        (b"GET / HTTP/1.1 ", BadRequest),
        (b"GET\t/ HTTP/1.1", BadRequest),
        (b"GET / HTTP/1.1\t", VersionNotSupported),
        (b"GET /", BadRequest),
        (b"GET", BadRequest),
        (b"GET / HTTP/1.1 extra", BadRequest),
        (b"G(T / HTTP/1.1", BadRequest),
        (b"GET /\x7f HTTP/1.1", BadRequest),
        (b"GET /\x00 HTTP/1.1", BadRequest),
        (b"GET /\xc3\xa9 HTTP/1.1", BadRequest),
        (b"GET / HTTP/1.2", VersionNotSupported),
        (b"GET / HTTP/2.0", VersionNotSupported),
        (b"GET / HTTP/0.9", VersionNotSupported),
        (b"GET / http/1.1", VersionNotSupported),
        (b"GET / HTTP/1.10", VersionNotSupported),
        (b"get / HTTP/1.1", NotImplemented),
        (b"Get / HTTP/1.1", NotImplemented),
        (b"CONNECT a:443 HTTP/1.1", NotImplemented),
        (b"TRACE / HTTP/1.1", NotImplemented),
        (b"PROPFIND / HTTP/1.1", NotImplemented),
    ] {
        assert_eq!(
            request_line(line),
            Err(refusal),
            "{}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn header_lines() {
    assert_eq!(header_line(b"Host: a.example"), Ok(("Host", "a.example")));
    assert_eq!(header_line(b"X-A:\t b c \t"), Ok(("X-A", "b c")));
    assert_eq!(header_line(b"X-A:"), Ok(("X-A", "")));
    assert_eq!(header_line(b"X-A:   "), Ok(("X-A", "")));
    assert_eq!(
        header_line("X-A: caf\u{e9}".as_bytes()),
        Ok(("X-A", "caf\u{e9}"))
    );
    assert_eq!(header_line(b"X-A: a:b"), Ok(("X-A", "a:b")));
    for bad in [
        // Obsolete folding: a line that starts with whitespace.
        &b" folded: x"[..],
        b"\tfolded: x",
        b" x",
        // Whitespace before the colon.
        b"Host : a",
        b"Host\t: a",
        // Not a token, or no name.
        b": a",
        b"no colon",
        b"",
        b"Ho st: a",
        b"Ho(st: a",
        b"H\xc3\xa9: a",
        b"Host\x00: a",
        // Control characters in the value.
        b"X: a\x00b",
        b"X: a\x01b",
        b"X: a\x1fb",
        b"X: a\x7fb",
        b"X: a\rb",
        b"X: a\nb",
        b"X: a\x0bb",
        // Not UTF-8: never repaired.
        b"X: \xff",
        b"X: \xc3",
        b"X: \xed\xa0\x80",
    ] {
        assert_eq!(
            header_line(bad),
            Err(BadRequest),
            "{}",
            String::from_utf8_lossy(bad)
        );
    }
}

#[test]
fn content_lengths() {
    assert_eq!(content_length("0"), Ok(0));
    assert_eq!(content_length("00042"), Ok(42));
    assert_eq!(
        content_length("9999999999999999999"),
        Ok(9_999_999_999_999_999_999)
    );
    for bad in [
        "",
        "-1",
        "+1",
        " 1",
        "1 ",
        "1,1",
        "1, 1",
        "0x10",
        "1e3",
        "1.0",
        "١",
        "99999999999999999999",
        "00000000000000000001",
    ] {
        assert_eq!(content_length(bad), Err(BadRequest), "{bad:?}");
    }
}

#[test]
fn transfer_encodings() {
    for ok in ["chunked", "Chunked", "CHUNKED", "cHuNkEd"] {
        assert_eq!(transfer_encoding(ok), Ok(()), "{ok}");
    }
    for (value, refusal) in [
        ("", BadRequest),
        (" ", BadRequest),
        ("chunked,", BadRequest),
        (",chunked", BadRequest),
        ("chunked, chunked", BadRequest),
        ("chunked,chunked", BadRequest),
        ("CHUNKED, chunked", BadRequest),
        ("chunked;q=1", BadRequest),
        ("chunked ;", BadRequest),
        ("\"chunked\"", BadRequest),
        ("chunked\u{0}", BadRequest),
        ("chunk", NotImplemented),
        ("gzip", NotImplemented),
        ("identity", NotImplemented),
        ("gzip, chunked", NotImplemented),
        ("chunked, gzip", NotImplemented),
        ("x-chunked", NotImplemented),
        ("chunked, identity", NotImplemented),
    ] {
        assert_eq!(transfer_encoding(value), Err(refusal), "{value:?}");
    }
}

#[test]
fn framing_refuses_every_ambiguity() {
    let v11 = Version::Http11;
    let v10 = Version::Http10;
    assert_eq!(body_framing(v11, &[], &[], 10), Ok(BodyFraming::Empty));
    assert_eq!(body_framing(v11, &["0"], &[], 10), Ok(BodyFraming::Empty));
    assert_eq!(
        body_framing(v11, &["10"], &[], 10),
        Ok(BodyFraming::Length(10))
    );
    assert_eq!(
        body_framing(v10, &["3"], &[], 10),
        Ok(BodyFraming::Length(3))
    );
    assert_eq!(
        body_framing(v11, &[], &["chunked"], 10),
        Ok(BodyFraming::Chunked)
    );
    for (version, lengths, codings, refusal) in [
        // Both headers: the classic desynchronization.
        (v11, &["4"][..], &["chunked"][..], BadRequest),
        (v11, &["0"], &["chunked"], BadRequest),
        (v11, &["4"], &["gzip"], BadRequest),
        (v11, &["4", "4"], &["chunked", "chunked"], BadRequest),
        // Transfer-Encoding in HTTP/1.0.
        (v10, &[], &["chunked"], BadRequest),
        // Two Transfer-Encoding headers, chunked twice included.
        (v11, &[], &["chunked", "chunked"], BadRequest),
        (v11, &[], &["gzip", "chunked"], BadRequest),
        (v11, &[], &["gzip"], NotImplemented),
        (v11, &[], &["chunked, gzip"], NotImplemented),
        (v11, &[], &["chunked;x"], BadRequest),
        // Two Content-Length headers, even equal.
        (v11, &["4", "4"], &[], BadRequest),
        (v11, &["4", "5"], &[], BadRequest),
        (v11, &["0", "0"], &[], BadRequest),
        (v11, &["4,4"], &[], BadRequest),
        (v11, &["-4"], &[], BadRequest),
        (v11, &[""], &[], BadRequest),
        // Over the body limit, before any byte of it.
        (v11, &["11"], &[], ContentTooLarge),
        (v11, &["9999999999999999999"], &[], ContentTooLarge),
    ] {
        assert_eq!(
            body_framing(version, lengths, codings, 10),
            Err(refusal),
            "{version:?} {lengths:?} {codings:?}"
        );
    }
}

#[test]
fn connection_lists_only_its_own_headers() {
    let read = |value: &str| {
        let mut options = ConnectionOptions::default();
        connection(value, &mut options).map(|()| options)
    };
    assert_eq!(
        read("close"),
        Ok(ConnectionOptions {
            close: true,
            keep_alive: false
        })
    );
    assert_eq!(
        read("Keep-Alive"),
        Ok(ConnectionOptions {
            close: false,
            keep_alive: true
        })
    );
    assert_eq!(
        read(
            "close, TE, Upgrade,keep-alive , transfer-encoding, content-length, trailer, proxy-connection, connection"
        ),
        Ok(ConnectionOptions {
            close: true,
            keep_alive: true
        })
    );
    for bad in [
        "",
        "close,",
        ", close",
        "close,,TE",
        // A hop-by-hop list may not remove an end-to-end header.
        "close, x-forwarded-for",
        "host",
        "authorization",
        "cookie",
        "x-real-ip",
        "keep-alive, content-type",
        "clos",
        "close;q",
        "\"close\"",
    ] {
        assert_eq!(read(bad), Err(BadRequest), "{bad:?}");
    }
}

#[test]
fn chunk_sizes() {
    assert_eq!(chunk_size(b"0"), Ok(0));
    assert_eq!(chunk_size(b"000"), Ok(0));
    assert_eq!(chunk_size(b"aF"), Ok(0xaf));
    assert_eq!(chunk_size(b"ffffffffffffffff"), Ok(u64::MAX));
    assert_eq!(chunk_size(b"0000000000000001"), Ok(1));
    for bad in [
        &b""[..],
        b"5;ext",
        b"5;",
        b"5 ;ext=1",
        b"5 ",
        b" 5",
        b"\t5",
        b"+5",
        b"-5",
        b"0x5",
        b"5g",
        b"1ffffffffffffffff",
        b"00000000000000000",
        b"\xef\xbc\x95",
    ] {
        assert_eq!(
            chunk_size(bad),
            Err(BadRequest),
            "{}",
            String::from_utf8_lossy(bad)
        );
    }
}

/// A chunked body read from `bytes` fed in pieces at `splits`: the body, and the bytes read.
fn read_chunked(
    bytes: &[u8],
    splits: &[usize],
    max_body: u64,
    trailers: HeadBudget,
) -> Result<(Vec<u8>, usize), Refusal> {
    let mut chunked = ChunkedBody::new(max_body, trailers);
    let mut body = Vec::new();
    let mut from = 0;
    for &at in splits.iter().chain(std::iter::once(&bytes.len())) {
        let at = at.clamp(from, bytes.len());
        let used = chunked.feed(&bytes[from..at], &mut body)?;
        if chunked.done() {
            assert_eq!(chunked.length(), u64::try_from(body.len()).unwrap());
            return Ok((body, from + used));
        }
        assert_eq!(used, at - from, "an unfinished body reads its whole piece");
        from = at;
    }
    // The connection ended before the body did.
    Err(BadRequest)
}

const ROOMY: HeadBudget = HeadBudget {
    lines: 100,
    bytes: 16 * 1024,
};

#[test]
fn chunked_bodies() {
    let bytes = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\nGET / HTTP/1.1\r\n";
    assert_eq!(
        read_chunked(bytes, &[], 100, ROOMY),
        Ok((b"hello world".to_vec(), 26))
    );
    let trailers = b"3\r\nabc\r\n0\r\nX-T: 1\r\nY: 2\r\n\r\n";
    assert_eq!(
        read_chunked(trailers, &[], 100, ROOMY),
        Ok((b"abc".to_vec(), trailers.len()))
    );
    let empty = b"0\r\n\r\n";
    assert_eq!(read_chunked(empty, &[], 0, ROOMY), Ok((Vec::new(), 5)));
    let upper = b"A\r\n0123456789\r\n0\r\n\r\n";
    assert_eq!(
        read_chunked(upper, &[], 10, ROOMY).map(|r| r.0.len()),
        Ok(10)
    );
    for (bad, refusal) in [
        // Chunk extensions are refused, not ignored.
        (&b"5;ext\r\nhello\r\n0\r\n\r\n"[..], BadRequest),
        (b"5;\r\nhello\r\n0\r\n\r\n", BadRequest),
        (b"0;ext\r\n\r\n", BadRequest),
        (b"5 \r\nhello\r\n0\r\n\r\n", BadRequest),
        (b" 5\r\nhello\r\n0\r\n\r\n", BadRequest),
        (b"0x5\r\nhello\r\n0\r\n\r\n", BadRequest),
        (b"-1\r\n", BadRequest),
        (b"\r\n", BadRequest),
        (b"00000000000000005\r\nhello\r\n0\r\n\r\n", BadRequest),
        (b"1ffffffffffffffff\r\n", BadRequest),
        // Bare LF and bare CR anywhere in the framing.
        (b"5\nhello\r\n0\r\n\r\n", BadRequest),
        (b"5\r\nhello\n0\r\n\r\n", BadRequest),
        (b"5\r\nhello\r\n0\n\r\n", BadRequest),
        (b"5\r\nhello\r\n0\r\n\n", BadRequest),
        (b"5\rhello\r\n0\r\n\r\n", BadRequest),
        (b"5\r\nhello\r0\r\n\r\n", BadRequest),
        // The data is followed by CRLF and nothing else.
        (b"3\r\nhello\r\n0\r\n\r\n", BadRequest),
        (b"5\r\nhell\r\n0\r\n\r\n", BadRequest),
        (b"5\r\nhello  \r\n0\r\n\r\n", BadRequest),
        // Trailers are valid header lines.
        (b"0\r\n folded: x\r\n\r\n", BadRequest),
        (b"0\r\nX : a\r\n\r\n", BadRequest),
        (b"0\r\nX: a\x01\r\n\r\n", BadRequest),
        (b"0\r\nX: \xff\r\n\r\n", BadRequest),
        (b"0\r\nno colon\r\n\r\n", BadRequest),
        // Over the body limit, refused at the size line.
        (b"b\r\n", ContentTooLarge),
        (b"6\r\nhello \r\n5\r\n", ContentTooLarge),
        (b"ffffffffffffffff\r\n", ContentTooLarge),
    ] {
        for at in 0..=bad.len() {
            assert_eq!(
                read_chunked(bad, &[at], 10, ROOMY).map(|_| ()),
                Err(refusal),
                "{} at {at}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}

#[test]
fn trailers_spend_what_the_head_left() {
    let bytes = b"0\r\nA: 1\r\nB: 2\r\n\r\n";
    // Two lines of 6 bytes, then the empty line of 2: 14 bytes and 2 lines exactly.
    let exact = HeadBudget::new(2, 14);
    assert_eq!(
        read_chunked(bytes, &[], 0, exact).map(|r| r.1),
        Ok(bytes.len())
    );
    for tight in [
        HeadBudget::new(1, 14),
        HeadBudget::new(2, 13),
        HeadBudget::new(0, 100),
    ] {
        for at in 0..=bytes.len() {
            assert_eq!(
                read_chunked(bytes, &[at], 0, tight).map(|_| ()),
                Err(HeaderFieldsTooLarge),
                "{tight:?} at {at}"
            );
        }
    }
    // With nothing left at all, even the empty line does not fit.
    assert_eq!(
        read_chunked(b"0\r\n\r\n", &[], 0, HeadBudget::new(5, 1)),
        Err(HeaderFieldsTooLarge)
    );
    // A long trailer is refused before it is kept.
    let long = format!("0\r\nX: {}", "y".repeat(10_000));
    let mut chunked = ChunkedBody::new(0, HeadBudget::new(10, 100));
    assert_eq!(
        chunked.feed(long.as_bytes(), &mut Vec::new()),
        Err(HeaderFieldsTooLarge)
    );
}

#[test]
fn head_budget() {
    let mut budget = HeadBudget::new(1, 10);
    assert_eq!(budget.charge(6, true), Ok(()));
    assert_eq!(budget, HeadBudget::new(0, 2));
    assert_eq!(budget.charge(0, true), Err(HeaderFieldsTooLarge));
    assert_eq!(budget.charge(1, false), Err(HeaderFieldsTooLarge));
    assert_eq!(budget.charge(0, false), Ok(()));
    assert_eq!(budget.charge(usize::MAX, false), Err(HeaderFieldsTooLarge));
}

#[test]
fn origin_forms() {
    assert_eq!(
        origin_form("/a/b?c=d&e"),
        Ok(OriginForm {
            path: "/a/b",
            query: Some("c=d&e")
        })
    );
    assert_eq!(
        origin_form("/"),
        Ok(OriginForm {
            path: "/",
            query: None
        })
    );
    assert_eq!(
        origin_form("/?"),
        Ok(OriginForm {
            path: "/",
            query: Some("")
        })
    );
    assert_eq!(
        origin_form("/a?b?c/d"),
        Ok(OriginForm {
            path: "/a",
            query: Some("b?c/d")
        })
    );
    for ok in [
        "/a-._~!$&'()*+,;=:@b",
        "/%20%2F%2f%00%ff",
        "/a?x=%7C&y=-._~!$'()*+,;=:@/?",
        "/..;/x",
        "/%252e%252e",
    ] {
        assert!(origin_form(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "*",
        "a/b",
        "example.com:443",
        "http://example.com/",
        "/a#frag",
        "/a?b#frag",
        "/a b",
        "/a|b",
        "/a?b|c",
        "/a?{x}",
        "/a?\"x\"",
        "/a\\b",
        "/a^b",
        "/a`b",
        "/a<b>",
        "/a[b]",
        "/%",
        "/%2",
        "/%2g",
        "/a?%zz",
        "/a?b=%",
        "/\u{e9}",
    ] {
        assert_eq!(origin_form(bad), Err(BadRequest), "{bad:?}");
    }
}

#[test]
fn path_segments() {
    assert_eq!(segments("/"), Ok(vec![String::new()]));
    assert_eq!(
        segments("/a//b"),
        Ok(vec!["a".to_owned(), String::new(), "b".to_owned()])
    );
    assert_eq!(
        segments("/a/b/"),
        Ok(vec!["a".to_owned(), "b".to_owned(), String::new()])
    );
    assert_eq!(segments("/caf%C3%A9"), Ok(vec!["caf\u{e9}".to_owned()]));
    assert_eq!(segments("/a+b%20c"), Ok(vec!["a+b c".to_owned()]));
    // Forms other servers treat as special are ordinary segments here.
    assert_eq!(
        segments("/..;/x"),
        Ok(vec!["..;".to_owned(), "x".to_owned()])
    );
    assert_eq!(segments("/%252e%252e"), Ok(vec!["%2e%2e".to_owned()]));
    assert_eq!(
        segments("/.a/a./..."),
        Ok(vec![".a".to_owned(), "a.".to_owned(), "...".to_owned()])
    );
    for bad in [
        "/.",
        "/..",
        "/a/./b",
        "/a/../b",
        "/a/..",
        "/%2e",
        "/%2E",
        "/%2e%2e",
        "/.%2E",
        "/a/%2e%2e/b",
        "/a%2fb",
        "/a%2Fb",
        "/a%5cb",
        "/a%5Cb",
        "/a%00b",
        "/a%01b",
        "/a%0d%0ab",
        "/a%7fb",
        "/%ff",
        "/%c3",
        "/%c0%ae%c0%ae",
        "/%ed%a0%80",
        "a",
        "",
    ] {
        assert_eq!(segments(bad), Err(BadRequest), "{bad:?}");
    }
}

#[test]
fn query_parameters() {
    let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    };
    assert_eq!(query(""), Ok(Vec::new()));
    assert_eq!(query("&&"), Ok(Vec::new()));
    assert_eq!(
        query("a=1&b&c=&=d"),
        Ok(pairs(&[("a", "1"), ("b", ""), ("c", ""), ("", "d")]))
    );
    assert_eq!(query("a=b=c"), Ok(pairs(&[("a", "b=c")])));
    assert_eq!(query("q=a+b%2Bc%20d"), Ok(pairs(&[("q", "a b+c d")])));
    assert_eq!(query("x=%26&y=%3D"), Ok(pairs(&[("x", "&"), ("y", "=")])));
    assert_eq!(
        query("caf%C3%A9=%E2%9C%93"),
        Ok(pairs(&[("caf\u{e9}", "\u{2713}")]))
    );
    assert_eq!(query("a=1&a=2"), Ok(pairs(&[("a", "1"), ("a", "2")])));
    for bad in ["a=%ff", "%c3=1", "a=%ed%a0%80", "a=%c0%af"] {
        assert_eq!(query(bad), Err(BadRequest), "{bad:?}");
    }
}

#[test]
fn statuses() {
    let all = [
        (Refusal::BadRequest, 400),
        (Refusal::ContentTooLarge, 413),
        (Refusal::UriTooLong, 414),
        (Refusal::UnsupportedMediaType, 415),
        (Refusal::ExpectationFailed, 417),
        (Refusal::HeaderFieldsTooLarge, 431),
        (Refusal::NotImplemented, 501),
        (Refusal::VersionNotSupported, 505),
    ];
    for (refusal, status) in all {
        assert_eq!(refusal.status(), status);
    }
}

/// A random well-formed chunked body: its bytes and its data.
fn chunked(rng: &mut Rng) -> (Vec<u8>, Vec<u8>) {
    let body: Vec<u8> = (0..rng.below(60))
        .map(|_| b"ab \r\n0;:\xc3"[rng.below(9)])
        .collect();
    let mut out = Vec::new();
    let mut rest = body.as_slice();
    while !rest.is_empty() {
        let (chunk, after) = rest.split_at(rng.below(rest.len()) + 1);
        let size = if rng.below(2) == 0 {
            format!("{:x}", chunk.len())
        } else {
            format!("{:03X}", chunk.len())
        };
        out.extend_from_slice(size.as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\r\n");
        rest = after;
    }
    out.extend_from_slice(b"0\r\n");
    for i in 0..rng.below(3) {
        out.extend_from_slice(format!("X-T{i}: v\r\n").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    (out, body)
}

#[test]
fn every_split_of_a_chunked_body_reads_as_the_whole() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..300 {
        let (mut bytes, body) = chunked(&mut rng);
        let len = bytes.len();
        // Bytes of a next request stay unread.
        bytes.extend_from_slice(b"GET / HTTP/1.1\r\n");
        let whole = read_chunked(&bytes, &[], 1 << 20, ROOMY);
        assert_eq!(whole, Ok((body, len)));
        for at in 0..=bytes.len() {
            assert_eq!(read_chunked(&bytes, &[at], 1 << 20, ROOMY), whole);
        }
        let mut splits: Vec<usize> = (0..rng.below(8))
            .map(|_| rng.below(bytes.len() + 1))
            .collect();
        splits.sort_unstable();
        assert_eq!(read_chunked(&bytes, &splits, 1 << 20, ROOMY), whole);
    }
}

/// The client's reading of a chunked body, through its response parser.
fn client_reads(bytes: &[u8]) -> Result<Vec<u8>, parse::Error> {
    let mut parser = parse::Parser::new(parse::Limits::default());
    parser.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")?;
    parser.feed(bytes)?;
    parser.finish().map(|response| response.body)
}

#[test]
fn mutated_bodies_never_panic_split_alike_and_agree_with_the_client() {
    // Random mutations of valid bodies (bytes replaced, inserted or removed, and truncations)
    // give a body or a refusal, the same whatever the splits. Where the client's lenient
    // reader and this strict one both accept, they read the same body.
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut both = 0usize;
    for _ in 0..4000 {
        let (mut bytes, _) = chunked(&mut rng);
        for _ in 0..rng.below(3) + 1 {
            if bytes.is_empty() {
                break;
            }
            let at = rng.below(bytes.len());
            match rng.below(5) {
                0 => bytes[at] = b"0aF \r\n:;-\x00\xff\t"[rng.below(12)],
                1 => bytes.insert(at, b"0\r\n:;"[rng.below(5)]),
                2 => {
                    bytes.remove(at);
                }
                3 => bytes.truncate(at),
                _ => {}
            }
        }
        let whole = read_chunked(&bytes, &[], 1 << 20, ROOMY);
        for _ in 0..4 {
            let at = rng.below(bytes.len() + 1);
            assert_eq!(read_chunked(&bytes, &[at], 1 << 20, ROOMY), whole);
        }
        // Compare only bodies that end exactly where the bytes do, as the client reads to the
        // close.
        if let (Ok((strict, used)), Ok(lenient)) = (&whole, client_reads(&bytes))
            && *used == bytes.len()
        {
            assert_eq!(strict, &lenient, "{}", String::from_utf8_lossy(&bytes));
            both += 1;
        }
    }
    assert!(
        both > 500,
        "the corpus exercises the differential test: {both}"
    );
}

#[test]
fn the_strict_reader_refuses_what_the_client_tolerates() {
    // The leniencies of `parse.rs` that the server does not share (ADR 0123 rule 10).
    for lenient in [&b"5;ext=1\r\nhello\r\n0\r\n\r\n"[..], b"0;x\r\n\r\n"] {
        assert!(client_reads(lenient).is_ok());
        assert_eq!(
            read_chunked(lenient, &[], 100, ROOMY).map(|_| ()),
            Err(BadRequest)
        );
    }
    // The client counts its trailers afresh; the server spends what the head left.
    let trailer = b"0\r\nX: 1\r\n\r\n";
    assert!(client_reads(trailer).is_ok());
    assert_eq!(
        read_chunked(trailer, &[], 0, HeadBudget::new(0, 100)),
        Err(HeaderFieldsTooLarge)
    );
    // The client repairs a trailer value that is not UTF-8; the server refuses it.
    let invalid = b"0\r\nX: \xff\r\n\r\n";
    assert!(client_reads(invalid).is_ok());
    assert_eq!(read_chunked(invalid, &[], 0, ROOMY), Err(BadRequest));
}

#[test]
fn chunked_reading_is_linear() {
    // One-byte chunks, the most framing per byte of body: the time per byte does not grow
    // with the body. A quadratic step would multiply the time by 16 more than the bytes do.
    let time = |n: usize| {
        let mut bytes = b"1\r\nx\r\n".repeat(n);
        bytes.extend_from_slice(b"0\r\n\r\n");
        let mut best = std::time::Duration::MAX;
        for _ in 0..3 {
            let start = std::time::Instant::now();
            let (body, _) = read_chunked(&bytes, &[], u64::MAX, ROOMY).unwrap();
            assert_eq!(body.len(), n);
            best = best.min(start.elapsed());
        }
        best
    };
    let small = time(1 << 14);
    let large = time(1 << 18);
    assert!(
        large < small * 16 * 8,
        "16x the bytes took {:?} against {:?}",
        large,
        small
    );
}
