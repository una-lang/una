//! The reply writer: the bytes of rule 13, every `BadReply`, injection attempts, and a round
//! trip through the client's parser.

use super::*;
use crate::parse;

fn reply(status: u16, headers: &[(&str, &str)], body: &str) -> Reply {
    Reply {
        status,
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
        body: body.to_owned(),
    }
}

const OPEN: Framing = Framing {
    head: false,
    close: false,
    date: 784_111_777,
};

fn written(reply: &Reply, framing: Framing) -> Result<String, BadReply> {
    let mut out = Vec::new();
    write(reply, framing, &mut out)?;
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn the_bytes_of_a_reply() {
    assert_eq!(
        written(&reply(201, &[("Location", "/orders/7"), ("x-a", "b\tc")], "made"), OPEN),
        Ok("HTTP/1.1 201 Created\r\nDate: Sun, 06 Nov 1994 08:49:37 GMT\r\nContent-Length: 4\r\nLocation: /orders/7\r\nx-a: b\tc\r\ncontent-type: text/plain; charset=utf-8\r\n\r\nmade".to_owned())
    );
    // The program's content type is kept, and the connection's close is written.
    let closing = Framing {
        close: true,
        ..OPEN
    };
    assert_eq!(
        written(&reply(200, &[("Content-Type", "application/json")], "{}"), closing),
        Ok("HTTP/1.1 200 OK\r\nDate: Sun, 06 Nov 1994 08:49:37 GMT\r\nContent-Length: 2\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}".to_owned())
    );
    // HEAD: the length of the body given, no body, no default type.
    let head = Framing { head: true, ..OPEN };
    assert_eq!(
        written(&reply(200, &[], "hello"), head),
        Ok(
            "HTTP/1.1 200 OK\r\nDate: Sun, 06 Nov 1994 08:49:37 GMT\r\nContent-Length: 5\r\n\r\n"
                .to_owned()
        )
    );
    // 204 and 304: no Content-Length, no type, no body.
    for status in [204, 304] {
        let text = written(&reply(status, &[("ETag", "\"x\"")], ""), OPEN).unwrap();
        assert!(
            !text.contains("Content-Length") && !text.contains("content-type"),
            "{text}"
        );
        assert!(text.ends_with("ETag: \"x\"\r\n\r\n"), "{text}");
    }
    // A status with no phrase of RFC 9110 has an empty reason.
    assert!(
        written(&reply(299, &[], ""), OPEN)
            .unwrap()
            .starts_with("HTTP/1.1 299 \r\n")
    );
    // An empty body still gets its length and type.
    assert!(
        written(&reply(200, &[], ""), OPEN)
            .unwrap()
            .contains("Content-Length: 0\r\n")
    );
}

#[test]
fn bad_replies_send_nothing() {
    let long = "v".repeat(MAX_REPLY_HEADER_BYTES);
    let many: Vec<(String, String)> = (0..=MAX_REPLY_HEADERS)
        .map(|i| (format!("x-{i}"), String::new()))
        .collect();
    let cases: Vec<(Reply, BadReply)> = vec![
        (reply(199, &[], ""), BadReply::Status),
        (reply(100, &[], ""), BadReply::Status),
        (reply(101, &[], ""), BadReply::Status),
        (reply(600, &[], ""), BadReply::Status),
        (reply(204, &[], "x"), BadReply::Body),
        (reply(304, &[], "x"), BadReply::Body),
        (reply(200, &[("", "a")], ""), BadReply::Name),
        (reply(200, &[("a b", "c")], ""), BadReply::Name),
        (reply(200, &[("a:b", "c")], ""), BadReply::Name),
        (reply(200, &[("a\r\nb", "c")], ""), BadReply::Name),
        (reply(200, &[("caf\u{e9}", "c")], ""), BadReply::Name),
        // Response splitting and header injection.
        (
            reply(200, &[("x", "a\r\nSet-Cookie: s=1")], ""),
            BadReply::Value,
        ),
        (reply(200, &[("x", "a\nb")], ""), BadReply::Value),
        (reply(200, &[("x", "a\rb")], ""), BadReply::Value),
        (reply(200, &[("x", "a\0b")], ""), BadReply::Value),
        (reply(200, &[("x", "a\x0bb")], ""), BadReply::Value),
        (reply(200, &[("x", "a\x7fb")], ""), BadReply::Value),
        (
            reply(200, &[("x", "a\r\n\r\nHTTP/1.1 200 OK\r\n")], ""),
            BadReply::Value,
        ),
        (
            Reply {
                status: 200,
                headers: many,
                body: String::new(),
            },
            BadReply::TooLarge,
        ),
        (reply(200, &[("x", long.as_str())], ""), BadReply::TooLarge),
    ];
    let mut cases = cases;
    for name in RESERVED_HEADERS {
        cases.push((reply(200, &[(name, "1")], ""), BadReply::Reserved));
        let upper = name.to_ascii_uppercase();
        cases.push((reply(200, &[(upper.as_str(), "1")], ""), BadReply::Reserved));
    }
    for (bad, expected) in cases {
        let mut out = b"kept".to_vec();
        assert_eq!(write(&bad, OPEN, &mut out), Err(expected), "{bad:?}");
        assert_eq!(out, b"kept");
    }
    // At the limits exactly, the reply is written.
    let most: Vec<(String, String)> = (0..MAX_REPLY_HEADERS)
        .map(|i| (format!("x-{i:02}"), String::new()))
        .collect();
    assert!(
        check(&Reply {
            status: 200,
            headers: most,
            body: String::new()
        })
        .is_ok()
    );
    let fits = "v".repeat(MAX_REPLY_HEADER_BYTES - 5);
    assert!(check(&reply(200, &[("x", fits.as_str())], "")).is_ok());
}

#[test]
fn host_answers() {
    let mut out = Vec::new();
    write_answer(503, true, true, 0, &mut out);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "HTTP/1.1 503 Service Unavailable\r\nDate: Thu, 01 Jan 1970 00:00:00 GMT\r\nContent-Length: 0\r\nRetry-After: 1\r\nConnection: close\r\n\r\n"
    );
    let mut out = Vec::new();
    write_answer(421, false, false, 0, &mut out);
    assert!(
        String::from_utf8(out)
            .unwrap()
            .starts_with("HTTP/1.1 421 Misdirected Request\r\n")
    );
    for refusal in [
        crate::strict::Refusal::BadRequest,
        crate::strict::Refusal::ContentTooLarge,
        crate::strict::Refusal::UriTooLong,
        crate::strict::Refusal::UnsupportedMediaType,
        crate::strict::Refusal::ExpectationFailed,
        crate::strict::Refusal::HeaderFieldsTooLarge,
        crate::strict::Refusal::NotImplemented,
        crate::strict::Refusal::VersionNotSupported,
    ] {
        assert!(!reason(refusal.status()).is_empty(), "{refusal:?}");
    }
    assert_eq!(CONTINUE, b"HTTP/1.1 100 Continue\r\n\r\n");
}

#[test]
fn dates() {
    for (seconds, date) in [
        (0, "Thu, 01 Jan 1970 00:00:00 GMT"),
        (784_111_777, "Sun, 06 Nov 1994 08:49:37 GMT"),
        (951_782_400, "Tue, 29 Feb 2000 00:00:00 GMT"),
        (1_709_164_799, "Wed, 28 Feb 2024 23:59:59 GMT"),
        (1_709_251_200, "Fri, 01 Mar 2024 00:00:00 GMT"),
        (4_102_444_800, "Fri, 01 Jan 2100 00:00:00 GMT"),
        (4_107_542_400, "Mon, 01 Mar 2100 00:00:00 GMT"),
        (253_402_300_799, "Fri, 31 Dec 9999 23:59:59 GMT"),
    ] {
        assert_eq!(http_date(seconds), date, "{seconds}");
    }
}

#[test]
fn the_client_reads_what_the_server_writes() {
    // Every reply the writer accepts is one response to the client's parser, with the same
    // status, the program's headers in order, and the same body.
    let bodies = ["", "x", "hello\r\n\r\nHTTP/1.1 200 OK\r\n", "caf\u{e9}"];
    for status in [200, 201, 204, 299, 304, 404, 503, 599] {
        for body in bodies {
            let body = if status == 204 || status == 304 {
                ""
            } else {
                body
            };
            let given = reply(status, &[("x-a", "1"), ("Set-Cookie", "s=1; Path=/")], body);
            let mut out = Vec::new();
            write(
                &given,
                Framing {
                    close: true,
                    ..OPEN
                },
                &mut out,
            )
            .unwrap();
            let mut parser = parse::Parser::new(parse::Limits::default());
            parser.feed(&out).unwrap();
            assert!(parser.done(), "{status} {body:?}");
            let response = parser.finish().unwrap();
            assert_eq!(response.status, status);
            assert_eq!(response.body, body.as_bytes());
            let names: Vec<&str> = response.headers.iter().map(|(n, _)| n.as_str()).collect();
            let at = names.iter().position(|n| *n == "x-a").unwrap();
            assert_eq!(names[at + 1], "Set-Cookie");
        }
    }
}

#[test]
fn writing_is_linear() {
    let time = |n: usize| {
        let given = reply(200, &[("x", "y")], &"z".repeat(n));
        let mut best = std::time::Duration::MAX;
        for _ in 0..3 {
            let start = std::time::Instant::now();
            let mut out = Vec::new();
            write(&given, OPEN, &mut out).unwrap();
            assert!(out.len() > n);
            best = best.min(start.elapsed());
        }
        best
    };
    let small = time(1 << 16);
    let large = time(1 << 20);
    assert!(large < small * 16 * 8 + std::time::Duration::from_millis(5));
}
