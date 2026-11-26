//! The edition ledger (ADR 0121 rule 9): its shape, its words against the lexer's tables, and
//! the guide's list of the constructs of the preview editions against it.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;

use una_diag::Code;
use una_project::Edition;
use una_project::ledger::{Construct, Entry, LEDGER, in_edition};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn each_entry_is_at_the_index_of_its_construct() {
    // `Construct::entry` is an index into the table, so the order of the two must agree.
    for (index, entry) in LEDGER.iter().enumerate() {
        assert_eq!(entry.id as usize, index, "{}", entry.name);
        assert!(std::ptr::eq(entry.id.entry(), entry), "{}", entry.name);
    }
    assert_eq!(Construct::ConstVar as usize, LEDGER.len() - 1);
}

#[test]
fn the_ledger_is_ordered_and_well_formed() {
    let mut names = BTreeSet::new();
    let mut clashes = BTreeSet::new();
    let changelog = std::fs::read_to_string(repository().join("CHANGELOG.md")).unwrap();
    for pair in LEDGER.windows(2) {
        assert!(
            (pair[0].edition, pair[0].record) <= (pair[1].edition, pair[1].record),
            "{} before {}: the ledger is ordered by edition, then record",
            pair[0].name,
            pair[1].name
        );
    }
    for entry in &LEDGER {
        // Only a construct of a preview edition after `0.3` has an entry (ADR 0121 rule 13).
        assert!(
            entry.edition.is_preview() && entry.edition >= Edition::V0_4,
            "{}",
            entry.name
        );
        assert!(names.insert(entry.name), "{} twice", entry.name);
        let number = entry.record.strip_prefix("ADR ").unwrap();
        assert!(
            number.len() == 4 && number.chars().all(|c| c.is_ascii_digit()),
            "{}: {}",
            entry.name,
            entry.record
        );
        assert!(
            repository().join("docs/adr").read_dir().unwrap().any(|f| f
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(number)),
            "{}: no record {number}",
            entry.name
        );
        assert!(entry.spec.starts_with("spec/"), "{}", entry.name);
        for clash in entry.clashes {
            assert!(
                clash.id.starts_with(&format!("{number}-")),
                "{}: the clash id {} names another record",
                entry.name,
                clash.id
            );
            assert!(clashes.insert(clash.id), "{} twice", clash.id);
        }
        for code in entry.codes {
            assert!(Code::from_str(code).is_ok(), "{}: {code}", entry.name);
        }
        for release in std::iter::once(&entry.landed).chain(entry.changed) {
            // The release that lands `const` and `var` is the one U7007 names (ADR 0137 rule
            // 1); until it is cut, its entries are under the changelog's `Unreleased`.
            let pending = *release == una_syntax::keywords::LET_CHANGED_IN
                && changelog.contains("\n## Unreleased\n");
            assert!(
                pending || changelog.contains(&format!("\n## {release}\n")),
                "{}: {release} is no release of the changelog",
                entry.name
            );
        }
    }
}

#[test]
fn const_and_var_landed_in_the_release_that_u7007_names() {
    // ADR 0137 rules 1 and 12: the entry names `const` and `var` as reserved keywords, `let` as
    // the changed form, and the release of U7007's message.
    let entry = Construct::ConstVar.entry();
    assert_eq!(entry.edition, Edition::V0_5);
    assert_eq!(entry.reserved, &["const", "var"]);
    assert_eq!(entry.landed, una_syntax::keywords::LET_CHANGED_IN);
    assert!(entry.clashes.iter().any(|clash| clash.id == "0137-let"
        && clash.program.contains("`let`")
        && clash.diagnostic.starts_with("U7007")));
    for word in entry.reserved {
        assert_eq!(
            una_syntax::Keyword::from_word_in(word, una_syntax::KeywordSet::V0_5)
                .map(una_syntax::Keyword::as_str),
            Some(*word)
        );
    }
}

#[test]
fn the_reserved_words_of_edition_0_4_are_those_of_the_lexer() {
    // ADR 0121 rule 11: a keyword the lexer reserves for edition 0.4 has its ledger entry. The
    // rows of `async` and `await` change only their message, so they are no construct's.
    let lexer: BTreeSet<&str> = una_syntax::keywords::RESERVED_IN_0_4
        .iter()
        .filter(|(_, message)| message.starts_with("reserved for edition 0.4"))
        .map(|(word, _)| *word)
        .collect();
    let ledger: BTreeSet<&str> = in_edition(Edition::V0_4)
        .flat_map(|entry| entry.reserved.iter().copied())
        .collect();
    assert_eq!(lexer, ledger);
}

#[test]
fn a_package_sees_the_constructs_of_its_preview_and_those_before() {
    for edition in [Edition::V0_1, Edition::V0_2, Edition::V0_3] {
        assert_eq!(in_edition(edition).count(), 0, "{edition:?}");
    }
    let four: Vec<&str> = in_edition(Edition::V0_4).map(|e| e.name).collect();
    assert!(
        four.contains(&"for ever") && !four.contains(&"sum"),
        "{four:?}"
    );
    assert_eq!(in_edition(Edition::V0_5).count(), LEDGER.len());
    assert_eq!(
        Construct::Sum.entry().unsettled[0].name,
        "the accumulation order of 0120's kernels (L, KB)"
    );
}

/// A row of the guide's list: edition, construct, record and unsettled points.
type Row = (String, String, String, Vec<String>);

fn row(entry: &Entry) -> Row {
    (
        entry.edition.name().to_owned(),
        entry.name.to_owned(),
        entry.record.to_owned(),
        entry.unsettled.iter().map(|u| u.name.to_owned()).collect(),
    )
}

#[test]
fn the_guide_lists_the_constructs_of_the_ledger() {
    // ADR 0121 rule 9: "A test checks that the guide's list and the ledger agree."
    let page =
        std::fs::read_to_string(repository().join("docs/guide/configure/package/edition.md"))
            .unwrap();
    let edition = page
        .split("\n## Edition\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the guide's edition page has an Edition part");
    let header = "| Edition | Construct | Record | Unsettled points |";
    let table = edition
        .split_once(header)
        .map(|(_, rest)| rest)
        .expect("the Edition part has the list of the constructs");
    let mut guide: Vec<Row> = Vec::new();
    for line in table.lines().skip(2) {
        if !line.starts_with('|') {
            break;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(cells.len(), 4, "{line}");
        let unquote = |cell: &str| cell.trim_matches('`').to_owned();
        let unsettled = if cells[3] == "none" {
            Vec::new()
        } else {
            cells[3].split("; ").map(str::to_owned).collect()
        };
        guide.push((
            unquote(cells[0]),
            unquote(cells[1]),
            cells[2].to_owned(),
            unsettled,
        ));
    }
    let ledger: Vec<Row> = LEDGER.iter().map(row).collect();
    assert_eq!(guide, ledger, "the guide's list and the ledger disagree");
}
