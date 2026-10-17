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
/// its result type, the kind it fails with and its preconditions, from the declaration line
/// `pub NAME[...](PARAMS) -> RESULT`, a `fails KIND` line and its `requires` lines.
#[derive(Debug, PartialEq)]
struct Signature {
    params: Vec<String>,
    result: String,
    fails: Option<String>,
    requires: Vec<String>,
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
    let clause = |keyword: &str| {
        declaration
            .lines()
            .filter_map(|line| line.trim().strip_prefix(keyword))
            .map(|rest| rest.trim().to_owned())
            .collect::<Vec<String>>()
    };
    Signature {
        params,
        result,
        fails: clause("fails ").into_iter().next(),
        requires: clause("requires "),
    }
}

/// The variants of a kind's declaration `pub kind K = A | B(x: T) | C`, each with its fields'
/// names.
fn variants_of(declaration: &str) -> Vec<(String, Vec<String>)> {
    declaration
        .split_once('=')
        .map(|(_, body)| {
            body.split('|')
                .filter_map(|variant| {
                    let variant = variant.trim();
                    let (name, fields) = variant.split_once('(').unwrap_or((variant, ""));
                    let fields = fields
                        .trim_end_matches(')')
                        .split(',')
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
/// starts with `Time`, that column of each row whose first cell has the call `` `name( ``.
fn cost_cells(text: &str, name: &str) -> Vec<String> {
    let call = format!("`{name}(");
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
            .filter(|row| row.first().is_some_and(|first| first.contains(&call)))
            .filter_map(|row| row.get(column).cloned())
            .collect();
    }
    Vec::new()
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

/// Where in `src` the identifier `name` occurs as code: whole, after a `.` only when `method`,
/// and followed by `(` when `call`.
fn occurrences(src: &str, code: &[bool], name: &str, call: bool, method: bool) -> Vec<usize> {
    let identifier = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_';
    let bytes = src.as_bytes();
    src.match_indices(name)
        .map(|(at, _)| at)
        .filter(|&at| {
            let end = at + name.len();
            code[at..end].iter().all(|c| *c)
                && (at == 0 || !identifier(&bytes[at - 1]) && (method || bytes[at - 1] != b'.'))
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
                && !occurrences(&src[body.clone()], &code[body.clone()], name, true, true)
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

/// How an example names the item `name`: a call, `name(...)` or, unless a built-in method has
/// the same name, `x.name(...)`; for a kind, the name itself.
fn naming(name: &str, kind: bool, builtin_names: &[String]) -> (bool, bool) {
    (!kind, !kind && !builtin_names.iter().any(|b| b == name))
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
    let builtin_names: Vec<String> = builtin_items.iter().map(|item| item.name.clone()).collect();
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
    // Every source a cost cites: an item it states a cost for must carry that cost.
    let cost_sources: std::collections::BTreeSet<&str> = reference["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["cost"]["source"].as_str())
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
                "variants",
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
        let declarations: Vec<&str> = item
            .variants
            .iter()
            .map(|variant| variant.declaration.as_str())
            .collect();
        let only = |keys: &[&str], problems: &mut Vec<String>| {
            for key in ["params", "returns", "requires", "fails", "variants"] {
                if !keys.contains(&key) && entry.get(key).is_some() {
                    problems.push(format!("{at}: a {} has no `{key}`", item.kind));
                }
            }
        };
        if item.kind == "function" {
            only(&["params", "returns", "requires", "fails"], &mut problems);
            // Every edition's declaration, so that the entry cannot describe only the newest.
            let signatures: Vec<Signature> = declarations.iter().map(|d| signature(d)).collect();
            if signatures.windows(2).any(|pair| pair[0] != pair[1]) {
                problems.push(format!(
                    "{at}: its declaration differs between editions, and an entry describes one"
                ));
            }
            let sig = &signatures[0];
            if entry.get("params").is_none() || entry.get("returns").is_none() {
                problems.push(format!("{at}: a function needs `params` and `returns`"));
            }
            let mut params = Vec::new();
            for (i, param) in list(entry, "params", &at, &mut problems).iter().enumerate() {
                let at = format!("{at} parameter {i}");
                shape(param, &at, &["name", "meaning"], &[], &mut problems);
                params.push(filled(param, "name", &at, &mut problems).to_owned());
                filled(param, "meaning", &at, &mut problems);
            }
            if params != sig.params {
                problems.push(format!(
                    "{at}: parameters {params:?}, the declaration's are {:?}",
                    sig.params
                ));
            }
            let returns = &entry["returns"];
            shape(
                returns,
                &format!("{at} returns"),
                &["type", "meaning"],
                &[],
                &mut problems,
            );
            filled(returns, "meaning", &at, &mut problems);
            if returns["type"].as_str() != Some(sig.result.as_str()) {
                problems.push(format!(
                    "{at}: returns {}, the declaration's result is {}",
                    returns["type"], sig.result
                ));
            }
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
            let failure_kind = sig
                .fails
                .as_deref()
                .and_then(|kind| listed("prelude", kind));
            let declared: Vec<Vec<String>> = failure_kind
                .map(|kind| {
                    kind.variants
                        .iter()
                        .map(|v| {
                            variants_of(&v.declaration)
                                .into_iter()
                                .map(|(n, _)| n)
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_else(|| vec![Vec::new()]);
            if declared.iter().any(|variants| *variants != fails) {
                problems.push(format!(
                    "{at}: fails with {fails:?}, the declaration's failure kind has {declared:?}"
                ));
            }
        } else if item.kind == "kind" {
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
            for declaration in &declarations {
                let declared = variants_of(declaration);
                if variants != declared {
                    problems.push(format!(
                        "{at}: variants {variants:?}, the declaration's are {declared:?}"
                    ));
                }
            }
        } else {
            only(&[], &mut problems);
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
        // The cost: the cell of the item's row in the table of an accepted ADR or of a spec
        // section, verbatim, with every term it uses defined.
        if entry.get("cost").is_none() {
            for source in &cost_sources {
                if cited(source).is_some_and(|text| !cost_cells(&text, name).is_empty()) {
                    problems.push(format!("{at}: {source} states its cost, and it has none"));
                }
            }
        }
        if let Some(cost) = entry.get("cost") {
            let at = format!("{at} cost");
            shape(cost, &at, &["time", "source", "terms"], &[], &mut problems);
            let time = filled(cost, "time", &at, &mut problems);
            let source = filled(cost, "source", &at, &mut problems);
            match cited(source) {
                None => problems.push(format!(
                    "{at}: cites `{source}`, which is neither an accepted ADR nor a spec section"
                )),
                Some(cited) => {
                    let mut cells = cost_cells(&cited, name);
                    cells.dedup();
                    if cells != [time] {
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
            for inside in big_o(time) {
                for word in inside.split(|c: char| !c.is_ascii_alphabetic()) {
                    if !word.is_empty() && !defined.contains(&word) {
                        problems.push(format!("{at}: `{word}` in `{time}` is not defined"));
                    }
                }
            }
        }
        // The example: code that `main` reaches calls the item, and a kind's example names the
        // kind and each of its variants. `each_example_of_the_reference_...` also requires that
        // it stops checking once the item is renamed.
        let example = filled(entry, "example", &at, &mut problems);
        let kind = item.kind == "kind";
        let (call, method) = naming(name, kind, &builtin_names);
        let mut names = vec![name.to_owned()];
        if kind {
            names.extend(
                declarations
                    .iter()
                    .flat_map(|d| variants_of(d))
                    .map(|(n, _)| n),
            );
        }
        let files = sources(&root().join(example));
        if files.is_empty() {
            problems.push(format!("{at}: its example {example} has no source"));
        }
        for wanted in &names {
            let (call, method) = if wanted == name {
                (call, method)
            } else {
                (false, false)
            };
            let used = files.iter().any(|(_, src)| {
                let code = code_bytes(src);
                reached_from_main(src, &code).iter().any(|range| {
                    !occurrences(
                        &src[range.clone()],
                        &code[range.clone()],
                        wanted,
                        call,
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

/// `una ARGS` in `dir`.
fn una(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// `src` with every place where it names `name` (see `naming`) renamed to a name that does not
/// exist, and how many there were.
fn renamed(src: &str, name: &str, call: bool, method: bool) -> (String, usize) {
    let code = code_bytes(src);
    let places = occurrences(src, &code, name, call, method);
    let mut out = String::new();
    let mut last = 0;
    for at in &places {
        out.push_str(&src[last..*at]);
        out.push_str(name);
        out.push_str(if call { "_absent" } else { "Absent" });
        last = at + name.len();
    }
    out.push_str(&src[last..]);
    (out, places.len())
}

#[test]
fn each_example_of_the_reference_checks_prints_its_output_and_needs_its_item() {
    let text = fs::read_to_string(root().join("docs/std/reference.json")).unwrap();
    let reference: serde_json::Value = serde_json::from_str(&text).unwrap();
    let builtin_names: Vec<String> = std_builtin_items()
        .into_iter()
        .map(|item| item.name)
        .collect();
    let items = std_items();
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
        // comment, a text or a built-in method of the same name.
        let kind = items
            .iter()
            .any(|item| item.module == module && item.name == name && item.kind == "kind");
        let (call, method) = naming(name, kind, &builtin_names);
        let copy = scratch.join("std-examples-renamed").join(module).join(name);
        let _ = fs::remove_dir_all(&copy);
        copy_tree(&source, &copy);
        let mut places = 0;
        for (path, src) in sources(&copy) {
            let (src, count) = renamed(&src, name, call, method);
            fs::write(path, src).unwrap();
            places += count;
        }
        let checked = una(&copy, &["--color", "never", "check", "."]);
        if places == 0 || checked.status.success() {
            problems.push(format!(
                "{example}: still checks with `{name}` renamed in {places} places, so it does not need it"
            ));
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
