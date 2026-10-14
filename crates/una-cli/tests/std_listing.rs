//! The standard library reference listing (#648), `docs/std/listing.json`:
//!
//! - it is the listing that the standard modules give now (`UNA_BLESS=1` rewrites it);
//! - every item has doc comments in every edition, and they cite spec sections that exist;
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

use una_cli::{Item, Tier, check, std_items, std_listing};
use una_project::Edition;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_listing_is_current() {
    let path = root().join("docs/std/listing.json");
    let listing = std_listing();
    // A test, not a library crate: `UNA_BLESS` asks it to rewrite the committed listing.
    #[allow(clippy::disallowed_methods)]
    let bless = std::env::var_os("UNA_BLESS").is_some();
    if bless {
        fs::write(&path, &listing).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == listing,
        "docs/std/listing.json is not the listing of the standard modules; run `UNA_BLESS=1 cargo test -p una-cli --test std_listing` and review the diff"
    );
}

#[test]
fn the_listing_is_the_same_on_every_run() {
    assert_eq!(std_listing(), std_listing());
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
    let items = std_items();
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
