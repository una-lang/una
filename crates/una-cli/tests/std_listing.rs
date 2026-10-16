//! The standard library reference listing (#648), `docs/std/listing.json`:
//!
//! - it is the listing that the standard modules give now (`UNA_BLESS=1` rewrites it);
//! - every item has doc comments in every edition, and they cite spec sections that exist;
//! - the doc comments hold no `<` or `&` outside code spans, and no list or fence lines, which
//!   the website would render as HTML or flatten (#695);
//! - each item's editions are the editions in which `una check` resolves its name: one probe
//!   package per item and edition, linear in their number.
//! - the hand-written reference, `docs/std/reference.json` (#853), agrees with the listing: each
//!   listed item is described or pending, with the declaration's parameters, result and failure
//!   variants, items that exist, cited costs, and an example that uses it, checks and prints its
//!   expected output.

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

/// The parts of a function's declaration the reference describes (#853): its parameters' names,
/// its result type and the kind it fails with, from the declaration line `pub NAME[...](PARAMS)
/// -> RESULT` and a `fails KIND` line.
struct Signature {
    params: Vec<String>,
    result: String,
    fails: Option<String>,
}

fn signature(declaration: &str) -> Signature {
    let first = declaration.lines().next().unwrap();
    let open = first.find('(').unwrap();
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
    let fails = declaration
        .lines()
        .find_map(|line| line.trim().strip_prefix("fails "))
        .map(|kind| kind.trim().to_owned());
    Signature {
        params,
        result,
        fails,
    }
}

/// The variant names of a kind's declaration `pub kind K = A | B(x: T) | C`.
fn variant_names(declaration: &str) -> Vec<String> {
    declaration
        .split_once('=')
        .map(|(_, body)| {
            body.split('|')
                .map(|v| v.trim().split('(').next().unwrap().trim().to_owned())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_reference_agrees_with_the_listing_and_its_examples_run() {
    let text = fs::read_to_string(root().join("docs/std/reference.json")).unwrap();
    let reference: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reference["schema"], "una.std-reference/1");
    let items: Vec<Item> = std_items();
    let listed = |module: &str, name: &str| {
        items
            .iter()
            .find(|item| item.module == module && item.name == name)
    };
    let by_name = |name: &str| items.iter().find(|item| item.name == name);
    let areas: Vec<&str> = reference["areas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    let mut problems = Vec::new();
    let mut covered = std::collections::BTreeSet::new();
    for entry in reference["entries"].as_array().unwrap() {
        let module = entry["module"].as_str().unwrap();
        let name = entry["name"].as_str().unwrap();
        let at = format!("reference {module} `{name}`");
        if !covered.insert(format!("{module} {name}")) {
            problems.push(format!("{at}: twice"));
        }
        let Some(item) = listed(module, name) else {
            problems.push(format!("{at}: not a listed item"));
            continue;
        };
        if !areas.contains(&entry["area"].as_str().unwrap_or("")) {
            problems.push(format!(
                "{at}: area {} is not one of the areas",
                entry["area"]
            ));
        }
        let declaration = &item.variants.last().unwrap().declaration;
        if item.kind == "function" {
            let sig = signature(declaration);
            let params: Vec<String> = entry["params"]
                .as_array()
                .map(|ps| {
                    ps.iter()
                        .map(|p| p["name"].as_str().unwrap().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            if params != sig.params {
                problems.push(format!(
                    "{at}: parameters {params:?}, the declaration's are {:?}",
                    sig.params
                ));
            }
            if entry["returns"]["type"].as_str() != Some(sig.result.as_str()) {
                problems.push(format!(
                    "{at}: returns {}, the declaration's result is {}",
                    entry["returns"]["type"], sig.result
                ));
            }
            let fails: Vec<String> = entry["fails"]
                .as_array()
                .map(|fs| {
                    fs.iter()
                        .map(|f| f["variant"].as_str().unwrap().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            let declared = sig
                .fails
                .as_deref()
                .and_then(by_name)
                .map(|kind| variant_names(&kind.variants.last().unwrap().declaration))
                .unwrap_or_default();
            if fails != declared {
                problems.push(format!(
                    "{at}: fails with {fails:?}, the declaration's failure kind has {declared:?}"
                ));
            }
        } else if entry.get("params").is_some()
            || entry.get("returns").is_some()
            || entry.get("fails").is_some()
        {
            problems.push(format!(
                "{at}: a {} has no parameters, result or failures",
                item.kind
            ));
        }
        let named = |list: &str| -> Vec<String> {
            entry[list]
                .as_array()
                .map(|xs| {
                    xs.iter()
                        .map(|x| x.get("item").unwrap_or(x).as_str().unwrap().to_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        for other in named("use_instead").iter().chain(named("see_also").iter()) {
            if by_name(other).is_none() {
                problems.push(format!("{at}: names `{other}`, which is not a listed item"));
            }
        }
        if let Some(cost) = entry.get("cost") {
            let source = cost["source"].as_str().unwrap_or("");
            let exists = if let Some(number) = source.strip_prefix("ADR ") {
                fs::read_dir(root().join("docs/adr")).unwrap().any(|e| {
                    e.unwrap()
                        .file_name()
                        .to_str()
                        .unwrap()
                        .starts_with(&format!("{number}-"))
                })
            } else {
                resolves(source)
            };
            if !exists {
                problems.push(format!(
                    "{at}: its cost cites `{source}`, which does not exist"
                ));
            }
        }
        // The example: a package that calls the item, checks clean and prints its output.
        let Some(example) = entry["example"].as_str() else {
            problems.push(format!("{at}: no example"));
            continue;
        };
        let dir = root().join(example);
        let src = fs::read_dir(dir.join("src"))
            .map(|files| {
                files
                    .map(|f| fs::read_to_string(f.unwrap().path()).unwrap())
                    .collect::<String>()
            })
            .unwrap_or_default();
        let used = if item.kind == "function" {
            src.contains(&format!("{name}("))
        } else {
            src.contains(name)
        };
        if !used {
            problems.push(format!("{at}: its example {example} does not use it"));
        }
    }
    for pending in reference["pending"].as_array().unwrap() {
        let pending = pending.as_str().unwrap();
        if !covered.insert(pending.to_owned()) {
            problems.push(format!(
                "reference: `{pending}` is both pending and described"
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

#[test]
fn each_example_of_the_reference_checks_and_prints_its_output() {
    let text = fs::read_to_string(root().join("docs/std/reference.json")).unwrap();
    let reference: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut examples: Vec<&str> = reference["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["example"].as_str())
        .collect();
    examples.sort_unstable();
    examples.dedup();
    let mut problems = Vec::new();
    for example in examples {
        let source = root().join(example);
        let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("std-examples")
            .join(example);
        let _ = fs::remove_dir_all(&copy);
        copy_tree(&source, &copy);
        let una = |args: &[&str]| {
            std::process::Command::new(env!("CARGO_BIN_EXE_una"))
                .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                .args(args)
                .current_dir(&copy)
                .output()
                .unwrap()
        };
        let formatted = una(&["fmt", "--check", "."]);
        if !formatted.status.success() {
            problems.push(format!(
                "{example}: not in canonical form:\n{}",
                String::from_utf8_lossy(&formatted.stdout)
            ));
        }
        let checked = una(&["--color", "never", "check", "."]);
        if !checked.status.success() {
            problems.push(format!(
                "{example}: `una check .` fails:\n{}",
                String::from_utf8_lossy(&checked.stderr)
            ));
            continue;
        }
        let run = una(&["run", "."]);
        let expected = fs::read_to_string(source.join("expected/output.txt")).unwrap_or_default();
        let printed = String::from_utf8_lossy(&run.stdout);
        if !run.status.success() || printed != expected {
            problems.push(format!("{example}: `una run .` prints:\n{printed}{}\nand expected/output.txt is:\n{expected}", String::from_utf8_lossy(&run.stderr)));
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
