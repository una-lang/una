//! The standard modules hold no task construct (#1148, ADR 0116 P9, until #1166).
//!
//! The residue label `L_res` counts the `tasks` statements and races of the package's own
//! modules precisely. Those of the standard modules are not recorded in a package's findings, so
//! the checker counts a reachable standard function that holds one as `⊤`, failing closed. This
//! test keeps that rule from ever applying unseen: a standard module that gains a `tasks`
//! statement or a `first of` fails it, and the change then needs #1166, which makes such
//! constructs feed `L_res` precisely.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::path::Path;

#[test]
fn no_standard_module_holds_a_tasks_statement_or_a_first_of() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/std");
    let mut scanned = 0;
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("una") {
            continue;
        }
        scanned += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            // Code only: a comment may name the constructs.
            let code = line.split("--").next().unwrap_or("");
            let words: Vec<&str> = code.split_whitespace().collect();
            let tasks = words.first() == Some(&"tasks");
            let first_of = words.windows(2).any(|pair| pair == ["first", "of"]);
            if tasks || first_of {
                found.push(format!("{}:{}: {line}", path.display(), number + 1));
            }
        }
    }
    assert!(
        scanned >= 5,
        "only {scanned} standard modules found in {}",
        dir.display()
    );
    assert!(
        found.is_empty(),
        "a standard module holds a task construct, which L_res counts only as top until #1166:\n{}",
        found.join("\n")
    );
}
