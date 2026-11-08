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
        "{OPEN}\n{}\n{}\n{}\n{}\n",
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
            "`text` is not a key of this line",
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
        (
            r#"{"at": 0, "connection": 1, "stop": true}"#,
            "`stop` is not a key of this line",
        ),
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

#[test]
fn a_key_given_twice_is_refused_in_every_form() {
    let open = r#""grant": "127.0.0.1:8080", "peer": "192.0.2.1", "reads": "paced""#;
    for (line, key) in [
        (
            format!(r#"{{"at": 9, "at": 0, "connection": 2, {open}}}"#),
            "at",
        ),
        (
            format!(r#"{{"at": 0, "connection": 7, "connection": 2, {open}}}"#),
            "connection",
        ),
        (
            format!(r#"{{"at": 0, "connection": 2, "grant": "nope", {open}}}"#),
            "grant",
        ),
        (
            format!(r#"{{"at": 0, "connection": 2, {open}, "peer": "192.0.2.9"}}"#),
            "peer",
        ),
        (
            format!(r#"{{"at": 0, "connection": 2, {open}, "reads": "eager"}}"#),
            "reads",
        ),
        (
            r#"{"at": 0, "connection": 1, "bytes": "QQ==", "bytes": "QUI="}"#.to_owned(),
            "bytes",
        ),
        (
            r#"{"at": 0, "connection": 1, "read": 9, "read": 1}"#.to_owned(),
            "read",
        ),
        (
            r#"{"at": 0, "connection": 1, "close": false, "close": true}"#.to_owned(),
            "close",
        ),
    ] {
        assert_eq!(
            parse(&format!("{OPEN}\n{line}\n"), &listeners()).unwrap_err(),
            format!("line 2: `{key}` appears twice"),
            "{line}"
        );
    }
}

#[test]
fn lines_end_with_lf_and_hold_one_object_alone() {
    // A script with no lines opens no connection; the last LF is optional.
    assert_eq!(parse("", &listeners()).unwrap(), []);
    assert_eq!(parse(OPEN, &listeners()).unwrap().len(), 1);
    assert_eq!(parse(&format!("{OPEN}\n"), &listeners()).unwrap().len(), 1);
    let close = r#"{"at": 1, "connection": 1, "close": true}"#;
    for (script, wanted) in [
        (
            "\n".to_owned(),
            "line 1: not a JSON object alone on its line",
        ),
        (
            format!("{OPEN}\n\n{close}\n"),
            "line 2: not a JSON object alone on its line",
        ),
        (
            format!("{OPEN}\n\n"),
            "line 2: not a JSON object alone on its line",
        ),
        (
            format!("{OPEN}\n   \n{close}\n"),
            "line 2: not a JSON object alone on its line",
        ),
        (
            format!("{OPEN}\n\t\n{close}\n"),
            "line 2: not a JSON object alone on its line",
        ),
        (
            format!("{OPEN}\n {close}\n"),
            "line 2: not a JSON object alone on its line",
        ),
        (
            format!("{OPEN}\n{close} \n"),
            "line 2: not a JSON object alone on its line",
        ),
        (
            format!("{OPEN}\r\n{close}\r\n"),
            "line 1: the line holds a CR",
        ),
        (format!("{OPEN}\n{close}\r"), "line 2: the line holds a CR"),
        (
            format!("{OPEN}\n{}\n", close.replace(", ", ",\r")),
            "line 2: the line holds a CR",
        ),
        (format!("{OPEN}\r{close}\n"), "line 1: the line holds a CR"),
        (
            format!("{OPEN}{close}\n"),
            "line 1: not a JSON object: trailing characters",
        ),
    ] {
        let error = parse(&script, &listeners()).unwrap_err();
        assert!(error.starts_with(wanted), "{script:?}: {error}");
    }
}

#[test]
fn many_keys_in_one_line_are_refused_at_the_first_unknown_one() {
    let mut line = String::from("{\"at\": 0");
    for k in 0..100_000 {
        line.push_str(&format!(", \"k{k}\": 1"));
    }
    line.push('}');
    assert_eq!(
        parse(&line, &listeners()).unwrap_err(),
        "line 1: `k0` is not a key of this line"
    );
}

#[test]
fn one_peer_address_opens_at_most_one_connection_per_port() {
    let script = |n: u32, peer: &str| {
        let mut text = String::new();
        for k in 0..n {
            text.push_str(&format!(
                "{{\"at\": 0, \"connection\": {k}, \"grant\": \"127.0.0.1:8080\", \"peer\": \"{peer}\", \"reads\": \"eager\"}}\n"
            ));
        }
        text
    };
    assert!(parse(&script(PEER_CONNECTIONS, "192.0.2.1"), &listeners()).is_ok());
    let error = parse(&script(PEER_CONNECTIONS + 1, "192.0.2.1"), &listeners()).unwrap_err();
    assert_eq!(
        error,
        format!(
            "line {}: `192.0.2.1` opens more than 64512 connections, one per port it has",
            PEER_CONNECTIONS + 1
        )
    );
    // Another peer address has its own ports.
    let mixed = format!(
        "{}{}",
        script(PEER_CONNECTIONS, "192.0.2.1"),
        r#"{"at": 0, "connection": 99999999, "grant": "127.0.0.1:8080", "peer": "192.0.2.2", "reads": "eager"}"#
    );
    assert!(parse(&mixed, &listeners()).is_ok());
}
