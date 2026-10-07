//! The client against local servers, plain and hostile (`spec/24`, ADR 0074, N-26): every test
//! serves on `127.0.0.1`, so none needs the network.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use una_http::client::{Error, Request, Response, Settings, send};
use una_http::grant::{Grant, Method};
use una_http::parse::Limits;

/// A server that answers each connection with `answer`, and keeps the requests it read.
struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start(answer: impl Fn(usize, &mut TcpStream) + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let answer = Arc::new(answer);
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { continue };
                let seen = Arc::clone(&seen);
                let answer = Arc::clone(&answer);
                std::thread::spawn(move || {
                    let request = read_request(&mut stream);
                    seen.lock().unwrap().push(request);
                    answer(index, &mut stream);
                });
            }
        });
        Server { port, requests }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn grant(&self, methods: &str) -> Grant {
        Grant::parse(&format!("http:http://127.0.0.1:{}:{methods}", self.port)).unwrap()
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// The head of a request, and its body by `Content-Length`.
fn read_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => bytes.push(byte[0]),
            _ => break,
        }
    }
    let head = String::from_utf8_lossy(&bytes).into_owned();
    let length = head
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .and_then(|n| n.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    let _ = stream.read_exact(&mut body);
    head + &String::from_utf8_lossy(&body)
}

fn get<'r>(url: &'r str, headers: &'r [(String, String)]) -> Request<'r> {
    Request {
        method: Method::Get,
        url,
        headers,
        body: &[],
        bearer: None,
    }
}

fn settings(timeout_ms: u64) -> Settings {
    Settings {
        limits: Limits::default(),
        timeout: Duration::from_millis(timeout_ms),
    }
}

fn reply(text: &str) -> impl Fn(usize, &mut TcpStream) + Send + Sync + 'static {
    let text = text.to_owned();
    move |_, stream| {
        let _ = stream.write_all(text.as_bytes());
    }
}

#[test]
fn a_get_sends_its_request_in_order_and_reads_the_response() {
    let server = Server::start(reply(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello",
    ));
    let url = server.url("/items?q=1");
    let headers = vec![("Accept".to_owned(), "text/plain".to_owned())];
    let response = send(
        &Request {
            bearer: Some(b"tok3n"),
            ..get(&url, &headers)
        },
        &[server.grant("get")],
        &settings(5000),
    )
    .unwrap();
    assert_eq!(
        response,
        Response {
            status: 200,
            headers: vec![
                ("Content-Type".to_owned(), "text/plain".to_owned()),
                ("Content-Length".to_owned(), "5".to_owned())
            ],
            body: "hello".to_owned()
        }
    );
    assert_eq!(
        server.requests(),
        vec![format!(
            "GET /items?q=1 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept-Encoding: identity\r\nConnection: close\r\nAccept: text/plain\r\nAuthorization: Bearer tok3n\r\n\r\n",
            server.port
        )]
    );
}

#[test]
fn a_post_sends_its_body_with_its_length() {
    let server = Server::start(reply("HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n"));
    let url = server.url("/new");
    let response = send(
        &Request {
            method: Method::Post,
            body: "{\"a\":1}".as_bytes(),
            ..get(&url, &[])
        },
        &[server.grant("post")],
        &settings(5000),
    )
    .unwrap();
    assert_eq!(response.status, 201);
    let request = &server.requests()[0];
    assert!(request.starts_with("POST /new HTTP/1.1\r\n"), "{request}");
    assert!(request.contains("Content-Length: 7\r\n"), "{request}");
    assert!(request.ends_with("\r\n\r\n{\"a\":1}"), "{request}");
}

#[test]
fn requests_outside_the_grants_or_malformed_never_connect() {
    let server = Server::start(reply("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"));
    let url = server.url("/");
    let fine = settings(5000);
    // Another method, another port, another scheme: denied before any connection.
    let post = Request {
        method: Method::Post,
        ..get(&url, &[])
    };
    assert_eq!(
        send(&post, &[server.grant("get")], &fine),
        Err(Error::Denied)
    );
    let other = format!("http://127.0.0.1:{}/", server.port + 1);
    assert_eq!(
        send(&get(&other, &[]), &[server.grant("get")], &fine),
        Err(Error::Denied)
    );
    let tls = format!("https://127.0.0.1:{}/", server.port);
    assert_eq!(
        send(&get(&tls, &[]), &[server.grant("get")], &fine),
        Err(Error::Denied)
    );
    assert_eq!(send(&get(&url, &[]), &[], &fine), Err(Error::Denied));
    // Malformed URLs and headers, including header injection and headers the client owns.
    let user = format!("http://u:p@127.0.0.1:{}/", server.port);
    assert_eq!(
        send(&get(&user, &[]), &[server.grant("get")], &fine),
        Err(Error::BadUrl)
    );
    for header in [
        ("Bad Name", "v"),
        ("Host", "evil"),
        ("Content-Length", "0"),
        ("Transfer-Encoding", "chunked"),
        ("Connection", "keep-alive"),
    ] {
        let headers = vec![(header.0.to_owned(), header.1.to_owned())];
        assert_eq!(
            send(&get(&url, &headers), &[server.grant("get")], &fine),
            Err(Error::BadRequest),
            "{header:?}"
        );
    }
    // A control character in a header value, but a tab, is `HeaderValue`, which the engines
    // turn into the abort U6004 (N-40). It is checked first, so it does not depend on the URL,
    // the grants or the other headers.
    for value in ["a\r\nInjected: 1", "a\n", "a\0", "a\x7f", "a\x01"] {
        let headers = vec![
            ("Bad Name".to_owned(), "v".to_owned()),
            ("X-A".to_owned(), value.to_owned()),
        ];
        for target in [url.as_str(), "not a url", "http://example.com/"] {
            assert_eq!(
                send(&get(target, &headers), &[server.grant("get")], &fine),
                Err(Error::HeaderValue),
                "{value:?} {target}"
            );
        }
    }
    let tab = vec![("X-A".to_owned(), "a\tb".to_owned())];
    assert_eq!(
        send(&get(&url, &tab), &[], &fine),
        Err(Error::Denied),
        "a tab is allowed"
    );
    assert!(server.requests().is_empty());
}

#[test]
fn redirects_are_followed_within_the_grants_only() {
    // 0 → 302 to /b; 1 → 200.
    let server = Server::start(|index, stream| {
        let text = if index == 0 {
            "HTTP/1.1 302 Found\r\nLocation: /b\r\nContent-Length: 0\r\n\r\n".to_owned()
        } else {
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_owned()
        };
        let _ = stream.write_all(text.as_bytes());
    });
    let url = server.url("/a");
    let response = send(
        &Request {
            bearer: Some(b"k"),
            ..get(&url, &[])
        },
        &[server.grant("get")],
        &settings(5000),
    )
    .unwrap();
    assert_eq!((response.status, response.body.as_str()), (200, "ok"));
    let requests = server.requests();
    assert!(requests[1].starts_with("GET /b HTTP/1.1\r\n"));
    // Same origin: the credential follows.
    assert!(requests[1].contains("Authorization: Bearer k\r\n"));

    // A redirect outside the grants is returned, not followed.
    let away = Server::start(reply(
        "HTTP/1.1 301 Moved\r\nLocation: http://example.com/\r\nContent-Length: 0\r\n\r\n",
    ));
    let url = away.url("/");
    let response = send(&get(&url, &[]), &[away.grant("get")], &settings(5000)).unwrap();
    assert_eq!(response.status, 301);
    assert_eq!(away.requests().len(), 1);

    // A loop stops after five redirects, with the sixth response returned.
    let looping = Server::start(reply(
        "HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n",
    ));
    let url = looping.url("/");
    let response = send(&get(&url, &[]), &[looping.grant("get")], &settings(5000)).unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(looping.requests().len(), 6);

    // A 307 after a post is returned; a 303 is followed as a get.
    let temp = Server::start(reply(
        "HTTP/1.1 307 Temporary\r\nLocation: /x\r\nContent-Length: 0\r\n\r\n",
    ));
    let url = temp.url("/");
    let post = Request {
        method: Method::Post,
        body: b"b",
        ..get(&url, &[])
    };
    let response = send(&post, &[temp.grant("get,post")], &settings(5000)).unwrap();
    assert_eq!(response.status, 307);
    let see_other = Server::start(|index, stream| {
        let text = if index == 0 {
            "HTTP/1.1 303 See Other\r\nLocation: /done\r\nContent-Length: 0\r\n\r\n"
        } else {
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
        };
        let _ = stream.write_all(text.as_bytes());
    });
    let url = see_other.url("/");
    let post = Request {
        method: Method::Post,
        body: b"b",
        ..get(&url, &[])
    };
    let response = send(&post, &[see_other.grant("get,post")], &settings(5000)).unwrap();
    assert_eq!(response.status, 200);
    assert!(see_other.requests()[1].starts_with("GET /done HTTP/1.1\r\n"));
}

#[test]
fn the_credential_never_follows_a_redirect_to_another_origin() {
    let target = Server::start(reply("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"));
    let location = target.url("/landing");
    let first = Server::start(move |_, stream| {
        let text =
            format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
        let _ = stream.write_all(text.as_bytes());
    });
    let url = first.url("/");
    let response = send(
        &Request {
            bearer: Some(b"secret-token"),
            ..get(&url, &[])
        },
        &[first.grant("get"), target.grant("get")],
        &settings(5000),
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert!(first.requests()[0].contains("Authorization: Bearer secret-token"));
    assert!(!target.requests()[0].contains("Authorization"));

    // N-30 #2: the program's own credentials stay with the first origin too. `Authorization`,
    // `Cookie` and `Proxy-Authorization`, in any case, are dropped on a redirect to another
    // origin; the other headers follow.
    let headers: Vec<(String, String)> = [
        ("authorization", "Basic dTpw"),
        ("Cookie", "session-abc"),
        ("PROXY-AUTHORIZATION", "Basic cHJveHk="),
        ("X-Trace", "t1"),
    ]
    .iter()
    .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
    .collect();
    let response = send(
        &get(&url, &headers),
        &[first.grant("get"), target.grant("get")],
        &settings(5000),
    )
    .unwrap();
    assert_eq!(response.status, 200);
    let sent = first.requests()[1].clone();
    assert!(sent.contains("Cookie: session-abc\r\n"), "{sent}");
    assert!(sent.contains("authorization: Basic dTpw\r\n"), "{sent}");
    let followed = target.requests()[1].to_ascii_lowercase();
    for name in ["authorization", "cookie", "proxy-authorization"] {
        assert!(!followed.contains(&format!("\r\n{name}:")), "{followed}");
    }
    assert!(followed.contains("\r\nx-trace: t1\r\n"), "{followed}");
}

#[test]
fn a_credential_given_twice_or_malformed_is_a_bad_request() {
    // N-30 #2 and #4: the program's `Authorization` with a key is ambiguous, and a key with a
    // control character would inject a header; both are `BadRequest`, before any connection.
    let server = Server::start(reply("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"));
    let url = server.url("/");
    let fine = settings(5000);
    let authorization = vec![("Authorization".to_owned(), "Basic dTpw".to_owned())];
    assert_eq!(
        send(
            &Request {
                bearer: Some(b"k"),
                ..get(&url, &authorization)
            },
            &[server.grant("get")],
            &fine
        ),
        Err(Error::BadRequest)
    );
    for key in [&b"k\r\nInjected: 1"[..], b"k\n", b"k\0", b"k\x7f", b"k\x01"] {
        assert_eq!(
            send(
                &Request {
                    bearer: Some(key),
                    ..get(&url, &[])
                },
                &[server.grant("get")],
                &fine
            ),
            Err(Error::BadRequest),
            "{key:?}"
        );
    }
    assert!(server.requests().is_empty());
    // Without a key, the program's own `Authorization` is sent as it is.
    send(&get(&url, &authorization), &[server.grant("get")], &fine).unwrap();
    assert!(server.requests()[0].contains("Authorization: Basic dTpw\r\n"));
}

#[test]
fn hostile_servers_meet_the_deadline_and_the_limits() {
    // Slowloris: a byte every 100 ms, forever. The total deadline ends it.
    let slow = Server::start(|_, stream| {
        let head = b"HTTP/1.1 200 OK\r\nX-Slow: ";
        for byte in head.iter().chain(std::iter::repeat(&b'a')) {
            if stream.write_all(&[*byte]).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let url = slow.url("/");
    let started = Instant::now();
    assert_eq!(
        send(&get(&url, &[]), &[slow.grant("get")], &settings(800)),
        Err(Error::TimedOut)
    );
    assert!(
        started.elapsed() < Duration::from_millis(2000),
        "{:?}",
        started.elapsed()
    );

    // A server that never answers.
    let silent = Server::start(|_, _| std::thread::sleep(Duration::from_secs(10)));
    let url = silent.url("/");
    assert_eq!(
        send(&get(&url, &[]), &[silent.grant("get")], &settings(500)),
        Err(Error::TimedOut)
    );

    // Oversized headers, and too many of them.
    let huge = Server::start(|_, stream| {
        let mut text = "HTTP/1.1 200 OK\r\n".to_owned();
        text.push_str(&format!("X-Big: {}\r\n", "x".repeat(70 * 1024)));
        let _ = stream.write_all(text.as_bytes());
    });
    let url = huge.url("/");
    assert_eq!(
        send(&get(&url, &[]), &[huge.grant("get")], &settings(5000)),
        Err(Error::Protocol)
    );
    let many = Server::start(|_, stream| {
        let mut text = "HTTP/1.1 200 OK\r\n".to_owned();
        for i in 0..101 {
            text.push_str(&format!("X-{i}: v\r\n"));
        }
        text.push_str("\r\n");
        let _ = stream.write_all(text.as_bytes());
    });
    let url = many.url("/");
    assert_eq!(
        send(&get(&url, &[]), &[many.grant("get")], &settings(5000)),
        Err(Error::Protocol)
    );

    // Malformed chunks.
    let chunks = Server::start(reply(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhelloXX\r\n0\r\n\r\n",
    ));
    let url = chunks.url("/");
    assert_eq!(
        send(&get(&url, &[]), &[chunks.grant("get")], &settings(5000)),
        Err(Error::Protocol)
    );

    // An endless body stops at the limit.
    let endless = Server::start(|_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        let block = vec![b'x'; 64 * 1024];
        while stream.write_all(&block).is_ok() {}
    });
    let url = endless.url("/");
    let small = Settings {
        limits: Limits {
            max_body: 1024 * 1024,
            ..Limits::default()
        },
        timeout: Duration::from_secs(10),
    };
    assert_eq!(
        send(&get(&url, &[]), &[endless.grant("get")], &small),
        Err(Error::TooLarge)
    );

    // A body that is not UTF-8.
    let binary = Server::start(|_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n\xff\xfe");
    });
    let url = binary.url("/");
    assert_eq!(
        send(&get(&url, &[]), &[binary.grant("get")], &settings(5000)),
        Err(Error::Protocol)
    );
}

#[test]
fn an_unreachable_origin_is_unreachable() {
    // A port nothing listens on: bind one, then close it.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/");
    let grant = Grant::parse(&format!("http:http://127.0.0.1:{port}")).unwrap();
    assert_eq!(
        send(&get(&url, &[]), &[grant], &settings(5000)),
        Err(Error::Unreachable)
    );
}

#[test]
fn the_longest_timeout_never_overflows_the_deadline() {
    // N-30 #3: a timeout past the end of time is a day at most; the request still completes.
    let server = Server::start(reply("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"));
    let url = server.url("/");
    let endless = Settings {
        limits: Limits::default(),
        timeout: Duration::MAX,
    };
    let response = send(&get(&url, &[]), &[server.grant("get")], &endless).unwrap();
    assert_eq!(response.status, 200);
}

#[test]
fn an_ipv6_origin_is_granted_and_reached_in_brackets() {
    // N-30 #5. A host without IPv6 loopback (some containers) has nothing to reach.
    let Ok(listener) = TcpListener::bind("[::1]:0") else {
        eprintln!("no IPv6 loopback here; the grant's parsing is tested in una-core");
        return;
    };
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = read_request(&mut stream);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nv6");
        }
    });
    let url = format!("http://[::1]:{port}/");
    let grant = Grant::parse(&format!("http:http://[::1]:{port}:get")).unwrap();
    let response = send(&get(&url, &[]), &[grant], &settings(5000)).unwrap();
    assert_eq!((response.status, response.body.as_str()), (200, "v6"));
}

#[test]
fn credentials_left_behind_never_return_in_the_chain() {
    // N-35: once a redirect leaves the first origin, the key and the program's credentials
    // stay dropped for the rest of the chain, even back at that origin (as `fetch` does), so
    // the other origin cannot choose where on the first one they go.
    let b_port = Arc::new(Mutex::new(0u16));
    let a_target = Arc::new(Mutex::new(String::new()));
    let to_b = Arc::clone(&b_port);
    let a = Server::start(move |index, stream| {
        let text = if index == 0 {
            let port = *to_b.lock().unwrap();
            format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/b\r\nContent-Length: 0\r\n\r\n"
            )
        } else {
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_owned()
        };
        let _ = stream.write_all(text.as_bytes());
    });
    *a_target.lock().unwrap() = a.url("/chosen-by-b");
    let back = Arc::clone(&a_target);
    let b = Server::start(move |_, stream| {
        let location = back.lock().unwrap().clone();
        let text =
            format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
        let _ = stream.write_all(text.as_bytes());
    });
    *b_port.lock().unwrap() = b.port;
    let url = a.url("/start");
    let headers: Vec<(String, String)> = [
        ("Cookie", "session-abc"),
        ("Proxy-Authorization", "Basic cHJveHk="),
        ("X-Trace", "t1"),
    ]
    .iter()
    .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
    .collect();
    let response = send(
        &Request {
            bearer: Some(b"secret-token"),
            ..get(&url, &headers)
        },
        &[a.grant("get"), b.grant("get")],
        &settings(5000),
    )
    .unwrap();
    assert_eq!((response.status, response.body.as_str()), (200, "ok"));
    let at_a = a.requests();
    assert_eq!(at_a.len(), 2, "{at_a:?}");
    assert!(at_a[0].contains("Authorization: Bearer secret-token\r\n"));
    assert!(at_a[0].contains("Cookie: session-abc\r\n"));
    let last = at_a[1].to_ascii_lowercase();
    assert!(last.starts_with("get /chosen-by-b http/1.1\r\n"), "{last}");
    for name in ["authorization", "cookie", "proxy-authorization"] {
        assert!(!last.contains(&format!("\r\n{name}:")), "{last}");
    }
    assert!(last.contains("\r\nx-trace: t1\r\n"), "{last}");
    assert!(!b.requests()[0].to_ascii_lowercase().contains("cookie"));
}
