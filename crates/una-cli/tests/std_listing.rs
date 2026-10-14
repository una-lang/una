//! The standard library reference listing (#648), `docs/std/listing.json`:
//!
//! - it is the listing that the standard modules give now (`UNA_BLESS=1` rewrites it);
//! - every item has doc comments in every edition, and they cite spec sections that exist;
//! - the doc comments hold no `<` or `&` outside code spans, and no list or fence lines, which
//!   the website would render as HTML or flatten (#695);
//! - each item's editions are the editions in which `una check` resolves its name: one probe
//!   package per item and edition, linear in their number.

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
    Builtin, Item, Tier, builtins, check, std_builtin_items, std_builtins, std_items, std_listing,
};
use una_project::Edition;

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
    let Some((chapter, section)) = reference
        .strip_prefix("spec/")
        .and_then(|rest| rest.split_once(" §"))
    else {
        return false;
    };
    let dir = root().join("docs/spec");
    let Some(file) = fs::read_dir(&dir).unwrap().find_map(|entry| {
        let path = entry.unwrap().path();
        let name = path.file_name()?.to_str()?.to_owned();
        name.starts_with(&format!("{chapter}-")).then_some(path)
    }) else {
        return false;
    };
    let text = fs::read_to_string(file).unwrap();
    text.lines()
        .any(|line| line.starts_with(&format!("## {section}. ")))
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
/// span, or a line that starts a list item or a code fence, is refused.
fn doc_problems(lines: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let text = lines.join(" ");
    // Outside code spans: the even segments between backticks.
    for (index, segment) in text.split('`').enumerate() {
        if index % 2 == 0 && segment.contains(['<', '&']) {
            problems.push(format!("`<` or `&` outside a code span: {segment:?}"));
        }
    }
    for line in lines {
        let line = line.trim_start();
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
    fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"{}\"\n",
            edition.name()
        ),
    )
    .unwrap();
    fs::write(
        dir.join("src/probe.una"),
        format!("module probe\n\n{import}{body}"),
    )
    .unwrap();
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
            "V" | "T" => "Int",
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
        (_, false) => "1",
        (_, true) => "\"k\"",
    }
}

/// The module of a probe of `b`: a call with arguments of its written parameter types whose
/// result is bound to its written result type, or, when `negative`, the same call with one
/// argument of another type, or its result bound to another type when it takes none.
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
    let body = if b.edit {
        format!("  let edit x: {receiver} = r\n  edit {call}\n")
    } else {
        let bound = if negative && b.params.is_empty() {
            if result == "Bool" {
                "Int".to_owned()
            } else {
                "Bool".to_owned()
            }
        } else {
            result
        };
        format!("  let x: {receiver} = r\n  let out: {bound} = {call}\n")
    };
    format!("module probe\n\npub probe(r: {receiver}) -> Unit\n=\n{body}  unit\n")
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
                    let dir = scratch.join(format!("{probes}"));
                    probes += 1;
                    fs::create_dir_all(dir.join("src")).unwrap();
                    fs::write(
                        dir.join("una.toml"),
                        format!(
                            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"{}\"\n",
                            edition.name()
                        ),
                    )
                    .unwrap();
                    fs::write(dir.join("src/probe.una"), &source).unwrap();
                    // A real verification.lock, so that no diagnostic of the package is
                    // filtered out.
                    fs::write(dir.join("verification.lock"), &lock).unwrap();
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
