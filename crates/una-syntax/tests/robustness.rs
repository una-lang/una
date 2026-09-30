//! Robustness of the lexer and parser: no input crashes them (M1 acceptance, ADR 0017
//! decision 7, and the nesting limit of ADR 0019).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_syntax::ast::{Body, DeclKind};
use una_syntax::{Interner, MAX_NESTING, SourceDb, lex, parse};

/// The diagnostics of `text` as "code line:col message", and the number of declarations.
///
/// Parsing at the nesting limit needs more stack than a test thread has in unoptimized builds,
/// so it runs on a thread with the stack `una check` gives it.
fn check(text: &str) -> (Vec<String>, usize) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn({
            let text = text.to_owned();
            move || check_here(&text)
        })
        .unwrap()
        .join()
        .unwrap()
}

fn check_here(text: &str) -> (Vec<String>, usize) {
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex(db.file(id), id, &mut interner);
    let parsed = parse(&lexed, id, &interner);
    let errors = lexed
        .errors
        .iter()
        .chain(&parsed.errors)
        .map(|error| {
            let diagnostic = error.to_diagnostic(&db);
            format!(
                "{} {}:{} {}",
                diagnostic.code,
                diagnostic.primary.start.line,
                diagnostic.primary.start.col,
                diagnostic.message
            )
        })
        .collect();
    (errors, parsed.file.decls.len())
}

fn nesting_error(line: usize, col: usize) -> String {
    format!(
        "U0101 {line}:{col} nesting deeper than {MAX_NESTING} levels, counting each chained operator; split the code with `let` or a helper function"
    )
}

// Deep nesting and long operator chains are reported, not a stack overflow.
#[test]
fn deep_nesting_is_one_error_and_the_next_declaration_still_parses() {
    let depth = 20_000;
    let expressions = [
        format!("{}1{}", "(".repeat(depth), ")".repeat(depth)),
        format!("{}true", "not ".repeat(depth)),
        format!("{}1", "- ".repeat(depth)),
        format!("true{}", " implies true".repeat(depth)),
        format!("1{}", " + 1".repeat(depth)),
        format!("1{}", " * 1".repeat(depth)),
        format!("true{}", " and true".repeat(depth)),
        format!("true{}", " or true".repeat(depth)),
        format!("x{}", ".f".repeat(depth)),
        format!("f{}", "()".repeat(depth)),
        format!("[{}]", "[".repeat(depth) + &"]".repeat(depth)),
        format!(
            "{}0{}",
            "if true then ".repeat(depth),
            " else 0".repeat(depth)
        ),
    ];
    for expression in expressions {
        let text = format!("module t\n\nf() -> Int = {expression}\n\ng() -> Int = 1\n");
        let (errors, decls) = check(&text);
        let preview: String = expression.chars().take(20).collect();
        let first: Vec<&String> = errors.iter().take(3).collect();
        assert!(
            errors.len() == 1 && errors[0].contains("nesting deeper"),
            "{preview}: {} errors, first {first:?}",
            errors.len()
        );
        assert_eq!(decls, 1, "{preview}: the next declaration parses");
    }
}

#[test]
fn deep_types_and_blocks_are_one_error() {
    let depth = 5_000;
    let ty = format!("{}Int{}", "List[".repeat(depth), "]".repeat(depth));
    let (errors, decls) = check(&format!(
        "module t\n\nf(x: {ty}) -> Int = 1\n\ng() -> Int = 1\n"
    ));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("nesting deeper"), "{errors:?}");
    assert_eq!(decls, 1);

    let mut blocks = String::from("module t\n\nf(x: Int) -> Int\n=\n");
    for level in 1..=depth {
        blocks.push_str(&format!("{}if x > 0\n", "  ".repeat(level)));
    }
    let (errors, decls) = check(&format!("{blocks}\ng() -> Int = 1\n"));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("nesting deeper"), "{errors:?}");
    assert_eq!(decls, 1);

    let mut chain = String::from("module t\n\nf(x: Int) -> Int\n=\n  if x == 0\n    0\n");
    for value in 1..=depth {
        chain.push_str(&format!("  else if x == {value}\n    {value}\n"));
    }
    chain.push_str("  else\n    -1\n\ng() -> Int = 1\n");
    let (errors, decls) = check(&chain);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("nesting deeper"), "{errors:?}");
    assert_eq!(decls, 1);
}

#[test]
fn nesting_up_to_the_limit_parses() {
    // The body is one level and each parenthesis one more.
    let parens = MAX_NESTING - 1;
    let text = format!(
        "module t\n\nf() -> Int = {}1{}\n",
        "(".repeat(parens),
        ")".repeat(parens)
    );
    assert_eq!(check(&text), (vec![], 1));
    let text = format!(
        "module t\n\nf() -> Int = {}1{}\n",
        "(".repeat(parens + 1),
        ")".repeat(parens + 1)
    );
    let column = "f() -> Int = ".len() + parens + 2;
    assert_eq!(check(&text), (vec![nesting_error(3, column)], 0));
    // Each operator of a chain is one level.
    let text = format!(
        "module t\n\nf() -> Int = 1{}\n",
        " + 1".repeat(MAX_NESTING - 1)
    );
    assert_eq!(check(&text), (vec![], 1));
}

#[test]
fn text_nested_in_interpolations_is_bounded() {
    // A literal nested in an interpolation is U0006 and has no interpolation of
    // its own, so `{` inside it is text.
    let depth = 5_000;
    let text = format!(
        "module t\n\nf() -> Text = \"{}\"{}\n",
        "{\"".repeat(depth),
        "}\"".repeat(depth)
    );
    let (errors, _) = check(&text);
    assert!(
        errors.iter().any(|error| error.starts_with("U0006")),
        "{errors:?}"
    );
}

/// Every `.una` file under `examples/`, with its text.
fn examples() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "una") {
                files.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            (path.display().to_string(), text)
        })
        .collect()
}

/// The line boundaries of `text` where a prefix must yield a diagnostic: right after the first
/// line of each multi-line declaration, and right after the `=` line of each long-form
/// function (ADR 0017 decision 7).
fn boundaries_that_must_fail(text: &str) -> Vec<usize> {
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex(db.file(id), id, &mut interner);
    let parsed = parse(&lexed, id, &interner);
    assert!(parsed.errors.is_empty() && lexed.errors.is_empty());
    let mut boundaries = Vec::new();
    for decl in &parsed.file.decls {
        let source = &text[decl.span.start..decl.span.end];
        if let Some(first_line) = source.find('\n') {
            boundaries.push(decl.span.start + first_line + 1);
        }
        if let DeclKind::Function(function) = &decl.kind
            && matches!(function.body, Body::Block(_))
        {
            let equals = source.find("\n=\n").unwrap();
            boundaries.push(decl.span.start + equals + 3);
        }
    }
    boundaries
}

#[test]
fn every_prefix_of_every_example_is_parsed_safely() {
    // ADR 0017 decision 7: for every example and every line boundary, parsing the prefix never
    // panics and every reported position lies inside the prefix; the prefixes of
    // `boundaries_that_must_fail` yield at least one diagnostic.
    let mut prefixes = 0;
    let mut required = 0;
    for (name, text) in examples() {
        let must_fail = boundaries_that_must_fail(&text);
        assert!(!must_fail.is_empty(), "{name}");
        let mut boundaries: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        if !text.ends_with('\n') {
            boundaries.push(text.len());
        }
        for end in boundaries {
            let prefix = &text[..end];
            let mut db = SourceDb::new();
            let id = db.add("t.una", prefix.as_bytes());
            let mut interner = Interner::new();
            let lexed = lex(db.file(id), id, &mut interner);
            let parsed = parse(&lexed, id, &interner);
            let errors: Vec<_> = lexed.errors.iter().chain(&parsed.errors).collect();
            for error in &errors {
                let mut spans = vec![error.span];
                spans.extend(
                    error
                        .repairs
                        .iter()
                        .filter_map(|repair| repair.edit.as_ref().map(|(span, _)| *span)),
                );
                for span in spans {
                    assert!(
                        span.start <= span.end && span.end <= prefix.len(),
                        "{name}, prefix of {end} bytes: {error:?}"
                    );
                }
            }
            if must_fail.contains(&end) {
                assert!(
                    !errors.is_empty(),
                    "{name}: the prefix ending at byte {end} parses without a diagnostic"
                );
                required += 1;
            }
            prefixes += 1;
        }
    }
    assert!(
        prefixes > 100 && required > 20,
        "{prefixes} prefixes, {required} required"
    );
}
