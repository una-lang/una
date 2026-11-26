//! The standard library reference listing (#648), `docs/std/listing.json`:
//!
//! - it is the listing that the standard modules give now (`UNA_BLESS=1` rewrites it);
//! - every item has doc comments in every edition, and they cite spec sections that exist;
//! - the doc comments hold no `<` or `&` outside code spans, and no list or fence lines, which
//!   the website would render as HTML or flatten (#695);
//! - each item's editions are the editions in which `una check` resolves its name: one probe
//!   package per item and edition, linear in their number.
//! - the hand-written reference, `docs/std/reference.json` (#853), agrees with the listing and
//!   the built-in listing: each item is described or pending, with every edition's parameters,
//!   result, preconditions and failure or kind variants, advice that holds in the editions it
//!   names, costs that are the cells of an accepted ADR's or a spec section's table with every
//!   term defined, and an example whose code that `main` reaches uses the item, which checks,
//!   prints its expected output and stops checking once the item is renamed.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::fs;
use std::path::{Path, PathBuf};

use una_cli::{
    Builtin, Item, Tier, Variant, builtins, check, std_builtin_items, std_builtins, std_items,
    std_listing,
};
use una_project::Edition;

#[path = "common/binding_words.rs"]
mod binding_words;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Fails unless `docs/std/<file>` is `generated`; `UNA_BLESS=1` rewrites it first.
fn current(file: &str, generated: &str) {
    let path = root().join("docs/std").join(file);
    // A test, not a library crate: `UNA_BLESS` asks it to rewrite the committed listing.
    #[allow(clippy::disallowed_methods)]
    let bless = std::env::var_os("UNA_BLESS").is_some();
    if bless {
        fs::write(&path, generated).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/std/{file} is not what the standard library gives; run `UNA_BLESS=1 cargo test -p una-cli --test std_listing` and review the diff"
    );
}

#[test]
fn the_listing_is_current() {
    current("listing.json", &std_listing());
}

#[test]
fn the_builtin_listing_is_current() {
    current("builtins.json", &std_builtins());
}

#[test]
fn the_listing_is_the_same_on_every_run() {
    assert_eq!(std_listing(), std_listing());
    assert_eq!(std_builtins(), std_builtins());
}

/// Whether `docs/spec/NN-*.md` has the section `§M`, a heading `## M.`.
fn resolves(reference: &str) -> bool {
    section(reference).is_some()
}

#[test]
fn every_item_has_doc_comments_that_cite_spec_sections_that_exist() {
    let items: Vec<Item> = std_items().into_iter().chain(std_builtin_items()).collect();
    assert!(!items.is_empty());
    let mut problems = Vec::new();
    for item in &items {
        for variant in &item.variants {
            if variant.docs.is_empty() {
                problems.push(format!(
                    "{} `{}` has no doc comment in editions {:?}",
                    item.module, item.name, variant.editions
                ));
            }
        }
        if item.spec.is_empty() {
            problems.push(format!(
                "{} `{}` cites no spec section",
                item.module, item.name
            ));
        }
        for reference in &item.spec {
            if !resolves(reference) {
                problems.push(format!(
                    "{} `{}` cites `{reference}`, which does not exist",
                    item.module, item.name
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// What in the doc comment `lines` the website could not render as written (#695): it joins
/// the lines into one Markdown paragraph that may hold HTML, so a `<` or `&` outside a code
/// span, or a line that starts a list item or a code fence, is refused. Code spans are told
/// apart by pairing single backticks, which holds only without a backslash (an escaped backtick
/// opens no span), with every backtick paired, and with no run of two or more (Markdown pairs
/// runs of equal length only); a bracket outside code spans (any link, image or link
/// definition), a heading, a quote and a control or invisible format character are refused too
/// (#861).
fn doc_problems(lines: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let text = lines.join(" ");
    // The segments between backticks are code spans only when no backslash escapes a backtick
    // and every backtick is paired; Markdown reads anything else differently (#861).
    if text.contains('\\') {
        problems.push(format!(
            "a backslash, which could escape a backtick: {text:?}"
        ));
    }
    if text.matches('`').count() % 2 == 1 {
        problems.push(format!("a backtick without its pair: {text:?}"));
    }
    // Markdown pairs runs of backticks of equal length only, so pairing single backticks is
    // exact only when no run is longer.
    if text.contains("``") {
        problems.push(format!("a run of backticks: {text:?}"));
    }
    if text.chars().any(|c| c.is_control())
        || text.contains([
            '\u{200b}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
            '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{feff}',
        ])
    {
        problems.push(format!("a control or invisible format character: {text:?}"));
    }
    // Outside code spans: the even segments between backticks.
    for (index, segment) in text.split('`').enumerate() {
        if index % 2 == 0 && segment.contains(['<', '&']) {
            problems.push(format!("`<` or `&` outside a code span: {segment:?}"));
        }
        // A bracket outside a code span: an inline, reference or image link, or a link
        // definition.
        if index % 2 == 0 && segment.contains(['[', ']']) {
            problems.push(format!(
                "a bracket, which could make a link or an image, outside a code span: {segment:?}"
            ));
        }
    }
    for line in lines {
        let line = line.trim_start();
        if line.starts_with(['#', '>']) {
            problems.push(format!("a line that starts a heading or a quote: {line:?}"));
        }
        let numbered = line
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if ["- ", "* ", "+ ", "```", "~~~"]
            .iter()
            .any(|start| line.starts_with(start))
            || numbered
        {
            problems.push(format!("a line that starts a list or a fence: {line:?}"));
        }
    }
    problems
}

#[test]
fn doc_comments_hold_no_raw_html_lists_or_fences() {
    let mut problems = Vec::new();
    for item in std_items().into_iter().chain(std_builtin_items()) {
        for variant in &item.variants {
            for problem in doc_problems(&variant.docs) {
                problems.push(format!("{} `{}`: {problem}", item.module, item.name));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_doc_comment_rules_refuse_each_case() {
    let lines = |text: &[&str]| text.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>();
    assert!(
        doc_problems(&lines(&[
            "Whether `a < b` holds, as `x & y` (`spec/14 §1`)."
        ]))
        .is_empty()
    );
    for bad in [
        &["Whether a < b."][..],
        &["Both a & b."],
        &["Spans `a` < `b`."],
        &["Items:", "- one"],
        &["* one"],
        &["+ one"],
        &["1. one"],
        &["```", "code", "```"],
        &["~~~"],
        &["An escaped \\`<img src=x onerror=f()>` backtick."],
        &["An unpaired `<b> backtick."],
        &["A [link](https://example.com/)."],
        &["An ![image](https://example.com/i.png)."],
        &["# A heading"],
        &["> A quote"],
        &["A right\u{202e}to-left override."],
        &["a ```<img src=x>`"],
        &["``<img>`x`"],
        &["A [reference][docs] link."],
        &["A [shortcut] link.", "[shortcut]: https://example.com/"],
    ] {
        assert!(!doc_problems(&lines(bad)).is_empty(), "accepted {bad:?}");
    }
}

/// A package of edition `edition` whose one module refers to `item` by name, in a position
/// where only name resolution matters to this test.
fn probe(dir: &Path, item: &Item, edition: Edition) {
    let name = &item.name;
    let (import, body) = match (item.module, item.kind) {
        ("std.testing", _) => (
            format!("import std.testing ({name})\n\n"),
            "pub probe() -> Unit = unit\n".to_owned(),
        ),
        // `spec/31 §1`: only an import of `std.host` brings these names into scope.
        ("std.host", "kind") => (
            format!("import std.host ({name})\n\n"),
            format!("pub probe(x: {name}) -> Unit = unit\n"),
        ),
        ("std.host", _) => (
            format!("import std.host ({name})\n\n"),
            format!("pub probe() -> Unit\n  uses @c: {name}\n=\n  unit\n"),
        ),
        (_, "kind") => (
            String::new(),
            format!("pub probe(x: {name}) -> Unit = unit\n"),
        ),
        (_, "capability") => (
            String::new(),
            format!("pub probe() -> Unit\n  uses @c: {name}\n=\n  unit\n"),
        ),
        _ => (
            String::new(),
            format!("pub probe() -> Unit\n=\n  let x = {name}()\n  unit\n"),
        ),
    };
    fs::create_dir_all(dir.join("src")).unwrap();
    write_manifest(dir, edition);
    // Edition 0.5 binds with `const` (ADR 0137).
    let source = format!("module probe\n\n{import}{body}");
    let source = if edition >= Edition::V0_5 {
        binding_words::of_0_5(&source)
    } else {
        source
    };
    fs::write(dir.join("src/probe.una"), source).unwrap();
}

#[test]
fn each_item_is_listed_in_exactly_the_editions_where_its_name_resolves() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("std_listing_probes");
    let _ = fs::remove_dir_all(&scratch);
    let mut problems = Vec::new();
    let mut probes = 0;
    for item in std_items() {
        let listed: Vec<&str> = item
            .variants
            .iter()
            .flat_map(|variant| variant.editions.iter().copied())
            .collect();
        for edition in Edition::ALL {
            let dir = scratch.join(format!("{probes}"));
            probes += 1;
            probe(&dir, &item, edition);
            let checked = check(&dir, Tier::Zero).unwrap();
            // A probe that does not parse never reaches name resolution, so it would say
            // nothing: it is a failure of the probe.
            if let Some(syntax) = checked
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code.as_str().starts_with("U01"))
            {
                problems.push(format!(
                    "the probe of {} `{}` in edition {} does not parse: {} {}",
                    item.module,
                    item.name,
                    edition.name(),
                    syntax.code.as_str(),
                    syntax.message
                ));
                continue;
            }
            let unknown = checked.diagnostics.iter().any(|diagnostic| {
                diagnostic.code.as_str() == "U1001"
                    && diagnostic.message.contains(&format!("`{}`", item.name))
            });
            let is_listed = listed.contains(&edition.name());
            if unknown == is_listed {
                problems.push(format!(
                    "{} `{}` in edition {}: listed {is_listed}, but `una check` {} its name",
                    item.module,
                    item.name,
                    edition.name(),
                    if unknown {
                        "does not resolve"
                    } else {
                        "resolves"
                    }
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_builtins_are_exactly_the_methods_the_checker_resolves() {
    // The checker's tables, by receiver, and the reference's entries, by the same grouping.
    let mut checker: Vec<(&str, String)> = una_check::builtin_methods();
    checker.sort();
    checker.dedup();
    let group = |b: &Builtin| match (b.name, b.receiver) {
        ("len", _) => "len",
        (_, "Map[K, V]") => "Map",
        (_, "Stack[T]") => "Stack",
        (_, "Queue[T]") => "Queue",
        (_, "Set[T]") => "Set",
        (_, "HashMap[K, V]") => "HashMap",
        (_, "HashSet[T]") => "HashSet",
        (_, "PriorityQueue[T, P]") => "PriorityQueue",
        (_, "BitSet") => "BitSet",
        (name, _) if name.starts_with("to_") => "conversion",
        _ => "fixed",
    };
    let mut listed: Vec<(&str, String)> = builtins()
        .iter()
        .map(|b| (group(b), b.name.to_owned()))
        .collect();
    listed.sort();
    listed.dedup();
    let missing: Vec<_> = checker.iter().filter(|m| !listed.contains(m)).collect();
    let extra: Vec<_> = listed.iter().filter(|m| !checker.contains(m)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "the checker resolves {missing:?}, which the reference lacks; the reference lists {extra:?}, which the checker does not resolve"
    );
}

/// The types a probe instantiates `F` with: one signed and one unsigned.
const FIXED_PROBES: [&str; 2] = ["I32", "U8"];

/// `ty` with its type parameters instantiated: `K` as `Text`, `V` and `T` as `Int`, `F` as
/// `fixed`.
fn instantiate(ty: &str, fixed: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        out.push_str(match word.as_str() {
            "K" => "Text",
            "V" | "T" | "P" => "Int",
            "F" => fixed,
            other => other,
        });
        word.clear();
    };
    for c in ty.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// A value of the instantiated type `ty`, or of another type when `wrong`.
fn literal(ty: &str, wrong: bool) -> &'static str {
    match (ty, wrong) {
        ("Text", false) => "\"k\"",
        ("Text", true) => "1",
        ("Bool", false) => "true",
        ("fn(Int) -> Int", false) => "fn(n) -> n",
        // A set of the receiver's type: the receiver itself.
        ("Set[Int]", false) => "r",
        // A hash set or a hash map of the receiver's type: the receiver itself.
        ("HashSet[Int]" | "HashMap[Text, Int]", false) => "r",
        ("BitSet", false) => "r",
        (_, false) => "1",
        (_, true) => "\"k\"",
    }
}

/// The module of a probe of `b`: a call with arguments of its written parameter types whose
/// result is bound to its written result type, or, when `negative`, the same call with one
/// argument of another type, or its result bound to another type when it takes none. A
/// receiver of `std.collections` is imported, which editions before 0.5 refuse with U1001.
fn builtin_probe(b: &Builtin, fixed: &str, negative: bool) -> String {
    let receiver = instantiate(b.receiver, fixed);
    let result = instantiate(&b.result, fixed);
    let args: Vec<String> = b
        .params
        .iter()
        .enumerate()
        .map(|(i, (label, ty))| {
            let ty = instantiate(ty, fixed);
            format!("{label}: {}", literal(&ty, negative && i == 0))
        })
        .collect();
    let call = if b.property {
        format!("x.{}", b.name)
    } else {
        format!("x.{}({})", b.name, args.join(", "))
    };
    let bound = if negative && b.params.is_empty() {
        if result == "Bool" {
            "Int".to_owned()
        } else {
            "Bool".to_owned()
        }
    } else {
        result.clone()
    };
    // An update whose result is not `Unit`, as `pop`, is the whole expression of a `let`.
    let body = if b.edit && result != "Unit" {
        format!("  let edit x: {receiver} = r\n  let out: {bound} = edit {call}\n")
    } else if b.edit {
        format!("  let edit x: {receiver} = r\n  edit {call}\n")
    } else {
        format!("  let x: {receiver} = r\n  let out: {bound} = {call}\n")
    };
    // A receiver of a collection type of `std.collections` names it, so the probe imports it
    // (ADR 0119 rule 0); before edition 0.5 the import itself is U1001.
    // A result that names the kind `Ranked`, as `pop_ranked`'s does, imports it too.
    let head = receiver.split('[').next().unwrap_or_default();
    let import = if una_core::Coll::from_name(head).is_some() {
        let ranked = if result.contains("Ranked[") {
            ", Ranked"
        } else {
            ""
        };
        format!("import std.collections ({head}{ranked})\n\n")
    } else {
        String::new()
    };
    format!("module probe\n\n{import}pub probe(r: {receiver}) -> Unit\n=\n{body}  unit\n")
}

#[test]
fn each_builtin_signature_checks_in_exactly_its_editions_and_a_wrong_call_does_not() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("std_builtin_probes");
    let lock =
        fs::read_to_string(root().join("tests/conformance/types/map_methods/verification.lock"))
            .unwrap();
    let _ = fs::remove_dir_all(&scratch);
    let mut problems = Vec::new();
    let mut probes = 0;
    for b in builtins() {
        let fixeds: &[&str] = if b.receiver == "F" || b.result == "F" {
            &FIXED_PROBES
        } else {
            &FIXED_PROBES[..1]
        };
        for fixed in fixeds {
            for edition in Edition::ALL {
                for negative in [false, true] {
                    let source = builtin_probe(&b, fixed, negative);
                    // Edition 0.5 binds with `const` and `var` (ADR 0137).
                    let source = if edition >= Edition::V0_5 {
                        binding_words::of_0_5(&source)
                    } else {
                        source
                    };
                    let dir = scratch.join(format!("{probes}"));
                    probes += 1;
                    fs::create_dir_all(dir.join("src")).unwrap();
                    write_manifest(&dir, edition);
                    fs::write(dir.join("src/probe.una"), &source).unwrap();
                    // A real lock, so that no diagnostic of the package is filtered out.
                    if edition < Edition::V0_4 {
                        fs::write(dir.join("verification.lock"), &lock).unwrap();
                    }
                    let checked = check(&dir, Tier::Zero).unwrap();
                    let errors: Vec<String> = checked
                        .diagnostics
                        .iter()
                        .filter(|d| d.severity == una_diag::Severity::Error)
                        .map(|d| format!("{} {}", d.code.as_str(), d.message))
                        .collect();
                    let available = b.since <= edition;
                    let what = format!(
                        "`{}` on {} ({fixed}) in edition {}, {}",
                        b.name,
                        b.receiver,
                        edition.name(),
                        if negative { "negative" } else { "positive" }
                    );
                    // Where the method does not exist, the probe must fail for that reason: a
                    // syntax error (edition 0.1 reserves `edit` and `with`) or an unresolved
                    // name (U1001, for the types it lacks). Where it exists, a probe that does
                    // not parse says nothing, and a negative probe must fail with a type error.
                    // In edition 0.1, `x.to_int()` on an `Int` resolves to the prelude's `to_int(t: Text)`
                    // by method syntax (`spec/18 §4` rule 2), so it fails as that call does.
                    let unavailable = |e: &String| {
                        e.starts_with("U01")
                            || e.starts_with("U1001 ")
                            || (b.name == "to_int"
                                && e.starts_with("U2001 argument `t` of `to_int` must be `Text`"))
                    };
                    if available && errors.iter().any(|e| e.starts_with("U01")) {
                        problems.push(format!(
                            "{what}: the probe does not parse: {errors:?}\n{source}"
                        ));
                    } else if !negative && available && !errors.is_empty() {
                        problems.push(format!(
                            "{what}: the written signature does not check: {errors:?}\n{source}"
                        ));
                    } else if !available && !errors.iter().any(unavailable) {
                        problems.push(format!(
                            "{what}: listed from edition {}, but here it does not fail as unavailable: {errors:?}\n{source}",
                            b.since.name()
                        ));
                    } else if negative && available && !errors.iter().any(|e| e.starts_with("U2")) {
                        problems.push(format!(
                            "{what}: a wrongly typed call does not fail with a type error: {errors:?}\n{source}"
                        ));
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The parts of a function's declaration the reference describes (#853): its parameters' names
/// and which of them are `edit`, the constraints of its type parameters, its result type, the
/// kind it fails with, its preconditions and the parameters whose effects (`uses of p`) and
/// failures (`fails of p`) it has, from the declaration line `pub NAME[...](PARAMS) -> RESULT`, a
/// `fails KIND` line, its `requires` lines and its `uses of p` and `fails of p` lines.
#[derive(Debug, PartialEq)]
struct Signature {
    params: Vec<String>,
    edits: Vec<bool>,
    constraints: Vec<String>,
    result: String,
    fails: Option<String>,
    requires: Vec<String>,
    uses_of: Vec<String>,
    fails_of: Vec<String>,
}

fn signature(declaration: &str) -> Signature {
    let first = declaration.lines().next().unwrap();
    let open = first.find('(').unwrap();
    // `[T: Ord, U]` between the name and `(`: the constraints after each `:`.
    let constraints = first[..open]
        .split_once('[')
        .map(|(_, inside)| inside.trim_end_matches(']'))
        .into_iter()
        .flat_map(|inside| inside.split(','))
        .filter_map(|param| param.split_once(':'))
        .flat_map(|(_, bounds)| bounds.split('+'))
        .map(|bound| bound.trim().to_owned())
        .collect();
    let mut depth = 0;
    let mut close = open;
    for (at, c) in first.char_indices().skip_while(|(at, _)| *at <= open) {
        match c {
            '[' | '(' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            ')' if depth == 0 => {
                close = at;
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    let mut params = Vec::new();
    let (mut part, mut level) = (String::new(), 0);
    for c in first[open + 1..close].chars() {
        match c {
            '[' | '(' => level += 1,
            ']' | ')' => level -= 1,
            ',' if level == 0 => {
                params.push(std::mem::take(&mut part));
                continue;
            }
            _ => {}
        }
        part.push(c);
    }
    if !part.trim().is_empty() {
        params.push(part);
    }
    let edits = params
        .iter()
        .map(|p| p.trim().starts_with("edit "))
        .collect();
    let params = params
        .iter()
        .map(|p| {
            p.split(':')
                .next()
                .unwrap()
                .trim()
                .trim_start_matches("edit ")
                .to_owned()
        })
        .collect();
    let result = first[close + 1..]
        .trim()
        .strip_prefix("->")
        .map(|r| r.trim().to_owned())
        .unwrap_or_default();
    let clause = |keyword: &str| {
        declaration
            .lines()
            .filter_map(|line| line.trim().strip_prefix(keyword))
            .map(|rest| rest.trim().to_owned())
            .collect::<Vec<String>>()
    };
    Signature {
        params,
        edits,
        constraints,
        result,
        fails: clause("fails ")
            .into_iter()
            .find(|kind| !kind.starts_with("of ")),
        requires: clause("requires "),
        uses_of: clause("uses of "),
        fails_of: clause("fails of "),
    }
}

/// One signature line of a built-in method's declaration, `[edit ]x.name[(PARAMS)] -> RESULT`
/// with its `where x: TYPE` line: whether the receiver is `edit`, its name and type, the
/// parameters' names and the result.
#[derive(Debug, PartialEq)]
struct Method {
    edit: bool,
    var: String,
    receiver: String,
    params: Vec<String>,
    result: String,
}

/// The signature lines of a built-in method's declaration, one per receiver type.
fn methods(declaration: &str) -> Vec<Method> {
    let mut found: Vec<Method> = Vec::new();
    for line in declaration.lines() {
        if let Some(receiver) = line.trim().strip_prefix("where ") {
            if let Some(last) = found.last_mut() {
                last.receiver = receiver
                    .split_once(':')
                    .map_or("", |(_, t)| t)
                    .trim()
                    .to_owned();
            }
            continue;
        }
        let edit = line.trim().starts_with("edit ");
        let line = line.trim().trim_start_matches("edit ");
        let Some((var, rest)) = line.split_once('.') else {
            continue;
        };
        let (params, result) = if rest.split("->").next().unwrap_or("").contains('(') {
            let sig = signature(rest);
            (sig.params, sig.result)
        } else {
            (
                Vec::new(),
                rest.split_once("->")
                    .map_or("", |(_, r)| r)
                    .trim()
                    .to_owned(),
            )
        };
        found.push(Method {
            edit,
            var: var.trim().to_owned(),
            receiver: String::new(),
            params,
            result,
        });
    }
    found
}

/// The receivers of a built-in method whose signature lines differ beyond the receiver, as the
/// entry's `by_receiver` describes them: an object from each receiver type of the declaration,
/// exactly those, to its `meaning`, `params` and `returns`, and its `requires` when it has any,
/// each checked against that receiver's signature line as `receiver`, `params` and `returns` are
/// for a method with one. Gives the preconditions, in the declaration's order of receivers.
fn by_receiver(
    by: &serde_json::Value,
    lines: &[Method],
    at: &str,
    problems: &mut Vec<String>,
) -> Vec<String> {
    let mut requires = Vec::new();
    let Some(object) = by.as_object() else {
        problems.push(format!("{at}: `by_receiver` is not an object"));
        return requires;
    };
    let declared: Vec<&str> = lines.iter().map(|m| m.receiver.as_str()).collect();
    for key in object.keys() {
        if !declared.contains(&key.as_str()) {
            problems.push(format!(
                "{at}: `by_receiver` names `{key}`, which is not a receiver of the declaration, {declared:?}"
            ));
        }
    }
    for line in lines {
        let at = format!("{at} by_receiver {}", line.receiver);
        let Some(receiver) = object.get(&line.receiver) else {
            problems.push(format!(
                "{at}: missing, and `by_receiver` describes every receiver of the declaration"
            ));
            continue;
        };
        shape(
            receiver,
            &at,
            &["meaning", "params", "returns"],
            &["requires"],
            problems,
        );
        // Its meaning names it as its signature line does, `m`, and says "changed in place"
        // exactly when the line is `edit`.
        let meaning = filled(receiver, "meaning", &at, problems);
        if !spans(meaning).any(|span| span == line.var) {
            problems.push(format!(
                "{at}: its meaning does not name it `{}`, as its signature line does",
                line.var
            ));
        }
        if meaning.contains("changed in place") != line.edit {
            problems.push(format!(
                "{at}: its meaning says \"changed in place\" {}, and its signature line is {}`edit`",
                if line.edit { "nowhere" } else { "" },
                if line.edit { "" } else { "not " }
            ));
        }
        let mut params = Vec::new();
        for (i, param) in list(receiver, "params", &at, problems).iter().enumerate() {
            let at = format!("{at} parameter {i}");
            shape(param, &at, &["name", "meaning"], &[], problems);
            params.push(filled(param, "name", &at, problems).to_owned());
            // No argument after the receiver of a built-in method is `edit`.
            if filled(param, "meaning", &at, problems).contains("changed in place") {
                problems.push(format!(
                    "{at}: says \"changed in place\", and it is not declared `edit`"
                ));
            }
        }
        if params != line.params {
            problems.push(format!(
                "{at}: parameters {params:?}, its signature line's are {:?}",
                line.params
            ));
        }
        let returns = &receiver["returns"];
        shape(
            returns,
            &format!("{at} returns"),
            &["type", "meaning"],
            &[],
            problems,
        );
        filled(returns, "meaning", &at, problems);
        if returns["type"].as_str() != Some(line.result.as_str()) {
            problems.push(format!(
                "{at}: returns {}, its signature line's result is {}",
                returns["type"], line.result
            ));
        }
        for (i, condition) in list(receiver, "requires", &at, problems).iter().enumerate() {
            let at = format!("{at} requires {i}");
            shape(condition, &at, &["condition", "meaning"], &[], problems);
            requires.push(filled(condition, "condition", &at, problems).to_owned());
            filled(condition, "meaning", &at, problems);
        }
    }
    requires
}

#[test]
fn by_receiver_refuses_each_case() {
    let lines = methods(
        "m.has(key: K) -> Bool\n  where m: Map[K, V]\nedit s.add(item: T) -> Unit\n  where s: Set[T]",
    );
    let good = serde_json::json!({
        "Map[K, V]": {
            "meaning": "the map `m`",
            "params": [{"name": "key", "meaning": "the key"}],
            "returns": {"type": "Bool", "meaning": "whether it is there"}
        },
        "Set[T]": {
            "meaning": "the set `s`, changed in place",
            "params": [{"name": "item", "meaning": "the item"}],
            "returns": {"type": "Unit", "meaning": "nothing"},
            "requires": [{"condition": "s.len >= 0", "meaning": "always"}]
        }
    });
    let mut problems = Vec::new();
    let requires = by_receiver(&good, &lines, "good", &mut problems);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(requires, ["s.len >= 0"]);
    // Each case breaks one rule, which must be refused.
    let cases: [(&str, &[&str], serde_json::Value); 9] = [
        ("not an object", &[], serde_json::json!([])),
        ("a missing receiver", &["Set[T]"], serde_json::Value::Null),
        ("an unknown receiver", &["List[T]"], good["Set[T]"].clone()),
        (
            "a meaning that does not name it",
            &["Map[K, V]", "meaning"],
            serde_json::json!("the map"),
        ),
        (
            "an update not changed in place",
            &["Set[T]", "meaning"],
            serde_json::json!("the set `s`"),
        ),
        (
            "other parameters",
            &["Map[K, V]", "params"],
            serde_json::json!([{"name": "item", "meaning": "the item"}]),
        ),
        (
            "another result",
            &["Map[K, V]", "returns"],
            serde_json::json!({"type": "Int", "meaning": "a count"}),
        ),
        (
            "an unknown key",
            &["Map[K, V]", "receiver"],
            serde_json::json!("Map[K, V]"),
        ),
        (
            "an argument changed in place",
            &["Set[T]", "params"],
            serde_json::json!([{"name": "item", "meaning": "the item, changed in place"}]),
        ),
    ];
    for (case, path, value) in cases {
        let mut broken = good.clone();
        match path {
            [] => broken = value,
            [receiver] if value.is_null() => {
                broken.as_object_mut().unwrap().remove(*receiver);
            }
            [receiver] => {
                broken[*receiver] = value;
            }
            [receiver, key, rest @ ..] => {
                assert!(rest.is_empty(), "{case}: a path has at most two steps");
                broken[*receiver][*key] = value;
            }
        }
        let mut problems = Vec::new();
        by_receiver(&broken, &lines, case, &mut problems);
        assert!(!problems.is_empty(), "{case}: accepted");
    }
}

/// The shape of a kind's declaration: a sum (`= A | B(x: T)`, its variants), a record (no `=`,
/// one `name: Type` line per field) or a definition over another type (`= Int where it >= 0`).
enum KindForm {
    Sum,
    Record(Vec<String>),
    Definition(String),
}

/// Whether `word` is a lowercase name, `[a-z_][a-z0-9_]*`.
fn lower_name(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The parts of `text` between the `separator`s outside brackets and parentheses.
fn split_top(text: &str, separator: char) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0, 0);
    for (at, c) in text.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            c if c == separator && depth == 0 => {
                parts.push(&text[start..at]);
                start = at + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// By structure, not by how it is written: after `=`, a sum is one or more constructors, an
/// uppercase name with or without fields in parentheses, separated by `|`, with a leading `|`,
/// several of them or fields, so that `= Only` stays a definition over a type `Only`; anything
/// else after `=` is a definition; with no `=`, the fields are the following lines of the form
/// `name: Type`.
fn kind_form(declaration: &str) -> KindForm {
    let first = declaration.lines().next().unwrap_or("");
    let Some((_, body)) = first.split_once('=') else {
        return KindForm::Record(
            declaration
                .lines()
                .skip(1)
                .filter_map(|line| line.trim().split_once(':'))
                .filter(|(field, ty)| lower_name(field) && !ty.trim().is_empty())
                .map(|(field, _)| field.to_owned())
                .collect(),
        );
    };
    let body = body.trim();
    let leading = body.starts_with('|');
    let parts = split_top(body.trim_start_matches('|'), '|');
    let constructor = |part: &str| {
        let part = part.trim();
        let (head, fields) = part.split_once('(').unwrap_or((part, ""));
        head.starts_with(|c: char| c.is_ascii_uppercase())
            && head.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && (fields.is_empty() || fields.ends_with(')'))
    };
    let fields = parts.iter().any(|part| part.contains('('));
    if parts.iter().all(|part| constructor(part)) && (leading || parts.len() > 1 || fields) {
        KindForm::Sum
    } else {
        KindForm::Definition(body.to_owned())
    }
}

/// The variants of a kind's declaration `pub kind K = A | B(x: T) | C`, each with its fields'
/// names. Variants and fields are split outside brackets, so that a field's type may have
/// several parameters, `Map[Text, Json]`.
fn variants_of(declaration: &str) -> Vec<(String, Vec<String>)> {
    declaration
        .split_once('=')
        .map(|(_, body)| {
            split_top(body, '|')
                .into_iter()
                .filter_map(|variant| {
                    let variant = variant.trim();
                    let (name, fields) = variant.split_once('(').unwrap_or((variant, ""));
                    let fields = split_top(fields.strip_suffix(')').unwrap_or(fields), ',')
                        .into_iter()
                        .filter_map(|field| field.split(':').next())
                        .map(|field| field.trim().to_owned())
                        .filter(|field| !field.is_empty())
                        .collect();
                    let name = name.trim();
                    (!name.is_empty()).then(|| (name.to_owned(), fields))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Pushes a problem unless `value` is an object with every key of `required` and no key outside
/// `required` and `optional`, so that a misspelled key cannot hide a missing one.
fn shape(
    value: &serde_json::Value,
    at: &str,
    required: &[&str],
    optional: &[&str],
    problems: &mut Vec<String>,
) {
    let Some(object) = value.as_object() else {
        problems.push(format!("{at}: not an object"));
        return;
    };
    for key in object.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            problems.push(format!("{at}: unknown key `{key}`"));
        }
    }
    for key in required {
        if !object.contains_key(*key) {
            problems.push(format!("{at}: no `{key}`"));
        }
    }
}

/// `value[key]`, which must be a text that is not blank.
fn filled<'v>(
    value: &'v serde_json::Value,
    key: &str,
    at: &str,
    problems: &mut Vec<String>,
) -> &'v str {
    match value[key].as_str() {
        Some(text) if !text.trim().is_empty() => text,
        _ => {
            problems.push(format!("{at}: `{key}` is not a text that says something"));
            ""
        }
    }
}

/// `value[key]` as a list, empty when it is absent.
fn list<'v>(
    value: &'v serde_json::Value,
    key: &str,
    at: &str,
    problems: &mut Vec<String>,
) -> &'v [serde_json::Value] {
    match value.get(key) {
        None => &[],
        Some(list) => list.as_array().map_or_else(
            || {
                problems.push(format!("{at}: `{key}` is not a list"));
                &[][..]
            },
            Vec::as_slice,
        ),
    }
}

/// The text of `docs/spec/NN-*.md` from the heading of `§M` to the next section, for the
/// reference `spec/NN §M`.
fn section(reference: &str) -> Option<String> {
    let (chapter, number) = reference.strip_prefix("spec/")?.split_once(" §")?;
    let file = fs::read_dir(root().join("docs/spec"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&format!("{chapter}-")))
        })?;
    let text = fs::read_to_string(file).unwrap();
    let heading = format!("## {number}. ");
    let start = text.lines().position(|line| line.starts_with(&heading))?;
    Some(
        text.lines()
            .skip(start)
            .enumerate()
            .take_while(|(index, line)| *index == 0 || !line.starts_with("## "))
            .map(|(_, line)| format!("{line}\n"))
            .collect(),
    )
}

/// The text where the cost `source` is stated: the Decision section of an accepted ADR,
/// `ADR NNNN`, or a spec section that exists, `spec/NN §M`.
fn cited(source: &str) -> Option<String> {
    if let Some(number) = source.strip_prefix("ADR ") {
        let path = fs::read_dir(root().join("docs/adr"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&format!("{number}-")))
            })?;
        let text = fs::read_to_string(path).unwrap();
        if !text.lines().any(|line| line.trim() == "- Status: Accepted") {
            return None;
        }
        // From `## Decision` to the next section, so that neither the context nor an amendment
        // can state a cost.
        let start = text.lines().position(|line| line == "## Decision")?;
        Some(
            text.lines()
                .skip(start + 1)
                .take_while(|line| !line.starts_with("## "))
                .map(|line| format!("{line}\n"))
                .collect(),
        )
    } else {
        section(source)
    }
}

/// The cells of a table row, split at each `|` that is not escaped as `\|`.
fn cells(line: &str) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut chars = line.trim().trim_start_matches('|').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cells.last_mut().unwrap().push('|');
                chars.next();
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().unwrap().push(c),
        }
    }
    if cells.last().is_some_and(|cell| cell.trim().is_empty()) {
        cells.pop();
    }
    cells.iter().map(|cell| cell.trim().to_owned()).collect()
}

/// The cost cells for `name` in `text`: in the first table whose header has a column that
/// starts with `Time`, that column of each row whose first cell has the call `` `name( `` or,
/// for a built-in method, `.name` as a whole word.
fn cost_cells(text: &str, name: &str, method: bool) -> Vec<String> {
    let call = format!("`{name}(");
    let names = |first: &str| {
        if method {
            let dotted = format!(".{name}");
            first.match_indices(&dotted).any(|(at, _)| {
                first[at + dotted.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
            })
        } else {
            first.contains(&call)
        }
    };
    let lines: Vec<&str> = text.lines().collect();
    for (at, line) in lines.iter().enumerate() {
        let starts = line.starts_with('|') && (at == 0 || !lines[at - 1].starts_with('|'));
        if !starts {
            continue;
        }
        let header = cells(line);
        let Some(column) = header.iter().position(|cell| cell.starts_with("Time")) else {
            continue;
        };
        return lines[at + 1..]
            .iter()
            .take_while(|row| row.starts_with('|'))
            .map(|row| cells(row))
            .filter(|row| row.first().is_some_and(|first| names(first)))
            .filter_map(|row| row.get(column).cloned())
            .collect();
    }
    Vec::new()
}

/// Every source that can state a cost, with its text: the Decision section of each accepted ADR,
/// `ADR NNNN`, and each section of the spec, `spec/NN §M`.
fn cost_sources() -> Vec<(String, String)> {
    let mut found = Vec::new();
    let names = |dir: &str| {
        let mut names: Vec<String> = fs::read_dir(root().join(dir))
            .unwrap()
            .filter_map(|entry| entry.unwrap().file_name().into_string().ok())
            .collect();
        names.sort();
        names
    };
    for name in names("docs/adr") {
        let number = name.split('-').next().unwrap_or("");
        if number.len() == 4 && number.chars().all(|c| c.is_ascii_digit()) {
            let source = format!("ADR {number}");
            if let Some(text) = cited(&source) {
                found.push((source, text));
            }
        }
    }
    for name in names("docs/spec") {
        let chapter = name.split('-').next().unwrap_or("");
        if chapter.len() != 2 || !chapter.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let text = fs::read_to_string(root().join("docs/spec").join(&name)).unwrap();
        for line in text.lines() {
            let Some((number, _)) = line.strip_prefix("## ").and_then(|h| h.split_once(". "))
            else {
                continue;
            };
            let source = format!("spec/{chapter} §{number}");
            if let Some(text) = section(&source) {
                found.push((source, text));
            }
        }
    }
    found
}

/// A cost stated in a sentence: the source, the sentence, and each item with its phrase.
type Stated = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

/// Costs that a source states in a sentence rather than in a cost table, so that the check
/// cannot find them by itself: the source, the sentence (with the backticks of its code spans
/// removed), and each item it gives a cost with the cost as the entry's `time` gives it. Each
/// part of that time between `, ` is words of the sentence, so that each item gets the phrase
/// that is about it. Review keeps this list; una#852 is to state every item's cost.
const STATED_COSTS: [Stated; 4] = [
    (
        "spec/21 §5",
        "O(log n) per lookup, insertion and removal",
        &[
            ("builtin has", "O(log n) per lookup"),
            ("builtin get", "O(log n) per lookup"),
            ("builtin set", "O(log n) per lookup, insertion and removal"),
            (
                "builtin remove",
                "O(log n) per lookup, insertion and removal",
            ),
            ("builtin with", "O(log n) per lookup, insertion and removal"),
            (
                "builtin without",
                "O(log n) per lookup, insertion and removal",
            ),
        ],
    ),
    (
        "ADR 0107",
        "O(1) index and set, amortized O(1) append, O(n) copy when the list is shared",
        &[
            ("prelude at", "O(1) index"),
            ("prelude first", "O(1) index"),
            (
                "prelude set",
                "O(1) index and set, O(n) copy when the list is shared",
            ),
            (
                "prelude append",
                "amortized O(1) append, O(n) copy when the list is shared",
            ),
        ],
    ),
    (
        "spec/23 §6",
        "parsing takes O(n + Σ k log k) time, which is O(n) when every object's keys arrive in order, as to_json writes them",
        &[("prelude parse_json", "O(n + Σ k log k)")],
    ),
    (
        "spec/23 §6",
        "writing takes O(n) time for an output of n scalar values, except that writing a Number of d digits takes O(d²)",
        &[(
            "prelude to_json",
            "O(n) time for an output of n scalar values, except that writing a Number of d digits takes O(d²)",
        )],
    ),
];

/// The receiver that a cost table's row or sentence is about where it names only the method,
/// for a method with several receivers: ADR 0070 is about texts, so its `.len` row is the cost
/// on a `Text`, and `spec/21 §5` is about maps, so its costs of `get`, `has`, `remove`, `set`,
/// `with` and `without` are those on a `Map`.
const COST_ON: [(&str, &str, &str); 7] = [
    ("ADR 0070", "len", "Text"),
    ("spec/21 §5", "get", "Map[K, V]"),
    ("spec/21 §5", "has", "Map[K, V]"),
    ("spec/21 §5", "remove", "Map[K, V]"),
    ("spec/21 §5", "set", "Map[K, V]"),
    ("spec/21 §5", "with", "Map[K, V]"),
    ("spec/21 §5", "without", "Map[K, V]"),
];

/// The keys of the reference whose values are prose.
const PROSE: [&str; 6] = ["meaning", "when", "why", "differs", "note", "requires_note"];

/// Every prose text under `value`, with where it is.
fn prose<'v>(value: &'v serde_json::Value, at: &str, found: &mut Vec<(String, &'v str)>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, inner) in object {
                match inner.as_str() {
                    Some(text) if PROSE.contains(&key.as_str()) => {
                        found.push((format!("{at} {key}"), text));
                    }
                    _ => prose(inner, &format!("{at} {key}"), found),
                }
            }
        }
        serde_json::Value::Array(list) => {
            for (index, inner) in list.iter().enumerate() {
                prose(inner, &format!("{at} {index}"), found);
            }
        }
        _ => {}
    }
}

/// The code spans of a prose text.
fn spans(text: &str) -> impl Iterator<Item = &str> {
    text.split('`').skip(1).step_by(2)
}

/// Whether a code span states a condition: it compares, or names an `old` value.
fn condition(span: &str) -> bool {
    ["==", "!=", "<=", ">=", " < ", " > ", "old("]
        .iter()
        .any(|op| span.contains(op))
}

/// `text` with each whole word `from` replaced by `to`.
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut last = 0;
    for (at, _) in text.match_indices(from) {
        let before = text[..at].chars().next_back();
        let after = text[at + from.len()..].chars().next();
        if before.is_some_and(word) || after.is_some_and(word) {
            continue;
        }
        out.push_str(&text[last..at]);
        out.push_str(to);
        last = at + from.len();
    }
    out.push_str(&text[last..]);
    out
}

/// A spec table row as an entry restates it: where its first cell holds calls,
/// `m.set(key: k, value: v)` or `a.bit_and(mask: b)`, `a.bit_or(mask: b)`, each argument's name,
/// `k`, is replaced by its label, `key`, in the whole row.
fn labeled_row(line: &str) -> String {
    let mut line = line.to_owned();
    let first = cells(&line).first().cloned().unwrap_or_default();
    for arg in first.split(['(', ',', ')']) {
        if let Some((label, name)) = arg.split_once(':')
            && lower_name(label.trim())
            && lower_name(name.trim())
        {
            line = replace_word(&line, name.trim(), label.trim());
        }
    }
    line
}

/// The contracts an entry may restate: the declaration's lines, and the spec sections that the
/// item cites, with their table rows labeled (`labeled_row`).
fn contracts(item: &Item, declaration: &str) -> String {
    let mut text = format!("{declaration}\n");
    for reference in &item.spec {
        for line in section(reference).unwrap_or_default().lines() {
            if line.starts_with('|') {
                text.push_str(&labeled_row(line));
            } else {
                text.push_str(line);
            }
            text.push('\n');
        }
    }
    text
}

/// The preconditions that a table row states, labeled (`labeled_row`): each code span
/// `requires C`, and each span `requires` followed by the span `C`. Also whether the row says
/// `requires` more often than that, in a form this does not read, which the check refuses
/// rather than read past.
fn row_requires(line: &str) -> (Vec<String>, bool) {
    let row = labeled_row(line);
    let parts: Vec<&str> = spans(&row).collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < parts.len() {
        if let Some(condition) = parts[at].strip_prefix("requires ") {
            found.push(condition.trim().to_owned());
        } else if parts[at].trim() == "requires"
            && let Some(condition) = parts.get(at + 1)
        {
            found.push(condition.trim().to_owned());
            at += 1;
        }
        at += 1;
    }
    let unread = row.matches("requires").count() > found.len();
    (found, unread)
}

/// The preconditions that the spec gives the built-in method `name`, which has no declaration
/// with `requires` lines: those of every table row, in every spec section of `sections`, whose
/// first cell calls it, `x.name(...)`. Also the sections where such a row says `requires` in a
/// form `row_requires` does not read.
fn method_requires(name: &str, sections: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let call = format!(".{name}(");
    let (mut found, mut unread) = (Vec::new(), Vec::new());
    for (source, text) in sections.iter().filter(|(s, _)| s.starts_with("spec/")) {
        for line in text.lines().filter(|line| line.starts_with('|')) {
            if cells(line)
                .first()
                .is_some_and(|first| first.contains(&call))
            {
                let (conditions, missed) = row_requires(line);
                found.extend(conditions);
                if missed {
                    unread.push(source.clone());
                }
            }
        }
    }
    (found, unread)
}

#[test]
fn the_requires_of_a_spec_row_are_read_in_both_forms_or_refused() {
    let section = |row: &str| {
        vec![(
            "spec/99 §1".to_owned(),
            format!("| Method | Contract |\n| --- | --- |\n{row}\n"),
        )]
    };
    let none: Vec<String> = Vec::new();
    assert_eq!(
        method_requires("f", &section("| `a.f(by: n)` | `requires 0 <= n`; total |")),
        (vec!["0 <= by".to_owned()], none.clone())
    );
    assert_eq!(
        method_requires(
            "f",
            &section("| `a.f(by: n)` | `requires` `0 <= n`; total |")
        ),
        (vec!["0 <= by".to_owned()], none.clone())
    );
    assert_eq!(
        method_requires(
            "f",
            &section("| `a.f(by: n)` | requires that `n` is positive |")
        ),
        (none.clone(), vec!["spec/99 §1".to_owned()])
    );
    assert_eq!(
        method_requires("g", &section("| `a.f(by: n)` | `requires 0 <= n` |")),
        (none.clone(), none)
    );
}

/// The lines of a declaration as `earlier` quotes them: trimmed, without `pub `.
fn declaration_lines(declaration: &str) -> Vec<&str> {
    declaration
        .lines()
        .map(|line| {
            let line = line.trim();
            line.strip_prefix("pub ").unwrap_or(line)
        })
        .collect()
}

/// The variants of the failure kind `kind` as it is declared in every one of `editions`, or
/// `None` when no one declaration covers them all.
fn failure_variants(kind: &Item, editions: &[&str]) -> Option<Vec<String>> {
    kind.variants
        .iter()
        .find(|v| editions.iter().all(|edition| v.editions.contains(edition)))
        .map(|v| {
            variants_of(&v.declaration)
                .into_iter()
                .map(|(n, _)| n)
                .collect()
        })
}

#[test]
fn a_failure_kind_is_read_in_the_editions_an_entry_describes() {
    let variant = |editions: Vec<&'static str>, declaration: &str| Variant {
        editions,
        declaration: declaration.to_owned(),
        docs: Vec::new(),
    };
    let kind = Item {
        name: "Problem".to_owned(),
        module: "prelude",
        kind: "kind",
        spec: Vec::new(),
        variants: vec![
            variant(vec!["0.2"], "pub kind Problem = A | B"),
            variant(vec!["0.3"], "pub kind Problem = A | B | C"),
        ],
    };
    let names = |names: &[&str]| Some(names.iter().map(|n| (*n).to_owned()).collect());
    assert_eq!(failure_variants(&kind, &["0.2"]), names(&["A", "B"]));
    assert_eq!(failure_variants(&kind, &["0.3"]), names(&["A", "B", "C"]));
    assert_eq!(failure_variants(&kind, &["0.2", "0.3"]), None);
}

/// The text inside each `O(...)` of the cost `time`.
fn big_o(time: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for (start, _) in time.match_indices("O(") {
        let mut depth = 0;
        for (at, c) in time[start + 1..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        found.push(&time[start + 2..start + 1 + at]);
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// For each byte of the Una source `src`, whether it is code: not in a comment, and not in the
/// literal part of a text literal, while an interpolation's expression is code (`spec/01`).
fn code_bytes(src: &str) -> Vec<bool> {
    #[derive(PartialEq)]
    enum In {
        Code,
        Text,
        Interpolation,
    }
    let bytes = src.as_bytes();
    let mut code = vec![true; bytes.len()];
    let mut state = In::Code;
    let mut at = 0;
    while at < bytes.len() {
        match state {
            In::Code | In::Interpolation => {
                if bytes[at..].starts_with(b"--") {
                    while at < bytes.len() && bytes[at] != b'\n' {
                        code[at] = false;
                        at += 1;
                    }
                    continue;
                }
                if bytes[at] == b'"' || (bytes[at] == b'}' && state == In::Interpolation) {
                    code[at] = false;
                    state = In::Text;
                }
            }
            In::Text => {
                code[at] = false;
                match bytes[at] {
                    b'\\' => {
                        // The escaped character, and a `\u{...}` escape to its `}`.
                        let end = if bytes[at + 1..].starts_with(b"u{") {
                            bytes[at..]
                                .iter()
                                .position(|b| *b == b'}')
                                .map_or(bytes.len() - 1, |close| at + close)
                        } else {
                            (at + 1).min(bytes.len() - 1)
                        };
                        for byte in &mut code[at..=end] {
                            *byte = false;
                        }
                        at = end;
                    }
                    b'"' => state = In::Code,
                    b'{' => state = In::Interpolation,
                    _ => {}
                }
            }
        }
        at += 1;
    }
    code
}

/// Where in `src` the identifier `name` occurs as code: whole, after a `.` only when `method`, not
/// after a `.` only when `free`, and followed by `(` when `call`.
fn occurrences(
    src: &str,
    code: &[bool],
    name: &str,
    call: bool,
    free: bool,
    method: bool,
) -> Vec<usize> {
    let identifier = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_';
    let bytes = src.as_bytes();
    src.match_indices(name)
        .map(|(at, _)| at)
        .filter(|&at| {
            let end = at + name.len();
            code[at..end].iter().all(|c| *c)
                && (if at > 0 && bytes[at - 1] == b'.' {
                    method
                } else {
                    free && (at == 0 || !identifier(&bytes[at - 1]))
                })
                && bytes.get(end).is_none_or(|b| !identifier(b))
                && (!call || bytes.get(end) == Some(&b'('))
        })
        .collect()
}

/// The byte ranges of the top-level functions of `src` that `main` reaches through calls: each
/// from its declaration line up to the next top-level line other than `=`.
fn reached_from_main(src: &str, code: &[bool]) -> Vec<std::ops::Range<usize>> {
    let mut tops: Vec<(usize, Option<String>)> = Vec::new();
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let code_text: String = line
            .char_indices()
            .filter(|(at, _)| code[start + at])
            .map(|(_, c)| c)
            .collect();
        if line.starts_with([' ', '\t']) || matches!(code_text.trim(), "" | "=") {
            continue;
        }
        let rest = code_text.trim_start_matches("pub ");
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let function = !name.is_empty() && rest[name.len()..].starts_with(['(', '[']);
        tops.push((start, function.then_some(name)));
    }
    let functions: Vec<(String, std::ops::Range<usize>)> = tops
        .iter()
        .enumerate()
        .filter_map(|(index, (start, name))| {
            let end = tops.get(index + 1).map_or(src.len(), |(next, _)| *next);
            name.clone().map(|name| (name, *start..end))
        })
        .collect();
    let mut reached = vec![false; functions.len()];
    let mut stack: Vec<usize> = functions
        .iter()
        .position(|(name, _)| name == "main")
        .into_iter()
        .collect();
    while let Some(caller) = stack.pop() {
        if reached[caller] {
            continue;
        }
        reached[caller] = true;
        let body = functions[caller].1.clone();
        for (callee, (name, _)) in functions.iter().enumerate() {
            if !reached[callee]
                && !occurrences(
                    &src[body.clone()],
                    &code[body.clone()],
                    name,
                    true,
                    true,
                    true,
                )
                .is_empty()
            {
                stack.push(callee);
            }
        }
    }
    functions
        .into_iter()
        .zip(reached)
        .filter_map(|((_, range), reached)| reached.then_some(range))
        .collect()
}

/// The Una sources of the package `dir`, each file's name and text, in name order.
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir.join("src"))
        .map(|files| files.map(|f| f.unwrap().path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            (path, text)
        })
        .collect()
}

/// How the example code `src` names `item`, as `(call, free, method)` for `occurrences`:
/// - a kind by its name;
/// - a function as a call, `name(...)`, or as a method call, `x.name(...)`;
/// - a built-in method only as `x.name(...)`, or `x.name` when it takes no arguments.
///
/// A method call `x.name(...)` can be a function's or a built-in method's, which only the
/// receiver's type decides: `shadowed` tells them apart through the checker.
fn naming(item: &Item) -> (bool, bool, bool) {
    let declaration = &item.variants.last().unwrap().declaration;
    match item.kind {
        "kind" => (false, true, false),
        "method" => (
            declaration.contains(&format!(".{}(", item.name)),
            false,
            true,
        ),
        _ => (true, true, true),
    }
}

/// The Una source `src` with a declaration of a function `name` of one `Unit` parameter
/// appended, in canonical form. It shadows the prelude function of that name (`spec/10 §11`
/// rule 5), so every call that resolved to the prelude function stops checking. A built-in
/// method of the receiver's type keeps checking: the checker tries it before any function in
/// scope (`crates/una-check/src/types/call.rs`, `method`), an order the spec is to state
/// (una#880).
fn shadowed(src: &str, name: &str) -> String {
    format!("{src}\n{name}(probe: Unit) -> Unit = probe\n")
}

/// Whether the report of `una --json check`, `report`, has an error located at one of `calls`,
/// each a file relative to the package and a line: a failure at a call of the item, not at the
/// appended declaration or anywhere else.
fn fails_at(report: &serde_json::Value, calls: &[(String, u64)]) -> bool {
    report["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|diagnostic| {
            let primary = &diagnostic["primary"];
            diagnostic["severity"] == "error"
                && calls.iter().any(|(file, line)| {
                    primary["file"] == file.as_str() && primary["start"]["line"] == *line
                })
        })
}

#[test]
fn a_shadowed_example_must_fail_at_a_call_of_the_item() {
    let report = |file: &str, line: u64| {
        serde_json::json!({ "schema": "una.diagnostics/1", "diagnostics": [{
            "severity": "error",
            "code": "U2005",
            "primary": { "file": file, "start": { "line": line, "col": 5 } }
        }], "fixable": 0 })
    };
    let calls = [("src/shop.una".to_owned(), 9)];
    assert!(fails_at(&report("src/shop.una", 9), &calls));
    assert!(!fails_at(&report("src/shop.una", 13), &calls));
    assert!(!fails_at(&report("src/other.una", 9), &calls));
    assert!(!fails_at(
        &serde_json::json!({ "diagnostics": [], "fixable": 0 }),
        &calls
    ));
}

/// The editions in which `item` exists.
fn editions(item: &Item) -> std::collections::BTreeSet<&'static str> {
    item.variants
        .iter()
        .flat_map(|variant| variant.editions.iter().copied())
        .collect()
}

/// An edition's place among the editions, by its number: `0.2` before `0.10`; a name that is
/// no edition sorts last.
fn edition_order(name: &str) -> (u32, u32) {
    Edition::ALL
        .iter()
        .any(|edition| edition.name() == name)
        .then(|| name.split_once('.'))
        .flatten()
        .and_then(|(major, minor)| Some((major.parse().ok()?, minor.parse().ok()?)))
        .unwrap_or((u32::MAX, u32::MAX))
}

#[test]
fn the_reference_agrees_with_the_listing() {
    let text = fs::read_to_string(root().join("docs/std/reference.json")).unwrap();
    let reference: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reference["schema"], "una.std-reference/1");
    let mut problems = Vec::new();
    shape(
        &reference,
        "reference",
        &[
            "schema",
            "note",
            "requires_note",
            "areas",
            "pending",
            "entries",
        ],
        &[],
        &mut problems,
    );
    filled(&reference, "note", "reference", &mut problems);
    filled(&reference, "requires_note", "reference", &mut problems);
    let builtin_items = std_builtin_items();
    let items: Vec<Item> = std_items().into_iter().chain(builtin_items).collect();
    let listed = |module: &str, name: &str| {
        items
            .iter()
            .find(|item| item.module == module && item.name == name)
    };
    let resolve = |qualified: &str| {
        qualified
            .split_once(' ')
            .and_then(|(module, name)| listed(module, name))
    };
    let areas: Vec<&str> = list(&reference, "areas", "reference", &mut problems)
        .iter()
        .filter_map(|area| area.as_str())
        .collect();
    let mut covered = std::collections::BTreeSet::new();
    // Every prose text follows the rules of doc comments (#861), so that a text the site would
    // refuse fails here first.
    let mut texts = Vec::new();
    prose(&reference, "reference", &mut texts);
    for (at, text) in texts {
        for problem in doc_problems(&[text.to_owned()]) {
            problems.push(format!("{at}: {problem}"));
        }
    }
    // Every source that can state a cost: an item whose row a cost table of one has must carry
    // a cost, and so must an item that `STATED_COSTS` gives a cost, in its source's words.
    let all_sources = cost_sources();
    for (source, sentence, of) in STATED_COSTS {
        let stated = all_sources
            .iter()
            .any(|(s, text)| s == source && text.replace('`', "").contains(sentence));
        if !stated {
            problems.push(format!("STATED_COSTS: {source} does not say `{sentence}`"));
        }
        for (item, time) in of {
            if time.split(", ").any(|part| !sentence.contains(part)) {
                problems.push(format!(
                    "STATED_COSTS: `{time}` for `{item}` is not words of `{sentence}`"
                ));
            }
        }
    }
    let constraint_names: std::collections::BTreeSet<String> = items
        .iter()
        .filter(|item| matches!(item.kind, "function" | "predicate"))
        .flat_map(|item| &item.variants)
        .flat_map(|variant| signature(&variant.declaration).constraints)
        .collect();
    for (index, entry) in list(&reference, "entries", "reference", &mut problems)
        .iter()
        .enumerate()
    {
        shape(
            entry,
            &format!("reference entry {index}"),
            &["module", "name", "area", "example"],
            &[
                "params",
                "returns",
                "requires",
                "fails",
                "effects_of",
                "variants",
                "fields",
                "definition",
                "receiver",
                "receivers",
                "by_receiver",
                "describes",
                "earlier",
                "use_instead",
                "see_also",
                "cost",
            ],
            &mut problems,
        );
        let at = format!("reference entry {index}");
        let module = filled(entry, "module", &at, &mut problems);
        let name = filled(entry, "name", &at, &mut problems);
        let at = format!("reference {module} `{name}`");
        if !covered.insert(format!("{module} {name}")) {
            problems.push(format!("{at}: twice"));
        }
        let Some(item) = listed(module, name) else {
            problems.push(format!("{at}: not a listed item"));
            continue;
        };
        if !areas.contains(&filled(entry, "area", &at, &mut problems)) {
            problems.push(format!(
                "{at}: area {} is not one of the areas",
                entry["area"]
            ));
        }
        // Its declarations, each with the editions that have it: variants that differ only in
        // their doc comments are one.
        let mut declarations: Vec<(Vec<&str>, &str)> = Vec::new();
        for variant in &item.variants {
            match declarations
                .iter_mut()
                .find(|(_, d)| *d == variant.declaration)
            {
                Some((editions, _)) => editions.extend(variant.editions.iter().copied()),
                None => declarations.push((variant.editions.clone(), &variant.declaration)),
            }
        }
        // The declaration it describes: the only one or, when the declaration differs between
        // editions, the one whose editions `describes` gives, with one `earlier` note for each
        // other declaration.
        let described = match entry.get("describes") {
            None if declarations.len() == 1 => 0,
            None => {
                problems.push(format!(
                    "{at}: its declaration differs between editions; give `describes` and `earlier`"
                ));
                declarations.len() - 1
            }
            Some(describes) => {
                let wanted: Vec<&str> = describes
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str())
                    .collect();
                let found = declarations.iter().position(|(e, _)| *e == wanted);
                if found.is_none() || declarations.len() == 1 {
                    problems.push(format!(
                        "{at}: `describes` {wanted:?} is not the editions of one of several declarations"
                    ));
                }
                found.unwrap_or(declarations.len() - 1)
            }
        };
        let (described_editions, declaration) = declarations[described].clone();
        // Each `earlier` note says how the declaration of its editions differs: it quotes, in a
        // code span, each line that the earlier declaration has and the described one lacks, and
        // each line after the first that the described one has and the earlier one lacks.
        let ours = declaration_lines(declaration);
        let mut noted: Vec<Vec<&str>> = Vec::new();
        let mut earlier_constraints = Vec::new();
        for (i, earlier) in list(entry, "earlier", &at, &mut problems)
            .iter()
            .enumerate()
        {
            let at = format!("{at} earlier {i}");
            shape(earlier, &at, &["editions", "differs"], &[], &mut problems);
            let differs = filled(earlier, "differs", &at, &mut problems);
            let editions: Vec<&str> = earlier["editions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str())
                .collect();
            if let Some((_, theirs)) = declarations
                .iter()
                .enumerate()
                .find(|(index, (e, _))| *index != described && *e == editions)
                .map(|(_, d)| d)
            {
                if matches!(item.kind, "function" | "predicate") {
                    earlier_constraints.extend(signature(theirs).constraints);
                }
                let theirs = declaration_lines(theirs);
                let quoted: Vec<&str> = spans(differs).collect();
                let lines: Vec<&str> = theirs
                    .iter()
                    .filter(|line| !ours.contains(*line))
                    .chain(ours.iter().skip(1).filter(|line| !theirs.contains(*line)))
                    .copied()
                    .collect();
                for line in &lines {
                    if !quoted.contains(line) {
                        problems.push(format!(
                            "{at}: `differs` does not quote `{line}`, where the declarations differ"
                        ));
                    }
                }
                if lines.is_empty() {
                    problems.push(format!(
                        "{at}: the declarations differ in no line `differs` can quote"
                    ));
                }
            }
            noted.push(editions);
        }
        let others: Vec<Vec<&str>> = declarations
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != described)
            .map(|(_, (editions, _))| editions.clone())
            .collect();
        if noted != others {
            problems.push(format!(
                "{at}: `earlier` notes {noted:?}, the other declarations' editions are {others:?}"
            ));
        }
        let only = |keys: &[&str], problems: &mut Vec<String>| {
            for key in [
                "params",
                "returns",
                "requires",
                "fails",
                "effects_of",
                "variants",
                "fields",
                "definition",
                "receiver",
                "receivers",
                "by_receiver",
            ] {
                if !keys.contains(&key) && entry.get(key).is_some() {
                    problems.push(format!("{at}: a {} has no `{key}`", item.kind));
                }
            }
        };
        // A parameter or receiver is "changed in place" exactly when it is declared `edit`.
        let in_place = |text: &str, edit: bool, at: &str, problems: &mut Vec<String>| {
            if text.contains("changed in place") != edit {
                problems.push(format!(
                    "{at}: says \"changed in place\" {}, and it is {}declared `edit`",
                    if edit { "nowhere" } else { "" },
                    if edit { "" } else { "not " }
                ));
            }
        };
        // Parameters and result, for functions and built-in methods.
        let check_params =
            |declared: &[String], edits: &[bool], result: &str, problems: &mut Vec<String>| {
                if entry.get("params").is_none() || entry.get("returns").is_none() {
                    problems.push(format!(
                        "{at}: a {} needs `params` and `returns`",
                        item.kind
                    ));
                }
                let mut params = Vec::new();
                for (i, param) in list(entry, "params", &at, problems).iter().enumerate() {
                    let at = format!("{at} parameter {i}");
                    shape(param, &at, &["name", "meaning"], &[], problems);
                    params.push(filled(param, "name", &at, problems).to_owned());
                    let meaning = filled(param, "meaning", &at, problems);
                    in_place(meaning, edits.get(i) == Some(&true), &at, problems);
                }
                if params != declared {
                    problems.push(format!(
                        "{at}: parameters {params:?}, the declaration's are {declared:?}"
                    ));
                }
                let returns = &entry["returns"];
                shape(
                    returns,
                    &format!("{at} returns"),
                    &["type", "meaning"],
                    &[],
                    problems,
                );
                filled(returns, "meaning", &at, problems);
                if returns["type"].as_str() != Some(result) {
                    problems.push(format!(
                        "{at}: returns {}, the declaration's result is {result}",
                        returns["type"]
                    ));
                }
            };
        // A predicate is checked as a function: it has parameters and a `Bool` result.
        if matches!(item.kind, "function" | "predicate") {
            only(
                &["params", "returns", "requires", "fails", "effects_of"],
                &mut problems,
            );
            let sig = signature(declaration);
            check_params(&sig.params, &sig.edits, &sig.result, &mut problems);
            let mut requires = Vec::new();
            for (i, condition) in list(entry, "requires", &at, &mut problems)
                .iter()
                .enumerate()
            {
                let at = format!("{at} requires {i}");
                shape(
                    condition,
                    &at,
                    &["condition", "meaning"],
                    &[],
                    &mut problems,
                );
                requires.push(filled(condition, "condition", &at, &mut problems).to_owned());
                filled(condition, "meaning", &at, &mut problems);
            }
            if requires != sig.requires {
                problems.push(format!(
                    "{at}: requires {requires:?}, the declaration's are {:?}",
                    sig.requires
                ));
            }
            let mut fails = Vec::new();
            for (i, failure) in list(entry, "fails", &at, &mut problems).iter().enumerate() {
                let at = format!("{at} fails {i}");
                shape(failure, &at, &["variant", "when"], &[], &mut problems);
                fails.push(filled(failure, "variant", &at, &mut problems).to_owned());
                filled(failure, "when", &at, &mut problems);
            }
            // The failure kind as it is declared in the editions the entry describes.
            let declared: Vec<String> = match sig.fails.as_deref() {
                None => Vec::new(),
                Some(kind) => match listed("prelude", kind)
                    .and_then(|kind| failure_variants(kind, &described_editions))
                {
                    Some(variants) => variants,
                    None => {
                        problems.push(format!(
                            "{at}: its failure kind `{kind}` has no declaration for all of {described_editions:?}"
                        ));
                        Vec::new()
                    }
                },
            };
            if declared != fails {
                problems.push(format!(
                    "{at}: fails with {fails:?}, the declaration's failure kind has {declared:?}"
                ));
            }
            // `uses of p` and `fails of p`: the function has the effects and the failures of `p`.
            // `effects_of` says both, so the declaration has both clauses, of the same parameter.
            match (
                entry.get("effects_of"),
                sig.uses_of.as_slice(),
                sig.fails_of.as_slice(),
            ) {
                (None, [], []) => {}
                (Some(effects), [param], [failures]) if param == failures => {
                    let at = format!("{at} effects_of");
                    shape(effects, &at, &["param", "meaning"], &[], &mut problems);
                    filled(effects, "meaning", &at, &mut problems);
                    if effects["param"].as_str() != Some(param.as_str()) {
                        problems.push(format!(
                            "{at}: names {}, the declaration's is `{param}`",
                            effects["param"]
                        ));
                    }
                }
                (_, uses, fails) => problems.push(format!(
                    "{at}: `effects_of` {}, the declaration has `uses of` {uses:?} and `fails of` {fails:?}",
                    entry
                        .get("effects_of")
                        .map_or("absent".to_owned(), |e| e.to_string())
                )),
            }
        } else if item.kind == "method" {
            let lines = methods(declaration);
            let Some(first) = lines.first() else {
                problems.push(format!("{at}: no signature line in its declaration"));
                continue;
            };
            let differ = lines
                .iter()
                .any(|m| m.params != first.params || m.result != first.result);
            // Its preconditions, which must be those of the spec rows that call it.
            let mut requires = Vec::new();
            // Signature lines that differ beyond the receiver, as `has` on a map and on a set,
            // describe each receiver on its own, in `by_receiver`; the others describe their
            // parameters and result once.
            if let Some(by) = entry.get("by_receiver") {
                only(&["by_receiver"], &mut problems);
                if !differ {
                    problems.push(format!(
                        "{at}: `by_receiver` is for signature lines that differ beyond the receiver, and its lines do not; give `params` and `returns` once"
                    ));
                }
                requires = by_receiver(by, &lines, &at, &mut problems);
            } else {
                only(
                    &["params", "returns", "requires", "receiver", "receivers"],
                    &mut problems,
                );
                if differ {
                    problems.push(format!(
                        "{at}: its signature lines differ beyond the receiver; describe each receiver in `by_receiver`"
                    ));
                }
                check_params(
                    &first.params,
                    &vec![false; first.params.len()],
                    &first.result,
                    &mut problems,
                );
                let declared: Vec<&str> = lines.iter().map(|m| m.receiver.as_str()).collect();
                let receivers: Vec<&serde_json::Value> = match (
                    entry.get("receiver"),
                    entry.get("receivers"),
                    declared.len(),
                ) {
                    (Some(one), None, 1) => vec![one],
                    (None, Some(many), n) if n > 1 => {
                        many.as_array().into_iter().flatten().collect()
                    }
                    _ => {
                        problems.push(format!(
                            "{at}: give `receiver` for one receiver type, `receivers` for several; the declaration has {declared:?}"
                        ));
                        Vec::new()
                    }
                };
                // Each receiver's meaning names it as its signature line does, `m`, and says
                // "changed in place" exactly when the line is `edit`.
                let mut types = Vec::new();
                for (i, receiver) in receivers.iter().enumerate() {
                    let at = format!("{at} receiver {i}");
                    shape(receiver, &at, &["type", "meaning"], &[], &mut problems);
                    let meaning = filled(receiver, "meaning", &at, &mut problems);
                    if let Some(line) = lines.get(i) {
                        if !spans(meaning).any(|span| span == line.var) {
                            problems.push(format!(
                                "{at}: its meaning does not name it `{}`, as its signature line does",
                                line.var
                            ));
                        }
                        in_place(meaning, line.edit, &at, &mut problems);
                    }
                    types.push(filled(receiver, "type", &at, &mut problems));
                }
                if !receivers.is_empty() && types != declared {
                    problems.push(format!(
                        "{at}: receivers {types:?}, the declaration's are {declared:?}"
                    ));
                }
                for (i, condition) in list(entry, "requires", &at, &mut problems)
                    .iter()
                    .enumerate()
                {
                    let at = format!("{at} requires {i}");
                    shape(
                        condition,
                        &at,
                        &["condition", "meaning"],
                        &[],
                        &mut problems,
                    );
                    requires.push(filled(condition, "condition", &at, &mut problems).to_owned());
                    filled(condition, "meaning", &at, &mut problems);
                }
            }
            // Read from every spec section, failing closed: a row that says `requires` in a form
            // not read, or a declaration with `requires` lines that no row states, is refused.
            let (stated, unread) = method_requires(name, &all_sources);
            for source in unread {
                problems.push(format!(
                    "{at}: a row of {source} that calls it says `requires` in a form the check does not read"
                ));
            }
            let declared: Vec<&str> = declaration
                .lines()
                .filter_map(|line| line.trim().strip_prefix("requires "))
                .collect();
            if !declared.is_empty() && stated.is_empty() {
                problems.push(format!(
                    "{at}: its declaration has `requires` {declared:?}, and no spec row states them"
                ));
            }
            if requires != stated {
                problems.push(format!(
                    "{at}: requires {requires:?}, the spec rows that call it give {stated:?}"
                ));
            }
        } else if item.kind == "kind" {
            match kind_form(declaration) {
                KindForm::Sum => {
                    only(&["variants"], &mut problems);
                    let mut variants = Vec::new();
                    for (i, variant) in list(entry, "variants", &at, &mut problems)
                        .iter()
                        .enumerate()
                    {
                        let at = format!("{at} variant {i}");
                        shape(
                            variant,
                            &at,
                            &["name", "fields", "meaning"],
                            &[],
                            &mut problems,
                        );
                        let mut fields = Vec::new();
                        for (j, field) in list(variant, "fields", &at, &mut problems)
                            .iter()
                            .enumerate()
                        {
                            let at = format!("{at} field {j}");
                            shape(field, &at, &["name", "meaning"], &[], &mut problems);
                            fields.push(filled(field, "name", &at, &mut problems).to_owned());
                            filled(field, "meaning", &at, &mut problems);
                        }
                        variants.push((
                            filled(variant, "name", &at, &mut problems).to_owned(),
                            fields,
                        ));
                        filled(variant, "meaning", &at, &mut problems);
                    }
                    let declared = variants_of(declaration);
                    if variants != declared {
                        problems.push(format!(
                            "{at}: variants {variants:?}, the declaration's are {declared:?}"
                        ));
                    }
                }
                KindForm::Record(declared) => {
                    only(&["fields"], &mut problems);
                    let mut fields = Vec::new();
                    for (j, field) in list(entry, "fields", &at, &mut problems).iter().enumerate() {
                        let at = format!("{at} field {j}");
                        shape(field, &at, &["name", "meaning"], &[], &mut problems);
                        fields.push(filled(field, "name", &at, &mut problems).to_owned());
                        filled(field, "meaning", &at, &mut problems);
                    }
                    if fields != declared || fields.is_empty() {
                        problems.push(format!(
                            "{at}: fields {fields:?}, the declaration's are {declared:?}"
                        ));
                    }
                }
                KindForm::Definition(declared) => {
                    only(&["definition"], &mut problems);
                    let definition = &entry["definition"];
                    let at = format!("{at} definition");
                    shape(definition, &at, &["type", "meaning"], &[], &mut problems);
                    filled(definition, "meaning", &at, &mut problems);
                    if definition["type"].as_str() != Some(declared.as_str()) {
                        problems.push(format!(
                            "{at}: {}, the declaration's is `{declared}`",
                            definition["type"]
                        ));
                    }
                }
            }
        } else {
            only(&[], &mut problems);
        }
        // A condition that its prose restates in a code span is, verbatim, part of a contract
        // of the declaration or of a spec section the item cites; a constraint it names, `Ord`,
        // is one of the declaration's (or, in an `earlier` note, of the earlier declaration's).
        let contract = contracts(item, declaration);
        let constraints: Vec<String> = if matches!(item.kind, "function" | "predicate") {
            signature(declaration).constraints
        } else {
            Vec::new()
        };
        let mut texts = Vec::new();
        prose(entry, &at, &mut texts);
        for (at, text) in texts {
            let earlier = at.contains(" earlier ");
            for span in spans(text) {
                if !earlier && condition(span) && !contract.contains(span) {
                    problems.push(format!(
                        "{at}: `{span}` is a condition that no contract of the declaration or of {:?} states",
                        item.spec
                    ));
                }
                let allowed: Vec<&String> = if earlier {
                    constraints.iter().chain(&earlier_constraints).collect()
                } else {
                    constraints.iter().collect()
                };
                if constraint_names.contains(span) && !allowed.iter().any(|c| c.as_str() == span) {
                    problems.push(format!(
                        "{at}: names the constraint `{span}`, which the declaration does not have"
                    ));
                }
            }
        }
        // What it names: listed items, `module name`, in every edition of this one, or from
        // the edition its advice gives.
        let own = editions(item);
        // Advice and related items: `use_instead` entries `{item, why, from_edition?}`, and
        // `see_also` entries `"module name"` or `{item, from_edition}`.
        let mut named = Vec::new();
        for (i, advice) in list(entry, "use_instead", &at, &mut problems)
            .iter()
            .enumerate()
        {
            let at = format!("{at} use_instead {i}");
            shape(
                advice,
                &at,
                &["item", "why"],
                &["from_edition"],
                &mut problems,
            );
            filled(advice, "why", &at, &mut problems);
            named.push((at, advice));
        }
        for (i, other) in list(entry, "see_also", &at, &mut problems)
            .iter()
            .enumerate()
        {
            let at = format!("{at} see_also {i}");
            if !other.is_string() {
                shape(other, &at, &["item", "from_edition"], &[], &mut problems);
            }
            named.push((at, other));
        }
        for (at, named) in named {
            let other = named
                .as_str()
                .unwrap_or_else(|| filled(named, "item", &at, &mut problems));
            let from = named.get("from_edition").map(|e| e.as_str().unwrap_or("?"));
            let Some(target) = resolve(other) else {
                problems.push(format!(
                    "{at}: names `{other}`, which is not a listed `module name`"
                ));
                continue;
            };
            // Editions compare as numbers, `0.10` after `0.2`.
            let theirs = editions(target);
            let shared = own
                .iter()
                .copied()
                .filter(|e| theirs.contains(e))
                .min_by_key(|e| edition_order(e));
            let after = |from: &str| {
                own.iter()
                    .copied()
                    .filter(|e| edition_order(e) >= edition_order(from))
                    .collect::<Vec<&str>>()
            };
            match from {
                None if own.is_subset(&theirs) => {}
                None => problems.push(format!(
                    "{at}: `{other}` is not in every edition of {own:?}; give `from_edition` {shared:?}"
                )),
                Some(_) if own.is_subset(&theirs) => problems.push(format!(
                    "{at}: `{other}` is in every edition, so `from_edition` is not needed"
                )),
                Some(from) if Some(from) != shared || after(from).iter().any(|e| !theirs.contains(e)) => {
                    problems.push(format!(
                        "{at}: `from_edition` {from:?} is not the first edition, {shared:?}, from which `{other}` is in every edition of the item"
                    ));
                }
                Some(_) => {}
            }
        }
        // The cost: the cell of the item's row in the cost table of an accepted ADR or of a spec
        // section, verbatim, or the cost that `STATED_COSTS` gives it from a sentence of one,
        // with every term it uses defined. Every source is searched, so an item whose cost one
        // states carries it.
        let method = item.kind == "method";
        let qualified = format!("{module} {name}");
        let stated: Vec<(&str, &str)> = STATED_COSTS
            .iter()
            .flat_map(|(source, _, of)| {
                of.iter()
                    .filter(|(item, _)| *item == qualified)
                    .map(move |(_, time)| (*source, *time))
            })
            .collect();
        if entry.get("cost").is_none() {
            for (source, text) in &all_sources {
                if !cost_cells(text, name, method).is_empty() {
                    problems.push(format!(
                        "{at}: {source} states its cost in a table, and it has none"
                    ));
                }
            }
            for (source, _) in &stated {
                problems.push(format!("{at}: {source} states its cost, and it has none"));
            }
        }
        if let Some(cost) = entry.get("cost") {
            let at = format!("{at} cost");
            shape(
                cost,
                &at,
                &["time", "source", "terms"],
                &["on"],
                &mut problems,
            );
            let time = filled(cost, "time", &at, &mut problems);
            let source = filled(cost, "source", &at, &mut problems);
            // With several receivers, `on` names the one whose cost is stated, as `COST_ON` ties
            // it to the source.
            let receivers: Vec<&str> = match entry.get("by_receiver").and_then(|by| by.as_object())
            {
                Some(by) => by.keys().map(String::as_str).collect(),
                None => entry["receivers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["type"].as_str())
                    .collect(),
            };
            let tied = COST_ON
                .iter()
                .find(|(s, n, _)| *s == source && *n == name)
                .map(|(_, _, on)| *on);
            match (cost.get("on").and_then(|on| on.as_str()), receivers.is_empty()) {
                (None, true) => {}
                (Some(on), false) if receivers.contains(&on) && tied == Some(on) => {}
                (on, _) => problems.push(format!(
                    "{at}: `on` {on:?}, which must name the receiver {tied:?} that COST_ON ties to {source}, one of {receivers:?}, and only then"
                )),
            }
            match cited(source) {
                None => problems.push(format!(
                    "{at}: cites `{source}`, which is neither an accepted ADR nor a spec section"
                )),
                Some(cited) => {
                    let mut cells = cost_cells(&cited, name, method);
                    cells.dedup();
                    if cells.is_empty() {
                        if !stated.contains(&(source, time)) {
                            problems.push(format!(
                                "{at}: `{time}` is neither in a row of {source}'s cost table for `{name}` nor a cost STATED_COSTS gives it from {source}"
                            ));
                        }
                    } else if cells != [time] {
                        problems.push(format!(
                            "{at}: `{time}` is not the cost {source} states for `{name}`: {cells:?}"
                        ));
                    }
                }
            }
            let mut defined: Vec<&str> = Vec::new();
            for (i, term) in list(cost, "terms", &at, &mut problems).iter().enumerate() {
                let at = format!("{at} term {i}");
                shape(term, &at, &["symbol", "meaning"], &[], &mut problems);
                let symbol = filled(term, "symbol", &at, &mut problems);
                filled(term, "meaning", &at, &mut problems);
                if !time.contains(symbol) {
                    problems.push(format!("{at}: `{symbol}` is not in `{time}`"));
                }
                defined.extend(symbol.split(|c: char| !c.is_ascii_alphabetic()));
            }
            // `log` is the logarithm, not a term.
            for inside in big_o(time) {
                for word in inside.split(|c: char| !c.is_ascii_alphabetic()) {
                    if !word.is_empty() && word != "log" && !defined.contains(&word) {
                        problems.push(format!("{at}: `{word}` in `{time}` is not defined"));
                    }
                }
            }
        }
        // The example: code that `main` reaches calls the item, and a kind's example names the
        // kind and each of its variants. `each_example_of_the_reference_...` also requires that
        // it stops checking once the item is renamed.
        let example = filled(entry, "example", &at, &mut problems);
        let mut names = vec![name.to_owned()];
        if item.kind == "kind" && matches!(kind_form(declaration), KindForm::Sum) {
            names.extend(variants_of(declaration).into_iter().map(|(n, _)| n));
        }
        let files = sources(&root().join(example));
        if files.is_empty() {
            problems.push(format!("{at}: its example {example} has no source"));
        }
        for wanted in &names {
            let used = files.iter().any(|(_, src)| {
                let code = code_bytes(src);
                let (call, free, method) = if wanted == name {
                    naming(item)
                } else {
                    (false, true, false)
                };
                reached_from_main(src, &code).iter().any(|range| {
                    !occurrences(
                        &src[range.clone()],
                        &code[range.clone()],
                        wanted,
                        call,
                        free,
                        method,
                    )
                    .is_empty()
                })
            });
            if !used {
                problems.push(format!(
                    "{at}: no code that `main` reaches in {example} uses `{wanted}`"
                ));
            }
        }
    }
    for pending in list(&reference, "pending", "reference", &mut problems) {
        let pending = pending.as_str().unwrap_or("");
        if !covered.insert(pending.to_owned()) {
            problems.push(format!(
                "reference: `{pending}` is both pending and described, or pending twice"
            ));
        }
    }
    let all: std::collections::BTreeSet<String> = items
        .iter()
        .map(|item| format!("{} {}", item.module, item.name))
        .collect();
    for missing in all.difference(&covered) {
        problems.push(format!(
            "reference: `{missing}` is neither described nor pending"
        ));
    }
    for extra in covered.difference(&all) {
        problems.push(format!("reference: `{extra}` is not a listed item"));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The manifest of every example: a package `shop` of edition 0.2 whose `main` may print, and
/// nothing more.
const EXAMPLE_MANIFEST: &str = "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"shop\"\ngrants = [\"console\"]\n";

/// The one key an example may add to `EXAMPLE_MANIFEST`, and only when its code uses `checked`
/// exactly once: an opaque predicate is called only there (`spec/03 §4`, ADR 0042), and the
/// escape budget is 0 by default.
const CHECKED_BUDGET: &str = "\n[escape_budget]\nchecked_per_module = 1\n";

/// `una ARGS` in `dir`.
fn una(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// `src` with every place where it names `item` (see `naming`) renamed to a name that does not
/// exist, and how many there were.
fn renamed(src: &str, item: &Item) -> (String, usize) {
    let name = item.name.as_str();
    let code = code_bytes(src);
    let (call, free, method) = naming(item);
    let places = occurrences(src, &code, name, call, free, method);
    let mut out = String::new();
    let mut last = 0;
    for at in &places {
        out.push_str(&src[last..*at]);
        out.push_str(name);
        out.push_str(if item.kind == "kind" {
            "Absent"
        } else {
            "_absent"
        });
        last = at + name.len();
    }
    out.push_str(&src[last..]);
    (out, places.len())
}

#[test]
fn each_example_of_the_reference_checks_prints_its_output_and_needs_its_item() {
    let text = fs::read_to_string(root().join("docs/std/reference.json")).unwrap();
    let reference: serde_json::Value = serde_json::from_str(&text).unwrap();
    let items: Vec<Item> = std_items().into_iter().chain(std_builtin_items()).collect();
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let mut problems = Vec::new();
    let mut run = std::collections::BTreeSet::new();
    for entry in reference["entries"].as_array().unwrap() {
        let (module, name) = (
            entry["module"].as_str().unwrap(),
            entry["name"].as_str().unwrap(),
        );
        let Some(example) = entry["example"].as_str() else {
            continue;
        };
        let source = root().join(example);
        if run.insert(example) {
            let copy = scratch.join("std-examples").join(example);
            let _ = fs::remove_dir_all(&copy);
            copy_tree(&source, &copy);
            // The manifest grants no more than printing, and allows one `checked` only to an
            // example that uses exactly one.
            let manifest = fs::read_to_string(source.join("una.toml")).unwrap_or_default();
            let checks: usize = sources(&source)
                .iter()
                .map(|(_, src)| {
                    occurrences(src, &code_bytes(src), "checked", true, true, false).len()
                })
                .sum();
            let allowed = if checks == 1 {
                format!("{EXAMPLE_MANIFEST}{CHECKED_BUDGET}")
            } else {
                EXAMPLE_MANIFEST.to_owned()
            };
            if manifest != allowed {
                problems.push(format!(
                    "{example}: una.toml is not the examples' manifest{}, with {checks} uses of `checked`: {manifest:?}",
                    if checks == 1 { " with the budget of one `checked`" } else { "" }
                ));
            }
            // The verification lock is the repository's own: the solver pins and nothing else.
            let lock = fs::read_to_string(source.join("verification.lock")).unwrap_or_default();
            if lock != fs::read_to_string(root().join("verification.lock")).unwrap() {
                problems.push(format!(
                    "{example}: verification.lock is not the repository's: {lock:?}"
                ));
            }
            let formatted = una(&copy, &["fmt", "--check", "."]);
            if !formatted.status.success() {
                problems.push(format!(
                    "{example}: not in canonical form:\n{}",
                    String::from_utf8_lossy(&formatted.stdout)
                ));
            }
            let checked = una(&copy, &["--color", "never", "check", "."]);
            if !checked.status.success() {
                problems.push(format!(
                    "{example}: `una check .` fails:\n{}",
                    String::from_utf8_lossy(&checked.stderr)
                ));
                continue;
            }
            let ran = una(&copy, &["run", "."]);
            let printed = String::from_utf8_lossy(&ran.stdout);
            match fs::read_to_string(source.join("expected/output.txt")) {
                Err(_) => problems.push(format!("{example}: no expected/output.txt")),
                Ok(expected) if !ran.status.success() || printed != expected => {
                    problems.push(format!(
                        "{example}: `una run .` prints:\n{printed}{}\nand expected/output.txt is:\n{expected}",
                        String::from_utf8_lossy(&ran.stderr)
                    ));
                }
                Ok(_) => {}
            }
        }
        // With the item renamed, the example must stop checking: it needs the item, not a
        // comment, a text or another item of the same name.
        let Some(item) = items
            .iter()
            .find(|item| item.module == module && item.name == name)
        else {
            problems.push(format!("{example}: `{module} {name}` is not a listed item"));
            continue;
        };
        let copy = scratch.join("std-examples-renamed").join(module).join(name);
        let _ = fs::remove_dir_all(&copy);
        copy_tree(&source, &copy);
        let mut places = 0;
        for (path, src) in sources(&copy) {
            let (src, count) = renamed(&src, item);
            fs::write(path, src).unwrap();
            places += count;
        }
        let checked = una(&copy, &["--color", "never", "check", "."]);
        if places == 0 || checked.status.success() {
            problems.push(format!(
                "{example}: still checks with `{name}` renamed in {places} places, so it does not need it"
            ));
        }
        // Where another item has the same name, a method call `x.name(...)` could be either's,
        // whatever types the code writes or infers: with a function `name` declared in the
        // example, which shadows the prelude function, an example of the prelude function stops
        // checking with an error at one of its calls of the item, and an example of a built-in
        // method keeps checking.
        let shared = items
            .iter()
            .any(|other| other.name == item.name && other.module != item.module);
        if shared && item.kind != "kind" {
            let copy = scratch
                .join("std-examples-shadowed")
                .join(module)
                .join(name);
            let _ = fs::remove_dir_all(&copy);
            copy_tree(&source, &copy);
            let (call, free, method) = naming(item);
            let mut calls = Vec::new();
            for (path, src) in sources(&copy) {
                let file = path
                    .strip_prefix(&copy)
                    .unwrap()
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<String>>()
                    .join("/");
                let code = code_bytes(&src);
                for at in occurrences(&src, &code, name, call, free, method) {
                    calls.push((file.clone(), src[..at].matches('\n').count() as u64 + 1));
                }
                fs::write(path, shadowed(&src, name)).unwrap();
            }
            let checked = una(&copy, &["--json", "--color", "never", "check", "."]);
            let report: serde_json::Value =
                serde_json::from_slice(&checked.stdout).unwrap_or_default();
            let builtin = item.kind == "method";
            let as_expected = if builtin {
                checked.status.success()
            } else {
                !checked.status.success() && fails_at(&report, &calls)
            };
            if !as_expected {
                problems.push(format!(
                    "{example}: with a function `{name}` declared beside it, it {}, so {}:\n{}",
                    if builtin {
                        "stops checking"
                    } else {
                        "has no error at a call of it"
                    },
                    if builtin {
                        "a call in it is the prelude function's, not the built-in method's"
                    } else {
                        "no call in it is the prelude function's"
                    },
                    String::from_utf8_lossy(&checked.stdout)
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n\n"));
}

/// A copy of the directory `from` at `to`, without `target/` or `.una/`.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            if entry.file_name() != "target" && entry.file_name() != ".una" {
                copy_tree(&entry.path(), &target);
            }
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The manifest of a probe package in `edition`: `una.toml` to edition 0.3, and `ita.una` with
/// its `ita.lock` from edition 0.4 (ADR 0122).
fn write_manifest(dir: &std::path::Path, edition: Edition) {
    if edition >= Edition::V0_4 {
        fs::write(
            dir.join("ita.una"),
            format!("ita probe\n  version 0.1.0\n  edition {}\n", edition.name()),
        )
        .unwrap();
        fs::write(
            dir.join("ita.lock"),
            "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n",
        )
        .unwrap();
    } else {
        fs::write(
            dir.join("una.toml"),
            format!(
                "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"{}\"\n",
                edition.name()
            ),
        )
        .unwrap();
    }
}
