//! The project files of edition 0.4 (ADR 0122 rules 3 to 5, 16 and 19): the lexer's project
//! mode, the three grammars, the canonical form, and robustness on mutated inputs.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_syntax::SourceFile;
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
  last root 3 "sha256:a1d4"

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
    assert_eq!(
        run.grants[2].limits[0].kind,
        ita::LimitKind::Rate("minute".to_owned())
    );
    assert_eq!(run.grants[2].limits[0].count.value, 100);
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
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  max_memory \"64\"\n",
            "the value of `max_memory`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  max_memory 64 MiB\n",
            "`MiB`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  max_memory 64\n  max_memory 32\n",
            "`max_memory` appears twice",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    console\n  max_memory 64\n",
            "`max_memory` comes before `grants`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  max_memory 64\n  entry main\n",
            "`entry` comes before `max_memory`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  memory 64\n",
            "`max_memory <MiB>`",
        ),
        ("needs\n", "starts with `ita <name>`"),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    http \"https://h\"\n      at most 3 per week\n",
            "`second`, `minute` or `hour`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    http \"https://h\"\n      at least 3 at once\n",
            "`most` after `at`",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    http \"https://h\"\n      waiting 3\n",
            "a limit",
        ),
        (
            "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    http \"https://h\"\n      at most 3 at twice\n",
            "`once` after `at`",
        ),
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
    assert_eq!(lock.registries[0].last_root.value, 3);
    assert_eq!(lock.registries[0].last_root_digest.value, "sha256:a1d4");
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
        // A registry section records the last root version the client verified (ADR 0122,
        // amendment of 2026-11-03, item 4): it is required, after `checkpoint`, once.
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\nregistry acme\n  root \"r\"\n  checkpoint 2 \"c\"\n",
            "a `last root` line",
        ),
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\nregistry acme\n  root \"r\"\n  last root 1 \"d\"\n  checkpoint 2 \"c\"\n",
            "in this order",
        ),
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\nregistry acme\n  root \"r\"\n  checkpoint 2 \"c\"\n  last root 1 \"d\"\n  last root 2 \"e\"\n",
            "once each",
        ),
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\nregistry acme\n  root \"r\"\n  checkpoint 2 \"c\"\n  last 1 \"d\"\n",
            "expected `last root",
        ),
        (
            "lock 1\n\nsolver\n  encoding \"e\"\n  name \"z3\"\n  version \"5\"\n  logic \"l\"\n  steps 1\n  seed 0\n\nregistry acme\n  root \"r\"\n  checkpoint 2 \"c\"\n  accepted acme 1\n  last root 1 \"d\"\n",
            "in this order",
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

/// The bounds are measured on the bytes as written, on both entries (`spec/30 §1` rule 4,
/// #2282): a file of more bytes than its bound whose `\r\n` line ends bring its decoded text
/// under the bound is refused whole, decoded or not, with the one message that states its
/// bytes. A file of exactly its bound in bytes is read.
#[test]
fn bounds_are_measured_on_the_bytes_on_both_entries() {
    fn past(head: &str, bound: usize) -> Vec<u8> {
        let mut text = head.replace('\n', "\r\n");
        while text.len() <= bound {
            text.push_str("-- x\r\n");
        }
        assert!(SourceFile::new("f", text.as_bytes()).text().len() <= bound);
        text.into_bytes()
    }
    let message = |bytes: usize, bound: usize, what: &str| {
        format!("the file holds {bytes} bytes, more than the {bound} {what}")
    };
    let manifest = past(
        "ita x\n  version 0.1.0\n  edition 0.4\n",
        ita::MAX_MANIFEST_BYTES,
    );
    let workspace = past("itaty\n  members\n    \"a\"\n", ita::MAX_MANIFEST_BYTES);
    let lock = past("lock 1\n", ita::MAX_LOCK_BYTES);
    let decoded = SourceFile::new("ita.una", &manifest);
    let written = std::str::from_utf8(&manifest).unwrap();
    for errors in [parse_manifest(&decoded).1, parse_manifest(written).1] {
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].range, 0..0);
        assert_eq!(
            errors[0].message,
            message(
                manifest.len(),
                ita::MAX_MANIFEST_BYTES,
                "a manifest may hold"
            )
        );
    }
    let decoded = SourceFile::new("itaty.una", &workspace);
    let written = std::str::from_utf8(&workspace).unwrap();
    for errors in [parse_workspace(&decoded).1, parse_workspace(written).1] {
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            message(workspace.len(), ita::MAX_MANIFEST_BYTES, "it may hold")
        );
    }
    let decoded = SourceFile::new("ita.lock", &lock);
    let written = std::str::from_utf8(&lock).unwrap();
    for errors in [parse_lock(&decoded).1, parse_lock(written).1] {
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            message(lock.len(), ita::MAX_LOCK_BYTES, "a lock may hold")
        );
    }
    // At the bound exactly, in bytes, the file is read.
    let mut text = String::from("ita x\r\n  version 0.1.0\r\n  edition 0.4\r\n");
    while text.len() + 6 <= ita::MAX_MANIFEST_BYTES {
        text.push_str("-- x\r\n");
    }
    text.push_str(&"-".repeat(ita::MAX_MANIFEST_BYTES - text.len()));
    assert_eq!(text.len(), ita::MAX_MANIFEST_BYTES);
    let (_, errors) = parse_manifest(&SourceFile::new("ita.una", text.as_bytes()));
    assert!(
        errors
            .iter()
            .all(|error| !error.message.contains("bytes, more than"))
    );
}

/// A file left unread past its bound keeps only its length, which the reader refuses with the
/// message of the bound (#2282).
#[test]
fn an_unread_file_is_refused_by_its_length() {
    let file = SourceFile::unread("ita.una", ita::MAX_MANIFEST_BYTES + 7);
    assert_eq!(file.text(), "");
    assert_eq!(file.raw_len(), ita::MAX_MANIFEST_BYTES + 7);
    let (_, errors) = parse_manifest(&file);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        format!(
            "the file holds {} bytes, more than the {} a manifest may hold",
            ita::MAX_MANIFEST_BYTES + 7,
            ita::MAX_MANIFEST_BYTES
        )
    );
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
        (
            "line at most 8 MiB",
            "`line` is a number of KiB, written `line at most <n> KiB`",
        ),
        (
            "body at most 1024",
            "`body` is a number of KiB, written `body at most <n> KiB`",
        ),
        (
            "buffers at most 1 KiB",
            "`buffers` is a number of MiB, written `buffers at most <n> MiB`",
        ),
        ("idle within 5 minutes", "`idle` is a number of seconds"),
        (
            "idle at most 5 seconds",
            "expected `idle within <n> seconds`",
        ),
        ("connections none", "expected `at most` and a number"),
        (
            "backlog at most 5",
            "`backlog` is not an item of a serve grant (spec/31 §2)",
        ),
        (
            "at most 5 per day",
            "a rate is per `second`, `minute` or `hour`",
        ),
        ("at most 5", "expected `at once`, or `per` and a period"),
        (
            "handshakes at most 5",
            "`handshakes` is a number per second: `handshakes at most 512 per second`",
        ),
        (
            "certificate \"a;b\"",
            "the path of `certificate` holds `;`, which ends an item of a grant",
        ),
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

/// The origin of a `serve` grant cannot carry an item after a `;`.
#[test]
fn serve_origins_hold_no_items() {
    let text = "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    serve \"http://127.0.0.1:8080;peer=none\"\n";
    let (manifest, errors) = parse_manifest(text);
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        ["the address of a serve grant holds no `;`; write each item on a line below it"]
    );
    assert!(manifest.run.unwrap().grants.is_empty());
}

#[test]
fn the_run_section_declares_a_memory_budget() {
    // ADR 0109 rule 4 under ADR 0122: `max_memory <MiB>` after `entry`, before `grants`; its
    // range is the project crate's check, so any integer reads here.
    let text = "ita x\n  version 0.1.0\n  edition 0.4\n  profile systems\n\nrun\n  entry main\n  max_memory 0\n  grants\n    console\n";
    let (manifest, errors) = parse_manifest(text);
    assert_eq!(messages(&errors), Vec::<String>::new());
    assert_eq!(manifest.profile.unwrap().value, "systems");
    let run = manifest.run.unwrap();
    let budget = run.max_memory.unwrap();
    assert_eq!(budget.value, 0);
    assert_eq!(&text[budget.range], "0");
    assert_eq!(run.grants.len(), 1);
    assert_eq!(format(text).as_deref(), Some(text));
}

#[test]
fn limit_lines_read_and_a_grant_with_a_broken_one_is_dropped() {
    let text = "ita x\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    http \"https://a\" get\n      at most 8 at once\n      waiting at most 1000\n      at most 2 per second\n    http \"https://b\"\n      at most 0 per hour\n      at most 1 per fortnight\n";
    let (manifest, errors) = parse_manifest(text);
    // The count's range is the manifest reader's to check; the period is the grammar's.
    assert_eq!(errors.len(), 1, "{:?}", messages(&errors));
    let run = manifest.run.unwrap();
    assert_eq!(run.grants.len(), 1);
    let kinds: Vec<&ita::LimitKind> = run.grants[0].limits.iter().map(|l| &l.kind).collect();
    assert_eq!(
        kinds,
        [
            &ita::LimitKind::AtOnce,
            &ita::LimitKind::Waiting,
            &ita::LimitKind::Rate("second".to_owned())
        ]
    );
}

/// Every token and comment of `lines`, depth first, with the line it is on.
fn flat(read: &ita::Lines) -> Vec<(Item, std::ops::Range<usize>)> {
    fn walk(line: &ita::Line, out: &mut Vec<(Item, std::ops::Range<usize>)>) {
        for comment in &line.comments_before {
            out.push((Item::Punct("--"), comment.clone()));
        }
        out.push((Item::Punct("line"), line.range.clone()));
        for token in &line.tokens {
            out.push((token.item.clone(), token.range.clone()));
        }
        if let Some(comment) = &line.trailing_comment {
            out.push((Item::Punct("--"), comment.clone()));
        }
        for child in &line.children {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for line in &read.lines {
        walk(line, &mut out);
    }
    for comment in &read.comments_after {
        out.push((Item::Punct("--"), comment.clone()));
    }
    out
}

/// A file with `\r\n` line ends reads as the one with `\n` alone, and every range indexes the
/// text as written (#2249): it names the same bytes, `\r\n` read as `\n`, and lies on
/// character boundaries.
#[test]
fn ranges_index_the_text_with_crlf_line_ends() {
    for lf in [MANIFEST, LOCK] {
        let crlf = lf.replace('\n', "\r\n");
        let (a, b) = (lines(lf), lines(&crlf));
        assert_eq!(b.text, crlf);
        assert!(a.errors.is_empty() && b.errors.is_empty());
        let (a, b) = (flat(&a), flat(&b));
        assert_eq!(a.len(), b.len());
        for ((item, at_lf), (item_crlf, at_crlf)) in a.iter().zip(&b) {
            assert_eq!(item, item_crlf);
            let written = crlf.get(at_crlf.clone()).unwrap();
            assert_eq!(written.replace("\r\n", "\n"), lf[at_lf.clone()]);
            assert!(!written.ends_with('\r'), "{written:?}");
        }
    }
    let (lf, _) = parse_lock(LOCK);
    let crlf_text = LOCK.replace('\n', "\r\n");
    let (crlf, errors) = parse_lock(&crlf_text);
    assert_eq!(messages(&errors), Vec::<String>::new());
    assert_eq!(crlf.packages.len(), lf.packages.len());
    for package in &crlf.packages {
        assert_eq!(
            crlf_text[package.name.range.clone()],
            package.name.value.to_string()
        );
        assert_eq!(
            crlf_text[package.version.range.clone()],
            package.version.value
        );
    }
    let (manifest, _) = parse_manifest(MANIFEST);
    let (manifest_crlf, _) = parse_manifest(&MANIFEST.replace('\n', "\r\n"));
    for (a, b) in manifest.contracts.iter().zip(&manifest_crlf.contracts) {
        let values = |c: &ita::ContractEntry| {
            c.clauses
                .iter()
                .map(|k| k.value.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(values(a), values(b));
    }
    assert_eq!(format(&MANIFEST.replace('\n', "\r\n")), format(MANIFEST));
}

/// A file decoded once, by the source database, is read as it is and never decoded again
/// (#2252): a lone `\r` just before a `\r\n` decodes to a `\r\n`, which is the lone `\r`
/// reported, never one more line end. Each reader reports it, and its range names the `\r`
/// in the text it indexes, decoded or as written.
#[test]
fn a_decoded_file_is_never_decoded_again() {
    let lone = "carriage return not followed by a line feed".to_owned();
    for (name, written) in [
        ("ita.lock", "lock 1\r\r\n"),
        (
            "ita.una",
            "ita demo\r\r\n  version 0.1.0\r\n  edition 0.4\r\n",
        ),
        ("itaty.una", "itaty\r\r\n"),
    ] {
        let file = SourceFile::new(name, written.as_bytes());
        assert!(file.text().contains("\r\n"), "{:?}", file.text());
        for read in [lines(&file), lines(written)] {
            assert_eq!(messages(&read.errors), vec![lone.clone()], "{name}");
            assert_eq!(read.text.get(read.errors[0].range.clone()), Some("\r"));
        }
        let errors = match name {
            "ita.lock" => parse_lock(&file).1,
            "ita.una" => parse_manifest(&file).1,
            _ => parse_workspace(&file).1,
        };
        assert!(messages(&errors).contains(&lone), "{name}: {errors:?}");
        assert_eq!(file.text().get(errors[0].range.clone()), Some("\r"));
    }
    let manifest = "ita demo\r\r\n  version 0.1.0\n  edition 0.4\n";
    assert_eq!(
        format(&SourceFile::new("ita.una", manifest.as_bytes())),
        None
    );
    assert_eq!(format(manifest), None);
    // The canonical form of a file with `\r\n` line ends is the same, decoded or written.
    let crlf = MANIFEST.replace('\n', "\r\n");
    let file = SourceFile::new("ita.una", crlf.as_bytes());
    assert_eq!(format(&file), format(MANIFEST));
    assert_eq!(format(&crlf), format(MANIFEST));
}

/// A decoded file keeps what its decoding found: an ill-formed UTF-8 sequence, read as
/// U+FFFD, is reported where it is, as `una check` reports it in a module (#2252).
#[test]
fn a_decoded_file_reports_an_ill_formed_sequence() {
    let file = SourceFile::new(
        "ita.una",
        b"ita demo\n  version 0.1.0\n  edition 0.4\n  about \"a\xffb\"\n",
    );
    let (_, errors) = parse_manifest(&file);
    assert_eq!(
        messages(&errors),
        vec!["invalid UTF-8 byte sequence".to_owned()]
    );
    assert_eq!(file.text().get(errors[0].range.clone()), Some("\u{fffd}"));
}

/// An error's range indexes the text as written too: an empty range at the end of a line ends
/// before its `\r`, and a range after a `\r\n` starts after it (#2249).
#[test]
fn error_ranges_index_the_text_with_crlf_line_ends() {
    let text = "lock 1\r\n\r\nsolver\r\n  encoding \"una-smt-1\"\r\n  name\r\n";
    let (_, errors) = parse_lock(text);
    assert!(!errors.is_empty());
    for error in &errors {
        let written = text.get(error.range.clone()).unwrap();
        assert!(
            !written.contains('\r') || written.contains("\r\n"),
            "{written:?}"
        );
    }
    let lf = text.replace("\r\n", "\n");
    let (_, lf_errors) = parse_lock(&lf);
    assert_eq!(messages(&errors), messages(&lf_errors));
    for (error, lf_error) in errors.iter().zip(&lf_errors) {
        assert_eq!(
            text[error.range.clone()].replace("\r\n", "\n"),
            lf[lf_error.range.clone()]
        );
    }
}
