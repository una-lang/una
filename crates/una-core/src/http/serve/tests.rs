use super::*;

fn grant(spec: &str) -> ServeGrant {
    match ServeGrant::parse(spec) {
        Ok(grant) => grant,
        Err(errors) => panic!("{spec}: {errors:?}"),
    }
}

fn errors(spec: &str) -> Vec<(Option<usize>, String)> {
    match ServeGrant::parse(spec) {
        Ok(grant) => panic!("{spec} was accepted: {grant:?}"),
        Err(errors) => errors.into_iter().map(|e| (e.item, e.message)).collect(),
    }
}

fn items_in_error(spec: &str) -> Vec<Option<usize>> {
    errors(spec).into_iter().map(|(item, _)| item).collect()
}

#[test]
fn a_grant_without_items_has_every_default() {
    let g = grant("serve:http://127.0.0.1:8080");
    assert!(!g.tls);
    assert_eq!(g.address, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(g.port, 8080);
    assert_eq!(g.listener(), "127.0.0.1:8080");
    assert_eq!(g.hosts, Hosts::Local);
    assert_eq!(g.files, None);
    assert!(!g.tls12);
    assert_eq!(g.tls_policy(), None);
    assert_eq!(
        g.limits,
        Limits {
            once: 256,
            rate: None,
            waiting: 1024,
            connections: 1024,
            peer: Some(64),
            handshakes: 512,
            requests: 1000,
            line: 8 * 1024,
            headers: 100,
            header: 16 * 1024,
            body: 1024 * 1024,
            buffers: 64 << 20,
            peer_buffers: Some(1 << 20),
            replies: 64 << 20,
            peer_replies: Some(1 << 20),
            delivered: 256 << 20,
            idle: 5,
            read: 20,
            write: 30,
            answer: 30,
        }
    );
    assert_eq!(g.limits.peer_handshakes(), 32);
}

#[test]
fn derived_defaults_follow_the_values_given() {
    let g = grant("serve:http://127.0.0.1:8080;connections=4096;buffers=1024;replies=128");
    assert_eq!(g.limits.peer, Some(256));
    assert_eq!(g.limits.peer_buffers, Some(16 << 20));
    assert_eq!(g.limits.peer_replies, Some(2 << 20));
    // The floors: 16 connections, 1 MiB.
    let small = grant("serve:http://127.0.0.1:8080;connections=1;buffers=1;replies=63");
    assert_eq!(small.limits.peer, Some(16));
    assert_eq!(small.limits.peer_buffers, Some(1 << 20));
    assert_eq!(small.limits.peer_replies, Some(1 << 20));
    let given = grant("serve:http://127.0.0.1:8080;peer=none;peer_buffers=none;peer_replies=3");
    assert_eq!(given.limits.peer, None);
    assert_eq!(given.limits.peer_buffers, None);
    assert_eq!(given.limits.peer_replies, Some(3 << 20));
    assert_eq!(
        grant("serve:http://127.0.0.1:1;handshakes=16")
            .limits
            .peer_handshakes(),
        4
    );
    assert_eq!(
        grant("serve:http://127.0.0.1:1;handshakes=160")
            .limits
            .peer_handshakes(),
        10
    );
}

#[test]
fn every_item_at_its_bounds() {
    let g = grant(
        "serve:https://[::]:8443;certificate=tls/chain.pem;key=tls/key.pem;tls12;2147483647/once;\
         1/hour;waiting=0;connections=1048576;peer=1048576;handshakes=1;requests=2147483647;\
         line=64;headers=1000;header=256;body=0;buffers=65536;peer_buffers=1;replies=1;\
         peer_replies=65536;delivered=65536;idle=1;read=3600;write=3600;answer=86400;hosts=*",
    );
    assert!(g.tls && g.tls12 && g.wildcard());
    assert_eq!(g.listener(), "[::]:8443");
    assert_eq!(
        g.files,
        Some(TlsFiles {
            certificate: "tls/chain.pem".to_owned(),
            key: "tls/key.pem".to_owned()
        })
    );
    assert_eq!(g.hosts, Hosts::Any);
    let l = g.limits;
    assert_eq!(l.once, MAX_COUNT);
    assert_eq!(
        l.rate,
        Some(Rate {
            count: 1,
            period: Period::Hour
        })
    );
    assert_eq!(
        (l.waiting, l.connections, l.peer),
        (0, 1 << 20, Some(1 << 20))
    );
    assert_eq!(
        (l.line, l.headers, l.header, l.body),
        (64 << 10, 1000, 256 << 10, 0)
    );
    assert_eq!(l.buffers, 64 << 30);
    assert_eq!(l.delivered, 64 << 30);
    assert_eq!((l.idle, l.read, l.write, l.answer), (1, 3600, 3600, 86_400));
    for low in [
        "1/once",
        "2147483647/second",
        "1/minute",
        "connections=1",
        "peer=1",
        "handshakes=2147483647",
        "requests=1",
        "line=1",
        "headers=1",
        "header=1",
        "body=1048576",
        "buffers=1",
        "peer_buffers=65536",
        "replies=65536",
        "peer_replies=1",
        "delivered=1",
        "waiting=2147483647",
        "idle=3600",
        "read=1",
        "write=1",
        "answer=1",
    ] {
        grant(&format!("serve:http://127.0.0.1:8080;{low}"));
    }
}

#[test]
fn values_out_of_range_name_their_item() {
    for (item, message) in [
        ("0/once", "`N/once` counts from 1 to 2147483647, not `0`"),
        (
            "2147483648/second",
            "`N/second` counts from 1 to 2147483647, not `2147483648`",
        ),
        (
            "waiting=2147483648",
            "`waiting` is a number from 0 to 2147483647, not `2147483648`",
        ),
        (
            "connections=0",
            "`connections` is a number from 1 to 1048576, not `0`",
        ),
        (
            "connections=1048577",
            "`connections` is a number from 1 to 1048576, not `1048577`",
        ),
        (
            "peer=0",
            "`peer` is a number from 1 to 1048576, or `none`, not `0`",
        ),
        (
            "handshakes=0",
            "`handshakes` is a number from 1 to 2147483647, not `0`",
        ),
        (
            "requests=0",
            "`requests` is a number from 1 to 2147483647, not `0`",
        ),
        ("line=0", "`line` is a number of KiB from 1 to 64, not `0`"),
        (
            "line=65",
            "`line` is a number of KiB from 1 to 64, not `65`",
        ),
        (
            "headers=1001",
            "`headers` is a number from 1 to 1000, not `1001`",
        ),
        (
            "header=257",
            "`header` is a number of KiB from 1 to 256, not `257`",
        ),
        (
            "body=1048577",
            "`body` is a number of KiB from 0 to 1048576, not `1048577`",
        ),
        (
            "buffers=0",
            "`buffers` is a number of MiB from 1 to 65536, not `0`",
        ),
        (
            "peer_buffers=65537",
            "`peer_buffers` is a number of MiB from 1 to 65536, or `none`, not `65537`",
        ),
        (
            "replies=65537",
            "`replies` is a number of MiB from 1 to 65536, not `65537`",
        ),
        (
            "peer_replies=0",
            "`peer_replies` is a number of MiB from 1 to 65536, or `none`, not `0`",
        ),
        (
            "delivered=0",
            "`delivered` is a number of MiB from 1 to 65536, not `0`",
        ),
        (
            "idle=0",
            "`idle` is a number of seconds from 1 to 3600, not `0`",
        ),
        (
            "read=3601",
            "`read` is a number of seconds from 1 to 3600, not `3601`",
        ),
        (
            "write=0",
            "`write` is a number of seconds from 1 to 3600, not `0`",
        ),
        (
            "answer=86401",
            "`answer` is a number of seconds from 1 to 86400, not `86401`",
        ),
        (
            "connections=none",
            "`connections` is a number from 1 to 1048576, not `none`",
        ),
        (
            "waiting=08",
            "`waiting` is a number from 0 to 2147483647, not `08`",
        ),
        (
            "waiting=+8",
            "`waiting` is a number from 0 to 2147483647, not `+8`",
        ),
        (
            "waiting=",
            "`waiting` is a number from 0 to 2147483647, not ``",
        ),
        (
            "waiting=99999999999999999999999",
            "`waiting` is a number from 0 to 2147483647, not `99999999999999999999999`",
        ),
    ] {
        assert_eq!(
            errors(&format!("serve:http://127.0.0.1:8080;tls=1;{item}")),
            vec![(Some(0), unknown("tls=1")), (Some(1), message.to_owned())],
            "{item}"
        );
    }
}

#[test]
fn unknown_repeated_and_second_rates_are_refused() {
    assert_eq!(
        items_in_error("serve:http://127.0.0.1:8080;backlog=5;;once;5/day;Waiting=1"),
        [Some(0), Some(1), Some(2), Some(3), Some(4)]
    );
    assert_eq!(
        errors("serve:http://127.0.0.1:8080;waiting=1;idle=2;waiting=1"),
        [(Some(2), "`waiting` is given twice".to_owned())]
    );
    assert_eq!(
        errors("serve:http://127.0.0.1:8080;10/second;600/minute"),
        [(
            Some(1),
            "`600/minute` is a second rate; a serve grant has one rate".to_owned()
        )]
    );
    assert_eq!(
        items_in_error(
            "serve:http://127.0.0.1:8080;1/once;2/once;peer=none;peer=3;hosts=*;hosts=*"
        ),
        [Some(1), Some(3), Some(5)]
    );
}

#[test]
fn tls_items_belong_to_https_grants_which_need_both_files() {
    assert_eq!(
        items_in_error("serve:http://127.0.0.1:8080;certificate=c.pem;key=k.pem;tls12"),
        [Some(0), Some(1), Some(2)]
    );
    assert_eq!(
        errors("serve:https://127.0.0.1:8443;certificate=c.pem"),
        [(None, "an https grant names its `key` file".to_owned())]
    );
    assert_eq!(
        errors("serve:https://127.0.0.1:8443"),
        [
            (
                None,
                "an https grant names its `certificate` file".to_owned()
            ),
            (None, "an https grant names its `key` file".to_owned())
        ]
    );
    assert_eq!(
        items_in_error("serve:https://127.0.0.1:8443;certificate=;key=k.pem"),
        [Some(0), None]
    );
    // A `;` ends an item, so a path never holds one: `b.pem` is an unknown item.
    assert_eq!(
        items_in_error("serve:https://127.0.0.1:8443;certificate=a;b.pem;key=k.pem"),
        [Some(1)]
    );
    let g = grant("serve:https://127.0.0.1:8443;key=k.pem;certificate=c.pem");
    assert_eq!(g.tls_policy(), Some(TlsPolicy::pinned(false)));
}

#[test]
fn the_tls_policy_is_pinned() {
    for tls12 in [false, true] {
        assert_eq!(
            TlsPolicy::pinned(tls12),
            TlsPolicy {
                tls12,
                require_ems: true,
                max_early_data: 0,
                stateless_tickets: false,
                session_cache: 256,
                key_log: false,
                alpn: &["http/1.1"],
                client_certificates: false,
            }
        );
    }
    let g = grant("serve:https://127.0.0.1:8443;certificate=c;key=k;tls12");
    assert_eq!(g.tls_policy().map(|p| p.tls12), Some(true));
}

#[test]
fn origins_are_canonical_ip_literals_with_a_port() {
    grant("serve:http://0.0.0.0:1");
    grant("serve:http://[::]:65535");
    grant("serve:http://[2001:db8::1]:80");
    for bad in [
        "serve:http://localhost:8080",
        "serve:http://127.0.0.1",
        "serve:http://127.0.0.1:",
        "serve:http://127.0.0.1:0",
        "serve:http://127.0.0.1:65536",
        "serve:http://127.0.0.1:08080",
        "serve:http://127.0.0.1:8080/",
        "serve:http://127.0.0.1:8080:get",
        "serve:ftp://127.0.0.1:21",
        "serve:HTTP://127.0.0.1:8080",
        "serve:http://[::ffff:127.0.0.1]:8080",
        "serve:http://[0:0:0:0:0:0:0:1]:8080",
        "serve:http://[2001:DB8::1]:8080",
        "serve:http://[fe80::1%en0]:8080",
        "serve:http://[ff02::1]:8080",
        "serve:http://224.0.0.1:8080",
        "serve:http://255.255.255.255:8080",
        "serve:http://127.000.0.1:8080",
        "serve:http://[::1:8080",
        "serve:http://user@127.0.0.1:8080",
        "serve:http://",
        "serve:",
        "http:http://127.0.0.1:8080",
    ] {
        assert_eq!(items_in_error(bad), [None], "{bad}");
    }
}

#[test]
fn hosts_lists_registered_names() {
    let g = grant("serve:http://127.0.0.1:8080;hosts=Shop.Example.,*.shop.example,api_v2.example");
    assert_eq!(
        g.hosts,
        Hosts::Names(vec![
            "shop.example".to_owned(),
            "*.shop.example".to_owned(),
            "api_v2.example".to_owned()
        ])
    );
    for bad in [
        "hosts=",
        "hosts=127.0.0.1",
        "hosts=[::1]",
        "hosts=a/b",
        "hosts=*.*.example",
        "hosts=a.*.example",
        "hosts=shop.example,SHOP.example.",
        "hosts=*,shop.example",
        "hosts=shop.example,*",
        "hosts=shop.example:80",
        "hosts=shop..example",
        "hosts=.example",
        "hosts=a,,b",
        "hosts=0x7f",
        "hosts=a.0x10",
        "hosts=a.0x",
        "hosts=1.2.3.0X",
        "hosts=*.",
        "hosts=**",
    ] {
        assert_eq!(
            items_in_error(&format!("serve:http://127.0.0.1:8080;{bad}")),
            [Some(0)],
            "{bad}"
        );
    }
}

#[test]
fn host_checks_follow_the_grant() {
    let local = Hosts::Local;
    assert!(local.allows("localhost", false));
    assert!(local.allows("127.0.0.1", true));
    assert!(local.allows("[::1]", true));
    assert!(local.allows("10.0.0.7", true));
    assert!(!local.allows("shop.example", false));
    assert!(!local.allows("localhost.example", false));
    assert!(!local.allows("rebind.attacker.example", false));
    assert!(Hosts::Any.allows("anything.example", false));
    assert!(Hosts::Any.allows("127.0.0.1", true));
    let names = grant("serve:http://0.0.0.0:80;hosts=shop.example,*.api.example").hosts;
    assert!(names.allows("shop.example", false));
    assert!(names.allows("v1.api.example", false));
    assert!(!names.allows("api.example", false));
    assert!(!names.allows("a.v1.api.example", false));
    assert!(!names.allows("www.shop.example", false));
    assert!(!names.allows("localhost", false));
    assert!(!names.allows("127.0.0.1", true));
}

#[test]
fn overlaps_are_found_in_one_pass() {
    let at = |spec: &str| {
        let g = grant(spec);
        (g.address, g.port)
    };
    let grants = [
        at("serve:http://127.0.0.1:8080"),
        at("serve:http://127.0.0.1:8080"),
        at("serve:http://0.0.0.0:8080"),
        at("serve:http://[::]:8080"),
        at("serve:https://[::1]:8080;certificate=c;key=k"),
        at("serve:http://[::1]:9090"),
        at("serve:http://[::]:9090"),
        at("serve:http://127.0.0.2:8080"),
        at("serve:http://127.0.0.1:8081"),
        at("serve:http://0.0.0.0:9090"),
    ];
    assert_eq!(overlapping(grants), [1, 2, 4, 6, 7]);
    assert_eq!(
        overlapping([at("serve:http://0.0.0.0:1"), at("serve:http://0.0.0.0:1")]),
        [1]
    );
    assert_eq!(overlapping(Vec::new()), Vec::<usize>::new());
}

#[test]
fn overlap_scales_linearly_in_grants() {
    // 1,000 to 100,000 distinct grants: the check is O(g log g), so the larger one stays
    // within a constant factor of the smaller per grant. A quadratic check would take
    // seconds on the largest.
    for count in [1_000u32, 10_000, 100_000] {
        let listeners = (0..count).map(|i| {
            let [_, a, b, c] = i.to_be_bytes();
            (IpAddr::V4(Ipv4Addr::new(10, a, b, c)), 8080)
        });
        let started = std::time::Instant::now();
        assert!(overlapping(listeners).is_empty());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{count}"
        );
    }
}

#[test]
fn rate_intervals_round_up_and_never_reach_zero() {
    let rate = |count, period| Rate { count, period }.interval_nanos();
    assert_eq!(rate(1, Period::Second), 1_000_000_000);
    assert_eq!(rate(3, Period::Second), 333_333_334);
    assert_eq!(rate(60, Period::Minute), 1_000_000_000);
    assert_eq!(rate(MAX_COUNT, Period::Second), 1);
    assert_eq!(rate(1, Period::Hour), 3_600_000_000_000);
}
