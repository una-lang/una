//! The keys of keyed hashing (`spec/28 §1`, ADR 0119 rules 9 and 10), in this process, through
//! the test hooks of `una-hash`, which no environment variable, flag or `una.toml` key reaches:
//! a program's output and trace do not depend on the key, a host without random bytes aborts
//! the run with U6009 before any effect and only when the run can make a table, the table
//! counter's limit is U6003, and the probe count of the index grows linearly under a random
//! key and quadratically when every key collides. The hooks are global to the process, so the
//! tests take one lock.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use una_hash::SipKey;
use una_hash::hooks;

/// One test at a time drives the hooks.
static HOOKS: Mutex<()> = Mutex::new(());

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A package of edition 0.5, which has the hash tables (ADR 0139 rule 6), named `name`, whose
/// entry module `prog` is `source`.
fn package(name: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("hash_keys")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        format!("ita {name}\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry prog\n  grants\n    console\n"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/runtime/set_run/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/prog.una"), source).unwrap();
    root
}

/// `una run` of `dir` in this process, with `extra` arguments: the exit code, stdout and
/// stderr.
fn run(dir: &Path, extra: &[&str]) -> (u8, String, String) {
    let mut args = vec!["una", "run", dir.to_str().unwrap(), "--grant", "console"];
    args.extend_from_slice(extra);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = una_cli::run(
        args,
        &mut stdout,
        &mut stderr,
        false,
        &|_| Ok(None),
        una_cli::Heap::unmeasured(),
    );
    (
        status.code(),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

/// The hooks off, then the process key fixed at `key`.
fn fixed(key: SipKey) {
    hooks::reset();
    hooks::fix_key(key);
}

/// A program whose output and trace list tables built, emptied in part and combined.
const TABLES: &str = "module prog

import std.collections (HashMap, HashSet, empty_hash_map, empty_hash_set)

pub main() -> Unit
  uses @console: Console[print]
=
  var m: HashMap[Text, Int] = empty_hash_map()
  for i in 0..<200
    edit m.set(key: \"k{i}\", value: i * i)
  for i in 0..<100
    edit m.remove(key: \"k{i * 2}\")
  var s: HashSet[Int] = empty_hash_set()
  for i in 0..<300
    edit s.add(item: (i * 7919) mod 1000)
  const t = s.intersection(other: s.without(item: 0)).union(other: s)
  @console.print(\"{m.keys()}\")
  @console.print(\"{m}\")
  @console.print(\"{t.items()}\")
  @console.print(\"{s.difference(other: t).len} {m.len} {s.len}\")
";

#[test]
fn output_and_trace_do_not_depend_on_the_key() {
    // ADR 0119 rule 10: the listing order is the order of the updates, never of the hashes, so
    // two keys, and a mode in which every key collides, give the same bytes of `una run --json`,
    // its output and its trace.
    let _guard = HOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = package("two_keys", TABLES);
    fixed(SipKey { k0: 1, k1: 2 });
    let first = run(&dir, &["--json"]);
    fixed(SipKey {
        k0: 0x0123_4567_89ab_cdef,
        k1: 0xfedc_ba98_7654_3210,
    });
    let second = run(&dir, &["--json"]);
    fixed(SipKey { k0: 3, k1: 4 });
    hooks::set_constant_hash(true);
    let colliding = run(&dir, &["--json"]);
    hooks::reset();
    assert_eq!(first.0, 0, "{}{}", first.1, first.2);
    assert!(first.1.contains("\\\"k1\\\""), "{}", first.1);
    assert_eq!(first, second);
    assert_eq!(first, colliding);
}

/// A program that prints, then makes a table.
const PRINTS_THEN_HASHES: &str = "module prog

import std.collections (HashSet, empty_hash_set)

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print(\"before\")
  const s: HashSet[Int] = empty_hash_set()
  @console.print(\"{s.len}\")
";

/// A program of edition 0.5 that declares a table but never makes one.
const REACHES_NO_TABLE: &str = "module prog

import std.collections (HashSet, Set, empty_hash_set, empty_set)

--- A table that `main` never reaches.
pub unused() -> HashSet[Int] = empty_hash_set()

pub main() -> Unit
  uses @console: Console[print]
=
  const s: Set[Int] = empty_set()
  @console.print(\"{s.len}\")
";

#[test]
fn a_host_without_random_bytes_aborts_only_a_run_that_can_make_a_table() {
    // ADR 0119 rule 9: the key is read before `main`, so the run aborts with U6009 before its
    // first effect, exit code 1; a run whose code reaches no table never reads it.
    let _guard = HOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let hashes = package("missing_source", PRINTS_THEN_HASHES);
    let plain = package("no_table", REACHES_NO_TABLE);
    hooks::reset();
    hooks::simulate_missing_source();
    let aborted = run(&hashes, &[]);
    let json = run(&hashes, &["--json"]);
    let normal = run(&plain, &[]);
    hooks::reset();
    assert_eq!(aborted.0, 1, "{}{}", aborted.1, aborted.2);
    assert_eq!(aborted.1, "", "nothing is printed before the abort");
    assert!(aborted.2.contains("U6009"), "{}", aborted.2);
    assert!(
        aborted
            .2
            .contains("the host gave no random bytes for keyed hashing"),
        "{}",
        aborted.2
    );
    let document: serde_json::Value = serde_json::from_str(&json.1).unwrap();
    assert_eq!(document["status"], "aborted");
    assert_eq!(document["output"], serde_json::json!([]));
    assert_eq!(document["trace"], serde_json::json!([]));
    assert_eq!(document["diagnostics"][0]["code"], "U6009");
    assert_eq!(normal, (0, "0\n".to_owned(), String::new()));
}

#[test]
fn a_table_past_the_counter_s_limit_is_u6003() {
    // ADR 0119 rule 9: the table counter never wraps; a table made when it has reached
    // 2^64 - 1 aborts the run with U6003, as an exhausted arena counter does.
    let _guard = HOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = package("counter_limit", PRINTS_THEN_HASHES);
    hooks::reset();
    hooks::start_tables_at(u64::MAX);
    hooks::fix_key(SipKey { k0: 5, k1: 6 });
    let (code, stdout, stderr) = run(&dir, &[]);
    hooks::reset();
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert_eq!(stdout, "before\n");
    assert!(stderr.contains("U6003"), "{stderr}");
}

/// A set of `COUNT` integers, each added then looked up.
const PROBES: &str = "module prog

import std.collections (HashSet, empty_hash_set)

pub main() -> Unit
  uses @console: Console[print]
=
  var s: HashSet[Int] = empty_hash_set()
  for i in 0..<COUNT
    edit s.add(item: i)
  var found = 0
  for i in 0..<COUNT
    if s.has(item: i)
      found = found + 1
  @console.print(\"{found}\")
";

/// The probes of the index that a run of `PROBES` at `n` makes, under a fixed key, with every
/// key colliding when `constant`.
fn probes(n: usize, constant: bool) -> u64 {
    let dir = package(
        &format!("probes_{n}_{constant}"),
        &PROBES.replace("COUNT", &n.to_string()),
    );
    fixed(SipKey { k0: 7, k1: 8 });
    hooks::set_constant_hash(constant);
    let (code, stdout, stderr) = run(&dir, &[]);
    let probes = hooks::probes();
    hooks::reset();
    assert_eq!((code, stdout), (0, format!("{n}\n")), "{stderr}");
    probes
}

#[test]
fn probes_grow_linearly_and_the_canary_grows_quadratically() {
    // ADR 0119 Costs: counted, not timed. Under a key, ten times the keys make about ten times
    // the probes (expected O(1) each); when every key collides, the canary, about a hundred
    // times, which shows the count would catch a hash that collides.
    let _guard = HOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (small, large) = (probes(1_000, false), probes(10_000, false));
    assert!(
        large < 15 * small,
        "10x the keys made {large} probes against {small}"
    );
    let (small, large) = (probes(300, true), probes(3_000, true));
    assert!(
        large > 50 * small,
        "with every key colliding, 10x the keys made only {large} probes against {small}"
    );
}
