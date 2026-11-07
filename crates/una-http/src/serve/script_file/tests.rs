// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;

fn listeners() -> Vec<String> {
    vec!["127.0.0.1:8080".to_owned(), "[::1]:8443".to_owned()]
}

const OPEN: &str = r#"{"at": 0, "connection": 1, "grant": "127.0.0.1:8080", "peer": "192.0.2.1", "reads": "paced"}"#;

#[test]
fn each_form_is_read_in_order() {
    let script = format!(
        "{OPEN}\n\n{}\n{}\n{}\n{}\n",
        r#"{"at": 5, "connection": 1, "bytes": "R0VUIC8gSFRUUC8xLjENCg=="}"#,
        r#"{"at": 5, "connection": 1, "read": 64}"#,
        r#"{"at": 7, "connection": 1, "close": true}"#,
        r#"{"at": 7, "connection": 2, "grant": "[::1]:8443", "peer": "2001:db8::1", "reads": "eager"}"#,
    );
    let events = parse(&script, &listeners()).unwrap();
    let actions: Vec<(u64, u64, &Action)> = events
        .iter()
        .map(|e| (e.at, e.connection, &e.action))
        .collect();
    assert_eq!(
        actions,
        [
            (
                0,
                1,
                &Action::Open {
                    grant: 0,
                    peer: "192.0.2.1".parse().unwrap(),
                    reads: PeerReads::Paced
                }
            ),
            (5, 1, &Action::Bytes(b"GET / HTTP/1.1\r\n".to_vec())),
            (5, 1, &Action::Read(64)),
            (7, 1, &Action::Close),
            (
                7,
                2,
                &Action::Open {
                    grant: 1,
                    peer: "2001:db8::1".parse().unwrap(),
                    reads: PeerReads::Eager
                }
            ),
        ]
    );
    assert_eq!(parse("", &listeners()).unwrap(), []);
}

#[test]
fn a_line_the_run_cannot_follow_is_refused_with_its_number() {
    let cases = [
        ("[1]", "not a JSON object"),
        ("{\"at\": 0", "not a JSON object"),
        (
            r#"{"at": -1, "connection": 1, "close": true}"#,
            "`at` is not a whole number",
        ),
        (
            r#"{"at": 0.5, "connection": 1, "close": true}"#,
            "`at` is not a whole number",
        ),
        (
            r#"{"connection": 1, "close": true}"#,
            "`at` is not a whole number",
        ),
        (
            r#"{"at": 0, "close": true}"#,
            "`connection` is not a whole number",
        ),
        (
            r#"{"at": 0, "connection": 9, "close": true}"#,
            "connection 9 is not open",
        ),
        (r#"{"at": 0, "connection": 1}"#, "has none of"),
        (
            r#"{"at": 0, "connection": 2, "grant": "127.0.0.1:9", "peer": "192.0.2.1", "reads": "eager"}"#,
            "`127.0.0.1:9` is the address of no `serve:` grant of the run",
        ),
        (
            r#"{"at": 0, "connection": 2, "grant": "127.0.0.1:8080", "peer": "localhost", "reads": "eager"}"#,
            "`peer` is not an IP address",
        ),
        (
            r#"{"at": 0, "connection": 2, "grant": "127.0.0.1:8080", "peer": "192.0.2.1", "reads": "lazy"}"#,
            "`reads` is `lazy`",
        ),
        (
            r#"{"at": 0, "connection": 2, "grant": "127.0.0.1:8080", "peer": "192.0.2.1"}"#,
            "`reads` is missing",
        ),
        (
            r#"{"at": 0, "connection": 2, "grant": "127.0.0.1:8080", "peer": "192.0.2.1", "reads": "eager", "text": "x"}"#,
            "`text` is not a key of this line",
        ),
        (OPEN, "connection 1 is opened twice"),
        (
            r#"{"at": 0, "connection": 1, "bytes": "QQ="}"#,
            "`bytes` is not base64 text",
        ),
        (
            r#"{"at": 0, "connection": 1, "text": "GET"}"#,
            "has none of",
        ),
        (r#"{"at": 0, "connection": 1, "read": 0}"#, "`read` is 0"),
        (
            r#"{"at": 0, "connection": 1, "close": false}"#,
            "`close` is not `true`",
        ),
        (
            r#"{"at": 0, "connection": 1, "close": true, "read": 1}"#,
            "`close` is not a key",
        ),
        (r#"{"at": 0, "connection": 1, "stop": true}"#, "has none of"),
    ];
    for (line, wanted) in cases {
        let script = format!("{OPEN}\n{line}\n");
        let error = parse(&script, &listeners()).unwrap_err();
        assert!(error.starts_with("line 2: "), "{line}: {error}");
        assert!(error.contains(wanted), "{line}: {error}");
    }
    let eager = OPEN.replace("paced", "eager");
    let error = parse(
        &format!("{eager}\n{}\n", r#"{"at": 0, "connection": 1, "read": 4}"#),
        &listeners(),
    )
    .unwrap_err();
    assert!(error.contains("reads eagerly"), "{error}");
    let closed = format!(
        "{OPEN}\n{}\n{}\n",
        r#"{"at": 1, "connection": 1, "close": true}"#,
        r#"{"at": 1, "connection": 1, "bytes": "QQ=="}"#
    );
    assert_eq!(
        parse(&closed, &listeners()).unwrap_err(),
        "line 3: connection 1 is closed already"
    );
    let earlier = format!(
        "{}\n{OPEN}\n",
        OPEN.replace("\"at\": 0", "\"at\": 4")
            .replace("\"connection\": 1", "\"connection\": 3")
    );
    assert_eq!(
        parse(&earlier, &listeners()).unwrap_err(),
        "line 2: `at` is 0, before the line above it, at 4"
    );
}

#[test]
fn base64_is_the_padded_standard_form_only() {
    assert_eq!(base64("").unwrap(), b"");
    assert_eq!(base64("QQ==").unwrap(), b"A");
    assert_eq!(base64("QUI=").unwrap(), b"AB");
    assert_eq!(base64("QUJD").unwrap(), b"ABC");
    assert_eq!(base64("+/8=").unwrap(), [0xfb, 0xff]);
    for bad in [
        "Q", "QQ", "QQ=", "QR==", "QUJ=", "Q===", "QQ==QUJD", "QU-D", "QU_D", "QUJD\n",
    ] {
        assert_eq!(base64(bad), None, "{bad}");
    }
}

#[test]
fn reading_is_linear_in_the_script() {
    let script = |n: u64| {
        let mut text = String::new();
        for k in 0..n {
            text.push_str(&format!(
                "{{\"at\": {k}, \"connection\": {k}, \"grant\": \"127.0.0.1:8080\", \"peer\": \"192.0.2.1\", \"reads\": \"eager\"}}\n{{\"at\": {k}, \"connection\": {k}, \"bytes\": \"QUJD\"}}\n"
            ));
        }
        text
    };
    let time = |text: &str| {
        let start = std::time::Instant::now();
        assert!(parse(text, &listeners()).is_ok());
        start.elapsed()
    };
    let (small, large) = (script(2_000), script(20_000));
    let a = time(&small);
    let b = time(&large);
    assert!(
        b < a * 40 + std::time::Duration::from_millis(50),
        "{a:?} {b:?}"
    );
}
