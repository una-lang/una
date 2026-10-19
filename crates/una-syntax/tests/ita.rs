//! The project files of edition 0.4 (ADR 0122 rules 3 to 5, 16 and 19): the lexer's project
//! mode, the three grammars, the canonical form, and robustness on mutated inputs.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_syntax::ita::{
    self, Item, LockOrigin, Origin, format, lines, parse_lock, parse_manifest, parse_workspace,
};

/// The example of ADR 0122, with every section this grammar has.
const MANIFEST: &str = r#"ita acme/ledger
  version 0.3.1
  edition 0.4
  profile app
  about "Double-entry ledgers with verified balances"
  license "Apache-2.0"
  home "https://ledger.example"
  publish to itaty

sources
  acme "https://ita.acme.example"
    root "sha256:5e0d1b7a"
    owners acme

needs
  http 1.4
    may Http[get]
  json 2.0
  acme/ledger_core 0.3 from acme
    may ledger.store.Store[put, find]
    avoid 0.3.4
  util from path "../util"
    accept name
  store from workspace

replace
  json 2.1.0 with path "patches/json"

trust
  acme/privacy 1 from itaty modules privacy.keys, privacy.more
    may Keys[hmac]

labels
  PII

trusted
  events.keys

escapes
  checked per module 1

boundaries
  gallery.thumbnailer
    may Files[read]
  webhooks.template

contracts
  pricing.update.update_prices
    @catalog: Catalog[price, set_price where not dry_run]

run
  entry store
  grants
    console
    files "data/" read
    http "https://api.payments.example" get, post
      at most 100 per minute

drive
  module store.loop
  ticks 10
  budget 1000
"#;

const LOCK: &str = r#"-- Written by `una ita`; do not edit.
lock 1

solver
  encoding "una-smt-1"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0

registry itaty
  root "sha256:9f2c"
  checkpoint 48211 "sha256:0b7e"

package http 1.4.2
  from itaty
  digest "sha256:3b1f"
  signed "ed25519:itauna:7c0a"
  logged 48190
  needs itaty json 2.1.0
  granted by store may Http[get]

package json 2.1.0
  from itaty
  digest "sha256:d41a"
  signed "ed25519:itauna:7c0a"
  logged 47002
  granted by http 1 from itaty may Http[get], Files[read]
  consented "sha256:d41a"
  trusted modules json.keys

package util 0.1.0
  from path "../util"
  digest "sha256:aa00"
  replaced
"#;

fn messages(errors: &[ita::Error]) -> Vec<String> {
    errors.iter().map(|e| e.message.clone()).collect()
}

#[test]
fn versions_are_one_token_and_text_is_not_interpolated() {
    let read = lines("a 1.4 0.1.0 1.0.0-rc.1 12 \"x\\{y\"\n");
    assert_eq!(read.errors, []);
    let items: Vec<&Item> = read.lines[0].tokens.iter().map(|t| &t.item).collect();
    assert_eq!(
        items,
        [
            &Item::Word("a".to_owned()),
            &Item::Version("1.4".to_owned()),
            &Item::Version("0.1.0".to_owned()),
            &Item::Version("1.0.0-rc.1".to_owned()),
            &Item::Int("12".to_owned()),
            &Item::Text("x{y".to_owned()),
        ]
    );
    for bad in [
        "a 01.2\n",
        "a 1.2.3.4\n",
        "a 1.2.3+b\n",
        "a 1.2.3-01\n",
        "a \"{x}\"\n",
    ] {
        assert!(!lines(bad).errors.is_empty(), "{bad:?} is an error");
    }
}

#[test]
fn keywords_are_plain_words() {
    let read = lines("from in trusted within module\n");
    assert_eq!(read.errors, []);
    assert!(
        read.lines[0]
            .tokens
            .iter()
            .all(|t| matches!(t.item, Item::Word(_)))
    );
}

#[test]
fn the_example_manifest_reads() {
    let (manifest, errors) = parse_manifest(MANIFEST);
    assert_eq!(messages(&errors), Vec::<String>::new());
    assert_eq!(manifest.name.unwrap().value.to_string(), "acme/ledger");
    assert_eq!(manifest.version.unwrap().value, "0.3.1");
    assert_eq!(manifest.edition.unwrap().value, "0.4");
    assert_eq!(manifest.publish_to.unwrap().value, "itaty");
    assert_eq!(manifest.sources.len(), 1);
    assert_eq!(manifest.sources[0].owners[0].value, "acme");
    assert_eq!(manifest.needs.len(), 5);
    let core = &manifest.needs[2];
    assert_eq!(core.name.value.owner.as_deref(), Some("acme"));
    assert_eq!(
        core.origin.as_ref().unwrap().value,
        Origin::Source("acme".to_owned())
    );
    assert_eq!(core.may[0].capability, "ledger.store.Store");
    assert_eq!(core.may[0].ops.len(), 2);
    assert_eq!(core.avoid[0].value, "0.3.4");
    assert!(manifest.needs[3].accept_name.is_some());
    assert_eq!(
        manifest.needs[3].origin.as_ref().unwrap().value,
        Origin::Path("../util".to_owned())
    );
    assert_eq!(
        manifest.needs[4].origin.as_ref().unwrap().value,
        Origin::Workspace
    );
    assert_eq!(manifest.replace[0].version.value, "2.1.0");
    assert_eq!(manifest.trust[0].major.as_ref().unwrap().value, "1");
    assert_eq!(manifest.trust[0].modules.len(), 2);
    assert_eq!(manifest.trust[0].may[0].capability, "Keys");
    assert_eq!(manifest.labels[0].value, "PII");
    assert_eq!(manifest.trusted[0].value, "events.keys");
    assert_eq!(manifest.escapes.unwrap().value, 1);
    assert_eq!(manifest.boundaries.len(), 2);
    assert!(manifest.boundaries[1].may.is_empty());
    assert_eq!(
        manifest.contracts[0].clauses[0].value,
        "@catalog: Catalog[price, set_price where not dry_run]"
    );
    let run = manifest.run.unwrap();
    assert_eq!(run.entry.unwrap().value, "store");
    let specs: Vec<&str> = run.grants.iter().map(|g| g.spec.as_str()).collect();
    assert_eq!(
        specs,
        [
            "console",
            "files:data/:read",
            "http:https://api.payments.example:get,post"
        ]
    );
    assert_eq!(run.grants[2].limits.len(), 1);
    let drive = manifest.drive.unwrap();
    assert_eq!(drive.ticks.unwrap().value, 10);
}

#[test]
fn the_grammar_is_closed() {
    let cases = [
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nforeign\n",
            "unknown section `foreign`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n  color red\n",
            "unknown head line `color`",
        ),
        ("ita x\n  version 0.1.0\n", "the head has no `edition` line"),
        (
            "ita x\n  version 0.1.0\n  version 0.2.0\n  edition 0.4\n",
            "the head line `version` appears twice",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nlabels\n  A\n\nlabels\n  B\n",
            "the section `labels` appears twice",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n\nneeds\n",
            "out of order",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nneeds\n  u from git\n",
            "there are no git dependencies",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nneeds\n  json 2\n",
            "at least `MAJOR.MINOR`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nneeds\n  json 2.0\n    may Http\n",
            "expected `[`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    foreign \"c\"\n",
            "there is no foreign grant",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    files \"d\" exec\n",
            "`read` and `write`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nescapes\n  checked 1\n",
            "checked per module",
        ),
        ("needs\n", "starts with `ita <name>`"),
    ];
    for (text, expected) in cases {
        let (_, errors) = parse_manifest(text);
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{text:?}: expected an error with {expected:?}, got {:?}",
            messages(&errors)
        );
    }
}

#[test]
fn the_example_lock_reads() {
    let (lock, errors) = parse_lock(LOCK);
    assert_eq!(messages(&errors), Vec::<String>::new());
    assert_eq!(lock.format.unwrap().value, 1);
    let solver = lock.solver.unwrap();
    assert_eq!(solver.version.value, "5.1.0");
    assert_eq!(solver.steps.value, 5_000_000);
    assert_eq!(lock.registries[0].checkpoint_size.value, 48211);
    assert_eq!(lock.packages.len(), 3);
    assert_eq!(lock.packages[0].needs[0].name.value.to_string(), "json");
    assert_eq!(lock.packages[1].granted[0].may.len(), 2);
    assert!(lock.packages[1].granted[0].identity.is_some());
    assert_eq!(lock.packages[1].trusted[0].value, "json.keys");
    assert!(lock.packages[2].replaced);
    assert_eq!(
        lock.packages[2].from.as_ref().unwrap().value,
        LockOrigin::Path("../util".to_owned())
    );
}

#[test]
fn the_lock_grammar_is_closed() {
    let cases = [
        ("lock 2\n", "lock format 2"),
        ("solver\n", "starts with `lock 1`"),
        ("lock 1\n", "no `solver` section"),
        (
            "lock 1\n\nsolver\n  name \"z3\"\n  encoding \"una-smt-1\"\n",
            "out of order",
        ),
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\npackage a 1.0.0\n  from git\n  digest \"d\"\n",
            "no git origins",
        ),
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\npackage a 1.0.0\n  digest \"d\"\n  from itaty\n",
            "out of order",
        ),
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\npackage a 1.0.0\n  from itaty\n  digest \"d\"\n  signed \"k\"\n",
            "both `signed` and `logged`",
        ),
    ];
    for (text, expected) in cases {
        let (_, errors) = parse_lock(text);
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{text:?}: expected an error with {expected:?}, got {:?}",
            messages(&errors)
        );
    }
}

#[test]
fn the_workspace_reads() {
    let text = "itaty\n  members\n    \"store\"\n    \"tools/report\"\n\ntrust\n  acme/privacy 1 from itaty modules privacy.keys\n";
    let (workspace, errors) = parse_workspace(text);
    assert_eq!(messages(&errors), Vec::<String>::new());
    assert_eq!(workspace.members.len(), 2);
    assert_eq!(workspace.trust.len(), 1);
    let (_, errors) = parse_workspace("itaty\n  members\n\nneeds\n");
    assert!(!errors.is_empty());
}

#[test]
fn the_canonical_form_orders_the_head_and_spacing() {
    let text = "-- the package\nita  x\n  edition   0.4\n  version 0.1.0 -- the release\n\n\n\nneeds\n  http  1.4\n    may Http[ get ,post ]\n";
    let canonical = format(text).unwrap();
    assert_eq!(
        canonical,
        "-- the package\nita x\n  version 0.1.0  -- the release\n  edition 0.4\n\nneeds\n  http 1.4\n    may Http[get, post]\n"
    );
    assert_eq!(format(&canonical).as_deref(), Some(canonical.as_str()));
    assert_eq!(format(MANIFEST).as_deref(), Some(MANIFEST));
    assert_eq!(format("ita x\n\tversion 1.0.0\n"), None);
}

/// A small deterministic generator, so the robustness test runs the same inputs everywhere.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n as u64)).unwrap()
    }
}

/// No input makes a reader or the formatter panic, and the canonical form is a fixed point:
/// mutations of the example files (bytes replaced, inserted, deleted and lines duplicated).
#[test]
fn mutated_inputs_never_panic_and_the_canonical_form_is_stable() {
    const PIECES: [&[u8]; 14] = [
        b"\n", b"  ", b"\t", b"\"", b"{", b"-", b"--", b".", b"1.", b"[", b"]", b"\xff", b"\r",
        b"\x00",
    ];
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    for seed in [MANIFEST, LOCK, "itaty\n  members\n    \"a\"\n"] {
        for _ in 0..1500 {
            let mut bytes = seed.as_bytes().to_vec();
            for _ in 0..=rng.below(4) {
                let at = rng.below(bytes.len() + 1);
                match rng.below(4) {
                    0 if at < bytes.len() => bytes[at] = PIECES[rng.below(PIECES.len())][0],
                    1 => {
                        let piece = PIECES[rng.below(PIECES.len())];
                        bytes.splice(at..at, piece.iter().copied());
                    }
                    2 if at < bytes.len() => {
                        let end = (at + rng.below(12)).min(bytes.len());
                        bytes.drain(at..end);
                    }
                    _ => {
                        let line_end = bytes[at..]
                            .iter()
                            .position(|b| *b == b'\n')
                            .map_or(bytes.len(), |p| at + p);
                        let copy = bytes[at..line_end].to_vec();
                        bytes.splice(at..at, copy);
                    }
                }
            }
            let text = String::from_utf8_lossy(&bytes);
            let _ = parse_manifest(&text);
            let _ = parse_lock(&text);
            let _ = parse_workspace(&text);
            if let Some(canonical) = format(&text) {
                assert_eq!(
                    format(&canonical).as_deref(),
                    Some(canonical.as_str()),
                    "the canonical form of {text:?} is not stable"
                );
            }
        }
    }
}

#[test]
fn files_past_their_bound_are_refused_whole() {
    let big = "-- x\n".repeat(ita::MAX_MANIFEST_BYTES / 5 + 1);
    let (_, errors) = parse_manifest(&big);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].range, 0..0);
}

/// The lines of a `serve` grant become the items of its text spec, one per line, each with
/// the range of its line (`spec/31 §2` rule 3).
#[test]
fn serve_grant_lines_become_the_text_spec() {
    let text = "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    serve \"https://0.0.0.0:8443\"\n      certificate \"tls/chain.pem\"\n      key \"tls/key.pem\"\n      tls12\n      at most 256 at once\n      at most 6000 per minute\n      waiting at most 0\n      connections at most 4096\n      peer none\n      handshakes at most 512 per second\n      requests at most 9\n      headers at most 100\n      line at most 8 KiB\n      header at most 16 KiB\n      body at most 64 KiB\n      buffers at most 64 MiB\n      peer_buffers at most 2 MiB\n      peer_replies none\n      delivered at most 256 MiB\n      idle within 1 second\n      answer within 10 seconds\n      hosts \"shop.example\", \"*.shop.example\"\n    serve \"http://127.0.0.1:8080\"\n";
    let (manifest, errors) = parse_manifest(text);
    assert_eq!(errors, vec![]);
    let run = manifest.run.unwrap();
    assert_eq!(
        run.grants[0].spec,
        "serve:https://0.0.0.0:8443;certificate=tls/chain.pem;key=tls/key.pem;tls12;256/once;\
         6000/minute;waiting=0;connections=4096;peer=none;handshakes=512;requests=9;headers=100;\
         line=8;header=16;body=64;buffers=64;peer_buffers=2;peer_replies=none;delivered=256;\
         idle=1;answer=10;hosts=shop.example,*.shop.example"
    );
    assert_eq!(run.grants[0].items.len(), 21);
    let first = run.grants[0].items[0].clone();
    assert_eq!(&text[first], "certificate \"tls/chain.pem\"");
    assert!(run.grants[0].limits.is_empty());
    assert_eq!(run.grants[1].spec, "serve:http://127.0.0.1:8080");
    assert!(run.grants[1].items.is_empty());
}

/// An item line of another form is an error at its line, and its grant is dropped.
#[test]
fn serve_item_lines_of_other_forms_are_errors() {
    for (line, message) in [
        ("line at most 8 MiB", "`line` is a number of KiB, written `line at most <n> KiB`"),
        ("body at most 1024", "`body` is a number of KiB, written `body at most <n> KiB`"),
        ("buffers at most 1 KiB", "`buffers` is a number of MiB, written `buffers at most <n> MiB`"),
        ("idle within 5 minutes", "`idle` is a number of seconds"),
        ("idle at most 5 seconds", "expected `idle within <n> seconds`"),
        ("connections none", "expected `at most` and a number"),
        ("backlog at most 5", "`backlog` is not an item of a serve grant (spec/31 §2)"),
        ("at most 5 per day", "a rate is per `second`, `minute` or `hour`"),
        ("at most 5", "expected `at once`, or `per` and a period"),
        ("handshakes at most 5", "`handshakes` is a number per second: `handshakes at most 512 per second`"),
        ("certificate \"a;b\"", "the path of `certificate` holds `;`, which ends an item of a grant"),
        ("hosts \"a,b\"", "`a,b` is not a host name"),
        ("tls12 on", "unexpected `on` at the end of this line"),
    ] {
        let text = format!(
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    serve \"http://127.0.0.1:8080\"\n      waiting at most 1\n      {line}\n"
        );
        let (manifest, errors) = parse_manifest(&text);
        let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, [message], "{line}");
        let at = text.find(line).unwrap();
        assert_eq!(errors[0].range.start, at, "{line}");
        assert!(manifest.run.unwrap().grants.is_empty(), "{line}");
    }
}
